//! 退场。GPUI 只画当前的状态：换了页、收了面板，下一帧旧的就没了。这里趁屏幕上还是旧的那一帧，
//! 用原生快照（`snapshotViewAfterScreenUpdates:NO`）把它留下来，盖在 GPUI 画面之上、玻璃之下，
//! 交给 Core Animation 送走；GPUI 那边照常画新的状态（返回时下面那一页带视差地回来）。
//!
//! 快照只拍 Metal 视图，所以玻璃和系统菜单的按钮不放在它里面，而是放在它旁边的一层（`overlay`）：
//! 否则快照里会带着一份标签栏，滑走时从真的那份底下透出来。
use std::cell::{Cell, RefCell};
use std::panic::Location;
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
/// 收起的面板滑出屏幕下沿的时长。
const SINK: f64 = 0.3;
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
    FRAME.with(|f| f.set(Instant::now()));
    watch_presents(handle.ui_view.as_ptr() as *mut AnyObject);
}

// ---- 哪一帧真的上屏了
//
// GPUI 画完一帧就把它交给 Metal，Metal 等 GPU 画好才上屏：真机上首页刚换上来的头几帧（字形、图片第一次进图集）
// 可能比画的时候晚好几帧才出现。原生视图的改动（挪开快照）却是下一次刷新就生效，只按 GPUI 画了几帧来等，
// 快照挪开时底下可能还什么都没有。所以在 Metal 图层上挂个钩子：每次 GPUI 要一块新的画布（`nextDrawable`），
// 给它登记「上屏时」的回调，记下已经上屏的最新一帧。只改这一个图层实例的类（像 KVO 那样），不动 GPUI。

static REQUESTED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static PRESENTED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
thread_local! { static WATCHED: Cell<bool> = const { Cell::new(false) }; }
#[link(name = "objc")]
unsafe extern "C" { fn object_setClass(obj: *mut AnyObject, cls: *const AnyClass) -> *const AnyClass; }

