//! 退场。GPUI 只画当前的状态：换了页，下一帧旧的那页就没了。这里趁屏幕上还是旧的那一帧，
//! 用原生快照（`snapshotViewAfterScreenUpdates:NO`）把它留下来，盖在 GPUI 画面之上、玻璃之下，
//! 交给 Core Animation 送走；GPUI 那边照常画新的状态（返回时下面那一页带视差地回来）。
//!
//! 快照只拍 Metal 视图，所以玻璃和系统菜单的按钮不放在它里面，而是放在它旁边的一层（`overlay`）：
//! 否则快照里会带着一份标签栏，滑走时从真的那份底下透出来。
use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};

use block2::RcBlock;
use gpui::*;
use objc2::msg_send;
use objc2::rc::Retained;
use objc2::encode::{Encoding, RefEncode};
use objc2::runtime::{AnyClass, AnyObject, ClassBuilder, MessageReceiver, Sel};
use objc2::sel;
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

/// 返回时旧页滑走、下面那页回来用的时长；GPUI 那边的视差用同一个时长和曲线（`ease_out_cubic`）。
pub const SLIDE: Duration = Duration::from_millis(380);
/// 下面那一页从左边多远回来（屏宽的比例），和系统导航的视差差不多。
pub const PARALLAX: f32 = 0.3;
/// 和 `ease_out_cubic` 同一条曲线的贝塞尔控制点。
const OUT_CUBIC: [f32; 4] = [0.33, 1., 0.68, 1.];
pub fn ease_out_cubic(t: f32) -> f32 { 1. - (1. - t).powi(3) }
/// 还剩 `span`（屏宽的比例）要滑时用多久：按比例缩短，但不短于 0.12s。
pub fn slide_time(span: f32) -> Duration { Duration::from_secs_f32((SLIDE.as_secs_f32() * span).max(0.12)) }

thread_local! { static METAL: Cell<*mut AnyObject> = const { Cell::new(std::ptr::null_mut()) }; }
thread_local! { static OVERLAY: Cell<*mut AnyObject> = const { Cell::new(std::ptr::null_mut()) }; }

/// 每帧开头记下 GPUI 的视图：改状态的地方（点击、手势）拿不到 `Window`，也要能拍快照。
pub fn attach(window: &Window) {
    let Ok(handle) = HasWindowHandle::window_handle(window) else { return; };
    let RawWindowHandle::UiKit(handle) = handle.as_raw() else { return; };
    METAL.with(|m| m.set(handle.ui_view.as_ptr() as *mut AnyObject));
}
fn metal() -> Option<*mut AnyObject> { Some(METAL.with(|m| m.get())).filter(|p| !p.is_null()) }
fn superview() -> Option<*mut AnyObject> {
    let m = metal()?;
    let s: *mut AnyObject = unsafe { msg_send![m, superview] };
    (!s.is_null()).then_some(s)
}

/// 玻璃和系统菜单的按钮放的那一层：Metal 视图的兄弟，一样大，盖在它上面。点到空处时它自己不接
/// （`hitTest` 返回空），触摸照旧落到 GPUI；里面的按钮照常接。Metal 视图还没进窗口时退回到它自己里面。
pub fn overlay(metal: *mut AnyObject) -> *mut AnyObject {
    let cur = OVERLAY.with(|o| o.get());
    if !cur.is_null() { return cur; }
    unsafe {
        let sup: *mut AnyObject = msg_send![metal, superview];
        if sup.is_null() { return metal; }
        let view: *mut AnyObject = msg_send![pass_class(), alloc];
        let frame: CGRect = msg_send![metal, frame];
        let view: *mut AnyObject = msg_send![view, initWithFrame: frame];
        // 2 | 16：宽高跟着父视图。
        let _: () = msg_send![view, setAutoresizingMask: 18usize];
        let _: () = msg_send![sup, insertSubview: view, aboveSubview: metal];
        OVERLAY.with(|o| o.set(view));
        view
    }
}
fn pass_class() -> &'static AnyClass {
    static CLASS: std::sync::OnceLock<&'static AnyClass> = std::sync::OnceLock::new();
    CLASS.get_or_init(|| {
        extern "C-unwind" fn hit(this: &AnyObject, _: Sel, p: CGPoint, ev: *mut AnyObject) -> *mut AnyObject {
            let hit: *mut AnyObject = unsafe { msg_send![super(this, AnyClass::get(c"UIView").unwrap()), hitTest: p, withEvent: ev] };
            if std::ptr::eq(hit, this) { std::ptr::null_mut() } else { hit }
        }
        let mut b = ClassBuilder::new(c"QuietPassThroughView", AnyClass::get(c"UIView").unwrap()).expect("register QuietPassThroughView");
        unsafe { b.add_method(sel!(hitTest:withEvent:), hit as extern "C-unwind" fn(_, _, _, _) -> _); }
        b.register()
    })
}

