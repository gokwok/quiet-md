//! 空间头像的「立起来」的样子（照 Craft，IMG_9147 / 9148）：圆角方块微微转一个角度，一圈浅色的边、一道柔和的影子。GPUI 的图片转不了角度，所以画成一张图（PNG，放在临时目录），首页的两份头像（GPUI 那份和浮层里系统那份）
//! 和设置页都用同一张，交接时不会对不上。按 图片 / 颜色 / 大小 / 角度 / 深浅色 缓存，换了就是另一个文件。
use std::path::{Path, PathBuf};

use gpui::Hsla;
use objc2::msg_send;
use objc2::runtime::{AnyClass, AnyObject};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};

/// 画出来的图比头像大多少（每边，按头像边长的比例）：留给转过的角和影子。
pub const PAD: f32 = 0.28;
/// 首页和设置页平时的角度（度，负的是逆时针）。
pub const ANGLE: f32 = -7.;

#[repr(C)]
struct CGContext { _private: [u8; 0] }
unsafe impl objc2::encode::RefEncode for CGContext { const ENCODING_REF: objc2::Encoding = objc2::Encoding::Pointer(&objc2::Encoding::Struct("CGContext", &[])); }
#[repr(C)]
struct CGColor { _private: [u8; 0] }
unsafe impl objc2::encode::RefEncode for CGColor { const ENCODING_REF: objc2::Encoding = objc2::Encoding::Pointer(&objc2::Encoding::Struct("CGColor", &[])); }
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGContextTranslateCTM(c: *mut CGContext, tx: f64, ty: f64);
    fn CGContextRotateCTM(c: *mut CGContext, angle: f64);
    fn CGContextSaveGState(c: *mut CGContext);
    fn CGContextRestoreGState(c: *mut CGContext);
    fn CGContextSetShadowWithColor(c: *mut CGContext, offset: CGSize, blur: f64, color: *const CGColor);
}

fn dir() -> PathBuf { std::env::temp_dir().join("quiet-md-tilt") }

/// 头像 `size`（点）转 `angle` 度的样子，图的边长是 `size * (1 + 2 * PAD)`。画不出来时为空。
pub fn image(icon: Option<&Path>, color: Hsla, size: f32, angle: f32) -> Option<PathBuf> {
    use std::hash::{Hash, Hasher};
    let dark = crate::theme::dark();
    let mut h = std::hash::DefaultHasher::new();
    icon.map(|p| p.to_string_lossy().into_owned()).hash(&mut h);
    format!("{color:?}").hash(&mut h);
    (size.to_bits(), angle.to_bits(), dark).hash(&mut h);
    let path = dir().join(format!("{:016x}.png", h.finish()));
    if path.is_file() { return Some(path); }
    let _ = std::fs::create_dir_all(dir());
    unsafe { draw(icon, color, f64::from(size), f64::from(angle), dark, &path) }.then_some(path)
}

