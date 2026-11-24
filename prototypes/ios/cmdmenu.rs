//! 命令列表（手机，照 Craft，录屏 2026-11-24）：一张原生玻璃卡片，里面一行一个命令，行距 40pt，竖着滚。
//! 「/」用它跟在光标下面（焦点留在正文）；块动作栏的「添加」用它停在屏幕顶上，第一行是筛选框（焦点和键盘归它）。
//! 有哪些行、点了做什么都由编辑器给（`touch.rs`），这里只管画；GPUI 那边只留一块占位（`slot`），卡片跟着占位走。

use super::assist::card::on;
use super::glass::{icon_image, ns_string, ui_color};
use crate::settings::tr;
use crate::editor_theme::Theme;
use gpui::*;
use objc2::runtime::{AnyClass, AnyObject};
use objc2::msg_send;
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;

pub const ROW: f32 = 40.;
/// 列表上下的留白，筛选框那一行的高。
pub const PAD: f32 = 8.;
pub const FIELD: f32 = 52.;

#[derive(Clone, PartialEq)]
pub enum Kind { Pick, Into, Back, Note, Off }
#[derive(Clone, PartialEq)]
pub struct Row { pub kind: Kind, pub icon: &'static str, pub label: String, pub icon_color: Hsla, pub color: Hsla, pub act: String, pub hot: bool, pub extra: Extra }
/// 行里少见的几样：表情当图标（文档自己的图标）、标成强调色的一段（UTF-16 起点和长度）、右边的小字。
#[derive(Clone, Default, PartialEq)]
pub struct Extra { pub emoji: Option<String>, pub mark: Option<(usize, usize)>, pub detail: String }
/// `page`：换了一级就从头滚、内容淡着换；`filter`：带筛选框。
#[derive(Clone, PartialEq)]
pub struct Look { pub page: String, pub rows: Vec<Row>, pub filter: bool }

#[repr(C)]
struct Span { at: usize, len: usize }
unsafe impl objc2::Encode for Span { const ENCODING: objc2::Encoding = objc2::Encoding::Struct("_NSRange", &[objc2::Encoding::ULongLong, objc2::Encoding::ULongLong]); }
struct Card { view: *mut AnyObject, scroll: *mut AnyObject, field: *mut AnyObject, head: [*mut AnyObject; 3], frame: Bounds<Pixels>, look: Option<Look>, dark: Option<bool>, shown: bool }
thread_local! {
    static CARD: RefCell<Option<Card>> = const { RefCell::new(None) };
    static SEEN: Cell<bool> = const { Cell::new(false) };
    static QUERY: RefCell<String> = const { RefCell::new(String::new()) };
    static ICONS: RefCell<HashMap<&'static str, *mut AnyObject>> = RefCell::new(HashMap::new());
}

/// 筛选框里的字（拼音组字时系统垫在音节间的分隔不算）。
pub fn query() -> String { QUERY.with(|q| q.borrow().clone()) }

/// 每帧开头调用：这一帧没有占位就收起，键盘跟着落下（同 `assist_card.rs`，收了要自己要一帧）。
pub fn begin(window: &Window) {
    SEEN.with(|s| s.set(false));
    window.on_next_frame(|window, _| if !SEEN.with(|s| s.get()) && CARD.with(|c| c.borrow().as_ref().is_some_and(|c| c.shown)) { hide(); window.refresh(); });
}
fn hide() {
    let Some((view, field)) = CARD.with(|c| c.borrow_mut().as_mut().map(|c| { c.shown = false; (c.view as usize, c.field as usize) })) else { return };
    QUERY.with(|q| q.borrow_mut().clear());
    unsafe {
        let typing: bool = msg_send![field as *mut AnyObject, isFirstResponder];
        if typing { let _: bool = msg_send![field as *mut AnyObject, resignFirstResponder]; }
        let body = block2::RcBlock::new(move || { let _: () = msg_send![view as *mut AnyObject, setAlpha: 0f64]; });
        let done = block2::RcBlock::new(move |_finished: objc2::runtime::Bool| if !CARD.with(|c| c.borrow().as_ref().is_some_and(|c| c.shown)) { let _: () = msg_send![view as *mut AnyObject, setHidden: true]; });
        let _: () = msg_send![AnyClass::get(c"UIView").unwrap(), animateWithDuration: 0.14f64, animations: &*body, completion: &*done];
    }
}

fn send(act: String) {
    super::menu::dispatch(move |w, window, cx| if let Some(ed) = w.editor.clone() { ed.update(cx, |ed, cx| ed.command(&act, window, cx)) });
}

/// 占位：盖住底下的正文，位置和大小由放它的元素定。
pub fn slot(look: Look) -> impl IntoElement {
    div().id("command-card").size_full().occlude().on_mouse_down(MouseButton::Left, |_, window, cx| { cx.stop_propagation(); window.prevent_default(); })
        .child(canvas(move |bounds, window, _| place(bounds, look.clone(), window), |_, _, _, _| {}).absolute().inset_0())
}
/// 这些行要多高（不封顶）。
pub fn height(look: &Look) -> f32 { look.rows.len() as f32 * ROW + PAD * 2. + if look.filter { FIELD - PAD } else { 0. } }

fn place(frame: Bounds<Pixels>, look: Look, window: &Window) {
    let Ok(handle) = HasWindowHandle::window_handle(window) else { return; };
    let RawWindowHandle::UiKit(handle) = handle.as_raw() else { return; };
    let parent = super::shot::overlay(handle.ui_view.as_ptr() as *mut AnyObject);
    SEEN.with(|s| s.set(true));
    let dark = crate::theme::dark();
    // 先弹键盘的那一下在借完 CARD 之后：系统会当场回调。
    let focus = CARD.with(|c| unsafe {
        let mut slot = c.borrow_mut();
        if slot.is_none() { *slot = build(parent); }
        let Some(card) = slot.as_mut() else { return None; };
        if card.dark != Some(dark) {
            let _: () = msg_send![card.view, setOverrideUserInterfaceStyle: if dark { 2isize } else { 1isize }];
            // 键盘不看视图的界面风格（1 = 深色，2 = 浅色），同助手的卡片。
            let set: unsafe extern "C" fn(*mut AnyObject, objc2::runtime::Sel, isize) = std::mem::transmute(objc2::ffi::objc_msgSend as *const ());
            set(card.field, objc2::sel!(setKeyboardAppearance:), if dark { 1 } else { 2 });
            card.dark = Some(dark);
        }
        let fresh = !card.shown;
        let (w, h) = (f64::from(frame.size.width), f64::from(frame.size.height));
        let to = rect(f64::from(frame.origin.x), f64::from(frame.origin.y), w, h);
        if fresh {
            card.shown = true;
            let _: () = msg_send![card.field, setText: ns_string("")];
            let _: () = msg_send![card.view, setFrame: to];
            let _: () = msg_send![card.view, setAlpha: 0f64];
            let _: () = msg_send![card.view, setHidden: false];
            let _: () = msg_send![parent, bringSubviewToFront: card.view];
            let view = card.view as usize;
            let body = block2::RcBlock::new(move || { let _: () = msg_send![view as *mut AnyObject, setAlpha: 1f64]; });
            let _: () = msg_send![AnyClass::get(c"UIView").unwrap(), animateWithDuration: 0.18f64, animations: &*body];
        } else if card.frame.size != frame.size {
            // 筛选、换级时卡片跟着变高变矮；只是跟着光标挪的不动画。
            let view = card.view as usize;
            let body = block2::RcBlock::new(move || { let _: () = msg_send![view as *mut AnyObject, setFrame: to]; });
            let _: () = msg_send![AnyClass::get(c"UIView").unwrap(), animateWithDuration: 0.22f64, delay: 0f64, options: 6usize, animations: &*body, completion: std::ptr::null_mut::<AnyObject>()];
        } else if card.frame != frame {
            let _: () = msg_send![card.view, setFrame: to];
        }
        let filter = look.filter;
        let page = card.look.as_ref().map(|l| &l.page) != Some(&look.page);
        if fresh || card.frame.size != frame.size || card.look.as_ref() != Some(&look) {
            let top = if filter { f64::from(FIELD) } else { 0. };
            for v in card.head { let _: () = msg_send![v, setHidden: !filter]; }
            let _: () = msg_send![card.field, setHidden: !filter];
            let _: () = msg_send![card.field, setTextColor: ui_color(Theme::text())];
            let hint = attributed(tr("筛选命令", "Filter commands"), Theme::text_caption());
            let _: () = msg_send![card.field, setAttributedPlaceholder: hint];
            let _: () = msg_send![card.head[0], setTintColor: ui_color(Theme::text_caption())];
            let _: () = msg_send![card.head[1], setTintColor: ui_color(Theme::text_disabled())];
            let _: () = msg_send![card.head[0], setFrame: rect(18., 15., 22., 22.)];
            let _: () = msg_send![card.field, setFrame: rect(52., 4., w - 52. - 52., 44.)];
            let _: () = msg_send![card.head[1], setFrame: rect(w - 48., 4., 44., 44.)];
            let _: () = msg_send![card.head[2], setFrame: rect(16., f64::from(FIELD) - 1. / 3., w - 32., 1. / 3.)];
            let _: () = msg_send![card.scroll, setFrame: rect(0., top, w, (h - top).max(0.))];
            if fresh || card.look.as_ref().is_none_or(|l| l.rows != look.rows) || card.frame.size.width != frame.size.width { fill(card.scroll, &look.rows, w); }
            if page || fresh { let _: () = msg_send![card.scroll, setContentOffset: CGPoint::new(0., 0.), animated: false]; }
            if page && !fresh {
                let fade: *mut AnyObject = msg_send![AnyClass::get(c"CATransition").unwrap(), animation];
                let _: () = msg_send![fade, setDuration: 0.2f64];
                let layer: *mut AnyObject = msg_send![card.scroll, layer];
                let _: () = msg_send![layer, addAnimation: fade, forKey: ns_string("page")];
            }
            card.look = Some(look);
        }
        card.frame = frame;
        // 按 return 进了下一级：系统让输入框放了焦点，拿回来。
        (filter && (fresh || page)).then_some(card.field)
    });
    if let Some(field) = focus { unsafe { let _: bool = msg_send![field, becomeFirstResponder]; } }
}

/// 把行排进滚动区。
unsafe fn fill(scroll: *mut AnyObject, rows: &[Row], w: f64) {
    unsafe {
        let kids: *mut AnyObject = msg_send![scroll, subviews];
        let kids: *mut AnyObject = msg_send![kids, copy];
        let n: usize = msg_send![kids, count];
        // 999：自己放的行；滚动条是系统的，留着。
        for i in 0..n { let kid: *mut AnyObject = msg_send![kids, objectAtIndex: i]; let tag: isize = msg_send![kid, tag]; if tag == 999 { let _: () = msg_send![kid, removeFromSuperview]; } }
        let _: () = msg_send![kids, release];
        let (row_h, pad) = (f64::from(ROW), f64::from(PAD));
        for (i, row) in rows.iter().enumerate() {
            let b: *mut AnyObject = msg_send![AnyClass::get(c"UIButton").unwrap(), buttonWithType: 0isize];
            let _: () = msg_send![b, setTag: 999isize];
            let _: () = msg_send![b, setFrame: rect(8., pad + row_h * i as f64, w - 16., row_h)];
            let layer: *mut AnyObject = msg_send![b, layer];
            let _: () = msg_send![layer, setCornerRadius: 14f64];
            let hot = row.hot;
            let paint = move |b: usize, down: bool| { let _: () = msg_send![b as *mut AnyObject, setBackgroundColor: ui_color(if down { Theme::fill_pressed() } else if hot { Theme::fill_hover() } else { transparent_black() })]; };
            paint(b as usize, false);
            let _: () = msg_send![b, setAccessibilityLabel: ns_string(&row.label)];
            if matches!(row.kind, Kind::Note | Kind::Off) { let _: () = msg_send![b, setEnabled: false]; } else {
                let addr = b as usize;
                // 1 = 按下，128 | 256 = 在外面松手、取消，64 = 在里面松手。
                on(b, 1, move || paint(addr, true));
                on(b, 64 | 128 | 256, move || paint(addr, false));
                let act = row.act.clone();
                on(b, 64, move || send(act.clone()));
            }
            // 照 Craft 量：图标中心离卡片左边 30，字从 55 起。
            let back = row.kind == Kind::Back;
            let mark: *mut AnyObject = msg_send![AnyClass::get(c"UIImageView").unwrap(), new];
            let (size, x) = if back { (14., 10.) } else { (18., 13.) };
            let _: () = msg_send![mark, setFrame: rect(x, (row_h - size) / 2., size, size)];
            if !row.icon.is_empty() { let _: () = msg_send![mark, setImage: image(row.icon)]; }
            let _: () = msg_send![mark, setTintColor: ui_color(row.icon_color)];
            let _: () = msg_send![b, addSubview: mark];
            let _: () = msg_send![mark, release];
            let text: *mut AnyObject = msg_send![AnyClass::get(c"UILabel").unwrap(), new];
            // 没有图标的菜单（编号），字从左边起。
            let bare = row.icon.is_empty() && row.extra.emoji.is_none() && row.kind != Kind::Note;
            let left = if bare { 14. } else if back { 32. } else { 47. };
            let side = if row.extra.detail.is_empty() { 0. } else { 76. };
            let _: () = msg_send![text, setFrame: rect(left, 0., w - 16. - left - if bare { 14. } else if side > 0. { side + 16. } else { 36. }, row_h)];
            let _: () = msg_send![text, setText: ns_string(&row.label)];
            let font: *mut AnyObject = msg_send![AnyClass::get(c"UIFont").unwrap(), systemFontOfSize: if back { 13f64 } else { 15f64 }];
            let _: () = msg_send![text, setFont: font];
            let _: () = msg_send![text, setTextColor: ui_color(row.color)];
            if let Some((at, len)) = row.extra.mark {
                let string: *mut AnyObject = msg_send![AnyClass::get(c"NSMutableAttributedString").unwrap(), alloc];
                let string: *mut AnyObject = msg_send![string, initWithString: ns_string(&row.label)];
                let total: usize = msg_send![string, length];
                if at + len <= total {
                    let _: () = msg_send![string, addAttribute: ns_string("NSColor"), value: ui_color(Theme::primary_500()), range: Span { at, len }];
                    let _: () = msg_send![text, setAttributedText: string];
                }
                let _: () = msg_send![string, release];
            }
            if let Some(emoji) = &row.extra.emoji {
                let glyph: *mut AnyObject = msg_send![AnyClass::get(c"UILabel").unwrap(), new];
                let _: () = msg_send![glyph, setFrame: rect(8., 0., 28., row_h)];
                let _: () = msg_send![glyph, setText: ns_string(emoji)];
                let _: () = msg_send![glyph, setTextAlignment: 1isize];
                let font: *mut AnyObject = msg_send![AnyClass::get(c"UIFont").unwrap(), systemFontOfSize: 18f64];
                let _: () = msg_send![glyph, setFont: font];
                let _: () = msg_send![b, addSubview: glyph];
                let _: () = msg_send![glyph, release];
            }
            if side > 0. {
                let detail: *mut AnyObject = msg_send![AnyClass::get(c"UILabel").unwrap(), new];
                let _: () = msg_send![detail, setFrame: rect(w - 16. - side - 12., 0., side, row_h)];
                let _: () = msg_send![detail, setText: ns_string(&row.extra.detail)];
                let _: () = msg_send![detail, setTextAlignment: 2isize];
                let font: *mut AnyObject = msg_send![AnyClass::get(c"UIFont").unwrap(), systemFontOfSize: 13f64];
                let _: () = msg_send![detail, setFont: font];
                let _: () = msg_send![detail, setTextColor: ui_color(Theme::text_caption())];
                let _: () = msg_send![b, addSubview: detail];
                let _: () = msg_send![detail, release];
            }
            let _: () = msg_send![b, addSubview: text];
            let _: () = msg_send![text, release];
            if row.kind == Kind::Into {
                let more: *mut AnyObject = msg_send![AnyClass::get(c"UIImageView").unwrap(), new];
                let _: () = msg_send![more, setFrame: rect(w - 16. - 28., (row_h - 14.) / 2., 14., 14.)];
                let _: () = msg_send![more, setImage: image("chevron-right")];
                let _: () = msg_send![more, setTintColor: ui_color(Theme::text_caption())];
                let _: () = msg_send![b, addSubview: more];
                let _: () = msg_send![more, release];
            }
            let _: () = msg_send![scroll, addSubview: b];
        }
        let _: () = msg_send![scroll, setContentSize: CGSize::new(w, rows.len() as f64 * row_h + pad * 2.)];
    }
}

unsafe fn build(parent: *mut AnyObject) -> Option<Card> {
    unsafe {
        let view: *mut AnyObject = msg_send![AnyClass::get(c"UIVisualEffectView")?, alloc];
        let effect: *mut AnyObject = msg_send![AnyClass::get(c"UIGlassEffect")?, effectWithStyle: 0isize];
        let view: *mut AnyObject = msg_send![view, initWithEffect: effect];
        if let (Some(corners), Some(corner)) = (AnyClass::get(c"UICornerConfiguration"), AnyClass::get(c"UICornerRadius")) {
            let fixed: *mut AnyObject = msg_send![corner, fixedRadius: 24f64];
            let config: *mut AnyObject = msg_send![corners, configurationWithUniformRadius: fixed];
            let _: () = msg_send![view, setCornerConfiguration: config];
        }
        let _: () = msg_send![view, setHidden: true];
        let content: *mut AnyObject = msg_send![view, contentView];
        let _: () = msg_send![content, setClipsToBounds: true];
        let scroll: *mut AnyObject = msg_send![AnyClass::get(c"UIScrollView")?, new];
        // 2 = UIScrollViewContentInsetAdjustmentNever：卡片自己避开了安全区。
        let _: () = msg_send![scroll, setContentInsetAdjustmentBehavior: 2isize];
        let _: () = msg_send![content, addSubview: scroll];
        let symbol = |name: &str, size: f64| -> *mut AnyObject {
            let config: *mut AnyObject = msg_send![AnyClass::get(c"UIImageSymbolConfiguration").unwrap(), configurationWithPointSize: size, weight: 4isize];
            msg_send![AnyClass::get(c"UIImage").unwrap(), systemImageNamed: ns_string(name), withConfiguration: config]
        };
        let mark: *mut AnyObject = msg_send![AnyClass::get(c"UIImageView")?, new];
        let _: () = msg_send![mark, setImage: symbol("command", 15.)];
        // 4 = UIViewContentModeCenter
        let _: () = msg_send![mark, setContentMode: 4isize];
        let _: () = msg_send![content, addSubview: mark];
        let field: *mut AnyObject = msg_send![AnyClass::get(c"UITextField")?, new];
        let font: *mut AnyObject = msg_send![AnyClass::get(c"UIFont")?, systemFontOfSize: 16f64];
        let _: () = msg_send![field, setFont: font];
        // 1 = UITextAutocorrectionTypeNo，0 = UITextAutocapitalizationTypeNone。
        let set: unsafe extern "C" fn(*mut AnyObject, objc2::runtime::Sel, isize) = std::mem::transmute(objc2::ffi::objc_msgSend as *const ());
        set(field, objc2::sel!(setAutocorrectionType:), 1);
        set(field, objc2::sel!(setAutocapitalizationType:), 0);
        let addr = field as usize;
        // 1 << 17 = UIControlEventEditingChanged：记下筛选的字，让编辑器重出一遍行。
        on(field, 1 << 17, move || {
            let text: *mut AnyObject = msg_send![addr as *mut AnyObject, text];
            let utf8: *const std::ffi::c_char = if text.is_null() { std::ptr::null() } else { msg_send![text, UTF8String] };
            let s = if utf8.is_null() { String::new() } else { std::ffi::CStr::from_ptr(utf8).to_string_lossy().into_owned() };
            let marked: *mut AnyObject = msg_send![addr as *mut AnyObject, markedTextRange];
            let s: String = if marked.is_null() { s } else { s.chars().filter(|c| !c.is_whitespace() && !matches!(c, '\'' | '’')).collect() };
            QUERY.with(|q| *q.borrow_mut() = s);
            send("query".into());
        });
        // 1 << 19 = UIControlEventEditingDidEndOnExit：按了 return，用第一条。
        on(field, 1 << 19, || {
            let first = CARD.with(|c| c.borrow().as_ref().and_then(|c| c.look.as_ref()?.rows.iter().find(|r| !matches!(r.kind, Kind::Note | Kind::Back)).map(|r| r.act.clone())));
            if let Some(act) = first { send(act); }
        });
        let _: () = msg_send![content, addSubview: field];
        let close: *mut AnyObject = msg_send![AnyClass::get(c"UIButton")?, buttonWithType: 0isize];
        let _: () = msg_send![close, setImage: symbol("xmark.circle.fill", 19.), forState: 0usize];
        let _: () = msg_send![close, setAccessibilityLabel: ns_string(tr("关闭", "Close"))];
        on(close, 64, || send("close".into()));
        let _: () = msg_send![content, addSubview: close];
        let line: *mut AnyObject = msg_send![AnyClass::get(c"UIView")?, new];
        let separator: *mut AnyObject = msg_send![AnyClass::get(c"UIColor")?, separatorColor];
        let _: () = msg_send![line, setBackgroundColor: separator];
        let _: () = msg_send![content, addSubview: line];
        let _: () = msg_send![parent, addSubview: view];
        Some(Card { view, scroll, field, head: [mark, close, line], frame: Bounds::default(), look: None, dark: None, shown: false })
    }
}

/// 图标按名字缓存：打一个字就要重出一遍行，不能每次都从 SVG 画。
unsafe fn image(name: &'static str) -> *mut AnyObject {
    if let Some(i) = ICONS.with(|m| m.borrow().get(name).copied()) { return i; }
    unsafe {
        // 编辑器自己那一套图标，没有的再用通用的。
        let i = icon_image(&format!("editor-icons/{name}"), 18.);
        let i = if i.is_null() { icon_image(name, 18.) } else { i };
        if !i.is_null() { let _: *mut AnyObject = msg_send![i, retain]; }
        ICONS.with(|m| m.borrow_mut().insert(name, i));
        i
    }
}
unsafe fn attributed(s: &str, color: Hsla) -> *mut AnyObject {
    unsafe {
        let attrs: *mut AnyObject = msg_send![AnyClass::get(c"NSDictionary").unwrap(), dictionaryWithObject: ui_color(color), forKey: ns_string("NSColor")];
        let a: *mut AnyObject = msg_send![AnyClass::get(c"NSAttributedString").unwrap(), alloc];
        let a: *mut AnyObject = msg_send![a, initWithString: ns_string(s), attributes: attrs];
        msg_send![a, autorelease]
    }
}
fn rect(x: f64, y: f64, w: f64, h: f64) -> CGRect { CGRect::new(CGPoint::new(x, y), CGSize::new(w, h)) }