// ---- 换页

thread_local! { static PAGE_SHOT: RefCell<Option<Retained<AnyObject>>> = const { RefCell::new(None) }; }
thread_local! { static DIM: RefCell<Option<Retained<AnyObject>>> = const { RefCell::new(None) }; }
thread_local! { static SLIDING: Cell<Option<Instant>> = const { Cell::new(None) }; }

/// 要换页了（状态还没变，屏幕上还是这一页）：把这一页拍下来盖在原处。下一帧 `ios::page` 看清了方向：
/// 是返回就送走（`slide_out`），不是就拿掉（`drop_page`）。没换成页（比如被「先保存」拦下了）也在那一帧拿掉。
pub fn capture_page() {
    if !super::is_compact() { return; }
    let (Some(metal), Some(sup)) = (metal(), superview()) else { return; };
    drop_page();
    unsafe {
        let snap: Option<Retained<AnyObject>> = msg_send![metal, snapshotViewAfterScreenUpdates: false];
        let Some(snap) = snap else { return; };
        let _: () = msg_send![&*snap, setUserInteractionEnabled: false];
        let frame: CGRect = msg_send![metal, frame];
        let _: () = msg_send![&*snap, setFrame: frame];
        let _: () = msg_send![sup, insertSubview: &*snap, aboveSubview: metal];
        PAGE_SHOT.with(|s| *s.borrow_mut() = Some(snap));
    }
}
/// 有一张等着 `ios::page` 处理的快照。
pub fn page_pending() -> bool { PAGE_SHOT.with(|s| s.borrow().is_some()) }
/// 不是返回：快照和下面这一帧要换上的东西无关，拿掉。
pub fn drop_page() {
    if let Some(v) = PAGE_SHOT.with(|s| s.borrow_mut().take()) { unsafe { let _: () = msg_send![&*v, removeFromSuperview]; } }
    if let Some(v) = DIM.with(|s| s.borrow_mut().take()) { unsafe { let _: () = msg_send![&*v, removeFromSuperview]; } }
}
/// 旧页正在滑走（或跟着手指）：只属于新页的玻璃等它走完再露面，离开的那一页的玻璃淡出。
pub fn sliding() -> bool { PAGE_SHOT.with(|s| s.borrow().is_some()) || SLIDING.with(|s| s.get()).is_some_and(|t| Instant::now() < t) }

/// 玻璃和系统菜单按钮的显隐都经过这里。旧页正在滑走时：要露面的推迟（返回假，下一帧再试），
/// 要藏起的淡出；别的时候照常立刻显隐。
pub fn show(view: *mut AnyObject, on: bool) -> bool {
    unsafe {
        let layer: *mut AnyObject = msg_send![view, layer];
        if on {
            if sliding() { return false; }
            let _: () = msg_send![layer, removeAnimationForKey: ns_string("vanish")];
            let _: () = msg_send![view, setHidden: false];
        } else if sliding() {
            let anim = basic("opacity", 1., 0., 0.15, [0.42, 0., 1., 1.]);
            let _: () = msg_send![anim, setFillMode: ns_string("forwards")];
            let _: () = msg_send![anim, setRemovedOnCompletion: false];
            // 淡完还没被叫回来才真的藏起；中途露面会先拿掉这段动画，这里就什么都不做。
            let done = RcBlock::new(move || {
                let layer: *mut AnyObject = msg_send![view, layer];
                let anim: *mut AnyObject = msg_send![layer, animationForKey: ns_string("vanish")];
                if !anim.is_null() { let _: () = msg_send![view, setHidden: true]; let _: () = msg_send![layer, removeAnimationForKey: ns_string("vanish")]; }
            });
            transaction(&done, || { let _: () = msg_send![layer, addAnimation: anim, forKey: ns_string("vanish")]; });
        } else {
            let _: () = msg_send![view, setHidden: true];
        }
    }
    true
}

fn width() -> f64 { metal().map_or(0., |m| unsafe { let f: CGRect = msg_send![m, frame]; f.size.width }) }