fn watch_presents(view: *mut AnyObject) {
    if WATCHED.with(|w| w.get()) || view.is_null() { return; }
    WATCHED.with(|w| w.set(true));
    unsafe {
        let layer: *mut AnyObject = msg_send![view, layer];
        let Some(metal) = AnyClass::get(c"CAMetalLayer") else { return; };
        if layer.is_null() { return; }
        let is_metal: bool = msg_send![layer, isKindOfClass: metal];
        if !is_metal { return; }
        let class: *const AnyClass = msg_send![layer, class];
        let Some(sub) = presents_class(&*class) else { return; };
        object_setClass(layer, sub);
    }
}
fn presents_class(base: &'static AnyClass) -> Option<&'static AnyClass> {
    static CLASS: std::sync::OnceLock<Option<&'static AnyClass>> = std::sync::OnceLock::new();
    static BASE: std::sync::OnceLock<&'static AnyClass> = std::sync::OnceLock::new();
    let _ = BASE.set(base);
    *CLASS.get_or_init(|| {
        extern "C-unwind" fn next(this: &AnyObject, _: Sel) -> *mut AnyObject {
            let base = *BASE.get().unwrap();
            let drawable: *mut AnyObject = unsafe { msg_send![super(this, base), nextDrawable] };
            // 模拟器的画布没有上屏回调：那里不挂，按帧数等（`below_on_screen`）。
            let hook: bool = !drawable.is_null() && unsafe { msg_send![drawable, respondsToSelector: sel!(addPresentedHandler:)] };
            if hook {
                let seq = REQUESTED.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                // 掉帧（没上屏）时 `presentedTime` 是 0，不算。
                let shown = RcBlock::new(move |d: *mut AnyObject| {
                    let t: f64 = unsafe { msg_send![d, presentedTime] };
                    if t > 0. { PRESENTED.fetch_max(seq, std::sync::atomic::Ordering::Relaxed); }
                });
                unsafe { let _: () = msg_send![drawable, addPresentedHandler: &*shown]; }
            }
            drawable
        }
        let mut b = ClassBuilder::new(c"QuietMdPresentedMetalLayer", base)?;
        unsafe { b.add_method(sel!(nextDrawable), next as extern "C-unwind" fn(_, _) -> _); }
        Some(b.register())
    })
}
fn presents_watched() -> bool { REQUESTED.load(std::sync::atomic::Ordering::Relaxed) > 0 }
/// 在绘制期间调用：这一帧的画布在绘制结束后才会取出。
pub(super) fn next_drawable_number() -> u64 { REQUESTED.load(std::sync::atomic::Ordering::Relaxed) + 1 }
pub(super) fn waiting_for_presentation(frame: u64) -> bool {
    let requested = REQUESTED.load(std::sync::atomic::Ordering::Relaxed);
    // 模拟器没有回调。真机的画布轮着用，再取出三块就不用继续等这块了。
    requested > 0 && PRESENTED.load(std::sync::atomic::Ordering::Relaxed) < frame && requested < frame + 3
}
// 拍快照以后：这一帧要等哪一块画布上屏（下面那页第一次连图片都画好的那一帧），和已经画了几帧（钩子装不上时按帧数等）。
thread_local! { static TARGET: Cell<(Option<u64>, u32)> = const { Cell::new((None, 0)) }; }
/// 下面那页（连图片）已经真的在屏幕上了，可以挪开快照。每帧（`below_drawn`）调一次。等太久就不等了。
fn below_on_screen() -> bool {
    if CAPTURED.with(|c| c.get()).elapsed() > PICTURE_WAIT { return true; }
    let pics = pictures_ready();
    let (target, frames) = TARGET.with(|t| t.get());
    if !presents_watched() { TARGET.with(|t| t.set((target, frames + 1))); return pics && frames + 1 >= 2; }
    match target {
        None => { if pics { TARGET.with(|t| t.set((Some(next_drawable_number()), frames))); } false }
        Some(t) => !waiting_for_presentation(t),
    }
}
thread_local! { static FRAME: Cell<Instant> = Cell::new(Instant::now()); }
// 拍下旧页的时候。
thread_local! { static CAPTURED: Cell<Instant> = Cell::new(Instant::now()); }
/// 下面那页最多等图片解码多久：再久就不等了，宁可露出空白也不能让手势卡住。
const PICTURE_WAIT: Duration = Duration::from_millis(400);
/// 刚画的这一帧里的图片都好了（或者已经等够了）。首页卡片的图片在文档里待久了会被放掉（`editor::image` 的缓存），
/// 回来要重新解码：这时挪开旧页，卡片先空着、随后图片跳出来，看着就是闪了一下。
fn pictures_ready() -> bool {
    !crate::editor::image::pictures_pending(FRAME.with(|f| f.get())) || CAPTURED.with(|c| c.get()).elapsed() > PICTURE_WAIT
}
pub(super) fn metal() -> Option<*mut AnyObject> { Some(METAL.with(|m| m.get())).filter(|p| !p.is_null()) }
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
        // 系统菜单从按钮往上找视图控制器来呈现：这一层不在 GPUI 那个控制器的视图里，自己带一个（一直留着）。
        // 不能挂成根控制器的子控制器（视图直接在窗口里，UIKit 的层级检查会抛异常），所以是游离的：
        // 能弹，系统日志里有一条「detached view controller」的警告。
        let vc: *mut AnyObject = msg_send![AnyClass::get(c"UIViewController").unwrap(), new];
        let _: () = msg_send![vc, setView: view];
        let _: () = msg_send![sup, insertSubview: view, aboveSubview: metal];
        OVERLAY.with(|o| o.set(view));
        view
    }
}
/// 弹着确认框时，玻璃和系统菜单的按钮照旧画着（页面看上去没变），但不接触摸：整层关掉交互，触摸落回 GPUI 的遮罩上。
pub fn set_inert(on: bool) {
    let view = OVERLAY.with(|o| o.get());
    if !view.is_null() { unsafe { let _: () = msg_send![view, setUserInteractionEnabled: !on]; } }
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
// 跟手返回：下面那一页已经画过几帧，和这期间手指滑到的比例（还没交给快照的）。
thread_local! { static BELOW: Cell<(u32, Option<f32>)> = const { Cell::new((0, None)) }; }

/// 要换页了（状态还没变，屏幕上还是这一页）：把这一页拍下来盖在原处。下一帧 `ios::page` 看清了方向：
/// 是返回就送走（`slide_out`），不是就拿掉（`drop_page`）。没换成页（比如被「先保存」拦下了）也在那一帧拿掉。
pub fn capture_page() {
    if !super::is_compact() { return; }
    let (Some(metal), Some(sup)) = (metal(), superview()) else { return; };
    remove_page();
    // 这一页上画着的图片留着，回来时不用重新解码（`editor::image::pin_shown`）。
    crate::editor::image::pin_shown();
    unsafe {
        // 只拍页面（Metal）和顶部那条玻璃带：玻璃按钮不跟着走，照 Craft 留在顶上原处淡出（`show`）；
        // 带子要拍进来，不然它在原处一收，快照顶上就露出没有玻璃的正文（文档往下滚过、或者有背景样式时顶上一直有带子）。
        let snap: Option<Retained<AnyObject>> = msg_send![metal, snapshotViewAfterScreenUpdates: false];
        let Some(snap) = snap else { return; };
        let _: () = msg_send![&*snap, setUserInteractionEnabled: false];
        let frame: CGRect = msg_send![metal, frame];
        // 快照剪成和屏幕一样的圆角（照 Craft：滑走时左边两个角是圆的，和机身的角一样弯）；
        // 阴影要画在剪裁之外，所以外面再套一层，动画和阴影都在外面这层上。
        let holder: Retained<AnyObject> = msg_send![AnyClass::get(c"UIView").unwrap(), new];
        let _: () = msg_send![&*holder, setUserInteractionEnabled: false];
        let _: () = msg_send![&*holder, setFrame: frame];
        let clip: Retained<AnyObject> = msg_send![AnyClass::get(c"UIView").unwrap(), new];
        let _: () = msg_send![&*clip, setFrame: CGRect::new(CGPoint::new(0., 0.), frame.size)];
        // 2 | 16：缩回卡片时外层的高度会变，剪裁这层跟着。
        let _: () = msg_send![&*clip, setAutoresizingMask: 18usize];
        let layer: *mut AnyObject = msg_send![&*clip, layer];
        let _: () = msg_send![layer, setCornerRadius: corner_radius()];
        let _: () = msg_send![layer, setCornerCurve: ns_string("continuous")];
        let _: () = msg_send![layer, setMasksToBounds: true];
        let _: () = msg_send![&*snap, setFrame: CGRect::new(CGPoint::new(0., 0.), frame.size)];
        let _: () = msg_send![&*clip, addSubview: &*snap];
        if let Some(band) = super::veil::band() {
            let copy: Option<Retained<AnyObject>> = msg_send![band, snapshotViewAfterScreenUpdates: false];
            if let Some(copy) = copy {
                let at: CGRect = msg_send![band, frame];
                let parent: *mut AnyObject = msg_send![band, superview];
                let at: CGRect = if parent.is_null() { at } else { msg_send![parent, convertRect: at, toView: metal] };
                let _: () = msg_send![&*copy, setFrame: at];
                let _: () = msg_send![&*clip, addSubview: &*copy];
            }
        }
        let _: () = msg_send![&*holder, addSubview: &*clip];
        let _: () = msg_send![sup, insertSubview: &*holder, aboveSubview: metal];
        // 马上提交：GPUI 下一帧就画新页，Metal 的画面不等 Core Animation 的事务，一画完就上屏；
        // 快照要是等到这一轮事件处理完才随事务提交，真机上会晚一帧——那一帧新页已经出来、快照还没盖上，就是那一下闪。
        let _: () = msg_send![AnyClass::get(c"CATransaction").unwrap(), flush];
        PAGE_SHOT.with(|s| *s.borrow_mut() = Some(holder));
        BELOW.with(|b| b.set((0, None)));
        CATCH.with(|c| c.set(None));
        SHOWN.with(|s| s.set(0.));
        TARGET.with(|t| t.set((None, 0)));
        CAPTURED.with(|c| c.set(Instant::now()));
        LANDED.with(|l| l.set(false));
        // 是从卡片打开的文档在返回：这次缩回那张卡片（`zoom_*`），卡片先空着。
        let leaving = LEAVING.with(|l| l.borrow_mut().take());
        let slot = leaving.filter(|id| FROM_CARD.with(|f| f.borrow().as_ref() == Some(id)));
        ZOOM.with(|z| *z.borrow_mut() = slot.map(|id| Zoom { id, card: None }));
    }
}

/// 屏幕的圆角半径。系统没有公开的接口，读 `UIScreen` 的 `_displayCornerRadius`；读不到（旧系统、没有圆角的屏）就是直角。
fn corner_radius() -> f64 {
    thread_local! { static RADIUS: Cell<Option<f64>> = const { Cell::new(None) }; }
    RADIUS.with(|r| *r.get().get_or_insert_with(|| unsafe {
        let screen: *mut AnyObject = msg_send![AnyClass::get(c"UIScreen").unwrap(), mainScreen];
        let name = sel!(_displayCornerRadius);
        let ok: bool = !screen.is_null() && msg_send![screen, respondsToSelector: name];
        if !ok { return 0.; }
        let v: f64 = MessageReceiver::send_message(screen, name, ());
        if v.is_finite() { v.max(0.) } else { 0. }
    }))
}
/// 有一张等着 `ios::page` 处理的快照。
pub fn page_pending() -> bool { PAGE_SHOT.with(|s| s.borrow().is_some()) }
/// 不是返回：快照和下面这一帧要换上的东西无关，拿掉。
pub fn drop_page() {
    remove_page();
    ZOOM.with(|z| z.borrow_mut().take());
}
fn remove_page() {
    if let Some(v) = PAGE_SHOT.with(|s| s.borrow_mut().take()) { unsafe { let _: () = msg_send![&*v, removeFromSuperview]; } }
    if let Some(v) = DIM.with(|s| s.borrow_mut().take()) { unsafe { let _: () = msg_send![&*v, removeFromSuperview]; } }
}
/// 旧页正在滑走（或跟着手指）：只属于新页的玻璃等它走完再露面，离开的那一页的玻璃淡出。
pub fn sliding() -> bool { PAGE_SHOT.with(|s| s.borrow().is_some()) || SLIDING.with(|s| s.get()).is_some_and(|t| Instant::now() < t) }

/// 玻璃和系统菜单按钮的显隐都经过这里。旧页正在滑走（缩回卡片）时：要露面的推迟（返回假，下一帧再试）；
/// 要藏起的分两种（照 Craft）：顶栏的按钮停在原处不动，也推迟，等过渡完了再淡出、和新页的按钮交接；
/// 别处的（文档底部的工具条）在原处淡出。别的时候照常立刻显隐。
pub fn show(view: *mut AnyObject, on: bool) -> bool {
    unsafe {
        let layer: *mut AnyObject = msg_send![view, layer];
        if on {
            if sliding() { return false; }
            let _: () = msg_send![layer, removeAnimationForKey: ns_string("vanish")];
            let _: () = msg_send![layer, removeAnimationForKey: ns_string("shrink")];
            let was_hidden: bool = msg_send![view, isHidden];
            let _: () = msg_send![view, setHidden: false];
            if was_hidden && just_slid() { materialize(layer); }
        } else if sliding() && in_top_bar(view) {
            return false;
        } else if sliding() || just_slid() || soft() {
            // 交接时旧的走得快（0.1 秒），新的稍晚再出来，两份不在原处叠着（照 Craft）。
            let anim = basic("opacity", 1., 0., if just_slid() { 0.1 } else { 0.15 }, [0.42, 0., 1., 1.]);
            let _: () = msg_send![anim, setFillMode: ns_string("forwards")];
            let _: () = msg_send![anim, setRemovedOnCompletion: false];
            // 淡完还没被叫回来才真的藏起；中途露面会先拿掉这段动画，这里就什么都不做。
            let done = RcBlock::new(move || {
                let layer: *mut AnyObject = msg_send![view, layer];
                let anim: *mut AnyObject = msg_send![layer, animationForKey: ns_string("vanish")];
                if !anim.is_null() { let _: () = msg_send![view, setHidden: true]; let _: () = msg_send![layer, removeAnimationForKey: ns_string("vanish")]; }
            });
            transaction(&done, || {
                let _: () = msg_send![layer, addAnimation: anim, forKey: ns_string("vanish")];
                // 顶栏的按钮交接时边淡边缩一点（照 Craft）。
                if just_slid() {
                    let shrink = basic("transform.scale", 1., 0.9, 0.1, [0.42, 0., 1., 1.]);
                    let _: () = msg_send![shrink, setFillMode: ns_string("forwards")];
                    let _: () = msg_send![shrink, setRemovedOnCompletion: false];
                    let _: () = msg_send![layer, addAnimation: shrink, forKey: ns_string("shrink")];
                }
            });
        } else {
            let _: () = msg_send![view, setHidden: true];
        }
    }
    true
}
/// 按钮在顶栏里（上沿在安全区下面一个顶栏高度以内）。
fn in_top_bar(view: *mut AnyObject) -> bool {
    let frame: CGRect = unsafe { msg_send![view, frame] };
    frame.origin.y < f64::from(super::insets().top) + f64::from(crate::app::HEADER_H)
}
/// 旧页在 `end` 之前一直在走（`sliding`）。走完的那一刻和交接（`just_slid`）结束时各让 GPUI 重画一次：
/// 新页的玻璃、空间头像要等过渡完才露面，而那时不一定还有别的事让它出帧——没有的话头像会一直空着，直到下一次触摸。
fn slide_until(end: Instant) {
    SLIDING.with(|s| s.set(Some(end)));
    let wait = end.saturating_duration_since(Instant::now());
    for after in [wait + Duration::from_millis(10), wait + Duration::from_millis(320)] {
        let tick = RcBlock::new(move |_timer: *mut AnyObject| super::menu::dispatch(|_, _, cx| cx.notify()));
        unsafe { let _: *mut AnyObject = msg_send![AnyClass::get(c"NSTimer").unwrap(), scheduledTimerWithTimeInterval: after.as_secs_f64(), repeats: false, block: &*tick]; }
    }
}
/// 过渡刚完时露面的顶栏按钮、空间头像：从小一点、透明里长出来，稍微弹一下（照 Craft 顶栏的交接，
/// 旧页的按钮同时边淡边缩）。
pub fn materialize(layer: *mut AnyObject) {
    unsafe {
        let ca = AnyClass::get(c"CATransaction").unwrap();
        let _: () = msg_send![ca, begin];
        let _: () = msg_send![ca, setDisableActions: true];
        // 控制点越过 1：先略过头再回来。
        let grow = basic("transform.scale", 0.78, 1., 0.34, [0.3, 1.45, 0.55, 1.]);
        let _: () = msg_send![layer, addAnimation: grow, forKey: ns_string("materialize")];
        let fade = basic("opacity", 0., 1., 0.2, [0.25, 0.1, 0.25, 1.]);
        let _: () = msg_send![layer, addAnimation: fade, forKey: ns_string("materialize-fade")];
        let _: () = msg_send![ca, commit];
    }
}
thread_local! { static SOFT: std::cell::Cell<Option<Instant>> = const { std::cell::Cell::new(None) }; }
/// 接下来一小会儿要藏的玻璃按钮淡出，不是一下消失（打开搜索时：照 Craft，顶上和右下的按钮跟着搜索页一起淡掉）。
pub fn soft_hide() { SOFT.with(|s| s.set(Some(Instant::now()))); }
fn soft() -> bool { SOFT.with(|s| s.get()).is_some_and(|t| t.elapsed() < Duration::from_millis(300)) }
/// 过渡刚完（0.3 秒内）：推迟到现在才藏起的顶栏按钮淡出，和新页淡入的按钮交接，不是一下没了。
pub fn just_slid() -> bool { SLIDING.with(|s| s.get()).is_some_and(|t| { let now = Instant::now(); now >= t && now - t < Duration::from_millis(300) }) }

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
        let path: *mut AnyObject = msg_send![AnyClass::get(c"UIBezierPath").unwrap(), bezierPathWithRoundedRect: bounds, cornerRadius: corner_radius()];
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

/// 跟手返回：快照跟着手指，`progress` 是已经滑过的屏宽比例。下面那一页还没真的上屏时（GPUI 换页要一两帧，
/// 真机上首页卡片多时更久）快照先不动，比例记着，等 `below_drawn` 再挪：不然头几帧快照底下露出来的是还没换掉的旧页，
/// 随后才变成首页，看着像闪了一下。
pub fn follow(progress: f32) {
    let ready = BELOW.with(|b| { let (n, _) = b.get(); if n < 2 { b.set((n, Some(progress))); } n >= 2 });
    if ready { CATCH.with(|c| if let Some((t, _)) = c.get() { c.set(Some((t, progress))); }); apply(progress); }
}
// 等下面那页上屏的这段时间里手指已经走了一段：快照不一下跳过去（看着像闪），而是在 `CATCH_UP` 里追上手指。
thread_local! { static CATCH: Cell<Option<(Instant, f32)>> = const { Cell::new(None) }; }
// 快照实际摆到的比例（追手指时比手指慢）：松手时从这里接着走。
thread_local! { static SHOWN: Cell<f32> = const { Cell::new(0.) }; }
const CATCH_UP: f32 = 0.14;
fn apply(progress: f32) {
    let k = CATCH.with(|c| match c.get() {
        Some((t, _)) => { let k = (t.elapsed().as_secs_f32() / CATCH_UP).min(1.); if k >= 1. { c.set(None); } ease_out_cubic(k) }
        None => 1.,
    });
    let shown = progress * k;
    SHOWN.with(|s| s.set(shown));
    move_to(shown);
}
/// 快照还原样盖着整屏（下面那页还没上屏、快照还没动）：状态栏先不换深浅（`ios::set_layout`）。
pub fn covering() -> bool { page_pending() && BELOW.with(|b| b.get().0) < 2 }
/// 每帧（换页那一层的 prepaint）调用：有快照在等，就数下面那一页画了几帧；画过一帧、又到了下一帧，就把记着的比例交给快照。
pub fn below_drawn() {
    // 这一帧画了哪些卡片（换页那一层的 prepaint 在页面内容之后，卡片都已经报过了）。
    let cards = CARDS.with(|c| std::mem::take(&mut *c.borrow_mut()));
    if !cards.is_empty() { LAST_CARDS.with(|l| *l.borrow_mut() = cards.clone()); }
    ZOOM.with(|z| if let Some(z) = z.borrow_mut().as_mut() && z.card.is_none() {
        let (w, h) = screen();
        z.card = cards.iter().find(|(id, _)| *id == z.id).map(|(_, b)| CGRect::new(CGPoint::new(f64::from(b.origin.x), f64::from(b.origin.y)), CGSize::new(f64::from(b.size.width), f64::from(b.size.height))))
            // 卡片得在屏幕上（横排里滑出去了、列表滚走了就照常滑走）。
            .filter(|r| { let (cx, cy) = (r.origin.x + r.size.width / 2., r.origin.y + r.size.height / 2.); r.size.width > 20. && (0. ..w).contains(&cx) && (0. ..h).contains(&cy) });
    });
    // 正在追手指：手指不动也每帧往前追。
    if PAGE_SHOT.with(|s| s.borrow().is_some()) && let Some((_, p)) = CATCH.with(|c| c.get()) { apply(p); }
    let waiting = OUT.with(|o| o.borrow().is_some()) || PAGE_SHOT.with(|s| s.borrow().is_some()) && BELOW.with(|b| b.get().0) < 2;
    if !waiting { return; }
    let ready = below_on_screen();
    // 点返回键（或早早松手）的那一次：等到知道了卡片在哪、下面那页也真的上屏了才走。
    if ready && let Some((snap, dim, from)) = OUT.with(|o| o.borrow_mut().take()) {
        match zoom_card() { Some(card) => zoom_out(snap, dim, card), None => { ZOOM.with(|z| z.borrow_mut().take()); slide_anim(snap, dim, from) } }
    }
    if PAGE_SHOT.with(|s| s.borrow().is_none()) || !ready { return; }
    let (_, pending) = BELOW.with(|b| b.get());
    BELOW.with(|b| b.set((2, pending)));
    if let Some(p) = pending { CATCH.with(|c| c.set(Some((Instant::now(), p)))); apply(p); }
}
fn move_to(progress: f32) {
    dress();
    if zoom_card().is_some() { return zoom_follow(progress); }
    let x = width() * f64::from(progress);
    PAGE_SHOT.with(|s| if let Some(snap) = s.borrow().as_ref() { unsafe { set_value(msg_send![&**snap, layer], "transform.translation.x", x); } });
    DIM.with(|d| if let Some(dim) = d.borrow().as_ref() { unsafe { let _: () = msg_send![&**dim, setAlpha: dim_at(f64::from(progress))]; } });
}

/// 返回：快照从 `from`（已经滑过的比例）往右滑出屏幕。没有快照时返回假，`ios::page` 用原来的滑入。
pub fn slide_out(from: f32) -> bool {
    if !page_pending() { return false; }
    let below_ready = BELOW.with(|b| b.get().0) >= 2;
    // 快照可能还没追上手指：从它实际在的地方走。
    let from = if below_ready && CATCH.with(|c| c.take()).is_some() { SHOWN.with(|s| s.get()) } else { from };
    BELOW.with(|b| b.set((2, None)));
    dress();
    let Some(snap) = PAGE_SHOT.with(|s| s.borrow_mut().take()) else { return false; };
    let dim = DIM.with(|d| d.borrow_mut().take());
    FROM_CARD.with(|f| f.borrow_mut().take());
    if let Some(card) = zoom_card() { zoom_out(snap, dim, card); return true; }
    // 点返回键：这一帧下面那页才开始画，卡片在哪还不知道，留到 `below_drawn`。
    // 下面那页还没真的上屏（图片没解码好、或者 GPU 还没画完，松手得早）：缩回卡片也等它，免得四周先空着。照常滑走的那种不等——GPUI 那边的视差已经开始了。
    if ZOOM.with(|z| z.borrow().is_some()) && (from <= 0. || !below_ready) { OUT.with(|o| *o.borrow_mut() = Some((snap, dim, from))); slide_until(Instant::now() + PICTURE_WAIT + Duration::from_millis(600)); return true; }
    // 跟手到这里卡片还是没找到：照常滑走。
    ZOOM.with(|z| z.borrow_mut().take());
    slide_anim(snap, dim, from);
    true
}
fn slide_anim(snap: Retained<AnyObject>, dim: Option<Retained<AnyObject>>, from: f32) {
    let w = width();
    let dur = slide_time(1. - from).as_secs_f64();
    slide_until(Instant::now() + Duration::from_secs_f64(dur));
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
}

/// 跟手返回松手时没过半：快照滑回原处。滑完把 `restore` 交给 GPUI（`ios::menu_action`），
/// 它把原来那页恢复好；快照再稍等一下淡掉，免得新的一帧还没上屏就露出下面那页。
pub fn slide_back(from: f32, restore: &'static str) {
    let from = if CATCH.with(|c| c.take()).is_some() { SHOWN.with(|s| s.get()) } else { from };
    BELOW.with(|b| b.set((2, None)));
    if zoom_card().is_some() { return zoom_back(restore); }
    ZOOM.with(|z| z.borrow_mut().take());
    let w = width();
    let dur = slide_time(from).as_secs_f64();
    slide_until(Instant::now() + Duration::from_secs_f64(dur));
    PAGE_SHOT.with(|s| unsafe {
        let Some(snap) = s.borrow().clone() else { return; };
        let done = RcBlock::new(move || super::menu::chosen(restore));
        transaction(&done, || {
            animate(msg_send![&*snap, layer], "transform.translation.x", w * f64::from(from), 0., dur, OUT_CUBIC);
            DIM.with(|d| if let Some(dim) = d.borrow().as_ref() { animate(msg_send![&**dim, layer], "opacity", dim_at(f64::from(from)), dim_at(0.), dur, OUT_CUBIC); let _: () = msg_send![&**dim, setAlpha: dim_at(0.)]; });
        });
    });
}
// 跟手返回取消后还盖着的快照，和它还要再等几帧。
thread_local! { static PARKED: RefCell<Option<(Retained<AnyObject>, Option<Retained<AnyObject>>)>> = const { RefCell::new(None) }; }
thread_local! { static PARK_FRAMES: Cell<u32> = const { Cell::new(0) }; }
/// 跟手返回取消、原来那页正要恢复：快照先原样盖着，等那一页实际画过 `frames` 帧（`parked_tick`）再淡。
/// 按时间等不行：真机上文档页重新排版、图片重新解码可能比固定的几十毫秒慢，快照先淡掉就会闪出下面那页。
pub fn park_page(frames: u32) {
    let Some(snap) = PAGE_SHOT.with(|s| s.borrow_mut().take()) else { return; };
    // 快照已经回到整屏、把下面那层暗整个盖住了：暗现在拿掉看不出来；留到快照淡掉以后再拿，整屏会一下亮回来。
    if let Some(dim) = DIM.with(|d| d.borrow_mut().take()) { unsafe { let _: () = msg_send![&*dim, removeFromSuperview]; } }
    PARKED.with(|p| *p.borrow_mut() = Some((snap, None)));
    PARK_FRAMES.with(|f| f.set(frames));
    // 和返回时一样，等恢复的那页真的上屏（`below_on_screen`，从现在算起最多等 `PICTURE_WAIT`）。
    TARGET.with(|t| t.set((None, 0)));
    CAPTURED.with(|c| c.set(Instant::now()));
}
/// 每帧调用：还盖着快照就数一帧，数完淡掉。返回真表示还要接着出帧。
pub fn parked_tick() -> bool {
    if PARKED.with(|p| p.borrow().is_none()) { return false; }
    let left = PARK_FRAMES.with(|f| { let n = f.get().saturating_sub(1); f.set(n); n });
    if left > 0 || !below_on_screen() { return true; }
    if let Some((snap, dim)) = PARKED.with(|p| p.borrow_mut().take()) {
        drop_page();
        PAGE_SHOT.with(|s| *s.borrow_mut() = Some(snap));
        DIM.with(|d| *d.borrow_mut() = dim);
        fade_page();
    }
    false
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
            // 下面那层暗和快照一起淡掉：只淡快照的话，快照淡完时恢复的那页上还压着一层暗，随后暗一拿掉，整屏一下亮回来，看着就是闪了一下。
            let layers: Vec<*mut AnyObject> = std::iter::once(msg_send![&*snap, layer]).chain(dim.as_ref().map(|d| msg_send![&**d, layer])).collect();
            for layer in layers {
                let from: f32 = msg_send![layer, opacity];
                let anim = basic("opacity", f64::from(from), 0., 0.1, [0.42, 0., 1., 1.]);
                // 晚 60ms 再开始：新的一帧先上屏。
                let now: f64 = msg_send![layer, convertTime: ca_now(), fromLayer: std::ptr::null_mut::<AnyObject>()];
                let _: () = msg_send![anim, setBeginTime: now + 0.06];
                let _: () = msg_send![anim, setFillMode: ns_string("backwards")];
                let _: () = msg_send![layer, addAnimation: anim, forKey: ns_string("fade")];
                let _: () = msg_send![layer, setOpacity: 0f32];
            }
        });
    }
}

