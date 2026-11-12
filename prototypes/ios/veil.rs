//! 首页和文档共用的顶部 Liquid Glass：内容从下面滚过，系统负责折射、反光和深浅色适配。
//!
//! GPUI 没有模糊，所以是原生浮层最底下的一块系统材质；浮层盖在 GPUI 上面，顶栏中间的小字标题会被它糊掉，
//! 所以标题也跟着搬到这块材质上，用系统的标签写。每帧有占位才显示（`slot`、`title`），没有就收起。
use std::cell::{Cell, RefCell};

use gpui::*;
use objc2::{msg_send, Encoding, RefEncode};
use objc2::runtime::{AnyClass, AnyObject};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

struct Veil { view: *mut AnyObject, glass: *mut AnyObject, frost: *mut AnyObject, smoke: *mut AnyObject, rim: *mut AnyObject, title: *mut AnyObject, sub: *mut AnyObject, frame: Bounds<Pixels>, dark: Option<bool>, home: bool, centered: bool, text: (String, String), shown: bool }
thread_local! { static VEIL: RefCell<Option<Veil>> = const { RefCell::new(None) }; }
thread_local! { static SEEN: Cell<(bool, bool)> = const { Cell::new((false, false)) }; }
// 这一帧允许显示带子。首页在绘制时读滚动位置，文档由正文的位置决定。
thread_local! { static ON: Cell<bool> = const { Cell::new(false) }; }

pub fn on() -> bool { ON.with(|o| o.get()) }

/// 每帧开头调用。`on`：这一帧要这条带子。带子或标题这一帧没人要，就各自收起。
pub fn begin(on: bool, window: &Window) {
    ON.with(|o| o.set(on));
    SEEN.with(|s| s.set((false, false)));
    if !on { VEIL.with(|v| if let Some(veil) = v.borrow_mut().as_mut() { unsafe {
        let layer: *mut AnyObject = msg_send![veil.view, layer];
        let _: () = msg_send![layer, removeAllAnimations];
        let _: () = msg_send![veil.view, setAlpha: 0f64];
        veil.shown = false;
    } }); }
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
    canvas(move |bounds, window, _| place(bounds, dark, false, window), |_, _, _, _| {}).absolute().inset_0()
}

/// 标题的占位（顶栏中间那一格）：两行小字写在带子上。
pub fn title(title: SharedString, folder: Option<SharedString>, ink: Hsla, sub: Hsla) -> impl IntoElement {
    canvas(move |bounds, _, _| write(bounds, &title, folder.as_deref().unwrap_or(""), ink, sub, false), |_, _, _, _| {}).w_full().h(px(34.))
}

/// 首页的滚动标题。绘制时读取偏移，惯性滚动和玻璃的位置在同一帧更新。
pub fn home_header(scroll: &ScrollHandle, title: SharedString) -> impl IntoElement {
    let scroll = scroll.clone();
    canvas(move |bounds, window, _| {
        if !on() { return; }
        let offset = -scroll.offset().y;
        // 大标题在行内居中：以字号框的下沿完全进入玻璃为界，不等行高和整行底部的留白也滚进去。
        let title_end = px(super::hub::TOP_GAP + (super::LARGE_TITLE_H + super::LARGE_TITLE_SIZE) / 2.);
        VEIL.with(|v| if let Some(veil) = v.borrow_mut().as_mut() { unsafe {
            if offset <= px(1.) && veil.shown { veil.shown = false; fade(veil.view, 0.); }
            if offset < title_end && !veil.text.0.is_empty() { veil.text = Default::default(); fade(veil.title, 0.); fade(veil.sub, 0.); }
        } });
        if offset <= px(1.) { return; }
        place(bounds, crate::theme::dark(), true, window);
        if offset >= title_end {
            let frame = Bounds::new(point(bounds.left() + px(72.), bounds.top() + super::insets().top + px(9.)), size(bounds.size.width - px(144.), px(34.)));
            write(frame, &title, "", crate::theme::Theme::text(), crate::theme::Theme::text_caption(), true);
        }
    }, |_, _, _, _| {}).absolute().top_0().left_0().right_0().h(px(super::chrome_top()))
}