/// 快照的左边一道阴影、它和下面那页之间一层暗（跟手返回时第一次动之前加上）。
fn dress() {
    PAGE_SHOT.with(|s| unsafe {
        let Some(snap) = s.borrow().clone() else { return; };
        let layer: *mut AnyObject = msg_send![&*snap, layer];
        let _: () = msg_send![layer, setShadowOpacity: 0.16f32];
        let _: () = msg_send![layer, setShadowRadius: 10f64];
        let _: () = msg_send![layer, setShadowOffset: CGSize::new(-2., 0.)];
        let bounds: CGRect = msg_send![&*snap, bounds];
        let path: *mut AnyObject = msg_send![AnyClass::get(c"UIBezierPath").unwrap(), bezierPathWithRect: bounds];
        let cg: *mut CGPath = msg_send![path, CGPath];
        let _: () = msg_send![layer, setShadowPath: cg];
        if DIM.with(|d| d.borrow().is_some()) { return; }
        let (Some(metal), Some(sup)) = (metal(), superview()) else { return; };
        let dim: Retained<AnyObject> = msg_send![AnyClass::get(c"UIView").unwrap(), new];
        let _: () = msg_send![&*dim, setUserInteractionEnabled: false];
        let _: () = msg_send![&*dim, setFrame: bounds];
        let black: *mut AnyObject = msg_send![AnyClass::get(c"UIColor").unwrap(), blackColor];
        let _: () = msg_send![&*dim, setBackgroundColor: black];
        let _: () = msg_send![&*dim, setAlpha: 0f64];
        let _: () = msg_send![sup, insertSubview: &*dim, aboveSubview: metal];
        DIM.with(|d| *d.borrow_mut() = Some(dim));
    });
}
/// 下面那页上的暗：完全盖着时 0.12，旧页走完时没有。
fn dim_at(progress: f64) -> f64 { 0.12 * (1. - progress) }

/// 跟手返回：快照跟着手指，`progress` 是已经滑过的屏宽比例。
pub fn follow(progress: f32) {
    dress();
    let x = width() * f64::from(progress);
    PAGE_SHOT.with(|s| if let Some(snap) = s.borrow().as_ref() { unsafe { set_value(msg_send![&**snap, layer], "transform.translation.x", x); } });
    DIM.with(|d| if let Some(dim) = d.borrow().as_ref() { unsafe { let _: () = msg_send![&**dim, setAlpha: dim_at(f64::from(progress))]; } });
}

/// 返回：快照从 `from`（已经滑过的比例）往右滑出屏幕。没有快照时返回假，`ios::page` 用原来的滑入。
pub fn slide_out(from: f32) -> bool {
    if !page_pending() { return false; }
    dress();
    let Some(snap) = PAGE_SHOT.with(|s| s.borrow_mut().take()) else { return false; };
    let dim = DIM.with(|d| d.borrow_mut().take());
    let w = width();
    let dur = slide_time(1. - from).as_secs_f64();
    SLIDING.with(|s| s.set(Some(Instant::now() + Duration::from_secs_f64(dur))));
    unsafe {
        let (gone, dim_gone) = (snap.clone(), dim.clone());
        let done = RcBlock::new(move || {
            let _: () = msg_send![&*gone, removeFromSuperview];
            if let Some(dim) = &dim_gone { let _: () = msg_send![&**dim, removeFromSuperview]; }
        });
        transaction(&done, || {
            animate(msg_send![&*snap, layer], "transform.translation.x", w * f64::from(from), w, dur, OUT_CUBIC);
            if let Some(dim) = &dim { animate(msg_send![&**dim, layer], "opacity", dim_at(f64::from(from)), 0., dur, OUT_CUBIC); let _: () = msg_send![&**dim, setAlpha: 0f64]; }
        });
    }
    true
}

