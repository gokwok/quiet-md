//! 原生 Liquid Glass（iOS 26 起的 `UIGlassEffect`）。
//!
//! iOS 上 GPUI 的 Metal 画面不透明，玻璃只能盖在它上面，折射的就是 GPUI 画的内容。
//! 所以玻璃上的图标也得是原生视图：GPUI 画不到它上面。原生视图不接触摸，
//! 点击落到下面同一位置的 GPUI 元素上，逻辑仍只有一份。
use std::cell::RefCell;

use gpui::*;
use objc2::msg_send;
use objc2::runtime::{AnyClass, AnyObject};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

/// 系统有没有 Liquid Glass。没有时各处照旧用 GPUI 画。
/// `QUIET_MD_IOS_NO_GLASS`：当作没有，用来在新系统上检查旧系统的版式。
pub fn supported() -> bool { AnyClass::get(c"UIGlassEffect").is_some() && std::env::var_os("QUIET_MD_IOS_NO_GLASS").is_none() }

/// 玻璃上的图标：名字（`icons/` 下的，或带目录的完整路径）、边长、颜色。
pub type Icon = (&'static str, f32, Hsla);

#[derive(Clone, PartialEq)]
struct Look { frame: Bounds<Pixels>, tint: Option<Hsla>, icon: Option<Icon>, label: Option<(SharedString, Hsla)>, tabs: Vec<Icon>, picked: Option<(usize, Hsla)>, radius: Option<f32>, bare: bool, actions: Vec<&'static str>, dark: bool }
struct Pane { key: &'static str, view: *mut AnyObject, image: *mut AnyObject, text: *mut AnyObject, tabs: Vec<*mut AnyObject>, hits: Vec<*mut AnyObject>, pill: *mut AnyObject, look: Option<Look>, seen: bool, shown: bool }

thread_local! { static PANES: RefCell<Vec<Pane>> = const { RefCell::new(Vec::new()) }; }

thread_local! { static ALLOWED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
/// 每帧开头调用。`allowed` 为假时所有玻璃立刻藏起：原生视图压在 GPUI 的一切之上，
/// 抽屉、面板、对话框盖上来时必须让开。这一帧画完后，没被 `pane` 点到的也藏起
/// （它的元素已经不在画面里）。
pub fn begin(allowed: bool, window: &Window) {
    ALLOWED.with(|a| a.set(allowed));
    PANES.with(|p| for pane in p.borrow_mut().iter_mut() { pane.seen = false; if !allowed { pane.set_shown(false); } });
    window.on_next_frame(|_, _| PANES.with(|p| for pane in p.borrow_mut().iter_mut() { if !pane.seen { pane.set_shown(false); } }));
}

impl Pane {
    fn set_shown(&mut self, on: bool) {
        if self.shown == on { return; }
        self.shown = on;
        unsafe {
            let _: () = msg_send![self.view, setHidden: !on];
            // 露面时淡入，和 GPUI 那边换页、收面板的过渡接上；藏起是立刻的，免得压在新盖上来的东西上。
            if on {
                let anim: *mut AnyObject = msg_send![AnyClass::get(c"CABasicAnimation").unwrap(), animationWithKeyPath: ns_string("opacity")];
                let from: *mut AnyObject = msg_send![AnyClass::get(c"NSNumber").unwrap(), numberWithDouble: 0f64];
                let _: () = msg_send![anim, setFromValue: from];
                let _: () = msg_send![anim, setDuration: 0.24f64];
                let layer: *mut AnyObject = msg_send![self.view, layer];
                let _: () = msg_send![layer, addAnimation: anim, forKey: ns_string("appear")];
            }
        }
    }
}

/// 放在 GPUI 元素里的占位：把这个元素的位置交给同名的玻璃。胶囊形，圆角跟着短边走。
/// 只有图标时图标居中；带文字时图标靠左、文字跟在后面（搜索条）。
pub fn pane(key: &'static str, tint: Option<Hsla>, icon: Option<Icon>, label: Option<(SharedString, Hsla)>) -> impl IntoElement {
    canvas(move |bounds, window, _| place(key, Look { frame: bounds, tint, icon, label, tabs: vec![], picked: None, radius: None, bare: false, actions: vec![], dark: crate::theme::dark() }, window), |_, _, _, _| {}).absolute().inset_0()
}

/// 圆角矩形的一块玻璃：自己拼的弹出菜单的底。首页的菜单已改用系统菜单（`menu.rs`），这两个留给系统菜单放不下的内容。
#[allow(dead_code)]
pub fn panel(key: &'static str, radius: f32) -> impl IntoElement {
    canvas(move |bounds, window, _| place(key, Look { frame: bounds, tint: None, icon: None, label: None, tabs: vec![], picked: None, radius: Some(radius), bare: false, actions: vec![], dark: crate::theme::dark() }, window), |_, _, _, _| {}).absolute().inset_0()
}
/// 玻璃菜单里的一行：自己没有玻璃，只有图标和字，叠在 `panel` 上面。
#[allow(dead_code)]
pub fn row(key: &'static str, icon: Icon, label: (SharedString, Hsla)) -> impl IntoElement {
    canvas(move |bounds, window, _| place(key, Look { frame: bounds, tint: None, icon: Some(icon), label: Some(label.clone()), tabs: vec![], picked: None, radius: Some(0.), bare: true, actions: vec![], dark: crate::theme::dark() }, window), |_, _, _, _| {}).absolute().inset_0()
}

/// 自己接触摸的玻璃圆钮：按下时的拉伸和高光由系统给，点了把 `action` 交给 `ios::menu_action`。
/// 这块地方的点击不再经过 GPUI。
pub fn tap_pane(key: &'static str, icon: Icon, action: &'static str) -> impl IntoElement {
    canvas(move |bounds, window, _| place(key, Look { frame: bounds, tint: None, icon: Some(icon), label: None, tabs: vec![], picked: None, radius: None, bare: false, actions: vec![action], dark: crate::theme::dark() }, window), |_, _, _, _| {}).absolute().inset_0()
}

/// 标签栏：一条玻璃上等分排开几个图标，`picked` 是当前那一格和垫在它下面的底色；`actions` 是各格点了交出去的标识
/// （给了就由玻璃自己接触摸，同 `tap_pane`）。
pub fn tab_pane(key: &'static str, tabs: Vec<Icon>, picked: Option<(usize, Hsla)>, actions: Vec<&'static str>) -> impl IntoElement {
    canvas(move |bounds, window, _| place(key, Look { frame: bounds, tint: None, icon: None, label: None, tabs: tabs.clone(), picked, radius: None, bare: false, actions: actions.clone(), dark: crate::theme::dark() }, window), |_, _, _, _| {}).absolute().inset_0()
}

/// 按下时玻璃轻轻缩一下再弹回：GPUI 画不到玻璃上面，按压的反馈只能由原生视图给。
pub fn pulse(key: &'static str) {
    PANES.with(|p| if let Some(pane) = p.borrow().iter().find(|pane| pane.key == key) { unsafe {
        let anim: *mut AnyObject = msg_send![AnyClass::get(c"CABasicAnimation").unwrap(), animationWithKeyPath: ns_string("transform.scale")];
        let to: *mut AnyObject = msg_send![AnyClass::get(c"NSNumber").unwrap(), numberWithDouble: 0.9f64];
        let _: () = msg_send![anim, setToValue: to];
        let _: () = msg_send![anim, setDuration: 0.11f64];
        let _: () = msg_send![anim, setAutoreverses: true];
        let layer: *mut AnyObject = msg_send![pane.view, layer];
        let _: () = msg_send![layer, addAnimation: anim, forKey: ns_string("pulse")];
    } });
}

/// 轻的触感反馈：长按出菜单、点悬浮按钮。
pub fn haptic() {
    let Some(class) = AnyClass::get(c"UIImpactFeedbackGenerator") else { return; };
    unsafe {
        let generator: *mut AnyObject = msg_send![class, alloc];
        // 0 = UIImpactFeedbackStyleLight
        let generator: *mut AnyObject = msg_send![generator, initWithStyle: 0isize];
        let _: () = msg_send![generator, impactOccurred];
        let _: () = msg_send![generator, release];
    }
}

unsafe fn ns_string(s: &str) -> *mut AnyObject {
    let c = std::ffi::CString::new(s).unwrap_or_default();
    unsafe { msg_send![AnyClass::get(c"NSString").unwrap(), stringWithUTF8String: c.as_ptr()] }
}

fn place(key: &'static str, look: Look, window: &Window) {
    let Ok(handle) = HasWindowHandle::window_handle(window) else { return; };
    let RawWindowHandle::UiKit(handle) = handle.as_raw() else { return; };
    let parent = handle.ui_view.as_ptr() as *mut AnyObject;
    PANES.with(|p| unsafe {
        let mut panes = p.borrow_mut();
        let ix = match panes.iter().position(|pane| pane.key == key) {
            Some(ix) => ix,
            None => {
                let Some(class) = AnyClass::get(c"UIVisualEffectView") else { return; };
                let view: *mut AnyObject = msg_send![class, alloc];
                let view: *mut AnyObject = msg_send![view, initWithEffect: std::ptr::null_mut::<AnyObject>()];
                let _: () = msg_send![view, setUserInteractionEnabled: false];
                if let Some(corners) = AnyClass::get(c"UICornerConfiguration") {
                    let capsule: *mut AnyObject = msg_send![corners, capsuleConfiguration];
                    let _: () = msg_send![view, setCornerConfiguration: capsule];
                }
                let image: *mut AnyObject = msg_send![AnyClass::get(c"UIImageView").unwrap(), alloc];
                let image: *mut AnyObject = msg_send![image, init];
                let content: *mut AnyObject = msg_send![view, contentView];
                let pill: *mut AnyObject = msg_send![AnyClass::get(c"UIView").unwrap(), alloc];
                let pill: *mut AnyObject = msg_send![pill, init];
                let _: () = msg_send![pill, setHidden: true];
                let _: () = msg_send![content, addSubview: pill];
                let _: () = msg_send![content, addSubview: image];
                let text: *mut AnyObject = msg_send![AnyClass::get(c"UILabel").unwrap(), alloc];
                let text: *mut AnyObject = msg_send![text, init];
                let font: *mut AnyObject = msg_send![AnyClass::get(c"UIFont").unwrap(), systemFontOfSize: 16f64];
                let _: () = msg_send![text, setFont: font];
                let _: () = msg_send![content, addSubview: text];
                // 先藏着，位置定好再露面。
                let _: () = msg_send![view, setHidden: true];
                let _: () = msg_send![parent, addSubview: view];
                panes.push(Pane { key, view, image, text, tabs: vec![], hits: vec![], pill, look: None, seen: false, shown: false });
                panes.len() - 1
            }
        };
        let pane = &mut panes[ix];
        pane.seen = true;
        pane.set_shown(ALLOWED.with(|a| a.get()));
        if pane.look.as_ref() == Some(&look) { return; }
        let old = pane.look.take();
        let (x, y, w, h) = (f64::from(look.frame.origin.x), f64::from(look.frame.origin.y), f64::from(look.frame.size.width), f64::from(look.frame.size.height));
        let _: () = msg_send![pane.view, setFrame: CGRect::new(CGPoint::new(x, y), CGSize::new(w, h))];
        // 深浅色跟应用的设置，不跟系统。
        let _: () = msg_send![pane.view, setOverrideUserInterfaceStyle: if look.dark { 2isize } else { 1isize }];
        if old.as_ref().map(|o| o.radius) != Some(look.radius) && let Some(r) = look.radius && let (Some(corners), Some(corner)) = (AnyClass::get(c"UICornerConfiguration"), AnyClass::get(c"UICornerRadius")) {
            let fixed: *mut AnyObject = msg_send![corner, fixedRadius: f64::from(r)];
            let config: *mut AnyObject = msg_send![corners, configurationWithUniformRadius: fixed];
            let _: () = msg_send![pane.view, setCornerConfiguration: config];
        }
        if look.bare {
            // 没有自己的玻璃，但要压在作底的那块上面。
            let _: () = msg_send![pane.view, setEffect: std::ptr::null_mut::<AnyObject>()];
            let _: () = msg_send![parent, bringSubviewToFront: pane.view];
        } else if old.as_ref().map(|o| o.tint) != Some(look.tint) {
            let effect: *mut AnyObject = msg_send![AnyClass::get(c"UIGlassEffect").unwrap(), effectWithStyle: 0isize];
            let _: () = msg_send![effect, setInteractive: !look.actions.is_empty()];
            if let Some(tint) = look.tint { let _: () = msg_send![effect, setTintColor: ui_color(tint)]; }
            let _: () = msg_send![pane.view, setEffect: effect];
        }
        // 带文字时内容从左边 14pt 起排，否则图标居中。
        let mut left = 14.;
        if let Some((name, size, color)) = look.icon {
            if old.as_ref().and_then(|o| o.icon).map(|i| (i.0, i.1)) != Some((name, size)) {
                let _: () = msg_send![pane.image, setImage: icon_image(name, size)];
            }
            let _: () = msg_send![pane.image, setTintColor: ui_color(color)];
            let s = f64::from(size);
            let ix = if look.label.is_some() { left } else { (w - s) / 2. };
            let _: () = msg_send![pane.image, setFrame: CGRect::new(CGPoint::new(ix, (h - s) / 2.), CGSize::new(s, s))];
            left += s + 8.;
        }
        let _: () = msg_send![pane.image, setHidden: look.icon.is_none()];
        if let Some((label, color)) = &look.label {
            let _: () = msg_send![pane.text, setText: ns_string(label)];
            let _: () = msg_send![pane.text, setTextColor: ui_color(*color)];
            // 只有文字的按钮，字居中（1 = NSTextAlignmentCenter）。
            let _: () = msg_send![pane.text, setTextAlignment: if look.icon.is_none() { 1isize } else { 0isize }];
            let _: () = msg_send![pane.text, setFrame: CGRect::new(CGPoint::new(left, 0.), CGSize::new((w - left - 14.).max(0.), h))];
        }
        let _: () = msg_send![pane.text, setHidden: look.label.is_none()];
        // 标签栏的图标各占一格，居中；选中那格下面垫一块胶囊。
        let content: *mut AnyObject = msg_send![pane.view, contentView];
        while pane.tabs.len() < look.tabs.len() {
            let tab: *mut AnyObject = msg_send![AnyClass::get(c"UIImageView").unwrap(), alloc];
            let tab: *mut AnyObject = msg_send![tab, init];
            let _: () = msg_send![content, addSubview: tab];
            pane.tabs.push(tab);
        }
        let cell = if look.tabs.is_empty() { 0. } else { (w - 8.) / look.tabs.len() as f64 };
        for (i, (name, size, color)) in look.tabs.iter().enumerate() {
            if old.as_ref().and_then(|o| o.tabs.get(i)).map(|t| (t.0, t.1)) != Some((*name, *size)) {
                let _: () = msg_send![pane.tabs[i], setImage: icon_image(name, *size)];
            }
            let _: () = msg_send![pane.tabs[i], setTintColor: ui_color(*color)];
            let s = f64::from(*size);
            let _: () = msg_send![pane.tabs[i], setFrame: CGRect::new(CGPoint::new(4. + cell * i as f64 + (cell - s) / 2., (h - s) / 2.), CGSize::new(s, s))];
        }
        if let Some((i, color)) = look.picked {
            let _: () = msg_send![pane.pill, setFrame: CGRect::new(CGPoint::new(4. + cell * i as f64, 4.), CGSize::new(cell, h - 8.))];
            let _: () = msg_send![pane.pill, setBackgroundColor: ui_color(color)];
            let layer: *mut AnyObject = msg_send![pane.pill, layer];
            let _: () = msg_send![layer, setCornerRadius: (h - 8.) / 2.];
        }
        let _: () = msg_send![pane.pill, setHidden: look.picked.is_none()];
        // 自己接触摸的：每个动作一个透明的原生按钮，等分排开，压在图标上面。
        if old.is_none() { let _: () = msg_send![pane.view, setUserInteractionEnabled: !look.actions.is_empty()]; }
        while pane.hits.len() < look.actions.len() {
            let id = look.actions[pane.hits.len()];
            let hit: *mut AnyObject = msg_send![AnyClass::get(c"UIButton").unwrap(), buttonWithType: 0isize];
            let handler = block2::RcBlock::new(move |_action: *mut AnyObject| super::menu::chosen(id));
            let action: *mut AnyObject = msg_send![AnyClass::get(c"UIAction").unwrap(), actionWithTitle: ns_string(""), image: std::ptr::null_mut::<AnyObject>(), identifier: std::ptr::null_mut::<AnyObject>(), handler: &*handler];
            // 64 = UIControlEventTouchUpInside
            let _: () = msg_send![hit, addAction: action, forControlEvents: 64usize];
            let _: () = msg_send![content, addSubview: hit];
            pane.hits.push(hit);
        }
        let span = if pane.hits.is_empty() { 0. } else { w / pane.hits.len() as f64 };
        for (i, hit) in pane.hits.iter().enumerate() {
            let _: () = msg_send![*hit, setFrame: CGRect::new(CGPoint::new(span * i as f64, 0.), CGSize::new(span, h))];
            let _: () = msg_send![content, bringSubviewToFront: *hit];
        }
        pane.look = Some(look);
    });
}

pub(super) unsafe fn ui_color(color: Hsla) -> *mut AnyObject {
    let c = Rgba::from(color);
    unsafe { msg_send![AnyClass::get(c"UIColor").unwrap(), colorWithRed: f64::from(c.r), green: f64::from(c.g), blue: f64::from(c.b), alpha: f64::from(c.a)] }
}

/// 把资源里的 SVG 图标画成模板图：UIKit 不认 SVG，颜色由 `tintColor` 给。
unsafe fn icon_image(name: &str, size: f32) -> *mut AnyObject {
    const SCALE: f32 = 3.;
    let Ok(Some(bytes)) = AssetSource::load(&crate::assets::Assets, &if name.contains('/') { format!("{name}.svg") } else { format!("icons/{name}.svg") }) else { return std::ptr::null_mut(); };
    let source = String::from_utf8_lossy(&bytes).replace("currentColor", "#fff");
    let Ok(tree) = resvg::usvg::Tree::from_str(&source, &resvg::usvg::Options::default()) else { return std::ptr::null_mut(); };
    let side = (size * SCALE).round() as u32;
    let Some(mut pixmap) = resvg::tiny_skia::Pixmap::new(side, side) else { return std::ptr::null_mut(); };
    let k = side as f32 / tree.size().width();
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(k, k), &mut pixmap.as_mut());
    let Ok(png) = pixmap.encode_png() else { return std::ptr::null_mut(); };
    unsafe {
        let data: *mut AnyObject = msg_send![AnyClass::get(c"NSData").unwrap(), dataWithBytes: png.as_ptr() as *const std::ffi::c_void, length: png.len()];
        let image: *mut AnyObject = msg_send![AnyClass::get(c"UIImage").unwrap(), imageWithData: data, scale: f64::from(SCALE)];
        // 2 = UIImageRenderingModeAlwaysTemplate
        msg_send![image, imageWithRenderingMode: 2isize]
    }
}
