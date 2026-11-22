//! 助手的卡片：一整张原生玻璃，贴在键盘上面。上面一排三颗小按钮切预设、助手和人设（系统菜单），右边关闭；
//! 下面是要求（多行），右下角发送。上一次的要求预填、全选，直接打字就换掉。
//! 和新建文件夹的卡片（`card.rs`）一个做法：GPUI 那边只留一块占位（`slot`），卡片跟着占位走；按钮走 `ios::menu_action`。
use std::cell::{Cell, RefCell};

use gpui::*;
use objc2::msg_send;
use objc2::runtime::{AnyClass, AnyObject};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use fdl_ai::tavern::library;
use crate::settings::tr;
use super::super::glass::{ns_string, ui_color};

pub const HEIGHT: f32 = (PAD + CHIP + 6. + TEXT + PAD) as f32;
const PAD: f64 = 14.;
const CHIP: f64 = 32.;
const TEXT: f64 = 104.;
const SEND: f64 = 36.;

struct Card { view: *mut AnyObject, chips: [*mut AnyObject; 3], text: *mut AnyObject, hint: *mut AnyObject, send: *mut AnyObject, frame: Bounds<Pixels>, dark: Option<bool>, shown: bool }
thread_local! {
    static CARD: RefCell<Option<Card>> = const { RefCell::new(None) };
    static SEEN: Cell<bool> = const { Cell::new(false) };
}

/// 每帧开头调用：这一帧没有占位，卡片就收起。
pub fn begin(window: &Window) {
    SEEN.with(|s| s.set(false));
    // 收起时 `endEditing` 让键盘落下，系统当场发「键盘将收起」；那时正在这一帧的回调里，GPUI 记下了键盘高度变 0，
    // 却没能排下一帧（窗口正忙），版面要等别的事来重画才落下去（约一秒半）。所以收了就自己要一帧。
    window.on_next_frame(|window, _| if !SEEN.with(|s| s.get()) && CARD.with(|c| c.borrow().as_ref().is_some_and(|c| c.shown)) { hide(); window.refresh(); });
}

fn hide() {
    CARD.with(|c| if let Some(card) = c.borrow_mut().as_mut().filter(|card| card.shown) { unsafe {
        card.shown = false;
        let _: bool = msg_send![card.view, endEditing: true];
        let _: () = msg_send![card.view, setHidden: true];
    } });
}

/// 输入框里写的要求，发送时取。拼音还没选字就点了发送：先把在拼的字提交（和系统「信息」一样），再去掉拼音键盘插在音节间的 U+2006。
pub fn text() -> String {
    // 先拿出指针再调：提交会触发文本变化回调，回调里还要借 CARD。
    let Some(field) = CARD.with(|c| c.borrow().as_ref().map(|card| card.text)) else { return String::new() };
    unsafe {
        let marked: *mut AnyObject = msg_send![field, markedTextRange];
        if !marked.is_null() { let _: () = msg_send![field, unmarkText]; }
    }
    text_of(field).replace('\u{2006}', "")
}

/// 占位：卡片贴着键盘，左右各留 16。
pub fn slot() -> AnyElement {
    let (_, _, viewport) = super::super::layout();
    div().id("assist-card").occlude().w(viewport.width - px(32.)).h(px(HEIGHT)).relative()
        .on_mouse_down(MouseButton::Left, |_, window, cx| { cx.stop_propagation(); window.prevent_default(); })
        .child(canvas(move |bounds, window, _| place(bounds, window), |_, _, _, _| {}).absolute().inset_0())
        .into_any_element()
}

