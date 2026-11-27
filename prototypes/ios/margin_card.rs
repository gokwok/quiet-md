//! 助手页底下的输入卡片：一整张原生玻璃，键盘收着时贴在屏幕下沿，键盘起来时贴在键盘上面。
//! 最上面一排是这句话附带的资源：「@」（点了在输入框里打一个 @，弹出搜索），后面每样一颗小胶囊（文件夹、文档、选中的块），
//! 点了出菜单「移除」；排不下可以横着滑。中间是要说的话（多行），打 @ 加字就是在搜要附带的东西（`mention`，列表由助手页画）。
//! 最下面一排：左边是助手（头像和名字，菜单里换助手、临时换核心）和权限（只读 / 可编辑）两颗胶囊，右边是发送，助手在说时换成停止。
//! 和助手写作的卡片（`assist_card.rs`）一个做法：GPUI 那边只留一块占位（`slot`），卡片跟着占位走；按钮走 `ios::menu_action`。
use std::cell::{Cell, RefCell};

use gpui::*;
use objc2::msg_send;
use objc2::runtime::{AnyClass, AnyObject};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use super::super::assist::card::{on, text_of};
use super::super::glass::{ns_string, ui_color};
use super::super::menu;

pub const HEIGHT: f32 = (PAD + TOP + 4. + TEXT + 2. + CHIP + PAD) as f32;
const PAD: f64 = 12.;
/// 资源那排的高。
const TOP: f64 = 28.;
const CHIP: f64 = 32.;
const TEXT: f64 = 50.;
const SEND: f64 = 32.;
const RADIUS: f64 = 26.;
/// 资源胶囊最宽多少。
const WIDEST: f64 = 168.;
/// 点了「@」。
pub const MENTION: &str = "margin-mention";

