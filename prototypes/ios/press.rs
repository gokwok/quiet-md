//! 点触：手指按在主页的文档、文件夹（一行或一张卡片）上，这一块就缩小一点，松手弹回（照 Craft，录屏 2026-11-25 19-41-04）。
//! GPUI 在手机上要等手指抬起才把按下和松开一起派发，也不能缩放一个元素：所以按下由挂在 Metal 视图上的原生手势认，
//! 缩小的是这一块的快照，垫一块页面的底色盖住原处，放在 Metal 视图里面——换页拍快照时（`shot::capture_page`）跟着旧页走。
//! 底下的画面一动（滚动、换页），原处对不上了，同一帧就拿掉（`finish_frame`）。
use std::cell::{Cell, RefCell};
use std::time::Instant;

use block2::RcBlock;
use gpui::*;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool, ClassBuilder, Sel};
use objc2::{msg_send, sel};
use objc2_core_foundation::{CGPoint, CGRect};

/// 一行缩到多小（Craft 实测：1320 宽的录屏里行首图标 81→87、行尾箭头 1245→1239，都以行的中心缩到 0.99）。
const ROW: f64 = 0.99;
/// 一张卡片缩到多小。录屏里没有点卡片，这是自己定的：比一行小得多，同样的 0.99 在卡片上看不出来。
const CARD: f64 = 0.97;
/// 比这高的算卡片。
const CARD_H: f32 = 90.;
/// 缩下去多久（Craft 实测 7 帧）；弹回多久；手指挪开这么远就不算按了。
const DOWN: f64 = 0.13;
/// 按下后等这么久才缩：甩一下滚动的手指这时已经挪开了。
const WAIT: f64 = 0.04;
const UP: f64 = 0.26;
const SLOP: f64 = 10.;

struct Region { key: SharedString, bounds: Bounds<Pixels>, bleed: Pixels, shield: bool }
struct Active { key: SharedString, bounds: Bounds<Pixels>, view: Retained<AnyObject>, face: Retained<AnyObject>, from: CGPoint, since: Instant, up: bool, adopted: bool }

thread_local! {
    static REGIONS: RefCell<Vec<Region>> = const { RefCell::new(Vec::new()) };
    static ACTIVE: RefCell<Option<Active>> = const { RefCell::new(None) };
    static INSTALLED: Cell<*mut AnyObject> = const { Cell::new(std::ptr::null_mut()) };
}

/// 每帧开头（`menu::begin`）。
pub fn begin() { REGIONS.with(|r| r.borrow_mut().clear()); }

fn region(key: SharedString, bleed: f32, shield: bool) -> impl IntoElement {
    canvas(move |bounds, window, _| {
        if !super::menu::pressable() { return; }
        let bounds = bounds.intersect(&window.content_mask().bounds);
        if bounds.size.width <= px(1.) || bounds.size.height <= px(1.) { return; }
        install();
        REGIONS.with(|r| r.borrow_mut().push(Region { key, bounds, bleed: px(bleed), shield }));
    }, |_, _, _, _| {}).absolute().inset_0()
}
/// 放在卡片或行里的占位：按住这一块会缩小。`bleed` 是画到格子外面的一圈（文件夹的影子）。
pub fn mark(key: impl Into<SharedString>, bleed: f32) -> impl IntoElement { region(key.into(), bleed, false) }
/// 放在行里自己接点击的小按钮上：按它不缩这一行。
pub fn shield() -> impl IntoElement { region("".into(), 0., true) }

/// 手势挂在 Metal 视图上，和 GPUI、系统菜单的长按各认各的，不取消它们的触摸。只挂一次。
/// 系统的长按手势（哪怕时长是 0）要等系统菜单那边先认完，抬手时才报；所以自己派生一个，直接看触摸，从来不认出什么。
fn install() {
    let Some(metal) = super::shot::metal() else { return };
    if INSTALLED.with(|i| i.replace(metal)) == metal { return; }
    unsafe {
        let g: *mut AnyObject = msg_send![watcher(), new];
        let _: () = msg_send![g, setCancelsTouchesInView: false];
        let _: () = msg_send![g, setDelaysTouchesEnded: false];
        let _: () = msg_send![metal, addGestureRecognizer: g];
        let _: () = msg_send![g, release];
    }
}

