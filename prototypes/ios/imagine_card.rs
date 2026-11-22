//! 配图的卡片：一整张原生玻璃，贴在键盘上面。上面一排是比例（1:1、2:3、3:2，系统的分段控件），右边「优化描述」开关（魔杖，只有图标）和关闭；
//! 下面是图像描述（多行），右下角发送。和助手的卡片（`assist_card.rs`）一个做法：GPUI 那边只留一块占位（`slot`），卡片跟着占位走。
use std::cell::{Cell, RefCell};

use gpui::*;
use objc2::msg_send;
use objc2::runtime::{AnyClass, AnyObject};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::settings::tr;
use super::super::assist::card::{on, text_of};
use super::super::glass::{ns_string, ui_color};

pub const HEIGHT: f32 = (PAD + SEG + 6. + TEXT + PAD) as f32;
const PAD: f64 = 14.;
const SEG: f64 = 32.;
const TEXT: f64 = 104.;
const SEND: f64 = 36.;
/// 分段的顺序和偏好里的比例键。
pub const RATIOS: [(&str, &str); 3] = [("1:1", "square"), ("2:3", "portrait"), ("3:2", "landscape")];

struct Card { view: *mut AnyObject, seg: *mut AnyObject, wand: *mut AnyObject, text: *mut AnyObject, hint: *mut AnyObject, send: *mut AnyObject, frame: Bounds<Pixels>, dark: Option<bool>, shown: bool }
const WAND: f64 = 36.;
thread_local! {
    static CARD: RefCell<Option<Card>> = const { RefCell::new(None) };
    static SEEN: Cell<bool> = const { Cell::new(false) };
    /// 给选中的块配图：输入框是可不填的补充要求，没有「优化描述」（一定先整理）。
    static PASSAGE: Cell<bool> = const { Cell::new(false) };
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

/// 写的描述：拼音还没选字就点了发送，先把在拼的字提交，再去掉拼音键盘插在音节间的 U+2006（同助手）。
pub fn text() -> String {
    let Some(field) = CARD.with(|c| c.borrow().as_ref().map(|card| card.text)) else { return String::new() };
    unsafe {
        let marked: *mut AnyObject = msg_send![field, markedTextRange];
        if !marked.is_null() { let _: () = msg_send![field, unmarkText]; }
    }
    text_of(field).replace('\u{2006}', "")
}

/// 「优化描述」开关的样子：开着是强调色的浅底，关着是灰的。
unsafe fn paint_wand(wand: *mut AnyObject, on: bool) {
    unsafe {
        let config: *mut AnyObject = if on { msg_send![AnyClass::get(c"UIButtonConfiguration").unwrap(), tintedButtonConfiguration] }
            else { msg_send![AnyClass::get(c"UIButtonConfiguration").unwrap(), grayButtonConfiguration] };
        let _: () = msg_send![config, setCornerStyle: 4isize];
        let symbol: *mut AnyObject = msg_send![AnyClass::get(c"UIImageSymbolConfiguration").unwrap(), configurationWithPointSize: 14f64, weight: 5isize];
        let image: *mut AnyObject = msg_send![AnyClass::get(c"UIImage").unwrap(), systemImageNamed: ns_string("wand.and.sparkles"), withConfiguration: symbol];
        let _: () = msg_send![config, setImage: image];
        let tint = if on { crate::theme::Theme::primary() } else { crate::theme::Theme::text_caption() };
        let _: () = msg_send![config, setBaseForegroundColor: ui_color(tint)];
        if on { let _: () = msg_send![config, setBaseBackgroundColor: ui_color(crate::theme::Theme::primary())]; }
        let _: () = msg_send![wand, setConfiguration: config];
    }
}

/// 开关换了样子（点了以后由工作区决定开不开得了）。
pub fn set_optimize(on: bool) { CARD.with(|c| if let Some(card) = c.borrow().as_ref() { unsafe { paint_wand(card.wand, on) } }); }

/// 整理描述时：发送钮换成转圈，比例、开关不能动，字淡下去（键盘留着）；关闭照样能点（就是取消）。
/// 打开前定下这次是不是给选中的块配图。
pub fn set_passage(on: bool) { PASSAGE.with(|p| p.set(on)); }

pub fn busy(on: bool) { CARD.with(|c| if let Some(card) = c.borrow().as_ref() { unsafe { set_busy(card, on) } }); }
unsafe fn set_busy(card: &Card, on: bool) {
    unsafe {
        let config: *mut AnyObject = msg_send![card.send, configuration];
        let config: *mut AnyObject = msg_send![config, copy];
        let _: () = msg_send![config, setShowsActivityIndicator: on];
        let _: () = msg_send![card.send, setConfiguration: config];
        let _: () = msg_send![config, release];
        let _: () = msg_send![card.send, setUserInteractionEnabled: !on];
        let _: () = msg_send![card.seg, setEnabled: !on];
        let _: () = msg_send![card.wand, setEnabled: !on];
        // 不改可编辑（改了键盘就收了，卡片底下空一截）；字淡一点表示这次按的是它。
        let _: () = msg_send![card.text, setAlpha: if on { 0.5f64 } else { 1f64 }];
        if !on { changed(card.text as usize, card.hint as usize, card.send as usize); }
    }
}

/// 收起键盘（卡片留着，确认页盖在上面）。
pub fn blur() { CARD.with(|c| if let Some(card) = c.borrow().as_ref() { unsafe { let _: bool = msg_send![card.view, endEditing: true]; } }); }
/// 回到输入框（确认页关掉了）。
pub fn focus() { CARD.with(|c| if let Some(card) = c.borrow().as_ref().filter(|c| c.shown) { unsafe { let _: bool = msg_send![card.text, becomeFirstResponder]; } }); }

/// 选中的比例键。
pub fn ratio() -> &'static str {
    let index: isize = CARD.with(|c| c.borrow().as_ref().map_or(-1, |card| unsafe { msg_send![card.seg, selectedSegmentIndex] }));
    RATIOS.get(index.max(0) as usize).map_or(RATIOS[1].1, |r| r.1)
}