fn place(frame: Bounds<Pixels>, window: &Window) {
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
            // 键盘不看视图的界面风格，得单独说（1 = 深色，2 = 浅色）；正在输入时换了要重载才生效。
            // 输入特性在输入框上是转发实现的，objc2 调试构建的方法检查找不到，直接走 objc_msgSend（同 assistant.rs）。
            let set: unsafe extern "C" fn(*mut AnyObject, objc2::runtime::Sel, isize) = std::mem::transmute(objc2::ffi::objc_msgSend as *const ());
            set(card.text, objc2::sel!(setKeyboardAppearance:), if dark { 1 } else { 2 });
            let typing: bool = msg_send![card.text, isFirstResponder];
            if typing { let _: () = msg_send![card.text, reloadInputViews]; }
            card.dark = Some(dark);
        }
        if card.frame != frame {
            let rect = CGRect::new(CGPoint::new(f64::from(frame.origin.x), f64::from(frame.origin.y)), CGSize::new(f64::from(frame.size.width), f64::from(frame.size.height)));
            // 已经露面的：跟着键盘升上来，用系统键盘的节奏接上。
            if card.shown {
                let (addr, to) = (card.view as usize, rect);
                let body = block2::RcBlock::new(move || { let _: () = msg_send![addr as *mut AnyObject, setFrame: to]; });
                // 动画期间照样接点击（默认会挡掉）：拼音候选条出现、键盘高度变了的那一下，正好是要点发送的时候。
                // 2 = AllowUserInteraction，4 = BeginFromCurrentState。
                let _: () = msg_send![AnyClass::get(c"UIView").unwrap(), animateWithDuration: 0.25f64, delay: 0f64, options: 6usize, animations: &*body, completion: std::ptr::null_mut::<AnyObject>()];
            } else { let _: () = msg_send![card.view, setFrame: rect]; }
            layout(card, f64::from(frame.size.width));
            card.frame = frame;
        }
        if !card.shown {
            card.shown = true;
            refresh(card);
            let last = library::Settings::load(&super::dir()).last;
            let _: () = msg_send![card.text, setText: ns_string(&last)];
            changed(card.text as usize, card.hint as usize, card.send as usize);
            let _: () = msg_send![parent, bringSubviewToFront: card.view];
            let _: () = msg_send![card.view, setHidden: false];
            // 从下面浮上来：淡入，同时由 0.96 弹到 1。
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
            let _: bool = msg_send![card.text, becomeFirstResponder];
            // 预填的整段选中：直接打字就换掉。
            if !last.is_empty() { let _: () = msg_send![card.text, selectAll: std::ptr::null_mut::<AnyObject>()]; }
        }
    });
}

unsafe fn build(parent: *mut AnyObject) -> Option<Card> {
    unsafe {
        let view: *mut AnyObject = msg_send![AnyClass::get(c"UIVisualEffectView")?, alloc];
        let effect: *mut AnyObject = msg_send![AnyClass::get(c"UIGlassEffect")?, effectWithStyle: 0isize];
        let view: *mut AnyObject = msg_send![view, initWithEffect: effect];
        if let (Some(corners), Some(corner)) = (AnyClass::get(c"UICornerConfiguration"), AnyClass::get(c"UICornerRadius")) {
            let fixed: *mut AnyObject = msg_send![corner, fixedRadius: 28f64];
            let config: *mut AnyObject = msg_send![corners, configurationWithUniformRadius: fixed];
            let _: () = msg_send![view, setCornerConfiguration: config];
        }
        let _: () = msg_send![view, setHidden: true];
        let content: *mut AnyObject = msg_send![view, contentView];
        let color = |name: &std::ffi::CStr| -> *mut AnyObject { msg_send![AnyClass::get(c"UIColor").unwrap(), performSelector: objc2::runtime::Sel::register(name)] };
        // 三颗小按钮：灰底胶囊，点了出系统菜单。
        let chip = || {
            let config: *mut AnyObject = msg_send![AnyClass::get(c"UIButtonConfiguration").unwrap(), grayButtonConfiguration];
            let _: () = msg_send![config, setCornerStyle: 4isize];
            // 1 = UIButtonConfigurationSizeSmall
            let _: () = msg_send![config, setButtonSize: 1isize];
            let _: () = msg_send![config, setBaseForegroundColor: color(c"labelColor")];
            let b: *mut AnyObject = msg_send![AnyClass::get(c"UIButton").unwrap(), buttonWithConfiguration: config, primaryAction: std::ptr::null_mut::<AnyObject>()];
            let _: () = msg_send![b, setShowsMenuAsPrimaryAction: true];
            let _: () = msg_send![content, addSubview: b];
            b
        };
        let chips = [chip(), chip(), chip()];
        // 7 = UIButtonTypeClose
        let close: *mut AnyObject = msg_send![AnyClass::get(c"UIButton").unwrap(), buttonWithType: 7isize];
        let _: () = msg_send![close, setTag: 6isize];
        on(close, 64, || super::super::menu::chosen(super::CLOSE));
        let _: () = msg_send![content, addSubview: close];
        let text: *mut AnyObject = msg_send![AnyClass::get(c"UITextView").unwrap(), new];
        let font: *mut AnyObject = msg_send![AnyClass::get(c"UIFont").unwrap(), preferredFontForTextStyle: ns_string("UICTFontTextStyleBody")];
        let _: () = msg_send![text, setFont: font];
        let clear: *mut AnyObject = msg_send![AnyClass::get(c"UIColor").unwrap(), clearColor];
        let _: () = msg_send![text, setBackgroundColor: clear];
        let _: () = msg_send![text, setTintColor: ui_color(super::mark())];
        let container: *mut AnyObject = msg_send![text, textContainer];
        let _: () = msg_send![container, setLineFragmentPadding: 0f64];
        let _: () = msg_send![content, addSubview: text];
        let hint: *mut AnyObject = msg_send![AnyClass::get(c"UILabel").unwrap(), new];
        let _: () = msg_send![hint, setFont: font];
        let _: () = msg_send![hint, setTextColor: color(c"placeholderTextColor")];
        let _: () = msg_send![hint, setText: ns_string(tr("想让助手写什么", "What should the assistant write"))];
        let _: () = msg_send![content, addSubview: hint];
        // 发送：朱红的实心圆，白色向上箭头；没写字时灰掉。
        let config: *mut AnyObject = msg_send![AnyClass::get(c"UIButtonConfiguration").unwrap(), filledButtonConfiguration];
        let _: () = msg_send![config, setCornerStyle: 4isize];
        let _: () = msg_send![config, setBaseBackgroundColor: ui_color(super::mark())];
        let symbol: *mut AnyObject = msg_send![AnyClass::get(c"UIImageSymbolConfiguration").unwrap(), configurationWithPointSize: 15f64, weight: 6isize];
        let arrow: *mut AnyObject = msg_send![AnyClass::get(c"UIImage").unwrap(), systemImageNamed: ns_string("arrow.up"), withConfiguration: symbol];
        let _: () = msg_send![config, setImage: arrow];
        let send: *mut AnyObject = msg_send![AnyClass::get(c"UIButton").unwrap(), buttonWithConfiguration: config, primaryAction: std::ptr::null_mut::<AnyObject>()];
        let _: () = msg_send![content, addSubview: send];
        on(send, 64, || super::super::menu::chosen(super::SEND));
        // 写字时：提示字和发送钮跟着变。UITextView 不是控件，听它的通知。
        let (t, h, s) = (text as usize, hint as usize, send as usize);
        let block = block2::RcBlock::new(move |_note: *mut AnyObject| changed(t, h, s));
        let center: *mut AnyObject = msg_send![AnyClass::get(c"NSNotificationCenter").unwrap(), defaultCenter];
        let _: *mut AnyObject = msg_send![center, addObserverForName: ns_string("UITextViewTextDidChangeNotification"), object: text, queue: std::ptr::null_mut::<AnyObject>(), usingBlock: &*block];
        let _: () = msg_send![parent, addSubview: view];
        Some(Card { view, chips, text, hint, send, frame: Bounds::default(), dark: None, shown: false })
    }
}

