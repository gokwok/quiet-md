//! 有背景的文档页顶上那条毛玻璃带（照 Craft）：纸滚到顶栏下面时，从这条带子后面透出来，是糊的。
//!
//! GPUI 没有模糊，所以是原生浮层最底下的一块系统材质；浮层盖在 GPUI 上面，顶栏中间的小字标题会被它糊掉，
//! 所以标题也跟着搬到这块材质上，用系统的标签写。每帧有占位才显示（`slot`、`title`），没有就收起。
use std::cell::{Cell, RefCell};

use gpui::*;
use objc2::msg_send;
use objc2::runtime::{AnyClass, AnyObject};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

struct Veil { view: *mut AnyObject, title: *mut AnyObject, sub: *mut AnyObject, frame: Bounds<Pixels>, dark: Option<bool>, text: (String, String), shown: bool }
thread_local! { static VEIL: RefCell<Option<Veil>> = const { RefCell::new(None) }; }
thread_local! { static SEEN: Cell<(bool, bool)> = const { Cell::new((false, false)) }; }
// 这一帧用不用这条带子（有背景、有玻璃、纸已经滚到顶栏下面）。标题看它决定画在哪边。
thread_local! { static ON: Cell<bool> = const { Cell::new(false) }; }

pub fn on() -> bool { ON.with(|o| o.get()) }

/// 每帧开头调用。`on`：这一帧要这条带子。带子或标题这一帧没人要，就各自收起。
pub fn begin(on: bool, window: &Window) {
    ON.with(|o| o.set(on));
    SEEN.with(|s| s.set((false, false)));
    window.on_next_frame(|_, _| {
        let (band, title) = SEEN.with(|s| s.get());
        VEIL.with(|v| if let Some(veil) = v.borrow_mut().as_mut() { unsafe {
            if !band && veil.shown { veil.shown = false; fade(veil.view, 0.); }
            if !title && !veil.text.0.is_empty() { veil.text = Default::default(); fade(veil.title, 0.); fade(veil.sub, 0.); }
        } });
    });
}

unsafe fn fade(view: *mut AnyObject, to: f64) {
    unsafe {
        let addr = view as usize;
        let body = block2::RcBlock::new(move || { let _: () = msg_send![addr as *mut AnyObject, setAlpha: to]; });
        let _: () = msg_send![AnyClass::get(c"UIView").unwrap(), animateWithDuration: 0.18f64, animations: &*body];
    }
}

/// 带子的占位：铺在这个元素的位置上。`dark`：上面的字是白的（背景深）。
pub fn slot(dark: bool) -> impl IntoElement {
    canvas(move |bounds, window, _| place(bounds, dark, window), |_, _, _, _| {}).absolute().inset_0()
}

/// 标题的占位（顶栏中间那一格）：两行小字写在带子上。
pub fn title(title: SharedString, folder: Option<SharedString>, ink: Hsla, sub: Hsla) -> impl IntoElement {
    canvas(move |bounds, _, _| write(bounds, &title, folder.as_deref().unwrap_or(""), ink, sub), |_, _, _, _| {}).w_full().h(px(34.))
}

fn place(frame: Bounds<Pixels>, dark: bool, window: &Window) {
    let Ok(handle) = HasWindowHandle::window_handle(window) else { return; };
    let RawWindowHandle::UiKit(handle) = handle.as_raw() else { return; };
    let parent = super::shot::overlay(handle.ui_view.as_ptr() as *mut AnyObject);
    SEEN.with(|s| s.set((true, s.get().1)));
    VEIL.with(|v| unsafe {
        let mut slot = v.borrow_mut();
        if slot.is_none() {
            let (Some(effect_view), Some(blur), Some(label)) = (AnyClass::get(c"UIVisualEffectView"), AnyClass::get(c"UIBlurEffect"), AnyClass::get(c"UILabel")) else { return; };
            // 7 = UIBlurEffectStyleSystemThinMaterial：下面的颜色透得出来，字看不清。
            let effect: *mut AnyObject = msg_send![blur, effectWithStyle: 7isize];
            let view: *mut AnyObject = msg_send![effect_view, alloc];
            let view: *mut AnyObject = msg_send![view, initWithEffect: effect];
            let _: () = msg_send![view, setUserInteractionEnabled: false];
            let _: () = msg_send![view, setAlpha: 0f64];
            // 垫在浮层最底下：玻璃按钮在它上面。
            let _: () = msg_send![parent, insertSubview: view, atIndex: 0isize];
            let content: *mut AnyObject = msg_send![view, contentView];
            let make = |size: f64, weight: f64| {
                let l: *mut AnyObject = msg_send![label, new];
                let font: *mut AnyObject = msg_send![AnyClass::get(c"UIFont").unwrap(), systemFontOfSize: size, weight: weight];
                let _: () = msg_send![l, setFont: font];
                let _: () = msg_send![l, setAlpha: 0f64];
                let _: () = msg_send![content, addSubview: l];
                l
            };
            *slot = Some(Veil { view, title: make(15., 0.3), sub: make(11., 0.), frame: Bounds::default(), dark: None, text: Default::default(), shown: false });
        }
        let veil = slot.as_mut().unwrap();
        if veil.frame != frame {
            veil.frame = frame;
            let rect = CGRect::new(CGPoint::new(f64::from(frame.origin.x), f64::from(frame.origin.y)), CGSize::new(f64::from(frame.size.width), f64::from(frame.size.height)));
            let _: () = msg_send![veil.view, setFrame: rect];
        }
        if veil.dark != Some(dark) { veil.dark = Some(dark); let _: () = msg_send![veil.view, setOverrideUserInterfaceStyle: if dark { 2isize } else { 1isize }]; }
        if !veil.shown { veil.shown = true; fade(veil.view, 1.); }
    });
}

fn write(frame: Bounds<Pixels>, title: &str, folder: &str, ink: Hsla, sub: Hsla) {
    SEEN.with(|s| s.set((s.get().0, true)));
    VEIL.with(|v| unsafe {
        let mut slot = v.borrow_mut();
        let Some(veil) = slot.as_mut() else { return; };
        let (x, w) = (f64::from(frame.origin.x - veil.frame.origin.x), f64::from(frame.size.width));
        let y = f64::from(frame.origin.y - veil.frame.origin.y);
        // 只有一行时竖直居中。
        let (ty, sy) = if folder.is_empty() { (y + 7., y + 27.) } else { (y, y + 20.) };
        let _: () = msg_send![veil.title, setFrame: CGRect::new(CGPoint::new(x, ty), CGSize::new(w, 20.))];
        let _: () = msg_send![veil.sub, setFrame: CGRect::new(CGPoint::new(x, sy), CGSize::new(w, 14.))];
        let _: () = msg_send![veil.title, setTextColor: super::glass::ui_color(ink)];
        let _: () = msg_send![veil.sub, setTextColor: super::glass::ui_color(sub)];
        if veil.text.0 != title || veil.text.1 != folder {
            let fresh = veil.text.0.is_empty();
            veil.text = (title.to_string(), folder.to_string());
            let _: () = msg_send![veil.title, setText: ns_string(title)];
            let _: () = msg_send![veil.sub, setText: ns_string(folder)];
            if fresh { fade(veil.title, 1.); fade(veil.sub, 1.); }
        }
    });
}

unsafe fn ns_string(s: &str) -> *mut AnyObject {
    let c = std::ffi::CString::new(s).unwrap_or_default();
    unsafe { msg_send![AnyClass::get(c"NSString").unwrap(), stringWithUTF8String: c.as_ptr()] }
}