// ---- 底部面板

#[derive(Clone)]
struct Seen { key: &'static Location<'static>, bounds: Bounds<Pixels> }
thread_local! { static SHEETS: RefCell<(Vec<Seen>, Vec<Seen>)> = const { RefCell::new((Vec::new(), Vec::new())) }; }
thread_local! { static SCRIM: Cell<(Option<f32>, Option<f32>)> = const { Cell::new((None, None)) }; }

/// 放在底部面板里的记号：每帧（prepaint）报一次面板在哪。`key` 是升起它的那一处代码。
pub fn sheet_marker(key: &'static Location<'static>) -> impl IntoElement {
    canvas(move |bounds, _, _| SHEETS.with(|s| s.borrow_mut().1.push(Seen { key, bounds })), |_, _, _, _| {}).absolute().inset_0()
}
/// 放在遮罩里的记号：这一帧有一层这么暗的遮罩。
pub fn scrim_marker(alpha: f32) -> impl IntoElement {
    canvas(move |_, _, _| SCRIM.with(|s| s.set((s.get().0, Some(alpha)))), |_, _, _, _| {}).absolute().inset_0()
}
/// 每帧 paint 时调用（这时所有面板都已 prepaint，屏幕上还是上一帧）：上一帧在、这一帧不在的面板
/// 拍下来往下滑出屏幕；遮罩没了就补一层原生的，淡掉。整页正在换的时候不管（旧页的快照里已经有它们）。
pub fn settle() -> impl IntoElement {
    canvas(|_, _, _| {}, |_, _, _, _| {
        let gone: Vec<Seen> = SHEETS.with(|s| {
            let mut s = s.borrow_mut();
            let (last, now) = &mut *s;
            let gone = last.iter().filter(|a| !now.iter().any(|b| b.key == a.key)).cloned().collect();
            *last = std::mem::take(now);
            gone
        });
        let scrim = SCRIM.with(|c| { let (last, now) = c.get(); c.set((now, None)); last.filter(|_| now.is_none()) });
        if !super::is_compact() || sliding() { return; }
        // 后画的在上面：先插的会被后插的垫在下面，所以从上往下插，遮罩最后。
        for g in gone.iter().rev() { sink(g.bounds); }
        if let Some(alpha) = scrim { fade_scrim(alpha); }
    }).absolute().size_0()
}

