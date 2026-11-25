//! 原生 Liquid Glass（iOS 26 起的 `UIGlassEffect`）。
//!
//! iOS 上 GPUI 的 Metal 画面不透明，玻璃只能盖在它上面，折射的就是 GPUI 画的内容。
//! 所以玻璃上的图标也得是原生视图：GPUI 画不到它上面。原生视图不接触摸，
//! 点击落到下面同一位置的 GPUI 元素上，逻辑仍只有一份。
use std::cell::RefCell;

use gpui::*;
use objc2::msg_send;
use objc2::runtime::{AnyClass, AnyObject};
use objc2::{Encoding, RefEncode};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

/// 系统有没有 Liquid Glass。没有时各处照旧用 GPUI 画。
/// `QUIET_MD_IOS_NO_GLASS`：当作没有，用来在新系统上检查旧系统的版式。
pub fn supported() -> bool { AnyClass::get(c"UIGlassEffect").is_some() && std::env::var_os("QUIET_MD_IOS_NO_GLASS").is_none() }
/// 悬浮玻璃钮上图标和字的墨色：浅色下接近纯黑（#0C0C0B，实测 Craft iOS 的截图；系统菜单钮自己画出来的也是这个深浅），比正文的 `Theme::text()` 深一档；深色下就是正文色。
pub fn ink() -> Hsla { if crate::theme::dark() { crate::theme::Theme::text() } else { rgb(0x0c0c0b).into() } }

/// 玻璃上的图标：名字（`icons/` 下的，或带目录的完整路径）、边长、颜色。
pub type Icon = (&'static str, f32, Hsla);
type Tap = (&'static str, Option<&'static str>);

/// 玻璃上一个会动的图形（代替图标）：`size` 见方，坐标在 0..size 里，非零规则填一种颜色。每帧换一份就是动画。
#[derive(Clone, PartialEq)]
pub struct Shape { pub size: f32, pub color: Hsla, pub segs: Vec<Seg> }
#[derive(Clone, Copy, PartialEq)]
pub enum Seg { Move(f32, f32), Line(f32, f32), Cubic(f32, f32, f32, f32, f32, f32), Close }

// 调试构建会核对消息的编码：CGPath 要和系统的对上（同 crop.rs）。
#[repr(C)] struct CGPath { _private: [u8; 0] }
unsafe impl RefEncode for CGPath { const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("CGPath", &[])); }
#[repr(C)] struct CGColor { _private: [u8; 0] }
unsafe impl RefEncode for CGColor { const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("CGColor", &[])); }

