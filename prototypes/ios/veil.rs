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

struct Veil { view: *mut AnyObject, glass: *mut AnyObject, frost: *mut AnyObject, smoke: *mut AnyObject, milk: *mut AnyObject, haze: *mut AnyObject, rim: *mut AnyObject, title: *mut AnyObject, sub: *mut AnyObject, frame: Bounds<Pixels>, dark: Option<bool>, home: bool, centered: bool, text: (String, String), shown: bool }
thread_local! { static VEIL: RefCell<Option<Veil>> = const { RefCell::new(None) }; }
/// 首页左上角的空间头像（系统视图：外框、图片、上次的样子）。
/// `blur`：同一个头像（立起来的那张图）的高斯模糊版，过渡完露面时先是它、再化成清楚的（`reveal`）。
struct Avatar { view: *mut AnyObject, img: *mut AnyObject, key: String, blur: *mut AnyObject }
thread_local! { static AVATAR: RefCell<Option<Avatar>> = const { RefCell::new(None) }; }
// 系统头像已经连着显示了几帧（藏起时归零）。
thread_local! { static AVATAR_FRAMES: Cell<u32> = const { Cell::new(0) }; }
/// 系统头像已经稳稳在屏幕上了，GPUI 那份可以不画。刚露面的头几帧两份都画：真机上 GPUI 的一帧和原生视图的改动
/// 不一定同一下上屏，交接的那一帧两边都没有，头像就闪一下（换页滑完、玻璃回来的时候）。
pub fn avatar_settled() -> bool { on() && AVATAR_FRAMES.with(|f| f.get()) >= 3 }
// 这一帧有人要：带子、标题、首页的空间头像。
thread_local! { static SEEN: Cell<(bool, bool, bool)> = const { Cell::new((false, false, false)) }; }
// 这一帧允许显示带子。首页在绘制时读滚动位置，文档由正文的位置决定。
thread_local! { static ON: Cell<bool> = const { Cell::new(false) }; }

pub fn on() -> bool { ON.with(|o| o.get()) }
/// 正显示着的那条带子（连同上面的小字标题）：换页拍快照时把它也拍进去（`shot::capture_page`），
/// 不然它在原处一收，快照顶上就露出没有玻璃的正文。
pub fn band() -> Option<*mut AnyObject> { VEIL.with(|v| v.borrow().as_ref().filter(|v| v.shown).map(|v| v.view)) }