/// 面板的圆角：剪快照用。外壳的菜单是 12，编辑器的是 6，按大的剪，角上不会露出底下的东西。
const SHEET_RADIUS: f64 = 12.;

/// 屏幕上 `b` 那一块的快照（还没放进任何视图，位置就是 `b`）。长按浮起的预览也用它。
pub fn snapshot(b: Bounds<Pixels>) -> Option<Retained<AnyObject>> {
    let metal = metal()?;
    let (x, y, w, h) = (f64::from(b.origin.x), f64::from(b.origin.y), f64::from(b.size.width), f64::from(b.size.height));
    if w < 1. || h < 1. { return None; }
    unsafe {
        let snap: Option<Retained<AnyObject>> = msg_send![metal, snapshotViewAfterScreenUpdates: false];
        let snap = snap?;
        let full: CGRect = msg_send![metal, frame];
        let _: () = msg_send![&*snap, setFrame: CGRect::new(CGPoint::new(-x, -y), full.size)];
        let clip: Retained<AnyObject> = msg_send![AnyClass::get(c"UIView").unwrap(), new];
        let _: () = msg_send![&*clip, setUserInteractionEnabled: false];
        let _: () = msg_send![&*clip, setClipsToBounds: true];
        let _: () = msg_send![&*clip, setFrame: CGRect::new(CGPoint::new(x, y), CGSize::new(w, h))];
        let _: () = msg_send![&*clip, addSubview: &*snap];
        Some(clip)
    }
}
/// 快照放进去的那一层（GPUI 视图的父视图，坐标和 GPUI 的一样）。
pub fn container() -> Option<*mut AnyObject> { superview() }