#[derive(Clone, PartialEq)]
struct Look { frame: Bounds<Pixels>, tint: Option<Hsla>, icon: Option<Icon>, label: Option<(SharedString, Hsla)>, tabs: Vec<Icon>, picked: Option<(usize, Hsla)>, radius: Option<f32>, bare: bool, heavy: bool, bar: bool, stacked: bool, actions: Vec<Tap>, dark: bool, shape: Option<Shape>, clip: Option<Bounds<Pixels>> }
struct Pane { key: &'static str, view: *mut AnyObject, image: *mut AnyObject, text: *mut AnyObject, tabs: Vec<*mut AnyObject>, hits: Vec<*mut AnyObject>, pill: *mut AnyObject, shape: *mut AnyObject, look: Option<Look>, seen: bool, shown: bool }

thread_local! { static PANES: RefCell<Vec<Pane>> = const { RefCell::new(Vec::new()) }; }

/// 顶栏同一个位置上的按钮，各页共用一个原生视图：返回、「…」，和中间那颗（浏览页的「选择」、文档页的目录），还有右下角的「+」。
/// 各用各的视图时，换页结束那一下旧的淡出、新的长出来（`shot::show` 的交接），一模一样的按钮看着就是闪了一下。
pub(super) fn slot(key: &'static str) -> &'static str {
    match key {
        "doc-back" | "browse-back" | "batch-back" | "trash-back" | "settings-home" | "settings-back" => "top-back",
        "doc-catalogue" | "browse-select" => "top-middle",
        "doc-more" | "browse-options" | "hub-more" => "top-more",
        "fab" | "fab-menu" | "doc-add" => "dock-plus",
        _ => key,
    }
}

thread_local! { static ALLOWED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
/// 每帧开头调用。`allowed` 为假时所有玻璃立刻藏起：原生视图压在 GPUI 的一切之上，
/// 抽屉、面板、对话框盖上来时必须让开。这一帧画完后，没被 `pane` 点到的也藏起
/// （它的元素已经不在画面里）。
pub fn begin(allowed: bool, window: &Window) {
    super::shot::attach(window);
    ALLOWED.with(|a| a.set(allowed));
    PANES.with(|p| for pane in p.borrow_mut().iter_mut() { pane.seen = false; if !allowed { pane.set_shown(false); } });
    window.on_next_frame(|_, _| PANES.with(|p| for pane in p.borrow_mut().iter_mut() { if !pane.seen { pane.set_shown(false); } }));
}

impl Pane {
    fn set_shown(&mut self, on: bool) {
        use super::shot::Turn;
        if self.shown == on { return; }
        let turn = super::shot::turn(self.view, on);
        if turn == Turn::Later { return; }
        self.shown = on;
        unsafe {
            let interactive = on && self.look.as_ref().is_some_and(|look| !look.actions.is_empty());
            let _: () = msg_send![self.view, setUserInteractionEnabled: interactive];
            let content: *mut AnyObject = msg_send![self.view, contentView];
            let (view, content, key) = (self.view as usize, content as usize, self.key);
            let glass = self.look.as_ref().filter(|l| !l.bare).map_or(std::ptr::null_mut(), |l| effect(l)) as usize;
            if on {
                // 露面时玻璃从没有长出来、里面的内容一起淡入，和换页、收面板的过渡接上。只动图层的不透明度管不住玻璃：
                // 内容先没了，空的玻璃块还在原处（录屏 2026-11-25 07-05-17）。
                let start = block2::RcBlock::new(move || {
                    let _: () = msg_send![view as *mut AnyObject, setEffect: std::ptr::null_mut::<AnyObject>()];
                    let _: () = msg_send![content as *mut AnyObject, setAlpha: 0f64];
                    let _: () = msg_send![view as *mut AnyObject, setHidden: false];
                });
                let body = block2::RcBlock::new(move || {
                    let _: () = msg_send![view as *mut AnyObject, setEffect: glass as *mut AnyObject];
                    let _: () = msg_send![content as *mut AnyObject, setAlpha: 1f64];
                });
                let ui = AnyClass::get(c"UIView").unwrap();
                let _: () = msg_send![ui, performWithoutAnimation: &*start];
                let none: Option<&block2::Block<dyn Fn(objc2::runtime::Bool)>> = None;
                // 交接时晚一点开始：原处那颗先走（照 Craft）。4 = BeginFromCurrentState。
                let _: () = msg_send![ui, animateWithDuration: 0.24f64, delay: if turn == Turn::Soft { 0.06f64 } else { 0. }, options: 4usize, animations: &*body, completion: none];
            } else if turn == Turn::Soft {
                let body = block2::RcBlock::new(move || {
                    let _: () = msg_send![view as *mut AnyObject, setEffect: std::ptr::null_mut::<AnyObject>()];
                    let _: () = msg_send![content as *mut AnyObject, setAlpha: 0f64];
                });
                // 散完还没被叫回来才真的藏起。
                let done = block2::RcBlock::new(move |_: objc2::runtime::Bool| PANES.with(|p| if p.try_borrow().is_ok_and(|p| p.iter().any(|p| p.key == key && !p.shown)) { let _: () = msg_send![view as *mut AnyObject, setHidden: true]; }));
                let _: () = msg_send![AnyClass::get(c"UIView").unwrap(), animateWithDuration: 0.18f64, delay: 0f64, options: 4usize, animations: &*body, completion: &*done];
            } else {
                // 藏起是立刻的，免得压在新盖上来的东西上。
                let _: () = msg_send![self.view, setHidden: true];
            }
        }
    }
}
/// 这块玻璃的材质。
unsafe fn effect(look: &Look) -> *mut AnyObject {
    unsafe {
        let effect: *mut AnyObject = msg_send![AnyClass::get(c"UIGlassEffect").unwrap(), effectWithStyle: 0isize];
        let _: () = msg_send![effect, setInteractive: !look.actions.is_empty()];
        if let Some(tint) = look.tint { let _: () = msg_send![effect, setTintColor: ui_color(tint)]; }
        effect
    }
}

/// 放在 GPUI 元素里的占位：把这个元素的位置交给同名的玻璃。胶囊形，圆角跟着短边走。
/// 只有图标时图标居中；带文字时图标靠左、文字跟在后面（搜索条）。
pub fn pane(key: &'static str, tint: Option<Hsla>, icon: Option<Icon>, label: Option<(SharedString, Hsla)>) -> impl IntoElement {
    canvas(move |bounds, window, _| place(key, Look { frame: bounds, tint, icon, label, tabs: vec![], picked: None, radius: None, bare: false, heavy: false, bar: false, stacked: false, actions: vec![], dark: crate::theme::dark(), shape: None, clip: None }, window), |_, _, _, _| {}).absolute().inset_0()
}

/// 同 `pane`，但放在会滚动的页面内容里：滚进顶上的返回钮那一条、或出了屏幕底就藏起，免得压在顶栏的玻璃上。
pub fn page_pane(key: &'static str, tint: Option<Hsla>, icon: Option<Icon>, label: Option<(SharedString, Hsla)>) -> impl IntoElement {
    canvas(move |bounds, window, _| {
        if bounds.origin.y < px(super::chrome_top()) || bounds.bottom() > window.viewport_size().height { return; }
        place(key, Look { frame: bounds, tint, icon, label, tabs: vec![], picked: None, radius: None, bare: false, heavy: false, bar: false, stacked: false, actions: vec![], dark: crate::theme::dark(), shape: None, clip: None }, window)
    }, |_, _, _, _| {}).absolute().inset_0()
}

/// 圆角矩形的一块玻璃：自己拼的弹出菜单的底。首页的菜单已改用系统菜单（`menu.rs`），这两个留给系统菜单放不下的内容。
#[allow(dead_code)]
pub fn panel(key: &'static str, radius: f32) -> impl IntoElement {
    canvas(move |bounds, window, _| place(key, Look { frame: bounds, tint: None, icon: None, label: None, tabs: vec![], picked: None, radius: Some(radius), bare: false, heavy: false, bar: false, stacked: false, actions: vec![], dark: crate::theme::dark(), shape: None, clip: None }, window), |_, _, _, _| {}).absolute().inset_0()
}
/// 玻璃菜单里的一行：自己没有玻璃，只有图标和字，叠在 `panel` 上面。
#[allow(dead_code)]
pub fn row(key: &'static str, icon: Icon, label: (SharedString, Hsla)) -> impl IntoElement {
    canvas(move |bounds, window, _| place(key, Look { frame: bounds, tint: None, icon: Some(icon), label: Some(label.clone()), tabs: vec![], picked: None, radius: Some(0.), bare: true, heavy: false, bar: false, stacked: false, actions: vec![], dark: crate::theme::dark(), shape: None, clip: None }, window), |_, _, _, _| {}).absolute().inset_0()
}

/// 同 `row`，图标换成会动的图形（助手在想、在写时的标记）。
pub fn shape_row(key: &'static str, shape: Shape, label: (SharedString, Hsla)) -> impl IntoElement {
    canvas(move |bounds, window, _| place(key, Look { frame: bounds, tint: None, icon: None, label: Some(label.clone()), tabs: vec![], picked: None, radius: Some(0.), bare: true, heavy: false, bar: false, stacked: false, actions: vec![], dark: crate::theme::dark(), shape: Some(shape.clone()), clip: None }, window), |_, _, _, _| {}).absolute().inset_0()
}

/// 块动作栏的一格：原生图标在上、文字在下，点击仍交给下面的 GPUI。`fill` 是垫底的淡色（只有「删除」垫）。
/// 动作栏横着滚：原生视图不归 GPUI 裁，按这一格所在的裁剪范围（`content_mask`）给它套一层遮罩，滚到边上的只露出里面那一截。
pub fn tile(key: &'static str, icon: Icon, label: (SharedString, Hsla), fill: Option<Hsla>) -> impl IntoElement {
    canvas(move |bounds, window, _| place(key, Look { frame: bounds, tint: fill, icon: Some(icon), label: Some(label.clone()), tabs: vec![], picked: None, radius: Some(16.), bare: true, heavy: false, bar: false, stacked: true, actions: vec![], dark: crate::theme::dark(), shape: None, clip: Some(window.content_mask().bounds) }, window), |_, _, _, _| {}).absolute().inset_0()
}

/// 自己接触摸的玻璃圆钮：按下时的拉伸和高光由系统给，点了把 `action` 交给 `ios::menu_action`。
/// 这块地方的点击不再经过 GPUI。
pub fn tap_pane(key: &'static str, icon: Icon, action: &'static str, label: &'static str) -> impl IntoElement {
    canvas(move |bounds, window, _| place(key, Look { frame: bounds, tint: None, icon: Some(icon), label: None, tabs: vec![], picked: None, radius: None, bare: false, heavy: false, bar: false, stacked: false, actions: vec![(action, Some(label))], dark: crate::theme::dark(), shape: None, clip: None }, window), |_, _, _, _| {}).absolute().inset_0()
}

/// 标签栏：一条玻璃上等分排开几个图标，`picked` 是当前那一格和垫在它下面的底色；`actions` 是各格点了交出去的标识
/// （给了就由玻璃自己接触摸，同 `tap_pane`）。
pub fn tab_pane(key: &'static str, tabs: Vec<Icon>, picked: Option<(usize, Hsla)>, actions: Vec<&'static str>) -> impl IntoElement {
    let actions: Vec<_> = actions.into_iter().map(|id| (id, None)).collect();
    canvas(move |bounds, window, _| place(key, Look { frame: bounds, tint: None, icon: None, label: None, tabs: tabs.clone(), picked, radius: None, bare: false, heavy: false, bar: false, stacked: false, actions: actions.clone(), dark: crate::theme::dark(), shape: None, clip: None }, window), |_, _, _, _| {}).absolute().inset_0()
}

/// 导航栏上只有字的玻璃钮（浏览页的「选择」）：字是 17 号中粗。不跟着页面滚。
pub fn bar_pane(key: &'static str, label: SharedString, ink: Hsla) -> impl IntoElement {
    canvas(move |bounds, window, _| place(key, Look { frame: bounds, tint: None, icon: None, label: Some((label.clone(), ink)), tabs: vec![], picked: None, radius: None, bare: false, heavy: false, bar: true, stacked: false, actions: vec![], dark: crate::theme::dark(), shape: None, clip: None }, window), |_, _, _, _| {}).absolute().inset_0()
}
/// 同 `page_pane`，但字是半粗的：这一排里最要紧的那颗。
pub fn page_pane_heavy(key: &'static str, tint: Option<Hsla>, icon: Option<Icon>, label: Option<(SharedString, Hsla)>) -> impl IntoElement {
    canvas(move |bounds, window, _| {
        if bounds.origin.y < px(super::chrome_top()) || bounds.bottom() > window.viewport_size().height { return; }
        place(key, Look { frame: bounds, tint, icon, label, tabs: vec![], picked: None, radius: None, bare: false, heavy: true, bar: false, stacked: false, actions: vec![], dark: crate::theme::dark(), shape: None, clip: None }, window)
    }, |_, _, _, _| {}).absolute().inset_0()
}

/// 同 `tab_pane`，但放在会滚动的页面内容里（规矩同 `page_pane`）。
pub fn page_tab_pane(key: &'static str, tabs: Vec<Icon>, picked: Option<(usize, Hsla)>, actions: Vec<&'static str>) -> impl IntoElement {
    let actions: Vec<_> = actions.into_iter().map(|id| (id, None)).collect();
    canvas(move |bounds, window, _| {
        if bounds.origin.y < px(super::chrome_top()) || bounds.bottom() > window.viewport_size().height { return; }
        place(key, Look { frame: bounds, tint: None, icon: None, label: None, tabs: tabs.clone(), picked, radius: None, bare: false, heavy: false, bar: false, stacked: false, actions: actions.clone(), dark: crate::theme::dark(), shape: None, clip: None }, window)
    }, |_, _, _, _| {}).absolute().inset_0()
}

/// 按下时玻璃轻轻缩一下再弹回：GPUI 画不到玻璃上面，按压的反馈只能由原生视图给。
pub fn pulse(key: &'static str) {
    let key = slot(key);
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
    // 设置 › 外观 › 启用触感 关了就不震。
    if !crate::settings::prefs().haptics { return; }
    let Some(class) = AnyClass::get(c"UIImpactFeedbackGenerator") else { return; };
    unsafe {
        let generator: *mut AnyObject = msg_send![class, alloc];
        // 0 = UIImpactFeedbackStyleLight
        let generator: *mut AnyObject = msg_send![generator, initWithStyle: 0isize];
        let _: () = msg_send![generator, impactOccurred];
        let _: () = msg_send![generator, release];
    }
}

/// 删除：比平常的轻触厚、长一点。一下重的，紧接着一下稍轻的余震（约 70ms 后），合起来像一记「咚」。
pub(super) fn delete_haptic() {
    if !crate::settings::prefs().haptics { return; }
    let Some(class) = AnyClass::get(c"UIImpactFeedbackGenerator") else { return; };
    unsafe {
        // 2 = UIImpactFeedbackStyleHeavy
        let first: *mut AnyObject = msg_send![class, alloc];
        let first: *mut AnyObject = msg_send![first, initWithStyle: 2isize];
        let _: () = msg_send![first, impactOccurredWithIntensity: 1f64];
        let _: () = msg_send![first, release];
        // 余震：performSelector 在主线程排一下，过一会儿再震（它会持有生成器直到震完）。
        let echo: *mut AnyObject = msg_send![class, alloc];
        let echo: *mut AnyObject = msg_send![echo, initWithStyle: 2isize];
        let _: () = msg_send![echo, prepare];
        let _: () = msg_send![echo, performSelector: objc2::sel!(impactOccurred), withObject: std::ptr::null_mut::<AnyObject>(), afterDelay: 0.07f64];
        let _: () = msg_send![echo, release];
    }
}

/// 切到另一个选项时的轻触感。
pub(super) fn selection_haptic() {
    if !crate::settings::prefs().haptics { return; }
    let Some(class) = AnyClass::get(c"UISelectionFeedbackGenerator") else { return; };
    unsafe {
        let generator: *mut AnyObject = msg_send![class, new];
        let _: () = msg_send![generator, selectionChanged];
        let _: () = msg_send![generator, release];
    }
}

pub(crate) unsafe fn ns_string(s: &str) -> *mut AnyObject {
    let c = std::ffi::CString::new(s).unwrap_or_default();
    unsafe { msg_send![AnyClass::get(c"NSString").unwrap(), stringWithUTF8String: c.as_ptr()] }
}

/// 只露出 `frame` 里落在 `clip` 之内的那一截（没有 `clip` 或整个都在里面就不套遮罩）。
unsafe fn clip(view: *mut AnyObject, frame: Bounds<Pixels>, clip: Option<Bounds<Pixels>>) {
    unsafe {
        let layer: *mut AnyObject = msg_send![view, layer];
        let inside = clip.map(|c| c.intersect(&frame));
        let Some(inside) = inside.filter(|i| *i != frame) else { let _: () = msg_send![layer, setMask: std::ptr::null_mut::<AnyObject>()]; return };
        let ca = AnyClass::get(c"CATransaction").unwrap();
        let _: () = msg_send![ca, begin];
        let _: () = msg_send![ca, setDisableActions: true];
        let mut mask: *mut AnyObject = msg_send![layer, mask];
        if mask.is_null() {
            mask = msg_send![AnyClass::get(c"CALayer").unwrap(), layer];
            let black: *mut AnyObject = msg_send![AnyClass::get(c"UIColor").unwrap(), blackColor];
            let black: *const CGColor = msg_send![black, CGColor];
            let _: () = msg_send![mask, setBackgroundColor: black];
            let _: () = msg_send![layer, setMask: mask];
        }
        let o = inside.origin - frame.origin;
        let _: () = msg_send![mask, setFrame: CGRect::new(CGPoint::new(f64::from(o.x), f64::from(o.y)), CGSize::new(f64::from(inside.size.width).max(0.), f64::from(inside.size.height).max(0.)))];
        let _: () = msg_send![ca, commit];
    }
}

fn place(key: &'static str, look: Look, window: &Window) {
    let Ok(handle) = HasWindowHandle::window_handle(window) else { return; };
    let RawWindowHandle::UiKit(handle) = handle.as_raw() else { return; };
    let parent = super::shot::overlay(handle.ui_view.as_ptr() as *mut AnyObject);
    let key = slot(key);
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
                panes.push(Pane { key, view, image, text, tabs: vec![], hits: vec![], pill, shape: std::ptr::null_mut(), look: None, seen: false, shown: false });
                panes.len() - 1
            }
        };
        let pane = &mut panes[ix];
        pane.seen = true;
        let allowed = ALLOWED.with(|a| a.get());
        if pane.look.as_ref() == Some(&look) { pane.set_shown(allowed); return; }
        // 共用的那颗换了内容（「选择」↔ 目录）：页面还没动时先不换，一动就在原处化成新的，和别的按钮一起交接。
        let swap = pane.look.as_ref().is_some_and(|o| o.icon.map(|i| i.0) != look.icon.map(|i| i.0) || o.label.as_ref().map(|l| &l.0) != look.label.as_ref().map(|l| &l.0));
        if swap && pane.shown && super::shot::held() { return; }
        let swap = swap && pane.shown && super::shot::sliding();
        if swap { cross(pane.image); cross(pane.text); }
        let old = pane.look.take();
        let (x, y, w, h) = (f64::from(look.frame.origin.x), f64::from(look.frame.origin.y), f64::from(look.frame.size.width), f64::from(look.frame.size.height));
        let frame = CGRect::new(CGPoint::new(x, y), CGSize::new(w, h));
        if swap {
            let view = pane.view as usize;
            let body = block2::RcBlock::new(move || { let _: () = msg_send![view as *mut AnyObject, setFrame: frame]; });
            let none: Option<&block2::Block<dyn Fn(objc2::runtime::Bool)>> = None;
            let _: () = msg_send![AnyClass::get(c"UIView").unwrap(), animateWithDuration: 0.34f64, delay: 0f64, usingSpringWithDamping: 0.86f64, initialSpringVelocity: 0f64, options: 4usize, animations: &*body, completion: none];
        } else { let _: () = msg_send![pane.view, setFrame: frame]; }
        clip(pane.view, look.frame, look.clip);
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
            if look.stacked {
                let _: () = msg_send![pane.view, setBackgroundColor: ui_color(look.tint.unwrap_or_else(gpui::transparent_black))];
                let layer: *mut AnyObject = msg_send![pane.view, layer];
                let _: () = msg_send![layer, setCornerRadius: 16f64];
            }
        } else if old.as_ref().map(|o| (o.tint, o.actions.is_empty())) != Some((look.tint, look.actions.is_empty())) {
            let _: () = msg_send![pane.view, setEffect: effect(&look)];
        }
        // 带文字时内容从左边 14pt 起排，否则图标居中。
        let mut left = 14.;
        if let Some((name, size, color)) = look.icon {
            if old.as_ref().and_then(|o| o.icon).map(|i| (i.0, i.1)) != Some((name, size)) {
                let _: () = msg_send![pane.image, setImage: icon_image(&if look.stacked { format!("editor-icons/{name}") } else { name.to_string() }, size)];
            }
            let _: () = msg_send![pane.image, setTintColor: ui_color(color)];
            let s = f64::from(size);
            let ix = if look.label.is_some() && !look.stacked { left } else { (w - s) / 2. };
            let _: () = msg_send![pane.image, setFrame: CGRect::new(CGPoint::new(ix, if look.stacked { 8. } else { (h - s) / 2. }), CGSize::new(s, s))];
            left += s + 8.;
        }
        let _: () = msg_send![pane.image, setHidden: look.icon.is_none()];
        if let Some(shape) = &look.shape {
            let content: *mut AnyObject = msg_send![pane.view, contentView];
            if pane.shape.is_null() {
                pane.shape = msg_send![AnyClass::get(c"CAShapeLayer").unwrap(), layer];
                let host: *mut AnyObject = msg_send![content, layer];
                let _: () = msg_send![host, addSublayer: pane.shape];
            }
            let s = f64::from(shape.size);
            let path: *mut AnyObject = msg_send![AnyClass::get(c"UIBezierPath").unwrap(), bezierPath];
            let pt = |x: f32, y: f32| CGPoint::new(f64::from(x), f64::from(y));
            for seg in &shape.segs {
                match *seg {
                    Seg::Move(x, y) => { let _: () = msg_send![path, moveToPoint: pt(x, y)]; }
                    Seg::Line(x, y) => { let _: () = msg_send![path, addLineToPoint: pt(x, y)]; }
                    Seg::Cubic(ax, ay, bx, by, x, y) => { let _: () = msg_send![path, addCurveToPoint: pt(x, y), controlPoint1: pt(ax, ay), controlPoint2: pt(bx, by)]; }
                    Seg::Close => { let _: () = msg_send![path, closePath]; }
                }
            }
            let cg: *mut CGPath = msg_send![path, CGPath];
            let fill: *const CGColor = msg_send![ui_color(shape.color), CGColor];
            // 每帧换形状：关掉图层自带的隐式动画，不然每一帧都慢半拍地追。
            let ca = AnyClass::get(c"CATransaction").unwrap();
            let _: () = msg_send![ca, begin];
            let _: () = msg_send![ca, setDisableActions: true];
            let _: () = msg_send![pane.shape, setFrame: CGRect::new(CGPoint::new(left, (h - s) / 2.), CGSize::new(s, s))];
            let _: () = msg_send![pane.shape, setPath: cg];
            let _: () = msg_send![pane.shape, setFillColor: fill];
            let _: () = msg_send![ca, commit];
            left += s + 8.;
        }
        if !pane.shape.is_null() { let _: () = msg_send![pane.shape, setHidden: look.shape.is_none()]; }
        if let Some((label, color)) = &look.label {
            // `heavy` 的字是半粗（0.3 = UIFontWeightSemibold）；`bar` 是导航栏上的字钮：17 号中粗（0.23 = UIFontWeightMedium），量的 Craft。
            let font: *mut AnyObject = if look.heavy { msg_send![AnyClass::get(c"UIFont").unwrap(), systemFontOfSize: 16f64, weight: 0.3f64] }
                else if look.bar { msg_send![AnyClass::get(c"UIFont").unwrap(), systemFontOfSize: 17f64, weight: 0.23f64] }
                else { msg_send![AnyClass::get(c"UIFont").unwrap(), systemFontOfSize: if look.stacked { 11f64 } else { 16f64 }] };
            let _: () = msg_send![pane.text, setFont: font];
            let _: () = msg_send![pane.text, setText: ns_string(label)];
            let _: () = msg_send![pane.text, setTextColor: ui_color(*color)];
            // 只有文字的按钮，字居中（1 = NSTextAlignmentCenter）。
            let _: () = msg_send![pane.text, setTextAlignment: if look.icon.is_none() || look.stacked { 1isize } else { 0isize }];
            let frame = if look.stacked { CGRect::new(CGPoint::new(2., h - 23.), CGSize::new((w - 4.).max(0.), 16.)) } else { CGRect::new(CGPoint::new(left, 0.), CGSize::new((w - left - 14.).max(0.), h)) };
            let _: () = msg_send![pane.text, setFrame: frame];
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
            // 4 = UIViewContentModeCenter：SF Symbols 不是正方形，照原样居中。
            if name.starts_with("sf:") { let _: () = msg_send![pane.tabs[i], setContentMode: 4isize]; }
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
        if old.as_ref().map(|o| &o.actions) != Some(&look.actions) {
            let _: () = msg_send![pane.view, setUserInteractionEnabled: pane.shown && !look.actions.is_empty()];
            for hit in pane.hits.drain(..) { let _: () = msg_send![hit, removeFromSuperview]; }
            for &(id, label) in &look.actions {
                let hit: *mut AnyObject = msg_send![AnyClass::get(c"UIButton").unwrap(), buttonWithType: 0isize];
                if let Some(label) = label {
                    let _: () = msg_send![hit, setAccessibilityIdentifier: ns_string(id)];
                    let _: () = msg_send![hit, setAccessibilityLabel: ns_string(label)];
                }
                let handler = block2::RcBlock::new(move |_action: *mut AnyObject| super::menu::chosen(id));
                let action: *mut AnyObject = msg_send![AnyClass::get(c"UIAction").unwrap(), actionWithTitle: ns_string(""), image: std::ptr::null_mut::<AnyObject>(), identifier: std::ptr::null_mut::<AnyObject>(), handler: &*handler];
                // 64 = UIControlEventTouchUpInside
                let _: () = msg_send![hit, addAction: action, forControlEvents: 64usize];
                let _: () = msg_send![content, addSubview: hit];
                pane.hits.push(hit);
            }
        }
        let span = if pane.hits.is_empty() { 0. } else { w / pane.hits.len() as f64 };
        for (i, hit) in pane.hits.iter().enumerate() {
            let _: () = msg_send![*hit, setFrame: CGRect::new(CGPoint::new(span * i as f64, 0.), CGSize::new(span, h))];
            let _: () = msg_send![content, bringSubviewToFront: *hit];
        }
        pane.look = Some(look);
        // 样子摆好了再露面（露面的动画要知道是什么玻璃）。
        pane.set_shown(allowed);
    });
}
/// 这个视图接下来的改动化过去，不是一下换掉。
unsafe fn cross(view: *mut AnyObject) {
    unsafe {
        let fade: *mut AnyObject = msg_send![AnyClass::get(c"CATransition").unwrap(), animation];
        let _: () = msg_send![fade, setDuration: 0.2f64];
        let layer: *mut AnyObject = msg_send![view, layer];
        let _: () = msg_send![layer, addAnimation: fade, forKey: ns_string("cross")];
    }
}

pub(super) unsafe fn ui_color(color: Hsla) -> *mut AnyObject {
    let c = Rgba::from(color);
    unsafe { msg_send![AnyClass::get(c"UIColor").unwrap(), colorWithRed: f64::from(c.r), green: f64::from(c.g), blue: f64::from(c.b), alpha: f64::from(c.a)] }
}

/// 把资源里的 SVG 图标画成模板图：UIKit 不认 SVG，颜色由 `tintColor` 给。
pub(super) unsafe fn icon_image(name: &str, size: f32) -> *mut AnyObject {
    // `sf:名字`：系统的 SF Symbols（原生玻璃上的图标，不占图标名额）。
    if let Some(name) = name.strip_prefix("sf:") { unsafe {
        let config: *mut AnyObject = msg_send![AnyClass::get(c"UIImageSymbolConfiguration").unwrap(), configurationWithPointSize: f64::from(size) * 0.82, weight: 5isize];
        let image: *mut AnyObject = msg_send![AnyClass::get(c"UIImage").unwrap(), systemImageNamed: ns_string(name), withConfiguration: config];
        return if image.is_null() { image } else { msg_send![image, imageWithRenderingMode: 2isize] };
    } }
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