fn changed(text: usize, hint: usize, send: usize) {
    let empty = text_of(text as *mut AnyObject).trim().is_empty();
    unsafe {
        let _: () = msg_send![hint as *mut AnyObject, setHidden: !text_of(text as *mut AnyObject).is_empty()];
        let _: () = msg_send![send as *mut AnyObject, setEnabled: !empty];
    }
}

/// 控件事件接一段代码（系统在主线程上回调）。
pub(crate) unsafe fn on(control: *mut AnyObject, events: usize, run: impl Fn() + 'static) {
    unsafe {
        let handler = block2::RcBlock::new(move |_action: *mut AnyObject| run());
        let action: *mut AnyObject = msg_send![AnyClass::get(c"UIAction").unwrap(), actionWithTitle: ns_string(""), image: std::ptr::null_mut::<AnyObject>(), identifier: std::ptr::null_mut::<AnyObject>(), handler: &*handler];
        let _: () = msg_send![control, addAction: action, forControlEvents: events];
    }
}

unsafe fn layout(card: &Card, w: f64) {
    unsafe {
        let content: *mut AnyObject = msg_send![card.view, contentView];
        let set = |v: *mut AnyObject, x: f64, y: f64, w: f64, h: f64| { let _: () = msg_send![v, setFrame: CGRect::new(CGPoint::new(x, y), CGSize::new(w, h))]; };
        let close: *mut AnyObject = msg_send![content, viewWithTag: 6isize];
        set(close, w - PAD - 30., PAD + 1., 30., 30.);
        // 三颗按钮按内容宽度排，排不下先缩最长的（标题末尾截断），短的保持原样。
        let room = w - PAD * 2. - 40. - 12.;
        let mut x = PAD;
        let fits: Vec<f64> = card.chips.iter().map(|c| { let s: CGSize = msg_send![*c, intrinsicContentSize]; s.width }).collect();
        let mut cap = f64::MAX;
        if fits.iter().sum::<f64>() > room {
            let mut sorted = fits.clone();
            sorted.sort_by(f64::total_cmp);
            let (mut used, mut left) = (0., sorted.len() as f64);
            for f in &sorted { if used + f * left > room { cap = ((room - used) / left).max(44.); break; } used += f; left -= 1.; }
        }
        for (c, fit) in card.chips.iter().zip(&fits) {
            set(*c, x, PAD, fit.min(cap), CHIP);
            x += fit.min(cap) + 6.;
        }
        let y = PAD + CHIP + 6.;
        set(card.text, PAD + 2., y, w - PAD * 2. - SEND - 10., TEXT);
        set(card.hint, PAD + 2., y + 8., w - PAD * 2. - SEND - 10., 22.);
        set(card.send, w - PAD - SEND, y + TEXT - SEND, SEND, SEND);
    }
}