fn place(frame: Bounds<Pixels>, dark: bool, home: bool, window: &Window) {
    let smoked = dark && home;
    let Ok(handle) = HasWindowHandle::window_handle(window) else { return; };
    let RawWindowHandle::UiKit(handle) = handle.as_raw() else { return; };
    let parent = super::shot::overlay(handle.ui_view.as_ptr() as *mut AnyObject);
    SEEN.with(|s| s.set((true, s.get().1)));
    VEIL.with(|v| unsafe {
        let mut slot = v.borrow_mut();
        if slot.is_none() {
            let (Some(effect_view), Some(glass), Some(blur), Some(label)) = (AnyClass::get(c"UIVisualEffectView"), AnyClass::get(c"UIGlassEffect"), AnyClass::get(c"UIBlurEffect"), AnyClass::get(c"UILabel")) else { return; };
            // 薄磨砂先柔化正文，再由 Clear 玻璃折射：底色透得出来，但经过的大字不会像透镜一样重影。
            let effect: *mut AnyObject = msg_send![glass, effectWithStyle: 1isize];
            let _: () = msg_send![effect, setInteractive: false];
            let material: *mut AnyObject = msg_send![effect_view, alloc];
            let material: *mut AnyObject = msg_send![material, initWithEffect: effect];
            let view: *mut AnyObject = msg_send![AnyClass::get(c"UIView").unwrap(), new];
            let _: () = msg_send![view, setClipsToBounds: true];
            // 6 = UIBlurEffectStyleSystemUltraThinMaterial，跟随应用的深浅色。
            let diffuse: *mut AnyObject = msg_send![blur, effectWithStyle: 6isize];
            let frost: *mut AnyObject = msg_send![effect_view, alloc];
            let frost: *mut AnyObject = msg_send![frost, initWithEffect: diffuse];
            let _: () = msg_send![view, addSubview: frost];
            let _: () = msg_send![view, addSubview: material];
            let _: () = msg_send![view, setUserInteractionEnabled: false];
            if let (Some(corners), Some(radius)) = (AnyClass::get(c"UICornerConfiguration"), AnyClass::get(c"UICornerRadius")) {
                let edge: *mut AnyObject = msg_send![radius, fixedRadius: 28f64];
                let config: *mut AnyObject = msg_send![corners, configurationWithUniformRadius: edge];
                let _: () = msg_send![material, setCornerConfiguration: config];
            }
            let _: () = msg_send![view, setAlpha: 0f64];
            // 垫在浮层最底下：玻璃按钮在它上面。
            let _: () = msg_send![parent, insertSubview: view, atIndex: 0isize];
            let content = view;
            // 首页深色参考 IMG_9084：空白处约 3–7/255，原生双层材质约 61/255。
            // 只给材质加黑色渐变，标题和按钮仍在它上面；压掉系统材质抬起的灰底。
            let smoke: *mut AnyObject = msg_send![AnyClass::get(c"CAGradientLayer").unwrap(), layer];
            let shades: *mut AnyObject = msg_send![AnyClass::get(c"NSMutableArray").unwrap(), array];
            for alpha in [0.98, 0.94, 0.88] {
                let color: *const CGColor = msg_send![super::glass::ui_color(black().opacity(alpha)), CGColor];
                let _: () = msg_send![shades, addObject: color as *mut AnyObject];
            }
            let _: () = msg_send![smoke, setColors: shades];
            let host: *mut AnyObject = msg_send![view, layer];
            let _: () = msg_send![host, addSublayer: smoke];
            // 底边的窄高光随横向位置变化，强调玻璃厚度，不给整块面板描边。
            let rim: *mut AnyObject = msg_send![AnyClass::get(c"CAGradientLayer").unwrap(), layer];
            let colors: *mut AnyObject = msg_send![AnyClass::get(c"NSMutableArray").unwrap(), array];
            for alpha in [0.02, 0.18, 0.04, 0.12, 0.02] {
                let color: *const CGColor = msg_send![super::glass::ui_color(white().opacity(alpha)), CGColor];
                let _: () = msg_send![colors, addObject: color as *mut AnyObject];
            }
            let _: () = msg_send![rim, setColors: colors];
            let _: () = msg_send![rim, setStartPoint: CGPoint::new(0., 0.)];
            let _: () = msg_send![rim, setEndPoint: CGPoint::new(1., 1.)];
            let host: *mut AnyObject = msg_send![view, layer];
            let _: () = msg_send![host, addSublayer: rim];
            let make = |size: f64, weight: f64| {
                let l: *mut AnyObject = msg_send![label, new];
                let font: *mut AnyObject = msg_send![AnyClass::get(c"UIFont").unwrap(), systemFontOfSize: size, weight: weight];
                let _: () = msg_send![l, setFont: font];
                let _: () = msg_send![l, setAlpha: 0f64];
                let _: () = msg_send![content, addSubview: l];
                l
            };
            *slot = Some(Veil { view, glass: material, frost, smoke, rim, title: make(15., 0.3), sub: make(11., 0.), frame: Bounds::default(), dark: None, home: false, centered: false, text: Default::default(), shown: false });
        }
        let veil = slot.as_mut().unwrap();
        if veil.frame != frame || veil.dark != Some(dark) || veil.home != home {
            veil.frame = frame;
            let rect = CGRect::new(CGPoint::new(f64::from(frame.origin.x), f64::from(frame.origin.y)), CGSize::new(f64::from(frame.size.width), f64::from(frame.size.height)));
            let _: () = msg_send![veil.view, setFrame: rect];
            // 裁掉屏幕外的顶边和两侧折射，只保留底边；文字保持在原本的坐标里。
            let glass = CGRect::new(CGPoint::new(-40., -80.), CGSize::new(rect.size.width + 80., rect.size.height + 80.));
            let _: () = msg_send![veil.frost, setFrame: glass];
            // 首页深色不露出系统玻璃最亮的底边，只留下面那条很淡的高光。
            let glass = CGRect::new(glass.origin, CGSize::new(glass.size.width, glass.size.height + if smoked { 3. } else { 0. }));
            let _: () = msg_send![veil.glass, setFrame: glass];
            let _: () = msg_send![veil.smoke, setFrame: CGRect::new(CGPoint::new(0., 0.), rect.size)];
            let _: () = msg_send![veil.rim, setFrame: CGRect::new(CGPoint::new(0., rect.size.height - 1.), CGSize::new(rect.size.width, 1.))];
        }
        if veil.dark != Some(dark) || veil.home != home {
            veil.dark = Some(dark);
            veil.home = home;
            let _: () = msg_send![veil.view, setOverrideUserInterfaceStyle: if dark { 2isize } else { 1isize }];
            let _: () = msg_send![veil.smoke, setHidden: !smoked];
            let _: () = msg_send![veil.rim, setOpacity: if smoked { 0.28f32 } else { 1f32 }];
        }
        if !veil.shown { veil.shown = true; fade(veil.view, 1.); }
    });
}