unsafe fn draw(icon: Option<&Path>, color: Hsla, size: f64, angle: f64, dark: bool, to: &Path) -> bool {
    unsafe {
        let full = size * (1. + 2. * f64::from(PAD));
        let picture: *mut AnyObject = match icon {
            Some(p) => { let s = std::ffi::CString::new(p.to_string_lossy().as_bytes()).unwrap_or_default();
                let ns: *mut AnyObject = msg_send![AnyClass::get(c"NSString").unwrap(), stringWithUTF8String: s.as_ptr()];
                msg_send![AnyClass::get(c"UIImage").unwrap(), imageWithContentsOfFile: ns] }
            None => std::ptr::null_mut(),
        };
        let fill = super::glass::ui_color(color);
        let format: *mut AnyObject = msg_send![AnyClass::get(c"UIGraphicsImageRendererFormat").unwrap(), preferredFormat];
        let _: () = msg_send![format, setOpaque: false];
        let renderer: *mut AnyObject = msg_send![AnyClass::get(c"UIGraphicsImageRenderer").unwrap(), alloc];
        let renderer: *mut AnyObject = msg_send![renderer, initWithSize: CGSize::new(full, full), format: format];
        let radius = (size * 0.24).round();
        let pic = picture as usize;
        let actions = block2::RcBlock::new(move |ctx: *mut AnyObject| {
            let c: *mut CGContext = msg_send![ctx, CGContext];
            let ui = AnyClass::get(c"UIColor").unwrap();
            CGContextTranslateCTM(c, full / 2., full / 2.);
            CGContextRotateCTM(c, angle.to_radians());
            let rect = CGRect::new(CGPoint::new(-size / 2., -size / 2.), CGSize::new(size, size));
            let path: *mut AnyObject = msg_send![AnyClass::get(c"UIBezierPath").unwrap(), bezierPathWithRoundedRect: rect, cornerRadius: radius];
            // 影子：往下偏一点，柔一点。
            CGContextSaveGState(c);
            let shade: *mut AnyObject = msg_send![ui, colorWithWhite: 0f64, alpha: if dark { 0.5f64 } else { 0.22f64 }];
            let shade: *const CGColor = msg_send![shade, CGColor];
            CGContextSetShadowWithColor(c, CGSize::new(0., size * 0.07), size * 0.16, shade);
            let _: () = msg_send![fill, setFill];
            let _: () = msg_send![path, fill];
            CGContextRestoreGState(c);
            // 图：铺满方块（同 `ObjectFit::Cover`）。
            CGContextSaveGState(c);
            let _: () = msg_send![path, addClip];
            if pic != 0 {
                let picture = pic as *mut AnyObject;
                let white: *mut AnyObject = msg_send![ui, whiteColor];
                let _: () = msg_send![white, setFill];
                let _: () = msg_send![path, fill];
                let s: CGSize = msg_send![picture, size];
                let k = (size / s.width.max(1.)).max(size / s.height.max(1.));
                let (w, h) = (s.width * k, s.height * k);
                let _: () = msg_send![picture, drawInRect: CGRect::new(CGPoint::new(-w / 2., -h / 2.), CGSize::new(w, h))];
            }
            CGContextRestoreGState(c);
            // 一圈浅色的边。
            let rim: *mut AnyObject = msg_send![ui, colorWithWhite: 1f64, alpha: if dark { 0.18f64 } else { 0.85f64 }];
            let _: () = msg_send![rim, setStroke];
            let edge: *mut AnyObject = msg_send![AnyClass::get(c"UIBezierPath").unwrap(), bezierPathWithRoundedRect: CGRect::new(CGPoint::new(-size / 2. + 0.75, -size / 2. + 0.75), CGSize::new(size - 1.5, size - 1.5)), cornerRadius: radius - 0.75];
            let _: () = msg_send![edge, setLineWidth: (size / 30.).clamp(1., 2.5)];
            let _: () = msg_send![edge, stroke];
        });
        let data: *mut AnyObject = msg_send![renderer, PNGDataWithActions: &*actions];
        let _: () = msg_send![renderer, release];
        if data.is_null() { return false; }
        let s = std::ffi::CString::new(to.to_string_lossy().as_bytes()).unwrap_or_default();
        let ns: *mut AnyObject = msg_send![AnyClass::get(c"NSString").unwrap(), stringWithUTF8String: s.as_ptr()];
        msg_send![data, writeToFile: ns, atomically: true]
    }
}

/// GPUI 里的立起来的头像：占 `size` 见方的位置（和平直的头像一样），图往四周多出 `PAD`。
pub fn element(icon: Option<&Path>, id: &str, size: f32, angle: f32) -> gpui::AnyElement {
    use gpui::{div, img, px, IntoElement, ParentElement, Styled};
    let color = crate::space::color(id);
    match image(icon, color, size, angle) {
        Some(p) => {
            let pad = size * PAD;
            div().size(px(size)).flex_none().relative()
                .child(img(p).absolute().left(px(-pad)).top(px(-pad)).size(px(size + 2. * pad)))
                .into_any_element()
        }
        None => crate::space::icon_el(icon, id, size),
    }
}

/// 会转动的头像：`ANGLE` 到 0 之间每半度一张图都放着（只露出当前那张），转的时候不用现解码，不会空一帧。
pub fn turning(icon: Option<&Path>, id: &str, size: f32, angle: f32) -> gpui::AnyElement {
    use gpui::{div, img, px, IntoElement, ParentElement, Styled};
    let color = crate::space::color(id);
    let pad = size * PAD;
    let steps = (ANGLE.abs() * 2.).round() as i32;
    let mut el = div().size(px(size)).flex_none().relative();
    for i in 0..=steps {
        let a = ANGLE * i as f32 / steps.max(1) as f32;
        let Some(p) = image(icon, color, size, a) else { continue };
        let on = (a - angle).abs() < 0.26;
        el = el.child(img(p).absolute().left(px(-pad)).top(px(-pad)).size(px(size + 2. * pad)).opacity(if on { 1. } else { 0. }));
    }
    el.into_any_element()
}