fn watcher() -> &'static AnyClass {
    static CLASS: std::sync::OnceLock<&'static AnyClass> = std::sync::OnceLock::new();
    CLASS.get_or_init(|| {
        unsafe fn at(g: &AnyObject, touches: *mut AnyObject) -> CGPoint { unsafe { let touch: *mut AnyObject = msg_send![touches, anyObject]; let view: *mut AnyObject = msg_send![g, view]; msg_send![touch, locationInView: view] } }
        extern "C-unwind" fn began(this: &AnyObject, _: Sel, touches: *mut AnyObject, _: *mut AnyObject) {
            let n: usize = unsafe { msg_send![this, numberOfTouches] };
            if n > 1 { release() } else { down(unsafe { at(this, touches) }) }
        }
        extern "C-unwind" fn moved(this: &AnyObject, _: Sel, touches: *mut AnyObject, _: *mut AnyObject) {
            let at = unsafe { at(this, touches) };
            if ACTIVE.with(|a| a.borrow().as_ref().is_some_and(|a| (at.x - a.from.x).hypot(at.y - a.from.y) > SLOP)) { release() }
        }
        extern "C-unwind" fn ended(this: &AnyObject, _: Sel, _: *mut AnyObject, _: *mut AnyObject) {
            release();
            // 5 是「没认出来」：收场，等下一次触摸。
            unsafe { let _: () = msg_send![this, setState: 5isize]; }
        }
        let mut b = ClassBuilder::new(c"QuietMdPressWatcher", AnyClass::get(c"UIGestureRecognizer").unwrap()).expect("register QuietMdPressWatcher");
        unsafe {
            b.add_method(sel!(touchesBegan:withEvent:), began as extern "C-unwind" fn(_, _, _, _));
            b.add_method(sel!(touchesMoved:withEvent:), moved as extern "C-unwind" fn(_, _, _, _));
            b.add_method(sel!(touchesEnded:withEvent:), ended as extern "C-unwind" fn(_, _, _, _));
            b.add_method(sel!(touchesCancelled:withEvent:), ended as extern "C-unwind" fn(_, _, _, _));
        }
        b.register()
    })
}

fn down(at: CGPoint) {
    if super::shot::sliding() { drop_now(); return; }
    let p = point(px(at.x as f32), px(at.y as f32));
    // 后报的画在上面。
    let hit = REGIONS.with(|r| {
        let r = r.borrow();
        if r.iter().any(|g| g.shield && g.bounds.contains(&p)) { return None; }
        r.iter().rev().find(|g| g.bounds.contains(&p)).map(|g| (g.key.clone(), g.bounds, g.bleed))
    });
    let (Some((key, bounds, bleed)), Some(metal)) = (hit, super::shot::metal()) else { drop_now(); return };
    let shown = Bounds::from_corners(bounds.origin - point(bleed, bleed), bounds.bottom_right() + point(bleed, bleed));
    // 上一下还没弹回就又按：先照它现在的样子拍，再拿掉。
    let face = super::shot::snapshot(shown);
    drop_now();
    let Some(face) = face else { return };
    unsafe {
        let frame: CGRect = msg_send![&*face, frame];
        let view: Retained<AnyObject> = msg_send![AnyClass::get(c"UIView").unwrap(), new];
        let _: () = msg_send![&*view, setUserInteractionEnabled: false];
        let _: () = msg_send![&*view, setFrame: frame];
        let _: () = msg_send![&*view, setBackgroundColor: super::glass::ui_color(super::home_bg())];
        let _: () = msg_send![&*face, setFrame: CGRect::new(CGPoint::new(0., 0.), frame.size)];
        let _: () = msg_send![&*view, addSubview: &*face];
        let _: () = msg_send![metal, addSubview: &*view];
        let to = if bounds.size.height > px(CARD_H) { CARD } else { ROW };
        let f = face.clone();
        ease(DOWN, WAIT, move || { let _: () = msg_send![&*f, setTransform: super::shot::scale(to)]; }, || {});
        ACTIVE.with(|a| *a.borrow_mut() = Some(Active { key, bounds, view, face, from: at, since: Instant::now(), up: false, adopted: false }));
    }
}

