//! 手机上的提示（照 Craft）：顶上两颗按钮之间的一条玻璃胶囊，一行粗一点的话，下面可以带一行能点的小字
//! （删除之后的「打开回收站」）。状态还是 GPUI 那一份（`Workspace::toast`）：它挂着的每一帧放一个占位，没有了就淡出。
use std::cell::{Cell, RefCell};

use gpui::*;
use objc2::msg_send;
use objc2::runtime::{AnyClass, AnyObject};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

struct Toast { view: *mut AnyObject, title: *mut AnyObject, sub: *mut AnyObject, hit: *mut AnyObject, frame: Bounds<Pixels>, text: (String, String, bool), dark: Option<bool>, shown: bool }
thread_local! { static TOAST: RefCell<Option<Toast>> = const { RefCell::new(None) }; }
thread_local! { static SEEN: Cell<bool> = const { Cell::new(false) }; }
// 这条提示下面那行小字和点了做什么（`ios::menu_action` 的标识）。跟着提示一起换。
thread_local! { static LINK: RefCell<Option<(SharedString, &'static str)>> = const { RefCell::new(None) }; }

/// 用不用这种提示：和玻璃同一个开关。
pub fn native() -> bool { super::is_compact() && super::glass::supported() }
/// 新的一条提示出来时清掉上一条的小字；要带小字的在 `show_toast` 之后再 `link`。
/// 留多久（秒）：带入口的多留一会儿。
pub fn linger() -> f32 { if LINK.with(|l| l.borrow().is_some()) { 4.5 } else { 2.7 } }
pub fn clear_link() { LINK.with(|l| *l.borrow_mut() = None); }
pub fn link(label: impl Into<SharedString>, action: &'static str) { LINK.with(|l| *l.borrow_mut() = Some((label.into(), action))); }

/// 每帧开头调用：这一帧没有提示就淡出。
pub fn begin(window: &Window) {
    SEEN.with(|s| s.set(false));
    window.on_next_frame(|_, _| if !SEEN.with(|s| s.get()) { TOAST.with(|t| if let Some(toast) = t.borrow_mut().as_mut().filter(|t| t.shown) { unsafe {
        toast.shown = false;
        let _: () = msg_send![toast.view, setUserInteractionEnabled: false];
        slide(toast.view, false);
    } }) });
}

// 出来时从上面落下来一点并淡入，走的时候反过来。
unsafe fn slide(view: *mut AnyObject, on: bool) {
    unsafe {
        #[repr(C)]
        #[derive(Clone, Copy)]
        struct Affine { a: f64, b: f64, c: f64, d: f64, tx: f64, ty: f64 }
        unsafe impl objc2::Encode for Affine { const ENCODING: objc2::Encoding = objc2::Encoding::Struct("CGAffineTransform", &[const { objc2::Encoding::Double }; 6]); }
        let shift = |ty: f64| Affine { a: 1., b: 0., c: 0., d: 1., tx: 0., ty };
        if on { let _: () = msg_send![view, setTransform: shift(-14.)]; }
        let addr = view as usize;
        let body = block2::RcBlock::new(move || {
            let _: () = msg_send![addr as *mut AnyObject, setAlpha: if on { 1f64 } else { 0f64 }];
            let _: () = msg_send![addr as *mut AnyObject, setTransform: shift(if on { 0. } else { -14. })];
        });
        let none: Option<&block2::Block<dyn Fn(objc2::runtime::Bool)>> = None;
        // 弹一下落定（阻尼 0.8）。2 = UIViewAnimationOptionAllowUserInteraction：落下来的那 0.32 秒里点「撤销」也算数。
        let _: () = msg_send![AnyClass::get(c"UIView").unwrap(), animateWithDuration: 0.32f64, delay: 0f64, usingSpringWithDamping: 0.8f64, initialSpringVelocity: 0f64, options: 2usize, animations: &*body, completion: none];
    }
}

/// 提示的占位：胶囊摆在这个元素的位置上。`danger` 时第一行是红的。
pub fn slot(title: SharedString, danger: bool) -> impl IntoElement {
    canvas(move |bounds, window, _| place(bounds, &title, danger, window), |_, _, _, _| {}).absolute().inset_0()
}

fn place(frame: Bounds<Pixels>, title: &str, danger: bool, window: &Window) {
    let Ok(handle) = HasWindowHandle::window_handle(window) else { return; };
    let RawWindowHandle::UiKit(handle) = handle.as_raw() else { return; };
    let parent = super::shot::overlay(handle.ui_view.as_ptr() as *mut AnyObject);
    SEEN.with(|s| s.set(true));
    let dark = crate::theme::dark();
    let link = LINK.with(|l| l.borrow().clone());
    TOAST.with(|t| unsafe {
        let mut slot = t.borrow_mut();
        if slot.is_none() {
            let (Some(effect_view), Some(glass), Some(label)) = (AnyClass::get(c"UIVisualEffectView"), AnyClass::get(c"UIGlassEffect"), AnyClass::get(c"UILabel")) else { return; };
            let view: *mut AnyObject = msg_send![effect_view, alloc];
            let effect: *mut AnyObject = msg_send![glass, effectWithStyle: 0isize];
            let view: *mut AnyObject = msg_send![view, initWithEffect: effect];
            let _: () = msg_send![view, setAlpha: 0f64];
            let _: () = msg_send![parent, addSubview: view];
            let content: *mut AnyObject = msg_send![view, contentView];
            let make = |size: f64, weight: f64| {
                let l: *mut AnyObject = msg_send![label, new];
                let font: *mut AnyObject = msg_send![AnyClass::get(c"UIFont").unwrap(), systemFontOfSize: size, weight: weight];
                let _: () = msg_send![l, setFont: font];
                // 1 = NSTextAlignmentCenter，5 = NSLineBreakByTruncatingMiddle（名字长时两头都留着）
                let _: () = msg_send![l, setTextAlignment: 1isize];
                let _: () = msg_send![l, setLineBreakMode: 5isize];
                let _: () = msg_send![content, addSubview: l];
                l
            };
            let (title, sub) = (make(15., 0.3), make(13., 0.));
            // 整条都能点：有小字时做它说的那件事。
            let hit: *mut AnyObject = msg_send![AnyClass::get(c"UIButton").unwrap(), buttonWithType: 0isize];
            let handler = block2::RcBlock::new(move |_action: *mut AnyObject| if let Some((_, action)) = LINK.with(|l| l.borrow().clone()) { super::glass::haptic(); super::menu::chosen(action); });
            let action: *mut AnyObject = msg_send![AnyClass::get(c"UIAction").unwrap(), actionWithTitle: ns_string(""), image: std::ptr::null_mut::<AnyObject>(), identifier: std::ptr::null_mut::<AnyObject>(), handler: &*handler];
            let _: () = msg_send![hit, addAction: action, forControlEvents: 64usize];
            let _: () = msg_send![content, addSubview: hit];
            *slot = Some(Toast { view, title, sub, hit, frame: Bounds::default(), text: Default::default(), dark: None, shown: false });
        }
        let toast = slot.as_mut().unwrap();
        let sub = link.as_ref().map(|l| l.0.to_string()).unwrap_or_default();
        let text = (title.to_string(), sub, danger);
        if toast.frame != frame || toast.text != text {
            let (w, h) = (f64::from(frame.size.width), f64::from(frame.size.height));
            if !toast.shown || toast.frame != frame {
                // 位置用 center / bounds 摆：出场动画在动 transform，这时不能设 frame。
                let _: () = msg_send![toast.view, setBounds: CGRect::new(CGPoint::new(0., 0.), CGSize::new(w, h))];
                let _: () = msg_send![toast.view, setCenter: CGPoint::new(f64::from(frame.origin.x) + w / 2., f64::from(frame.origin.y) + h / 2.)];
                if let (Some(corners), Some(corner)) = (AnyClass::get(c"UICornerConfiguration"), AnyClass::get(c"UICornerRadius")) {
                    let fixed: *mut AnyObject = msg_send![corner, fixedRadius: h / 2.];
                    let config: *mut AnyObject = msg_send![corners, configurationWithUniformRadius: fixed];
                    let _: () = msg_send![toast.view, setCornerConfiguration: config];
                }
            }
            (toast.frame, toast.text) = (frame, text);
            let two = !toast.text.1.is_empty();
            let _: () = msg_send![toast.title, setFrame: CGRect::new(CGPoint::new(18., if two { h / 2. - 20. } else { h / 2. - 10. }), CGSize::new(w - 36., 20.))];
            let _: () = msg_send![toast.sub, setFrame: CGRect::new(CGPoint::new(18., h / 2. + 1.), CGSize::new(w - 36., 18.))];
            let _: () = msg_send![toast.hit, setFrame: CGRect::new(CGPoint::new(0., 0.), CGSize::new(w, h))];
            let _: () = msg_send![toast.title, setText: ns_string(&toast.text.0)];
            let _: () = msg_send![toast.sub, setText: ns_string(&toast.text.1)];
            let _: () = msg_send![toast.sub, setHidden: !two];
            let color = |name: &std::ffi::CStr| -> *mut AnyObject { msg_send![AnyClass::get(c"UIColor").unwrap(), performSelector: objc2::runtime::Sel::register(name)] };
            let _: () = msg_send![toast.title, setTextColor: color(if danger { c"systemRedColor" } else { c"labelColor" })];
            let _: () = msg_send![toast.sub, setTextColor: color(c"secondaryLabelColor")];
        }
        if toast.dark != Some(dark) { toast.dark = Some(dark); let _: () = msg_send![toast.view, setOverrideUserInterfaceStyle: if dark { 2isize } else { 1isize }]; }
        if !toast.shown {
            toast.shown = true;
            let _: () = msg_send![toast.view, setUserInteractionEnabled: true];
            let _: () = msg_send![parent, bringSubviewToFront: toast.view];
            slide(toast.view, true);
        }
    });
}

unsafe fn ns_string(s: &str) -> *mut AnyObject {
    let c = std::ffi::CString::new(s).unwrap_or_default();
    unsafe { msg_send![AnyClass::get(c"NSString").unwrap(), stringWithUTF8String: c.as_ptr()] }
}