/// 跟手返回松手时没过半：快照滑回原处。滑完把 `restore` 交给 GPUI（`ios::menu_action`），
/// 它把原来那页恢复好；快照再稍等一下淡掉，免得新的一帧还没上屏就露出下面那页。
pub fn slide_back(from: f32, restore: &'static str) {
    let w = width();
    let dur = slide_time(from).as_secs_f64();
    SLIDING.with(|s| s.set(Some(Instant::now() + Duration::from_secs_f64(dur))));
    PAGE_SHOT.with(|s| unsafe {
        let Some(snap) = s.borrow().clone() else { return; };
        let done = RcBlock::new(move || super::menu::chosen(restore));
        transaction(&done, || {
            animate(msg_send![&*snap, layer], "transform.translation.x", w * f64::from(from), 0., dur, OUT_CUBIC);
            DIM.with(|d| if let Some(dim) = d.borrow().as_ref() { animate(msg_send![&**dim, layer], "opacity", dim_at(f64::from(from)), dim_at(0.), dur, OUT_CUBIC); let _: () = msg_send![&**dim, setAlpha: dim_at(0.)]; });
        });
    });
}
/// 原来那页已经恢复（这一帧会画出来）：快照稍后淡掉、拿走。
pub fn fade_page() {
    let Some(snap) = PAGE_SHOT.with(|s| s.borrow_mut().take()) else { return; };
    let dim = DIM.with(|d| d.borrow_mut().take());
    unsafe {
        let (gone, dim_gone) = (snap.clone(), dim.clone());
        let done = RcBlock::new(move || {
            let _: () = msg_send![&*gone, removeFromSuperview];
            if let Some(dim) = &dim_gone { let _: () = msg_send![&**dim, removeFromSuperview]; }
        });
        transaction(&done, || {
            let layer: *mut AnyObject = msg_send![&*snap, layer];
            let anim = basic("opacity", 1., 0., 0.1, [0.42, 0., 1., 1.]);
            // 晚 60ms 再开始：新的一帧先上屏。
            let now: f64 = msg_send![layer, convertTime: ca_now(), fromLayer: std::ptr::null_mut::<AnyObject>()];
            let _: () = msg_send![anim, setBeginTime: now + 0.06];
            let _: () = msg_send![anim, setFillMode: ns_string("backwards")];
            let _: () = msg_send![layer, addAnimation: anim, forKey: ns_string("fade")];
            let _: () = msg_send![layer, setOpacity: 0f32];
        });
    }
}

// ---- Core Animation

/// `CGPathRef`：只用来把 `UIBezierPath` 的路径交给图层，编码要和系统的对上（调试构建会核对）。
#[repr(C)]
struct CGPath { _private: [u8; 0] }
unsafe impl RefEncode for CGPath { const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("CGPath", &[])); }

unsafe fn ns_string(s: &str) -> *mut AnyObject {
    let c = std::ffi::CString::new(s).unwrap_or_default();
    unsafe { msg_send![AnyClass::get(c"NSString").unwrap(), stringWithUTF8String: c.as_ptr()] }
}
unsafe fn number(v: f64) -> *mut AnyObject { unsafe { msg_send![AnyClass::get(c"NSNumber").unwrap(), numberWithDouble: v] } }
fn ca_now() -> f64 { unsafe extern "C" { fn CACurrentMediaTime() -> f64; } unsafe { CACurrentMediaTime() } }

/// 不带隐式动画地设一个图层的值。
unsafe fn set_value(layer: *mut AnyObject, path: &str, v: f64) {
    unsafe {
        let ca = AnyClass::get(c"CATransaction").unwrap();
        let _: () = msg_send![ca, begin];
        let _: () = msg_send![ca, setDisableActions: true];
        let _: () = msg_send![layer, setValue: number(v), forKeyPath: ns_string(path)];
        let _: () = msg_send![ca, commit];
    }
}
unsafe fn basic(path: &str, from: f64, to: f64, dur: f64, curve: [f32; 4]) -> *mut AnyObject {
    unsafe {
        let anim: *mut AnyObject = msg_send![AnyClass::get(c"CABasicAnimation").unwrap(), animationWithKeyPath: ns_string(path)];
        let _: () = msg_send![anim, setFromValue: number(from)];
        let _: () = msg_send![anim, setToValue: number(to)];
        let _: () = msg_send![anim, setDuration: dur];
        let timing: *mut AnyObject = MessageReceiver::send_message(AnyClass::get(c"CAMediaTimingFunction").unwrap(), sel!(functionWithControlPoints::::), (curve[0], curve[1], curve[2], curve[3]));
        let _: () = msg_send![anim, setTimingFunction: timing];
        anim
    }
}
/// 把图层的值设成 `to`，同时加一段从 `from` 过去的动画。
unsafe fn animate(layer: *mut AnyObject, path: &str, from: f64, to: f64, dur: f64, curve: [f32; 4]) {
    unsafe {
        let anim = basic(path, from, to, dur, curve);
        let _: () = msg_send![layer, setValue: number(to), forKeyPath: ns_string(path)];
        let _: () = msg_send![layer, addAnimation: anim, forKey: ns_string(path)];
    }
}
/// 一组动画，全部结束后调 `done`。
unsafe fn transaction(done: &RcBlock<dyn Fn()>, body: impl FnOnce()) {
    unsafe {
        let ca = AnyClass::get(c"CATransaction").unwrap();
        let _: () = msg_send![ca, begin];
        let _: () = msg_send![ca, setDisableActions: true];
        let _: () = msg_send![ca, setCompletionBlock: &**done];
        body();
        let _: () = msg_send![ca, commit];
    }
}