/// 一颗胶囊上写什么、菜单里有什么：标题、SF Symbol（有 `picture` 时用图）、菜单（`menu::Item`，点了交给 `ios::menu_action`）。
#[derive(Clone, PartialEq)]
/// `off`：灰着点不动（比如酒馆核心不调工具，权限无从谈起）。
pub struct Chip { pub title: String, pub glyph: &'static str, pub picture: Option<(SharedString, std::sync::Arc<[u8]>)>, pub menu: Vec<menu::Item>, pub off: bool }

struct Card {
    view: *mut AnyObject, at: *mut AnyObject, strip: *mut AnyObject, res: Vec<*mut AnyObject>, chips: [*mut AnyObject; 2],
    text: *mut AnyObject, hint: *mut AnyObject, send: *mut AnyObject, frame: Bounds<Pixels>, dark: Option<bool>, shown: bool, busy: Option<bool>, faces: Vec<Chip>,
}
thread_local! {
    static CARD: RefCell<Option<Card>> = const { RefCell::new(None) };
    static SEEN: Cell<bool> = const { Cell::new(false) };
    /// 下次露面时要不要弹出键盘。
    static FOCUS: Cell<bool> = const { Cell::new(false) };
    /// 助手在说（发送钮是停止，空着也能点）。
    static BUSY: Cell<bool> = const { Cell::new(false) };
    /// 输入框末尾正在打的 `@…`：@ 后面的字。
    static MENTION_AT: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// 每帧开头调用：这一帧没有占位，卡片就收起。
pub fn begin(window: &Window) {
    SEEN.with(|s| s.set(false));
    window.on_next_frame(|window, _| if !SEEN.with(|s| s.get()) && CARD.with(|c| c.borrow().as_ref().is_some_and(|c| c.shown)) { hide(); window.refresh(); });
}

fn hide() {
    CARD.with(|c| if let Some(card) = c.borrow_mut().as_mut().filter(|card| card.shown) { unsafe {
        card.shown = false;
        let _: bool = msg_send![card.view, endEditing: true];
        let _: () = msg_send![card.view, setHidden: true];
    } });
}

/// 下次露面时弹出键盘（新对话）。已经露面的当场弹。
pub fn focus() {
    FOCUS.with(|f| f.set(true));
    CARD.with(|c| if let Some(card) = c.borrow().as_ref().filter(|c| c.shown) { unsafe { let _: bool = msg_send![card.text, becomeFirstResponder]; } FOCUS.with(|f| f.set(false)); });
}

/// 收起键盘。
pub fn blur() { CARD.with(|c| if let Some(card) = c.borrow().as_ref() { unsafe { let _: bool = msg_send![card.view, endEditing: true]; } }); }

/// 取走输入框里的话（清空）。拼音还没选字就点了发送：先提交在拼的字，再去掉拼音键盘插在音节间的 U+2006。
pub fn take() -> String {
    let Some(field) = CARD.with(|c| c.borrow().as_ref().map(|card| card.text)) else { return String::new() };
    unsafe {
        let marked: *mut AnyObject = msg_send![field, markedTextRange];
        if !marked.is_null() { let _: () = msg_send![field, unmarkText]; }
    }
    let text = text_of(field).replace('\u{2006}', "");
    unsafe { let _: () = msg_send![field, setText: ns_string("")]; }
    CARD.with(|c| if let Some(card) = c.borrow().as_ref() { changed(card.text as usize, card.hint as usize, card.send as usize); });
    text
}

/// 输入框末尾正在打的 `@…`（@ 后面的字），没在打是 `None`。
pub fn mention() -> Option<String> { MENTION_AT.with(|m| m.borrow().clone()) }

/// 「@」：在输入框末尾打一个 @（前面不是空白就先空一格），弹出键盘。
pub fn insert_at() {
    let Some((field, hint, send)) = CARD.with(|c| c.borrow().as_ref().map(|card| (card.text, card.hint, card.send))) else { return };
    let s = text_of(field);
    let next = if s.is_empty() || s.ends_with(char::is_whitespace) { format!("{s}@") } else { format!("{s} @") };
    unsafe {
        let _: () = msg_send![field, setText: ns_string(&next)];
        let _: bool = msg_send![field, becomeFirstResponder];
    }
    changed(field as usize, hint as usize, send as usize);
}

/// 挑好了要附带的东西：把末尾的 `@…` 拿掉。
pub fn clear_mention() {
    let Some((field, hint, send)) = CARD.with(|c| c.borrow().as_ref().map(|card| (card.text, card.hint, card.send))) else { return };
    unsafe {
        let marked: *mut AnyObject = msg_send![field, markedTextRange];
        if !marked.is_null() { let _: () = msg_send![field, unmarkText]; }
    }
    let s = text_of(field).replace('\u{2006}', "");
    if let Some(at) = mention_at(&s) { unsafe { let _: () = msg_send![field, setText: ns_string(s[..at].trim_end())]; } }
    changed(field as usize, hint as usize, send as usize);
}

/// 末尾的 `@…` 从哪个字节起：@ 在开头或空白后面，后面到末尾没有空白。
fn mention_at(s: &str) -> Option<usize> {
    let at = s.rfind('@')?;
    let tail = &s[at + 1..];
    if tail.chars().any(char::is_whitespace) || tail.chars().count() > 24 { return None; }
    (at == 0 || s[..at].ends_with(char::is_whitespace)).then_some(at)
}

/// 占位：左右各留 12。`top` 是资源胶囊，`chips` 是助手和权限；`busy`：助手在说，发送换成停止。
pub fn slot(top: Vec<Chip>, chips: [Chip; 2], busy: bool) -> AnyElement {
    let (_, _, viewport) = super::super::layout();
    let faces: std::rc::Rc<Vec<Chip>> = std::rc::Rc::new(chips.into_iter().chain(top).collect());
    div().id("margin-card").occlude().w(viewport.width - px(24.)).h(px(HEIGHT)).relative()
        .on_mouse_down(MouseButton::Left, |_, window, cx| { cx.stop_propagation(); window.prevent_default(); })
        .child(canvas(move |bounds, window, _| place(bounds, &faces, busy, window), |_, _, _, _| {}).absolute().inset_0())
        .into_any_element()
}

/// `faces`：先是助手、权限两颗，后面是资源。
fn place(frame: Bounds<Pixels>, faces: &[Chip], busy: bool, window: &Window) {
    let Ok(handle) = HasWindowHandle::window_handle(window) else { return; };
    let RawWindowHandle::UiKit(handle) = handle.as_raw() else { return; };
    let parent = super::super::shot::overlay(handle.ui_view.as_ptr() as *mut AnyObject);
    SEEN.with(|s| s.set(true));
    let dark = crate::theme::dark();
    CARD.with(|c| unsafe {
        let mut slot = c.borrow_mut();
        if slot.is_none() { *slot = build(parent); }
        let Some(card) = slot.as_mut() else { return; };
        if card.dark != Some(dark) {
            let _: () = msg_send![card.view, setOverrideUserInterfaceStyle: if dark { 2isize } else { 1isize }];
            // 键盘不看视图的界面风格，得单独说（1 = 深色，2 = 浅色）；同 `assist_card.rs`。
            let set: unsafe extern "C" fn(*mut AnyObject, objc2::runtime::Sel, isize) = std::mem::transmute(objc2::ffi::objc_msgSend as *const ());
            set(card.text, objc2::sel!(setKeyboardAppearance:), if dark { 1 } else { 2 });
            let typing: bool = msg_send![card.text, isFirstResponder];
            if typing { let _: () = msg_send![card.text, reloadInputViews]; }
            card.dark = Some(dark);
        }
        let refaced = card.faces != faces;
        if refaced {
            face(card.chips[0], &faces[0], false);
            face(card.chips[1], &faces[1], false);
            let top = &faces[2..];
            while card.res.len() < top.len() { let b = chip(card.strip, true); card.res.push(b); }
            for (i, b) in card.res.iter().enumerate() {
                let _: () = msg_send![*b, setHidden: i >= top.len()];
                if let Some(c) = top.get(i) { face(*b, c, true); }
            }
            let hint = crate::localized_format!("给 {} 发消息", "Message {}", faces[0].title);
            let _: () = msg_send![card.hint, setText: ns_string(&hint)];
            card.faces = faces.to_vec();
        }
        if card.busy != Some(busy) { BUSY.with(|b| b.set(busy)); send_face(card.send, busy); card.busy = Some(busy); changed(card.text as usize, card.hint as usize, card.send as usize); }
        if card.frame != frame || refaced {
            let rect = CGRect::new(CGPoint::new(f64::from(frame.origin.x), f64::from(frame.origin.y)), CGSize::new(f64::from(frame.size.width), f64::from(frame.size.height)));
            if card.shown && card.frame.origin.y != frame.origin.y {
                // 跟着键盘升降，用系统键盘的节奏。2 = AllowUserInteraction，4 = BeginFromCurrentState。
                let (addr, to) = (card.view as usize, rect);
                let body = block2::RcBlock::new(move || { let _: () = msg_send![addr as *mut AnyObject, setFrame: to]; });
                let _: () = msg_send![AnyClass::get(c"UIView").unwrap(), animateWithDuration: 0.25f64, delay: 0f64, options: 6usize, animations: &*body, completion: std::ptr::null_mut::<AnyObject>()];
            } else { let _: () = msg_send![card.view, setFrame: rect]; }
            layout(card, f64::from(frame.size.width));
            card.frame = frame;
        }
        if !card.shown {
            card.shown = true;
            let _: () = msg_send![parent, bringSubviewToFront: card.view];
            if let Some(class) = AnyClass::get(c"UIGlassEffect") {
                let effect: *mut AnyObject = msg_send![class, effectWithStyle: 0isize];
                let _: () = msg_send![card.view, setEffect: effect];
            }
            let _: () = msg_send![card.view, setHidden: false];
            // 淡入，由 0.96 弹到 1（同 `assist_card.rs`）。
            let layer: *mut AnyObject = msg_send![card.view, layer];
            let anim: *mut AnyObject = msg_send![AnyClass::get(c"CABasicAnimation").unwrap(), animationWithKeyPath: ns_string("opacity")];
            let from: *mut AnyObject = msg_send![AnyClass::get(c"NSNumber").unwrap(), numberWithDouble: 0f64];
            let _: () = msg_send![anim, setFromValue: from];
            let _: () = msg_send![anim, setDuration: 0.22f64];
            let _: () = msg_send![layer, addAnimation: anim, forKey: ns_string("appear")];
            let spring: *mut AnyObject = msg_send![AnyClass::get(c"CASpringAnimation").unwrap(), animationWithKeyPath: ns_string("transform.scale")];
            let small: *mut AnyObject = msg_send![AnyClass::get(c"NSNumber").unwrap(), numberWithDouble: 0.96f64];
            let _: () = msg_send![spring, setFromValue: small];
            let _: () = msg_send![spring, setDamping: 16f64];
            let _: () = msg_send![spring, setStiffness: 260f64];
            let d: f64 = msg_send![spring, settlingDuration];
            let _: () = msg_send![spring, setDuration: d];
            let _: () = msg_send![layer, addAnimation: spring, forKey: ns_string("grow")];
        }
        if FOCUS.with(|f| f.take()) { let _: bool = msg_send![card.text, becomeFirstResponder]; }
    });
}

/// 一颗灰色的胶囊，点了出菜单。`small`：资源那排的（小一号）。
unsafe fn chip(parent: *mut AnyObject, small: bool) -> *mut AnyObject {
    unsafe {
        let label: *mut AnyObject = msg_send![AnyClass::get(c"UIColor").unwrap(), labelColor];
        let config: *mut AnyObject = msg_send![AnyClass::get(c"UIButtonConfiguration").unwrap(), grayButtonConfiguration];
        let _: () = msg_send![config, setCornerStyle: 4isize];
        // 1 = UIButtonConfigurationSizeSmall，2 = Mini
        let _: () = msg_send![config, setButtonSize: if small { 2isize } else { 1isize }];
        let _: () = msg_send![config, setBaseForegroundColor: label];
        let b: *mut AnyObject = msg_send![AnyClass::get(c"UIButton").unwrap(), buttonWithConfiguration: config, primaryAction: std::ptr::null_mut::<AnyObject>()];
        let _: () = msg_send![b, setShowsMenuAsPrimaryAction: true];
        let _: () = msg_send![parent, addSubview: b];
        b
    }
}

unsafe fn build(parent: *mut AnyObject) -> Option<Card> {
    unsafe {
        // 没有玻璃的系统不建卡片。
        AnyClass::get(c"UIGlassEffect")?;
        let view: *mut AnyObject = msg_send![AnyClass::get(c"UIVisualEffectView")?, alloc];
        let view: *mut AnyObject = msg_send![view, initWithEffect: std::ptr::null_mut::<AnyObject>()];
        if let (Some(corners), Some(corner)) = (AnyClass::get(c"UICornerConfiguration"), AnyClass::get(c"UICornerRadius")) {
            let fixed: *mut AnyObject = msg_send![corner, fixedRadius: RADIUS];
            let config: *mut AnyObject = msg_send![corners, configurationWithUniformRadius: fixed];
            let _: () = msg_send![view, setCornerConfiguration: config];
        }
        let _: () = msg_send![view, setHidden: true];
        let content: *mut AnyObject = msg_send![view, contentView];
        let color = |name: &std::ffi::CStr| -> *mut AnyObject { msg_send![AnyClass::get(c"UIColor").unwrap(), performSelector: objc2::runtime::Sel::register(name)] };
        let chips = [chip(content, false), chip(content, false)];
        // 「@」：和资源胶囊一样大的圆，点了在输入框里打 @。
        let at = chip(content, true);
        let _: () = msg_send![at, setShowsMenuAsPrimaryAction: false];
        let config: *mut AnyObject = msg_send![at, configuration];
        let symbol: *mut AnyObject = msg_send![AnyClass::get(c"UIImageSymbolConfiguration").unwrap(), configurationWithPointSize: 12f64, weight: 5isize];
        let image: *mut AnyObject = msg_send![AnyClass::get(c"UIImage").unwrap(), systemImageNamed: ns_string("at"), withConfiguration: symbol];
        let _: () = msg_send![config, setImage: image];
        let _: () = msg_send![at, setConfiguration: config];
        on(at, 64, || super::super::menu::chosen(MENTION));
        let strip: *mut AnyObject = msg_send![AnyClass::get(c"UIScrollView").unwrap(), new];
        let _: () = msg_send![strip, setShowsHorizontalScrollIndicator: false];
        let _: () = msg_send![strip, setAlwaysBounceHorizontal: false];
        let _: () = msg_send![content, addSubview: strip];
        let text: *mut AnyObject = msg_send![AnyClass::get(c"UITextView").unwrap(), new];
        let font: *mut AnyObject = msg_send![AnyClass::get(c"UIFont").unwrap(), preferredFontForTextStyle: ns_string("UICTFontTextStyleBody")];
        let _: () = msg_send![text, setFont: font];
        let clear: *mut AnyObject = msg_send![AnyClass::get(c"UIColor").unwrap(), clearColor];
        let _: () = msg_send![text, setBackgroundColor: clear];
        let _: () = msg_send![text, setTintColor: ui_color(crate::theme::Theme::primary())];
        let container: *mut AnyObject = msg_send![text, textContainer];
        let _: () = msg_send![container, setLineFragmentPadding: 0f64];
        let _: () = msg_send![content, addSubview: text];
        let hint: *mut AnyObject = msg_send![AnyClass::get(c"UILabel").unwrap(), new];
        let _: () = msg_send![hint, setFont: font];
        let _: () = msg_send![hint, setTextColor: color(c"placeholderTextColor")];
        let _: () = msg_send![hint, setText: ns_string(crate::settings::tr("给 MarginAI 发消息", "Message MarginAI"))];
        let _: () = msg_send![content, addSubview: hint];
        let config: *mut AnyObject = msg_send![AnyClass::get(c"UIButtonConfiguration").unwrap(), filledButtonConfiguration];
        let _: () = msg_send![config, setCornerStyle: 4isize];
        let send: *mut AnyObject = msg_send![AnyClass::get(c"UIButton").unwrap(), buttonWithConfiguration: config, primaryAction: std::ptr::null_mut::<AnyObject>()];
        let _: () = msg_send![content, addSubview: send];
        on(send, 64, || super::super::menu::chosen(super::SEND));
        let (t, h, s) = (text as usize, hint as usize, send as usize);
        let block = block2::RcBlock::new(move |_note: *mut AnyObject| changed(t, h, s));
        let center: *mut AnyObject = msg_send![AnyClass::get(c"NSNotificationCenter").unwrap(), defaultCenter];
        let _: *mut AnyObject = msg_send![center, addObserverForName: ns_string("UITextViewTextDidChangeNotification"), object: text, queue: std::ptr::null_mut::<AnyObject>(), usingBlock: &*block];
        let _: () = msg_send![parent, addSubview: view];
        Some(Card { view, at, strip, res: vec![], chips, text, hint, send, frame: Bounds::default(), dark: None, shown: false, busy: None, faces: vec![] })
    }
}

/// 发送：强调色的实心圆、白色向上箭头；在说时是停止（方块）。
unsafe fn send_face(send: *mut AnyObject, busy: bool) {
    unsafe {
        let config: *mut AnyObject = msg_send![send, configuration];
        let _: () = msg_send![config, setBaseBackgroundColor: ui_color(if busy { crate::theme::Theme::text() } else { crate::theme::Theme::primary() })];
        let symbol: *mut AnyObject = msg_send![AnyClass::get(c"UIImageSymbolConfiguration").unwrap(), configurationWithPointSize: if busy { 11f64 } else { 14f64 }, weight: 6isize];
        let image: *mut AnyObject = msg_send![AnyClass::get(c"UIImage").unwrap(), systemImageNamed: ns_string(if busy { "stop.fill" } else { "arrow.up" }), withConfiguration: symbol];
        let _: () = msg_send![config, setImage: image];
        let _: () = msg_send![send, setConfiguration: config];
    }
}

fn changed(text: usize, hint: usize, send: usize) {
    let s = text_of(text as *mut AnyObject);
    let busy = BUSY.with(|b| b.get());
    // 拼音还没选字时音节间插着 U+2006：不算空白。
    let plain = s.replace('\u{2006}', "");
    let at = mention_at(&plain).map(|i| plain[i + 1..].to_owned());
    if MENTION_AT.with(|m| m.replace(at.clone()) != at) { menu::dispatch(|_, _, cx| cx.notify()); }
    unsafe {
        let _: () = msg_send![hint as *mut AnyObject, setHidden: !s.is_empty()];
        let _: () = msg_send![send as *mut AnyObject, setEnabled: busy || !s.trim().is_empty()];
    }
}

unsafe fn layout(card: &Card, w: f64) {
    unsafe {
        let set = |v: *mut AnyObject, x: f64, y: f64, w: f64, h: f64| { let _: () = msg_send![v, setFrame: CGRect::new(CGPoint::new(x, y), CGSize::new(w, h))]; };
        let fit = |c: *mut AnyObject| -> f64 { let s: CGSize = msg_send![c, intrinsicContentSize]; s.width };
        // 资源那排：「@」，后面的胶囊排在能横滑的一条里。
        set(card.at, PAD, PAD, TOP, TOP);
        let strip_x = PAD + TOP + 6.;
        set(card.strip, strip_x, PAD, w - strip_x - PAD, TOP);
        let mut x = 0.;
        for b in card.res.iter().take(card.faces.len().saturating_sub(2)) { let bw = fit(*b).min(WIDEST); set(*b, x, 0., bw, TOP); x += bw + 6.; }
        let _: () = msg_send![card.strip, setContentSize: CGSize::new((x - 6.).max(0.), TOP)];
        let ty = PAD + TOP + 4.;
        set(card.text, PAD + 4., ty, w - PAD * 2. - 8., TEXT);
        set(card.hint, PAD + 4., ty + 8., w - PAD * 2. - 8., 22.);
        let y = ty + TEXT + 2.;
        // 两颗胶囊按内容宽度排，排不下先缩助手那颗（名字可能很长）。
        let room = w - PAD * 2. - SEND - 12.;
        let perm = fit(card.chips[1]).min(room / 2.);
        let who = fit(card.chips[0]).min(room - perm - 6.).max(44.);
        set(card.chips[0], PAD, y, who, CHIP);
        set(card.chips[1], PAD + who + 6., y, perm, CHIP);
        set(card.send, w - PAD - SEND, y, SEND, SEND);
    }
}

/// 一颗胶囊的标题、图标（或头像）和菜单。菜单项点了交给 `ios::menu_action`。
unsafe fn face(b: *mut AnyObject, chip: &Chip, small: bool) {
    unsafe {
        let config: *mut AnyObject = msg_send![b, configuration];
        let _: () = msg_send![config, setTitle: ns_string(&chip.title)];
        let image: *mut AnyObject = match &chip.picture {
            Some((key, bytes)) => menu::picture_sized(key, bytes, if small { 16. } else { 20. }),
            None => {
                let symbol: *mut AnyObject = msg_send![AnyClass::get(c"UIImageSymbolConfiguration").unwrap(), configurationWithPointSize: if small { 10f64 } else { 11f64 }, weight: 5isize];
                msg_send![AnyClass::get(c"UIImage").unwrap(), systemImageNamed: ns_string(chip.glyph), withConfiguration: symbol]
            }
        };
        let _: () = msg_send![config, setImage: image];
        let _: () = msg_send![config, setImagePadding: if chip.picture.is_some() { 6f64 } else { 5f64 }];
        let _: () = msg_send![config, setTitleLineBreakMode: 4isize];
        let _: () = msg_send![b, setConfiguration: config];
        let m: *mut AnyObject = if chip.menu.is_empty() { std::ptr::null_mut() } else { menu::menu("", std::ptr::null_mut(), &chip.menu, false) };
        let _: () = msg_send![b, setMenu: m];
        let _: () = msg_send![b, setEnabled: !chip.off];
    }
}