fn sink(b: Bounds<Pixels>) {
    let (Some(metal), Some(sup)) = (metal(), superview()) else { return; };
    let Some(clip) = snapshot(b) else { return; };
    let y = f64::from(b.origin.y);
    unsafe {
        let full: CGRect = msg_send![metal, frame];
        let layer: *mut AnyObject = msg_send![&*clip, layer];
        let _: () = msg_send![layer, setCornerRadius: SHEET_RADIUS];
        let _: () = msg_send![sup, insertSubview: &*clip, aboveSubview: metal];
        let drop = full.size.height - y;
        let done = RcBlock::new(move || { let _: () = msg_send![&*clip, removeFromSuperview]; });
        transaction(&done, || animate(layer, "transform.translation.y", 0., drop, SINK, OUT_CUBIC));
    }
}

fn fade_scrim(alpha: f32) {
    let (Some(metal), Some(sup)) = (metal(), superview()) else { return; };
    unsafe {
        let view: Retained<AnyObject> = msg_send![AnyClass::get(c"UIView").unwrap(), new];
        let _: () = msg_send![&*view, setUserInteractionEnabled: false];
        let full: CGRect = msg_send![metal, frame];
        let _: () = msg_send![&*view, setFrame: full];
        let black: *mut AnyObject = msg_send![AnyClass::get(c"UIColor").unwrap(), blackColor];
        let _: () = msg_send![&*view, setBackgroundColor: black];
        let _: () = msg_send![&*view, setAlpha: 0f64];
        let _: () = msg_send![sup, insertSubview: &*view, aboveSubview: metal];
        let layer: *mut AnyObject = msg_send![&*view, layer];
        let done = RcBlock::new(move || { let _: () = msg_send![&*view, removeFromSuperview]; });
        transaction(&done, || animate(layer, "opacity", f64::from(alpha), 0., SINK, OUT_CUBIC));
    }
}