/// 三颗按钮的标题和菜单照当前的设置重画。选了哪一项就存下来。
unsafe fn refresh(card: &Card) {
    unsafe {
        let dir = super::dir();
        let s = library::Settings::load(&dir);
        let default = tr("默认", "Default");
        let mut presets = vec![(String::new(), default.to_owned())];
        presets.extend(library::presets(&dir, &s.names).into_iter().map(|l| (l.id, l.name)));
        let preset_name = presets.iter().find(|(id, _)| *id == s.preset).map_or(default.to_owned(), |(_, n)| n.clone());
        let chars: Vec<(String, String)> = s.chars.iter().map(|c| (c.id.clone(), c.name.clone())).collect();
        let personas: Vec<(String, String)> = s.personas.iter().map(|c| (c.id.clone(), c.name.clone())).collect();
        let face = |b: *mut AnyObject, title: &str, glyph: &str| {
            let config: *mut AnyObject = msg_send![b, configuration];
            let _: () = msg_send![config, setTitle: ns_string(title)];
            let symbol: *mut AnyObject = msg_send![AnyClass::get(c"UIImageSymbolConfiguration").unwrap(), configurationWithPointSize: 11f64, weight: 5isize];
            let image: *mut AnyObject = msg_send![AnyClass::get(c"UIImage").unwrap(), systemImageNamed: ns_string(glyph), withConfiguration: symbol];
            let _: () = msg_send![config, setImage: image];
            let _: () = msg_send![config, setImagePadding: 5f64];
            // 4 = NSLineBreakByTruncatingTail：窄了截断，不折成两行。
            let _: () = msg_send![config, setTitleLineBreakMode: 4isize];
            let _: () = msg_send![b, setConfiguration: config];
        };
        face(card.chips[0], &preset_name, "slider.horizontal.3");
        face(card.chips[1], &s.char().name, "sparkle");
        face(card.chips[2], &s.persona().name, "person");
        let menu = |title: &str, items: &[(String, String)], current: &str, pick: fn(&mut library::Settings, String)| -> *mut AnyObject {
            let list: *mut AnyObject = msg_send![AnyClass::get(c"NSMutableArray").unwrap(), array];
            for (id, name) in items {
                let picked = id.clone();
                let handler = block2::RcBlock::new(move |_action: *mut AnyObject| {
                    let mut s = library::Settings::load(&super::dir());
                    pick(&mut s, picked.clone());
                    super::save(&s);
                    CARD.with(|c| if let Some(card) = c.borrow().as_ref() { refresh(card); layout(card, f64::from(card.frame.size.width)); });
                });
                let action: *mut AnyObject = msg_send![AnyClass::get(c"UIAction").unwrap(), actionWithTitle: ns_string(name), image: std::ptr::null_mut::<AnyObject>(), identifier: std::ptr::null_mut::<AnyObject>(), handler: &*handler];
                // 1 = UIMenuElementStateOn
                if id == current { let _: () = msg_send![action, setState: 1isize]; }
                let _: () = msg_send![list, addObject: action];
            }
            msg_send![AnyClass::get(c"UIMenu").unwrap(), menuWithTitle: ns_string(title), image: std::ptr::null_mut::<AnyObject>(), identifier: std::ptr::null_mut::<AnyObject>(), options: 0usize, children: list]
        };
        let _: () = msg_send![card.chips[0], setMenu: menu(tr("预设", "Preset"), &presets, &s.preset, |s, id| s.preset = id)];
        let _: () = msg_send![card.chips[1], setMenu: menu(tr("助手", "Assistant"), &chars, &s.char, |s, id| s.char = id)];
        let _: () = msg_send![card.chips[2], setMenu: menu(tr("人设", "Persona"), &personas, &s.persona, |s, id| s.persona = id)];
    }
}

pub(crate) fn text_of(field: *mut AnyObject) -> String {
    unsafe {
        let text: *mut AnyObject = msg_send![field, text];
        if text.is_null() { return String::new(); }
        let utf8: *const std::ffi::c_char = msg_send![text, UTF8String];
        if utf8.is_null() { String::new() } else { std::ffi::CStr::from_ptr(utf8).to_string_lossy().into_owned() }
    }
}