fn write(frame: Bounds<Pixels>, title: &str, folder: &str, ink: Hsla, sub: Hsla, centered: bool) {
    SEEN.with(|s| s.set((s.get().0, true)));
    VEIL.with(|v| unsafe {
        let mut slot = v.borrow_mut();
        let Some(veil) = slot.as_mut() else { return; };
        let (x, w) = (f64::from(frame.origin.x - veil.frame.origin.x), f64::from(frame.size.width));
        let y = f64::from(frame.origin.y - veil.frame.origin.y);
        // 首页标题和「文件夹」共用字号，系统粗体对应 GPUI 的 FontWeight::BOLD。
        if veil.centered != centered {
            veil.centered = centered;
            let (size, weight) = if centered { (f64::from(super::hub::SECTION_TITLE_SIZE), 0.4) } else { (15., 0.3) };
            let font: *mut AnyObject = msg_send![AnyClass::get(c"UIFont").unwrap(), systemFontOfSize: size, weight: weight];
            let _: () = msg_send![veil.title, setFont: font];
        }
        let height = if centered { f64::from(super::hub::SECTION_TITLE_LINE) } else { 20. };
        let (ty, sy) = if folder.is_empty() { (y + (34. - height) / 2., y + 27.) } else { (y, y + 20.) };
        let _: () = msg_send![veil.title, setFrame: CGRect::new(CGPoint::new(x, ty), CGSize::new(w, height))];
        let _: () = msg_send![veil.sub, setFrame: CGRect::new(CGPoint::new(x, sy), CGSize::new(w, 14.))];
        let _: () = msg_send![veil.title, setTextAlignment: if centered { 1isize } else { 0isize }];
        let _: () = msg_send![veil.sub, setTextAlignment: if centered { 1isize } else { 0isize }];
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

#[repr(C)]
struct CGColor { _private: [u8; 0] }
unsafe impl RefEncode for CGColor { const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("CGColor", &[])); }
