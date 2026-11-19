//! 应用图标「¶」：一个段落符号，八种底色，设置 › 外观里换。默认的「朱」是包里的主图标，别的是备用图标
//! （Info.plist 的 `CFBundleAlternateIcons`，母版和 PNG 在 `prototypes/ios/icons`，`icons/build.sh` 重出）。
//! 换的时候系统自己会弹一句「图标已更改」。设置里的小样和关于页顶上的图标用同一份母版出的 276px 图，编进二进制。
use std::cell::Cell;
use std::sync::{Arc, LazyLock};

use gpui::*;
use objc2::msg_send;
use objc2::runtime::{AnyClass, AnyObject};

use crate::app::Workspace;
use crate::theme::Theme;

/// 名字、备用图标的键（主图标没有）、276px 的图。
const ICONS: [(&str, &str, Option<&str>, &[u8]); 8] = [
    ("朱", "Vermilion", None, include_bytes!("icons/Large.png")),
    ("墨", "Ink", Some("mo"), include_bytes!("icons/LargeMo.png")),
    ("墨绿", "Green", Some("molv"), include_bytes!("icons/LargeMolv.png")),
    ("靛青", "Indigo", Some("dian"), include_bytes!("icons/LargeDian.png")),
    ("茄紫", "Aubergine", Some("qie"), include_bytes!("icons/LargeQie.png")),
    ("赭石", "Ochre", Some("zhe"), include_bytes!("icons/LargeZhe.png")),
    ("雾蓝", "Slate", Some("wu"), include_bytes!("icons/LargeWu.png")),
    ("宣纸", "Paper", Some("xuan"), include_bytes!("icons/LargeXuan.png")),
];
// 刚点的那一个：系统换图标是异步的，不等它。
thread_local! { static PICKED: Cell<Option<usize>> = const { Cell::new(None) }; }

fn app() -> *mut AnyObject { unsafe { msg_send![AnyClass::get(c"UIApplication").unwrap(), sharedApplication] } }

fn current() -> usize {
    if let Some(i) = PICKED.with(|p| p.get()) { return i }
    let name = unsafe {
        let s: *mut AnyObject = msg_send![app(), alternateIconName];
        if s.is_null() { return 0 }
        let c: *const std::ffi::c_char = msg_send![s, UTF8String];
        std::ffi::CStr::from_ptr(c).to_string_lossy().into_owned()
    };
    // 认不出的键（旧版本的备用图标）当作主图标。
    ICONS.iter().position(|i| i.2 == Some(name.as_str())).unwrap_or(0)
}

fn set(i: usize) {
    unsafe {
        let ok: bool = msg_send![app(), supportsAlternateIcons];
        if !ok { return }
        let name: *mut AnyObject = match ICONS[i].2 {
            Some(n) => { let c = std::ffi::CString::new(n).unwrap_or_default(); msg_send![AnyClass::get(c"NSString").unwrap(), stringWithUTF8String: c.as_ptr()] }
            None => std::ptr::null_mut(),
        };
        let none: *mut AnyObject = std::ptr::null_mut();
        // 换完后系统会在进程外弹一句「你已更改……的图标」：版式改不了，私有的 _setAlternateIconName: 也照样弹（iOS 27 模拟器实测）。
        let _: () = msg_send![app(), setAlternateIconName: name, completionHandler: none];
    }
    PICKED.with(|p| p.set(Some(i)));
}

/// 第 `i` 张图标解码好的图（RenderImage 要 BGRA）。
fn image(i: usize) -> Option<Arc<RenderImage>> {
    static IMAGES: LazyLock<Vec<Option<Arc<RenderImage>>>> = LazyLock::new(|| ICONS.iter().map(|&(_, _, _, png)| {
        let mut buf = image::load_from_memory(png).ok()?.into_rgba8();
        for px in buf.pixels_mut() { px.0.swap(0, 2); }
        Some(Arc::new(RenderImage::new(vec![image::Frame::new(buf)])))
    }).collect());
    IMAGES[i].clone()
}

/// 图标的小样，边长 `s`，圆角照主屏上的比例。
fn thumb(i: usize, s: f32) -> Div {
    div().flex_none().size(px(s)).rounded(px(s * 0.225)).overflow_hidden()
        .children(image(i).map(|im| img(im).size(px(s)).rounded(px(s * 0.225))))
}

/// 现在用着的那张图标（关于页顶上）。
pub fn current_icon(s: f32) -> Div { thumb(current(), s) }

/// 设置 › 外观 › 应用图标（照 Craft）：一行四张，名字在下面，选中的外面一圈强调色、名字也是强调色。
pub fn grid(cx: &mut Context<Workspace>) -> Div {
    let at = current();
    let cell = |i: usize, cx: &mut Context<Workspace>| {
        let (zh, en, _, _) = ICONS[i];
        let on = i == at;
        div().id(("app-icon-grid", i)).w(px(76.)).flex().flex_col().items_center().gap(px(8.))
            .child(div().p(px(3.)).rounded(px(20.)).border_2().border_color(if on { Theme::primary() } else { gpui::transparent_black() }).child(thumb(i, 64.)))
            .child(div().text_size(px(15.)).line_height(px(20.)).font_weight(if on { FontWeight::SEMIBOLD } else { FontWeight::NORMAL })
                .text_color(if on { Theme::primary() } else { crate::settings::row_ink() }).child(crate::settings::tr(zh, en)))
            .on_click(cx.listener(move |_, _, _, cx| { if i != current() { super::glass::haptic(); set(i); } cx.notify(); }))
    };
    div().w_full().flex().flex_col().gap(px(18.)).children((0..ICONS.len()).step_by(4).map(|r| {
        div().w_full().flex().justify_around().children((r..(r + 4).min(ICONS.len())).map(|i| cell(i, cx)))
    }))
}