/// 占位：卡片贴着键盘，左右各留 16。
pub fn slot() -> AnyElement {
    let (_, _, viewport) = super::super::layout();
    div().id("imagine-card").occlude().w(viewport.width - px(32.)).h(px(HEIGHT)).relative()
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
            // 键盘不看视图的界面风格（1 = 深色，2 = 浅色），同助手的卡片。
            let set: unsafe extern "C" fn(*mut AnyObject, objc2::runtime::Sel, isize) = std::mem::transmute(objc2::ffi::objc_msgSend as *const ());
            set(card.text, objc2::sel!(setKeyboardAppearance:), if dark { 1 } else { 2 });
            let typing: bool = msg_send![card.text, isFirstResponder];
            if typing { let _: () = msg_send![card.text, reloadInputViews]; }
            card.dark = Some(dark);
        }
        if card.frame != frame {
            let rect = CGRect::new(CGPoint::new(f64::from(frame.origin.x), f64::from(frame.origin.y)), CGSize::new(f64::from(frame.size.width), f64::from(frame.size.height)));
            if card.shown {
                let (addr, to) = (card.view as usize, rect);
                let body = block2::RcBlock::new(move || { let _: () = msg_send![addr as *mut AnyObject, setFrame: to]; });
                let _: () = msg_send![AnyClass::get(c"UIView").unwrap(), animateWithDuration: 0.25f64, delay: 0f64, options: 6usize, animations: &*body, completion: std::ptr::null_mut::<AnyObject>()];
            } else { let _: () = msg_send![card.view, setFrame: rect]; }
            layout(card, f64::from(frame.size.width));
            card.frame = frame;
        }
        if !card.shown {
            card.shown = true;
            // 每次打开从空白开始，比例照上一次（设置 › 图像 里的那个）。
            let key = crate::ios::assistant::image::size_key();
            let index = RATIOS.iter().position(|r| r.1 == key).unwrap_or(1) as isize;
            let _: () = msg_send![card.seg, setSelectedSegmentIndex: index];
            let _: () = msg_send![card.text, setText: ns_string("")];
            paint_wand(card.wand, crate::ios::assistant::prompts::lib().optimize);
            let passage = PASSAGE.with(Cell::get);
            let _: () = msg_send![card.wand, setHidden: passage];
            let _: () = msg_send![card.hint, setText: ns_string(if passage { tr("补充要求（可不填）", "Anything to add (optional)") } else { tr("描述想要的画面", "Describe the image") })];
            set_busy(card, false);
            changed(card.text as usize, card.hint as usize, card.send as usize);
            let _: () = msg_send![parent, bringSubviewToFront: card.view];
            let _: () = msg_send![card.view, setHidden: false];
            // 从下面浮上来：淡入，同时由 0.96 弹到 1（同助手）。
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
        // 比例：系统的分段控件，换了就记进偏好（下次打开、设置页都照它）。
        let items: *mut AnyObject = msg_send![AnyClass::get(c"NSMutableArray").unwrap(), array];
        for (label, _) in RATIOS { let _: () = msg_send![items, addObject: ns_string(label)]; }
        let seg: *mut AnyObject = msg_send![AnyClass::get(c"UISegmentedControl").unwrap(), alloc];
        let seg: *mut AnyObject = msg_send![seg, initWithItems: items];
        let _: () = msg_send![content, addSubview: seg];
        let s = seg as usize;
        // 4096 = UIControlEventValueChanged
        on(seg, 4096, move || {
            let index: isize = msg_send![s as *mut AnyObject, selectedSegmentIndex];
            let Some(&(_, key)) = RATIOS.get(index.max(0) as usize) else { return };
            super::super::glass::haptic();
            super::super::menu::dispatch(move |w, _, cx| if crate::settings::prefs().image_size != key { w.set_pref(|p| p.image_size = key.into(), cx); });
        });
        // 「优化描述」：只有图标的小钮，点了交给工作区（没有文本模型时开不了）。
        let wand: *mut AnyObject = msg_send![AnyClass::get(c"UIButton").unwrap(), buttonWithType: 0isize];
        let label = ns_string(tr("优化描述", "Optimize Description"));
        let _: () = msg_send![wand, setAccessibilityLabel: label];
        paint_wand(wand, false);
        on(wand, 64, || { super::super::glass::haptic(); super::super::menu::chosen(super::OPTIMIZE) });
        let _: () = msg_send![content, addSubview: wand];
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
        let _: () = msg_send![text, setTintColor: ui_color(crate::theme::Theme::primary())];
        let container: *mut AnyObject = msg_send![text, textContainer];
        let _: () = msg_send![container, setLineFragmentPadding: 0f64];
        let _: () = msg_send![content, addSubview: text];
        let hint: *mut AnyObject = msg_send![AnyClass::get(c"UILabel").unwrap(), new];
        let _: () = msg_send![hint, setFont: font];
        let _: () = msg_send![hint, setTextColor: color(c"placeholderTextColor")];
        let _: () = msg_send![hint, setText: ns_string(tr("描述想要的画面", "Describe the image"))];
        let _: () = msg_send![content, addSubview: hint];
        // 发送：强调色的实心圆，白色向上箭头；没写字时灰掉。
        let config: *mut AnyObject = msg_send![AnyClass::get(c"UIButtonConfiguration").unwrap(), filledButtonConfiguration];
        let _: () = msg_send![config, setCornerStyle: 4isize];
        let _: () = msg_send![config, setBaseBackgroundColor: ui_color(crate::theme::Theme::primary())];
        let symbol: *mut AnyObject = msg_send![AnyClass::get(c"UIImageSymbolConfiguration").unwrap(), configurationWithPointSize: 15f64, weight: 6isize];
        let arrow: *mut AnyObject = msg_send![AnyClass::get(c"UIImage").unwrap(), systemImageNamed: ns_string("arrow.up"), withConfiguration: symbol];
        let _: () = msg_send![config, setImage: arrow];
        let send: *mut AnyObject = msg_send![AnyClass::get(c"UIButton").unwrap(), buttonWithConfiguration: config, primaryAction: std::ptr::null_mut::<AnyObject>()];
        let _: () = msg_send![content, addSubview: send];
        on(send, 64, || super::super::menu::chosen(super::SEND));
        let (t, h, s) = (text as usize, hint as usize, send as usize);
        let block = block2::RcBlock::new(move |_note: *mut AnyObject| changed(t, h, s));
        let center: *mut AnyObject = msg_send![AnyClass::get(c"NSNotificationCenter").unwrap(), defaultCenter];
        let _: *mut AnyObject = msg_send![center, addObserverForName: ns_string("UITextViewTextDidChangeNotification"), object: text, queue: std::ptr::null_mut::<AnyObject>(), usingBlock: &*block];
        let _: () = msg_send![parent, addSubview: view];
        Some(Card { view, seg, wand, text, hint, send, frame: Bounds::default(), dark: None, shown: false })
    }
}