/// 松手、挪开了：弹回去（长按出菜单时是系统那张浮起的接过去，`menu::highlight`）。点得很快也等它先缩到底，看得出按着了。
pub fn release() {
    let Some((view, face, wait)) = ACTIVE.with(|a| a.borrow_mut().as_mut().filter(|a| !a.up).map(|a| { a.up = true; (a.view.clone(), a.face.clone(), (WAIT + DOWN - a.since.elapsed().as_secs_f64()).max(0.)) })) else { return };
    unsafe {
        ease(UP, wait, move || { let _: () = msg_send![&*face, setTransform: super::shot::scale(1.)]; }, move || remove(&view));
    }
}

/// 长按的菜单出来了，系统那张从缩小的样子浮起：这一块不弹回，盖着原处等 GPUI 把它空出来（下面一两帧）再拿掉。
pub fn hand_over() {
    let Some((view, face)) = ACTIVE.with(|a| a.borrow_mut().as_mut().map(|a| { a.up = true; (a.view.clone(), a.face.clone()) })) else { return };
    unsafe { ease(0.01, 0.1, move || { let _: () = msg_send![&*face, setAlpha: 0f64]; }, move || remove(&view)); }
}

fn remove(view: &Retained<AnyObject>) {
    unsafe { let _: () = msg_send![&**view, removeFromSuperview]; }
    ACTIVE.with(|a| { let mut a = a.borrow_mut(); if a.as_ref().is_some_and(|a| Retained::as_ptr(&a.view) == Retained::as_ptr(view)) { *a = None; } });
}

/// 马上拿掉。
pub fn drop_now() { if let Some(a) = ACTIVE.with(|a| a.borrow_mut().take()) { unsafe { let _: () = msg_send![&*a.view, removeFromSuperview]; } } }

/// 换页拍快照：按着的这一块挪进旧页的快照里（`into` 和 Metal 视图同一套坐标），在那里接着弹回去。
pub fn adopt(into: &AnyObject) {
    ACTIVE.with(|a| if let Some(a) = a.borrow_mut().as_mut() { a.adopted = true; unsafe { let _: () = msg_send![into, addSubview: &*a.view]; } });
}

/// 这一刻屏幕上缩到了几分之几（没按着是 1）：这时给 `bounds` 拍照，要按缩小后的那一块拍、再放回原大。
pub fn scale_now(bounds: Bounds<Pixels>) -> f64 {
    ACTIVE.with(|a| a.borrow().as_ref().filter(|a| !a.adopted && a.bounds.contains(&bounds.center())).map_or(1., |a| unsafe {
        let layer: *mut AnyObject = msg_send![&*a.face, layer];
        let shown: *mut AnyObject = msg_send![layer, presentationLayer];
        let name: *mut AnyObject = msg_send![AnyClass::get(c"NSString").unwrap(), stringWithUTF8String: c"transform.scale.x".as_ptr()];
        let value: *mut AnyObject = msg_send![if shown.is_null() { layer } else { shown }, valueForKeyPath: name];
        if value.is_null() { 1. } else { let v: f64 = msg_send![value, doubleValue]; if v > 0.5 { v } else { 1. } }
    }))
}

/// 这一帧画好了：按着的那一块不在原处了就拿掉（已经跟着旧页走的不管）。
pub fn finish_frame() {
    let gone = ACTIVE.with(|a| a.borrow().as_ref().is_some_and(|a| !a.adopted && !REGIONS.with(|r| r.borrow().iter().any(|g| g.key == a.key && g.bounds == a.bounds))));
    if gone { drop_now(); }
}

unsafe fn ease(dur: f64, delay: f64, body: impl Fn() + 'static, done: impl Fn() + 'static) {
    let body = RcBlock::new(body);
    let done = RcBlock::new(move |_: Bool| done());
    // 缓出（2 << 16）、从现在的样子接着走（4）、不拦触摸（2）。
    unsafe { let _: () = msg_send![AnyClass::get(c"UIView").unwrap(), animateWithDuration: dur, delay: delay, options: (2usize << 16) | 6, animations: &*body, completion: &*done]; }
}