// ---- 缩回卡片
//
// 照 Craft（和系统的缩放转场）：从卡片打开的文档返回时，旧页不往右滑走，而是边跟着手指边缩小，
// 松手后缩回它那张卡片的位置和大小，再淡出露出真的卡片；这期间卡片画成空着的一格（`zoom_slot`）。
// 卡片不在屏幕上（滚走了、横排里滑出去了）就照常滑走。

struct Zoom { id: String, card: Option<CGRect> }
thread_local! { static ZOOM: RefCell<Option<Zoom>> = const { RefCell::new(None) }; }
// 缩回过卡片、还没换过页：下面那页一直不动（不然缩完以后 `ios::page` 又补上返回的视差，首页晃一下）。
thread_local! { static LANDED: Cell<bool> = const { Cell::new(false) }; }
// 从卡片打开的那篇文档；正在返回离开的那篇（`ios::doc_back` 记下，拍快照时对上了才缩回）。
thread_local! { static FROM_CARD: RefCell<Option<String>> = const { RefCell::new(None) }; }
thread_local! { static LEAVING: RefCell<Option<String>> = const { RefCell::new(None) }; }
// 这一帧画了的卡片和它们的位置。
thread_local! { static CARDS: RefCell<Vec<(String, Bounds<Pixels>)>> = const { RefCell::new(Vec::new()) }; }
// 上一帧画了的卡片（点卡片时从这里找它在哪）。
thread_local! { static LAST_CARDS: RefCell<Vec<(String, Bounds<Pixels>)>> = const { RefCell::new(Vec::new()) }; }
// 点开时拍下的那张卡片的样子（卡片 id、位置、快照）：打开时从它放大，返回时飞回去的路上渐渐变回它。
thread_local! { static FACE: RefCell<Option<(String, CGRect, Retained<AnyObject>)>> = const { RefCell::new(None) }; }
// 这次换页是从卡片放大打开。
thread_local! { static OPENING: Cell<bool> = const { Cell::new(false) }; }
// 点卡片那一刻的整屏（首页）：放大时垫在文档下面。打开文档不一定经过 `capture_page`。
thread_local! { static UNDER: RefCell<Option<Retained<AnyObject>>> = const { RefCell::new(None) }; }
// 点返回键时等着知道卡片位置的快照。
thread_local! { static OUT: RefCell<Option<(Retained<AnyObject>, Option<Retained<AnyObject>>, f32)>> = const { RefCell::new(None) }; }
/// 卡片的圆角（`render_home_card` 的 20）。
const CARD_RADIUS: f64 = 20.;

/// 点卡片打开文档：记下是哪张，趁它还在屏幕上拍下它的样子，接下来的换页从它放大（`open_zoom`）。
pub fn from_card(id: &str) {
    FROM_CARD.with(|f| *f.borrow_mut() = Some(id.to_string()));
    FACE.with(|f| f.borrow_mut().take());
    OPENING.with(|o| o.set(false));
    if !super::is_compact() { return; }
    let (w, h) = screen();
    let Some(b) = LAST_CARDS.with(|l| l.borrow().iter().rev().find(|(i, _)| i == id).map(|(_, b)| *b)) else { return };
    let r = CGRect::new(CGPoint::new(f64::from(b.origin.x), f64::from(b.origin.y)), CGSize::new(f64::from(b.size.width), f64::from(b.size.height)));
    let (cx, cy) = (r.origin.x + r.size.width / 2., r.origin.y + r.size.height / 2.);
    if r.size.width < 20. || !(0. ..w).contains(&cx) || !(0. ..h).contains(&cy) { return; }
    // 拍成一张真的图：Metal 视图的轻量快照（`snapshotView`）过一会儿再拿出来用就是空的，返回时要用的就是它。
    let Some(face) = bitmap(r) else { return };
    unsafe { let layer: *mut AnyObject = msg_send![&*face, layer]; let _: () = msg_send![layer, setCornerRadius: CARD_RADIUS]; let _: () = msg_send![layer, setCornerCurve: ns_string("continuous")]; }
    FACE.with(|f| *f.borrow_mut() = Some((id.to_string(), r, face)));
    let under: Option<Retained<AnyObject>> = metal().and_then(|m| unsafe { msg_send![m, snapshotViewAfterScreenUpdates: false] });
    if let Some(u) = &under { unsafe { let _: () = msg_send![&**u, setUserInteractionEnabled: false]; } }
    // 先把首页的样子盖在 Metal 上面、马上提交：真机上文档的第一帧可能在 `open_zoom` 缩小、遮罩之前就上了屏，
    // 整屏的文档闪一下（录屏 2026-11-21 07-01-53）。`open_zoom` 再把它挪到 Metal 下面垫着；
    // 万一没换成页（比如被「先保存」拦下），过一会儿自己拿掉。
    if let (Some(u), Some(metal), Some(sup)) = (&under, metal(), superview()) {
        unsafe {
            let frame: CGRect = msg_send![metal, frame];
            let _: () = msg_send![&**u, setFrame: frame];
            let _: () = msg_send![sup, insertSubview: &**u, aboveSubview: metal];
            let _: () = msg_send![AnyClass::get(c"CATransaction").unwrap(), flush];
            let _: () = msg_send![&**u, performSelector: objc2::sel!(removeFromSuperview), withObject: std::ptr::null_mut::<AnyObject>(), afterDelay: 0.9f64];
        }
    }
    UNDER.with(|u| *u.borrow_mut() = under);
    OPENING.with(|o| o.set(true));
}