fn changed(text: usize, hint: usize, send: usize) {
    let written = text_of(text as *mut AnyObject);
    unsafe {
        let _: () = msg_send![hint as *mut AnyObject, setHidden: !written.is_empty()];
        let _: () = msg_send![send as *mut AnyObject, setEnabled: !written.trim().is_empty() || PASSAGE.with(Cell::get)];
    }
}

unsafe fn layout(card: &Card, w: f64) {
    unsafe {
        let content: *mut AnyObject = msg_send![card.view, contentView];
        let set = |v: *mut AnyObject, x: f64, y: f64, w: f64, h: f64| { let _: () = msg_send![v, setFrame: CGRect::new(CGPoint::new(x, y), CGSize::new(w, h))]; };
        let close: *mut AnyObject = msg_send![content, viewWithTag: 6isize];
        set(close, w - PAD - 30., PAD + 1., 30., 30.);
        set(card.wand, w - PAD - 30. - 10. - WAND, PAD, WAND, SEG);
        set(card.seg, PAD, PAD, (w - PAD * 2. - 40. - WAND - 10.).min(210.), SEG);
        let y = PAD + SEG + 6.;
        set(card.text, PAD + 2., y, w - PAD * 2. - SEND - 10., TEXT);
        set(card.hint, PAD + 2., y + 8., w - PAD * 2. - SEND - 10., 22.);
        set(card.send, w - PAD - SEND, y + TEXT - SEND, SEND, SEND);
    }
}