/// 每帧开头调用。`on`：这一帧要这条带子。带子或标题这一帧没人要，就各自收起。
pub fn begin(on: bool, window: &Window) {
    ON.with(|o| o.set(on));
    SEEN.with(|s| s.set((false, false, false)));
    if !on { VEIL.with(|v| if let Some(veil) = v.borrow_mut().as_mut() { unsafe {
        let layer: *mut AnyObject = msg_send![veil.view, layer];
        let _: () = msg_send![layer, removeAllAnimations];
        let _: () = msg_send![veil.view, setAlpha: 0f64];
        veil.shown = false;
    } }); }
    window.on_next_frame(|_, _| {
        let (band, title, avatar) = SEEN.with(|s| s.get());
        if !avatar { AVATAR_FRAMES.with(|f| f.set(0)); AVATAR.with(|a| if let Some(a) = a.borrow().as_ref() { unsafe { let _: () = msg_send![a.view, setHidden: true]; } }); }
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
/// `avatar`：左上角空间头像的位置和样子（图片，没有图片时的底色）。有带子的时候（`on`）头像一直是浮层里的一个系统视图，
/// 滚没滚都是它，GPUI 那边不画（`hub.rs`）：带子浮起来会把 GPUI 画的糊掉，而在两者之间来回换会闪。照 Craft：滚动后头像仍留在顶栏。
pub fn home_header(scroll: &ScrollHandle, title: SharedString, avatar: Option<(Bounds<Pixels>, Option<std::path::PathBuf>, Hsla)>) -> impl IntoElement {
    let scroll = scroll.clone();
    canvas(move |bounds, window, _| {
        if !on() { return; }
        let offset = -scroll.offset().y;
        if let Some((frame, image, color)) = &avatar { write_avatar(*frame, image.as_deref(), *color, window); }
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
    SEEN.with(|s| { let (_, t, a) = s.get(); s.set((true, t, a)) });
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
            // 首页浅色照 Craft（IMG_9146）：下面的字只剩约 13% 的深浅、微微发糊，还认得出。系统材质糊得太开（字整个没了），
            // 所以这时不用玻璃和材质：一层只糊一点点的磨砂（`haze`，系统模糊停在动画的一小段上），上面一层八成的白。
            let milk: *mut AnyObject = msg_send![AnyClass::get(c"CALayer").unwrap(), layer];
            let wash: *const CGColor = msg_send![super::glass::ui_color(crate::theme::rgbaf(255, 255, 255, 0.8)), CGColor];
            let _: () = msg_send![milk, setBackgroundColor: wash];
            let _: () = msg_send![milk, setHidden: true];
            let _: () = msg_send![host, addSublayer: milk];
            let haze: *mut AnyObject = msg_send![effect_view, alloc];
            let haze: *mut AnyObject = msg_send![haze, initWithEffect: std::ptr::null_mut::<AnyObject>()];
            let _: () = msg_send![haze, setHidden: true];
            let _: () = msg_send![view, insertSubview: haze, atIndex: 0isize];
            let soft: *mut AnyObject = msg_send![blur, effectWithStyle: 1isize];
            let (haze_addr, soft_addr) = (haze as usize, soft as usize);
            let body = block2::RcBlock::new(move || { let _: () = msg_send![haze_addr as *mut AnyObject, setEffect: soft_addr as *mut AnyObject]; });
            let animator: *mut AnyObject = msg_send![AnyClass::get(c"UIViewPropertyAnimator").unwrap(), alloc];
            // 3 = UIViewAnimationCurveLinear。动画一直停着、一直留着（停着的动画不能放掉）。
            let animator: *mut AnyObject = msg_send![animator, initWithDuration: 1f64, curve: 3isize, animations: &*body];
            let _: () = msg_send![animator, setPausesOnCompletion: true];
            let _: () = msg_send![animator, pauseAnimation];
            let _: () = msg_send![animator, setFractionComplete: 0.08f64];
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
            *slot = Some(Veil { view, glass: material, frost, smoke, milk, haze, rim, title: make(15., 0.3), sub: make(11., 0.), frame: Bounds::default(), dark: None, home: false, centered: false, text: Default::default(), shown: false });
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
            let _: () = msg_send![veil.milk, setFrame: CGRect::new(CGPoint::new(0., 0.), rect.size)];
            let _: () = msg_send![veil.haze, setFrame: CGRect::new(CGPoint::new(0., 0.), rect.size)];
            let _: () = msg_send![veil.rim, setFrame: CGRect::new(CGPoint::new(0., rect.size.height - 1.), CGSize::new(rect.size.width, 1.))];
        }
        if veil.dark != Some(dark) || veil.home != home {
            veil.dark = Some(dark);
            veil.home = home;
            let _: () = msg_send![veil.view, setOverrideUserInterfaceStyle: if dark { 2isize } else { 1isize }];
            let _: () = msg_send![veil.smoke, setHidden: !smoked];
            let milky = home && !dark;
            let _: () = msg_send![veil.glass, setHidden: milky];
            let _: () = msg_send![veil.frost, setHidden: milky];
            let _: () = msg_send![veil.haze, setHidden: !milky];
            let _: () = msg_send![veil.milk, setHidden: !milky];
            let _: () = msg_send![veil.rim, setOpacity: if smoked { 0.28f32 } else if milky { 0.5f32 } else { 1f32 }];
        }
        if !veil.shown { veil.shown = true; fade(veil.view, 1.); }
    });
}

/// 空间头像：和 `space::icon_el` 同样的圆角方块，有图片铺图片，没有就是底色。放在浮层里、带子的上面，不跟着带子淡入淡出。
fn write_avatar(frame: Bounds<Pixels>, image: Option<&std::path::Path>, color: Hsla, window: &Window) {
    SEEN.with(|s| { let (b, t, _) = s.get(); s.set((b, t, true)) });
    let Ok(handle) = HasWindowHandle::window_handle(window) else { return; };
    let RawWindowHandle::UiKit(handle) = handle.as_raw() else { return; };
    let parent = super::shot::overlay(handle.ui_view.as_ptr() as *mut AnyObject);
    AVATAR.with(|slot| unsafe {
        let mut slot = slot.borrow_mut();
        if slot.is_none() {
            let view: *mut AnyObject = msg_send![AnyClass::get(c"UIView").unwrap(), new];
            let _: () = msg_send![view, setClipsToBounds: true];
            let _: () = msg_send![view, setUserInteractionEnabled: false];
            let img: *mut AnyObject = msg_send![AnyClass::get(c"UIImageView").unwrap(), new];
            // 2 = UIViewContentModeScaleAspectFill，同 `ObjectFit::Cover`。
            let _: () = msg_send![img, setContentMode: 2isize];
            let _: () = msg_send![view, addSubview: img];
            let _: () = msg_send![parent, addSubview: view];
            let blur: *mut AnyObject = msg_send![AnyClass::get(c"UIImageView").unwrap(), new];
            let _: () = msg_send![blur, setUserInteractionEnabled: false];
            let _: () = msg_send![blur, setAlpha: 0f64];
            let _: () = msg_send![parent, addSubview: blur];
            *slot = Some(Avatar { view, img, key: String::new(), blur });
        }
        let a = slot.as_mut().unwrap();
        // 带子后建时会插到最底下，头像总在它上面；这里只保证还在浮层里。
        let sup: *mut AnyObject = msg_send![a.view, superview];
        if sup != parent { let _: () = msg_send![parent, addSubview: a.view]; let _: () = msg_send![parent, addSubview: a.blur]; }
        // 头像是立起来的样子（`tilt`）：一张已经转好角度、带影子的图，比头像四周各大 `tilt::PAD`。和 GPUI 那份同一张图。
        let size = f64::from(frame.size.width);
        let pad = size * f64::from(super::tilt::PAD);
        let outer = CGRect::new(CGPoint::new(f64::from(frame.origin.x) - pad, f64::from(frame.origin.y) - pad), CGSize::new(size + pad * 2., size + pad * 2.));
        let _: () = msg_send![a.view, setClipsToBounds: false];
        let _: () = msg_send![a.view, setFrame: outer];
        let _: () = msg_send![a.img, setFrame: CGRect::new(CGPoint::new(0., 0.), outer.size)];
        let tilted = super::tilt::image(image, color, frame.size.width.into(), super::tilt::ANGLE);
        let key = format!("{size}{}", tilted.as_ref().map(|p| p.display().to_string()).unwrap_or_default());
        if a.key != key {
            a.key = key;
            let picture: *mut AnyObject = match &tilted {
                Some(p) => msg_send![AnyClass::get(c"UIImage").unwrap(), imageWithContentsOfFile: ns_string(&p.to_string_lossy())],
                None => std::ptr::null_mut(),
            };
            let _: () = msg_send![a.img, setImage: picture];
            let clear: *mut AnyObject = msg_send![AnyClass::get(c"UIColor").unwrap(), clearColor];
            let _: () = msg_send![a.view, setBackgroundColor: clear];
            let _: () = msg_send![a.blur, setImage: blur_image(picture)];
        }
        let _: () = msg_send![a.blur, setFrame: outer];
        let was_hidden: bool = msg_send![a.view, isHidden];
        let _: () = msg_send![a.view, setHidden: false];
        // 返回首页的过渡刚完（照 Craft）：文档页的返回键先很快淡掉，这里稍晚一点从一团模糊里清楚起来，不是一下出现。
        if was_hidden && super::shot::just_slid() { reveal(a); }
    });
    AVATAR_FRAMES.with(|f| f.set(f.get().saturating_add(1)));
}

/// 头像从模糊到清楚：模糊版先浮出来再淡掉，清楚的那个同时淡入（都晚 `DELAY` 开始，让返回键先走）。
unsafe fn reveal(a: &Avatar) {
    const DELAY: f64 = 0.05;
    const DUR: f64 = 0.3;
    unsafe {
        let number = |v: f64| -> *mut AnyObject { msg_send![AnyClass::get(c"NSNumber").unwrap(), numberWithDouble: v] };
        let array = |vs: &[f64]| -> *mut AnyObject {
            let list: *mut AnyObject = msg_send![AnyClass::get(c"NSMutableArray").unwrap(), array];
            for v in vs { let _: () = msg_send![list, addObject: number(*v)]; }
            list
        };
        let ca = AnyClass::get(c"CATransaction").unwrap();
        let _: () = msg_send![ca, begin];
        let _: () = msg_send![ca, setDisableActions: true];
        let sharp: *mut AnyObject = msg_send![a.view, layer];
        let now: f64 = msg_send![sharp, convertTime: ca_now(), fromLayer: std::ptr::null_mut::<AnyObject>()];
        let anim: *mut AnyObject = msg_send![AnyClass::get(c"CAKeyframeAnimation").unwrap(), animationWithKeyPath: ns_string("opacity")];
        let _: () = msg_send![anim, setValues: array(&[0., 0., 1.])];
        let _: () = msg_send![anim, setKeyTimes: array(&[0., 0.35, 1.])];
        let _: () = msg_send![anim, setDuration: DUR];
        let _: () = msg_send![anim, setBeginTime: now + DELAY];
        let _: () = msg_send![anim, setFillMode: ns_string("backwards")];
        let _: () = msg_send![sharp, addAnimation: anim, forKey: ns_string("reveal")];
        let soft: *mut AnyObject = msg_send![a.blur, layer];
        let anim: *mut AnyObject = msg_send![AnyClass::get(c"CAKeyframeAnimation").unwrap(), animationWithKeyPath: ns_string("opacity")];
        let _: () = msg_send![anim, setValues: array(&[0., 0.85, 0.])];
        let _: () = msg_send![anim, setKeyTimes: array(&[0., 0.3, 1.])];
        let _: () = msg_send![anim, setDuration: DUR];
        let _: () = msg_send![anim, setBeginTime: now + DELAY];
        let _: () = msg_send![anim, setFillMode: ns_string("backwards")];
        let _: () = msg_send![soft, addAnimation: anim, forKey: ns_string("reveal")];
        let _: () = msg_send![ca, commit];
    }
}
fn ca_now() -> f64 { unsafe extern "C" { fn CACurrentMediaTime() -> f64; } unsafe { CACurrentMediaTime() } }

#[repr(C)]
struct CGImage { _private: [u8; 0] }
unsafe impl RefEncode for CGImage { const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("CGImage", &[])); }
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" { fn CGImageRelease(image: *mut CGImage); }
#[link(name = "CoreImage", kind = "framework")]
unsafe extern "C" {}

/// 一张图（立起来的头像）的高斯模糊版，大小不变。
unsafe fn blur_image(picture: *mut AnyObject) -> *mut AnyObject {
    unsafe {
        if picture.is_null() { return std::ptr::null_mut(); }
        let scale: f64 = msg_send![picture, scale];
        let cg: *mut CGImage = msg_send![picture, CGImage];
        let Some(ci_class) = AnyClass::get(c"CIImage") else { return std::ptr::null_mut(); };
        let input: *mut AnyObject = msg_send![ci_class, imageWithCGImage: cg];
        let extent: CGRect = msg_send![input, extent];
        let filter: *mut AnyObject = msg_send![AnyClass::get(c"CIFilter").unwrap(), filterWithName: ns_string("CIGaussianBlur")];
        if filter.is_null() { return std::ptr::null_mut(); }
        let _: () = msg_send![filter, setValue: input, forKey: ns_string("inputImage")];
        let radius: *mut AnyObject = msg_send![AnyClass::get(c"NSNumber").unwrap(), numberWithDouble: 4. * scale];
        let _: () = msg_send![filter, setValue: radius, forKey: ns_string("inputRadius")];
        let output: *mut AnyObject = msg_send![filter, outputImage];
        let context: *mut AnyObject = msg_send![AnyClass::get(c"CIContext").unwrap(), context];
        let out: *mut CGImage = msg_send![context, createCGImage: output, fromRect: extent];
        if out.is_null() { return std::ptr::null_mut(); }
        let image: *mut AnyObject = msg_send![AnyClass::get(c"UIImage").unwrap(), imageWithCGImage: out, scale: scale, orientation: 0isize];
        CGImageRelease(out);
        image
    }
}


fn write(frame: Bounds<Pixels>, title: &str, folder: &str, ink: Hsla, sub: Hsla, centered: bool) {
    SEEN.with(|s| { let (b, _, a) = s.get(); s.set((b, true, a)) });
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