/// 换页那一帧（`ios::page`）：这次是从卡片打开的话，开始放大，返回真（这一页就不再做自己的滑入）。
/// 照 Craft：文档从卡片的位置和大小长到整屏（Metal 视图整个缩放、剪成卡片的形状，下面垫着首页的快照），
/// 卡片的样子盖在上面一起放大、很快淡掉。
pub fn open_zoom() -> bool {
    if !OPENING.with(|o| o.replace(false)) { return false; }
    let Some(card) = FACE.with(|f| f.borrow().as_ref().map(|(_, r, _)| *r)) else { return false };
    let (Some(metal), Some(sup)) = (metal(), superview()) else { return false };
    let (w, h) = screen();
    if w <= 0. { return false; }
    const DUR: f64 = 0.46;
    slide_until(Instant::now() + Duration::from_secs_f64(DUR));
    // 首页的样子挪到 Metal 下面垫着：点卡片时拍的那张；`capture_page` 刚拍的那张不要。
    if let Some(v) = PAGE_SHOT.with(|s| s.borrow_mut().take()) { unsafe { let _: () = msg_send![&*v, removeFromSuperview]; } }
    if let Some(v) = DIM.with(|d| d.borrow_mut().take()) { unsafe { let _: () = msg_send![&*v, removeFromSuperview]; } }
    let home = UNDER.with(|u| u.borrow_mut().take());
    BELOW.with(|b| b.set((2, None)));
    let s = card.size.width / w;
    unsafe {
        if let Some(home) = &home {
            let frame: CGRect = msg_send![metal, frame];
            let _: () = msg_send![&**home, setFrame: frame];
            let _: () = msg_send![sup, insertSubview: &**home, belowSubview: metal];
        }
        // Metal 视图：缩到卡片的宽、左上角对齐卡片；只露出卡片那么高（遮罩在缩放前的坐标里）。
        let start = Affine { a: s, b: 0., c: 0., d: s, tx: card.origin.x + s * w / 2. - w / 2., ty: card.origin.y + s * h / 2. - h / 2. };
        let mask: Retained<AnyObject> = msg_send![AnyClass::get(c"UIView").unwrap(), new];
        let black: *mut AnyObject = msg_send![AnyClass::get(c"UIColor").unwrap(), blackColor];
        let _: () = msg_send![&*mask, setBackgroundColor: black];
        let _: () = msg_send![&*mask, setFrame: CGRect::new(CGPoint::new(0., 0.), CGSize::new(w, card.size.height / s))];
        let ml: *mut AnyObject = msg_send![&*mask, layer];
        let _: () = msg_send![ml, setCornerRadius: CARD_RADIUS / s];
        let _: () = msg_send![ml, setCornerCurve: ns_string("continuous")];
        let _: () = msg_send![metal, setMaskView: &*mask];
        let _: () = msg_send![metal, setTransform: start];
        // 卡片的样子：放在原处，跟着一起放大到整屏的宽，很快淡掉。
        let face = FACE.with(|f| f.borrow().as_ref().map(|(_, _, v)| v.clone()));
        if let Some(face) = &face {
            let _: () = msg_send![&**face, removeFromSuperview];
            let _: () = msg_send![&**face, setTransform: scale(1.)];
            let _: () = msg_send![&**face, setFrame: card];
            let _: () = msg_send![&**face, setAlpha: 1f64];
            let _: () = msg_send![sup, insertSubview: &**face, aboveSubview: metal];
        }
        // 马上提交起点（缩小、遮罩、卡片的样子）：同 `capture_page`，Metal 一画完新页就上屏、不等事务，
        // 不提交的话第一帧整屏的文档先露出来、下一帧才缩回卡片，就是那一下闪。
        let _: () = msg_send![AnyClass::get(c"CATransaction").unwrap(), flush];
        let (m, mk) = (metal as usize, &*mask as *const AnyObject as usize);
        let radius = corner_radius();
        let end_face = face.as_ref().map(|f| &**f as *const AnyObject as usize);
        let grow = 1. / s;
        let face_end = Affine { a: grow, b: 0., c: 0., d: grow, tx: w / 2. - (card.origin.x + card.size.width / 2.) + (grow - 1.) * 0., ty: card.size.height * grow / 2. - (card.origin.y + card.size.height / 2.) };
        let (keep_mask, keep_home, keep_face) = (mask.clone(), home.clone(), face.clone());
        spring(DUR, move || {
            let _: () = msg_send![m as *mut AnyObject, setTransform: scale(1.)];
            let _: () = msg_send![mk as *mut AnyObject, setFrame: CGRect::new(CGPoint::new(0., 0.), CGSize::new(w, h))];
            let ml: *mut AnyObject = msg_send![mk as *mut AnyObject, layer];
            let _: () = msg_send![ml, setCornerRadius: radius];
            if let Some(f) = end_face { let _: () = msg_send![f as *mut AnyObject, setTransform: face_end]; }
        }, move || {
            let _: () = msg_send![m as *mut AnyObject, setMaskView: std::ptr::null_mut::<AnyObject>()];
            let _ = &keep_mask;
            if let Some(h) = &keep_home { let _: () = msg_send![&**h, removeFromSuperview]; }
            if let Some(f) = &keep_face { let _: () = msg_send![&**f, removeFromSuperview]; let _: () = msg_send![&**f, setTransform: scale(1.)]; }
        });
        // 卡片的样子很快淡掉（约 0.16 秒），露出下面长大的文档，不叠出两份标题。
        if let Some(f) = end_face {
            let fade = RcBlock::new(move || { let _: () = msg_send![f as *mut AnyObject, setAlpha: 0f64]; });
            let none: Option<&block2::Block<dyn Fn(objc2::runtime::Bool)>> = None;
            let _: () = msg_send![AnyClass::get(c"UIView").unwrap(), animateWithDuration: 0.16f64, delay: 0.02f64, options: 0usize, animations: &*fade, completion: none];
        }
    }
    true
}
/// 要从文档 `id` 返回了（下一步就拍快照）。
pub fn leaving(id: Option<String>) { LEAVING.with(|l| *l.borrow_mut() = id); }
/// 这次返回是缩回卡片：下面那页原地不动（没有视差）。
pub fn zooming() -> bool { ZOOM.with(|z| z.borrow().is_some()) || LANDED.with(|l| l.get()) }
/// 这张卡片正等着文档缩回来：画成空着的一格。
pub fn zoom_slot(id: &str) -> bool { ZOOM.with(|z| z.borrow().as_ref().is_some_and(|z| z.id == id)) }
fn zoom_card() -> Option<CGRect> { ZOOM.with(|z| z.borrow().as_ref().and_then(|z| z.card)) }
/// 放在卡片里的记号：每帧报一次卡片在哪。
pub fn card_marker(id: &str) -> impl IntoElement {
    let id = id.to_string();
    canvas(move |b, _, _| CARDS.with(|c| c.borrow_mut().push((id.clone(), b))), |_, _, _, _| {}).absolute().inset_0()
}

/// 屏幕上 `r` 那一块拍成一张图，装在一个图片视图里（位置就是 `r`）。
fn bitmap(r: CGRect) -> Option<Retained<AnyObject>> {
    let metal = metal()?;
    unsafe {
        let window: *mut AnyObject = msg_send![metal, window];
        if window.is_null() { return None; }
        let bounds: CGRect = msg_send![window, bounds];
        let at: CGRect = msg_send![metal, convertRect: r, toView: window];
        let format: *mut AnyObject = msg_send![AnyClass::get(c"UIGraphicsImageRendererFormat").unwrap(), preferredFormat];
        let renderer: *mut AnyObject = msg_send![AnyClass::get(c"UIGraphicsImageRenderer").unwrap(), alloc];
        let renderer: *mut AnyObject = msg_send![renderer, initWithSize: r.size, format: format];
        let draw = CGRect::new(CGPoint::new(-at.origin.x, -at.origin.y), bounds.size);
        let actions = RcBlock::new(move |_ctx: *mut AnyObject| { let _: bool = msg_send![window, drawViewHierarchyInRect: draw, afterScreenUpdates: false]; });
        let image: *mut AnyObject = msg_send![renderer, imageWithActions: &*actions];
        let _: () = msg_send![renderer, release];
        if image.is_null() { return None; }
        let view: Retained<AnyObject> = msg_send![AnyClass::get(c"UIImageView").unwrap(), new];
        let _: () = msg_send![&*view, setImage: image];
        let _: () = msg_send![&*view, setFrame: r];
        let _: () = msg_send![&*view, setClipsToBounds: true];
        let _: () = msg_send![&*view, setUserInteractionEnabled: false];
        Some(view)
    }
}

