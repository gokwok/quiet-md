//! 应用图标：三种底色的「页边」，设置 › 外观里换。默认的「墨」是包里的主图标，另外两张是备用图标
//! （Info.plist 的 `CFBundleAlternateIcons`，图在 `prototypes/ios/icons`）。换的时候系统自己会弹一句「图标已更改」。
use std::cell::Cell;

use gpui::*;
use objc2::msg_send;
use objc2::runtime::{AnyClass, AnyObject};

use crate::app::Workspace;
use crate::theme::Theme;

/// 名字、备用图标的键（主图标没有）、底色。颜色都取自图标母版（`icons/*.svg`）。
const ICONS: [(&str, &str, Option<&str>, u32); 3] = [("墨", "Ink", None, 0x34322F), ("墨绿", "Green", Some("molv"), 0x2F5D50), ("朱", "Vermilion", Some("zhu"), 0xD8502F)];
const PAPER: u32 = 0xF4F1EA;
const INK: u32 = 0x262523;
const LINE: u32 = 0x8E8A83;
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
        let _: () = msg_send![app(), setAlternateIconName: name, completionHandler: none];
    }
    PICKED.with(|p| p.set(Some(i)));
}

/// 图标的小样，照母版的比例画（母版 1024）。
fn thumb(ground: u32, s: f32) -> Div {
    let k = s / 1024.;
    let bar = |y: f32, w: f32, h: f32, c: u32| div().absolute().left(px(372. * k)).top(px(y * k)).w(px(w * k)).h(px(h * k)).rounded_full().bg(rgb(c));
    // 圆角不裁子元素：纸的右下角自己圆。
    div().flex_none().size(px(s)).rounded(px(s * 0.225)).relative().bg(rgb(ground))
        .child(div().absolute().left(px(248. * k)).top(px(248. * k)).right_0().bottom_0().rounded_tl(px(112. * k)).rounded_br(px(s * 0.225)).bg(rgb(PAPER)))
        .child(bar(380., 236., 76., INK)).child(bar(508., 372., 52., LINE)).child(bar(600., 232., 52., LINE))
}

/// 三张小样排一行，选中的那张外面一圈强调色。
pub fn picker(cx: &mut Context<Workspace>) -> Div {
    let at = current();
    div().flex().items_center().gap(px(10.)).children(ICONS.iter().enumerate().map(|(i, &(zh, en, _, ground))| {
        div().id(("app-icon", i)).flex().flex_col().items_center().gap(px(4.)).cursor_pointer()
            .child(div().p(px(3.)).rounded(px(15.)).border_2().border_color(if i == at { Theme::primary() } else { gpui::transparent_black() }).child(thumb(ground, 48.)))
            .child(div().text_size(px(11.)).line_height(px(14.)).text_color(if i == at { Theme::text() } else { Theme::text_caption() }).child(crate::settings::tr(zh, en)))
            .on_click(cx.listener(move |_, _, _, cx| { if i != current() { set(i); } cx.notify(); }))
    }))
}