fn screen() -> (f64, f64) { metal().map_or((0., 0.), |m| unsafe { let f: CGRect = msg_send![m, frame]; (f.size.width, f.size.height) }) }

#[repr(C)]
#[derive(Clone, Copy)]
struct Affine { a: f64, b: f64, c: f64, d: f64, tx: f64, ty: f64 }
unsafe impl objc2::Encode for Affine { const ENCODING: Encoding = Encoding::Struct("CGAffineTransform", &[const { Encoding::Double }; 6]); }
fn scale(s: f64) -> Affine { Affine { a: s, b: 0., c: 0., d: s, tx: 0., ty: 0. } }

/// 快照外面那层摆成：中心 `center`，未缩放的大小 `w`×`h`（高度变小就是把底下剪掉），缩放 `s`，圆角 `radius`（屏幕上看到的）。
/// 在 `UIView` 的动画里调用就是动画到这个样子。
unsafe fn place(view: *mut AnyObject, center: CGPoint, w: f64, h: f64, s: f64, radius: f64) {
    unsafe {
        let _: () = msg_send![view, setBounds: CGRect::new(CGPoint::new(0., 0.), CGSize::new(w, h))];
        let _: () = msg_send![view, setCenter: center];
        let _: () = msg_send![view, setTransform: scale(s)];
        let layer: *mut AnyObject = msg_send![view, layer];
        let _: () = msg_send![layer, setCornerRadius: radius / s.max(0.01)];
    }
}
/// 缩放时外层自己剪成圆角，不要滑走时的那道阴影（剪掉了也画不出来）。
unsafe fn zoom_dress(view: *mut AnyObject) {
    unsafe {
        let layer: *mut AnyObject = msg_send![view, layer];
        let _: () = msg_send![layer, setShadowOpacity: 0f32];
        let _: () = msg_send![layer, setCornerCurve: ns_string("continuous")];
        let _: () = msg_send![layer, setMasksToBounds: true];
    }
}
/// 跟手：左边跟着手指，越往右越小（到 0.4 为止），竖直方向居中。
fn zoom_follow(progress: f32) {
    let (w, h) = screen();
    let p = f64::from(progress);
    let s = (1. - 0.9 * p).max(0.4);
    PAGE_SHOT.with(|v| if let Some(v) = v.borrow().as_ref() { unsafe {
        zoom_dress(&**v as *const _ as *mut _);
        place(&**v as *const _ as *mut _, CGPoint::new(p * w + s * w / 2., h / 2.), w, h, s, corner_radius() * s);
    } });
    DIM.with(|d| if let Some(dim) = d.borrow().as_ref() { unsafe { let _: () = msg_send![&**dim, setAlpha: dim_at(p)]; } });
}
/// 一段弹簧动画，`done` 在结束后调用。
unsafe fn spring(dur: f64, body: impl Fn() + 'static, done: impl Fn() + 'static) {
    unsafe {
        let body = RcBlock::new(body);
        let done = RcBlock::new(move |_: objc2::runtime::Bool| done());
        let _: () = msg_send![AnyClass::get(c"UIView").unwrap(), animateWithDuration: dur, delay: 0f64, usingSpringWithDamping: 0.88f64, initialSpringVelocity: 0f64, options: 0usize, animations: &*body, completion: &*done];
    }
}
/// 松手（或点了返回键）：缩到卡片上，卡片露出来以后快照淡掉。
fn zoom_out(snap: Retained<AnyObject>, dim: Option<Retained<AnyObject>>, card: CGRect) {
    let (w, _) = screen();
    const DUR: f64 = 0.42;
    // 照 Craft：飞到后半程，真的卡片已经在下面画好了，快照一边落一边淡成它（真正的交叉淡化，不靠拍下来的卡片图——
    // Metal 的画面拍不成能留着用的图）。`REVEAL` 之后卡片不再空着。
    const REVEAL: f64 = 0.3;
    slide_until(Instant::now() + Duration::from_secs_f64(DUR + 0.1));
    unsafe {
        let view = &*snap as *const AnyObject as *mut AnyObject;
        zoom_dress(view);
        let se = card.size.width / w;
        let target = CGPoint::new(card.origin.x + card.size.width / 2., card.origin.y + card.size.height / 2.);
        let (v, d) = (view as usize, dim.as_ref().map(|d| &**d as *const AnyObject as usize));
        let (keep, keep_dim) = (snap.clone(), dim.clone());
        spring(DUR, move || {
            place(v as *mut AnyObject, target, w, card.size.height / se, se, CARD_RADIUS);
            if let Some(d) = d { let _: () = msg_send![d as *mut AnyObject, setAlpha: 0f64]; }
        }, move || {
            let _: () = msg_send![&*keep, removeFromSuperview];
            if let Some(dim) = &keep_dim { let _: () = msg_send![&**dim, removeFromSuperview]; }
        });
        // 到 `REVEAL`：卡片画回来（`zoom_slot` 不再为真），快照在剩下的路上淡掉。
        let reveal = RcBlock::new(move |_timer: *mut AnyObject| {
            ZOOM.with(|z| z.borrow_mut().take());
            LANDED.with(|l| l.set(true));
            super::menu::dispatch(|_, _, cx| cx.notify());
            let fade = RcBlock::new(move || { let _: () = msg_send![v as *mut AnyObject, setAlpha: 0f64]; });
            let none: Option<&block2::Block<dyn Fn(objc2::runtime::Bool)>> = None;
            // 先等一帧，让卡片真的画出来再开始淡。
            let _: () = msg_send![AnyClass::get(c"UIView").unwrap(), animateWithDuration: DUR * 0.5, delay: 0.02f64, options: 0usize, animations: &*fade, completion: none];
        });
        let _: *mut AnyObject = msg_send![AnyClass::get(c"NSTimer").unwrap(), scheduledTimerWithTimeInterval: DUR * REVEAL, repeats: false, block: &*reveal];
    }
}
/// 跟手没过半：放大回整屏，再交给 GPUI 恢复原来那页（和 `slide_back` 一样）。
fn zoom_back(restore: &'static str) {
    let (w, h) = screen();
    const DUR: f64 = 0.36;
    slide_until(Instant::now() + Duration::from_secs_f64(DUR));
    let Some(snap) = PAGE_SHOT.with(|s| s.borrow().clone()) else { return; };
    let dim = DIM.with(|d| d.borrow().clone());
    unsafe {
        let v = &*snap as *const AnyObject as usize;
        let d = dim.as_ref().map(|d| &**d as *const AnyObject as usize);
        spring(DUR, move || {
            place(v as *mut AnyObject, CGPoint::new(w / 2., h / 2.), w, h, 1., corner_radius());
            if let Some(d) = d { let _: () = msg_send![d as *mut AnyObject, setAlpha: dim_at(0.)]; }
        }, move || {
            ZOOM.with(|z| z.borrow_mut().take());
            super::menu::chosen(restore);
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
