//! 设置 › 助手设置 › 语音。模型是应用偏好，API Key 只在设备钥匙串里保存。
use std::{cell::Cell, ffi::{CStr, CString}, ptr::null_mut};

use gpui::{prelude::FluentBuilder, *};
use objc2::{class, msg_send, runtime::AnyObject};

use crate::{app::Workspace, settings::{Page, prefs, row_ink, tr}, theme::Theme, ui};
use super::{general, menu};

#[path = "assistant_roles.rs"]
pub mod roles;
#[path = "assistant_preview.rs"]
pub mod preview;
#[path = "assistant_text.rs"]
pub mod text;
#[path = "assistant_llm.rs"]
pub mod llm;
#[path = "assistant_writing.rs"]
pub mod writing;
#[path = "assistant_sheet.rs"]
pub mod sheet;
#[path = "assistant_image.rs"]
pub mod image;
#[path = "assistant_image_prompts.rs"]
pub mod prompts;

pub const DEFAULT_MODEL: &str = "eleven_v4";
const MODEL_ACTION: &str = "speech-model-";
/// 一张模型卡片：(id, 模型名, 名称, 版本, 一句话, 标签)。
pub(super) type Card = (&'static str, &'static str, &'static str, &'static str, (&'static str, &'static str), (&'static str, &'static str));
/// 前两条的说法照 ElevenLabs 应用里的模型卡片。
const MODELS: &[Card] = &[
    ("speech-model-v4", DEFAULT_MODEL, "Eleven v4", "v4", ("情感幅度与保真度最高", "Maximum emotional range and fidelity"), ("最高质量", "Highest quality")),
    ("speech-model-v4-turbo", "eleven_v4_turbo", "Eleven v4 Turbo", "v4", ("v4 的低延迟版本", "Lower-latency v4"), ("快速", "Fast")),
    ("speech-model-v3", "eleven_v3", "Eleven v3", "v3", ("富有表现力", "Expressive model"), ("高质量", "High quality")),
    ("speech-model-v3-conversational", "eleven_v3_conversational", "Eleven v3 Conversational", "v3", ("为实时对话调校", "Tuned for live conversation"), ("对话", "Conversational")),
    ("speech-model-multilingual-v2", "eleven_multilingual_v2", "Eleven Multilingual v2", "v2", ("自然稳定的多语种朗读", "Lifelike, stable multilingual speech"), ("高质量", "High quality")),
    ("speech-model-flash-v2-5", "eleven_flash_v2_5", "Eleven Flash v2.5", "v2.5", ("超低延迟，多语种", "Ultra-low latency, multilingual"), ("最快", "Fastest")),
    ("speech-model-turbo-v2-5", "eleven_turbo_v2_5", "Eleven Turbo v2.5", "v2.5", ("质量与速度兼顾，多语种", "Balanced quality and speed"), ("快速", "Fast")),
    ("speech-model-flash-v2", "eleven_flash_v2", "Eleven Flash v2", "v2", ("超低延迟，仅英语", "Ultra-low latency, English only"), ("仅英语", "English")),
    ("speech-model-turbo-v2", "eleven_turbo_v2", "Eleven Turbo v2", "v2", ("质量与速度兼顾，仅英语", "Balanced, English only"), ("仅英语", "English")),
];

thread_local! { static KEY_SAVED: Cell<Option<bool>> = const { Cell::new(None) }; }

/// 第二级页面回到哪一页：语音的音色列表、角色设置回语音，选头像回角色设置；模型列表、服务商回文本；画风、人物回图像；写作的几页见 `writing::parent`。
pub fn parent(page: Page) -> Option<Page> {
    if let Some(p) = writing::parent(page) { return Some(p); }
    match page { Page::AssistantVoices | Page::AssistantRole => Some(Page::AssistantSpeech), Page::AssistantAvatar => Some(roles::avatars_parent()), Page::AssistantModels | Page::AssistantProvider => Some(Page::AssistantText), Page::AssistantStyle | Page::AssistantPerson => Some(Page::AssistantImage), _ => None }
}

/// 打开助手设置的某一页时：重新看一眼钥匙串，文本页顺便取模型列表。
pub fn opened(page: Page, cx: &mut Context<Workspace>) {
    match page {
        Page::AssistantSpeech => { refresh_key_status(); roles::reload(); }
        Page::AssistantImage | Page::AssistantStyle | Page::AssistantPerson => { image::opened(); prompts::opened(); }
        Page::AssistantText | Page::AssistantModels | Page::AssistantProvider => text::opened(page, cx),
        Page::AssistantWriting | Page::AssistantPreset | Page::AssistantEntries | Page::AssistantEntry | Page::AssistantRegexes | Page::AssistantRegex | Page::AssistantCard => writing::opened(page, cx),
        _ => {}
    }
}

pub fn voice(cx: &mut Context<Workspace>) -> AnyElement {
    // 照系统设置的一行：左边服务商，右边灰字写现在的状态，点开是 API Key 的对话框。
    let status = match KEY_SAVED.with(Cell::get) {
        Some(true) => tr("已连接", "Connected"),
        Some(false) => tr("添加 API Key", "Add API Key"),
        None => tr("钥匙串暂不可用", "Keychain unavailable"),
    };
    let provider = div().id("elevenlabs-provider").w_full().h(px(54.)).px(px(18.)).flex().items_center().gap(px(10.))
        .active(|d| d.bg(Theme::fill_pressed()))
        .child(div().flex_1().min_w_0().text_size(px(17.)).text_color(row_ink()).child("ElevenLabs"))
        .child(div().flex_none().text_size(px(17.)).text_color(Theme::text_caption()).child(status))
        .child(ui::icon("chevron-right", 14., crate::settings::row_chevron()))
        .on_click(cx.listener(|w, _, _, cx| open_key(w, cx))).into_any_element();
    let p = prefs();
    let current = MODELS.iter().find(|m| m.1 == p.speech_model).map_or(DEFAULT_MODEL, |m| m.1);
    div().w_full().pt(px(18.)).flex().flex_col().gap(px(32.))
        .child(general::titled(Some(tr("服务商连接", "Provider Connection")), vec![provider]))
        .child(model_cards("speech-models", tr("语音模型", "Speech Model"), MODELS, current, |p, id| p.speech_model = id.into(), cx))
        .child(roles::section(&roles::all(), cx))
        .into_any_element()
}

fn open_key(w: &mut Workspace, cx: &mut Context<Workspace>) {
    super::glass::haptic();
    refresh_key_status();
    if !edit_key() { w.show_toast_kind(ui::ToastKind::Error, tr("无法打开 API Key 设置", "Could not open API key settings"), cx); }
    cx.notify();
}

/// 模型卡片（照 ElevenLabs 应用）：一排横着滑的卡片，左上角版本号，下面名称、一句话和标签；选中的描一圈并打勾。语音和图像共用。
pub(super) fn model_cards(id: &'static str, title: &'static str, cards: &'static [Card], current: &str, pick: fn(&mut crate::settings::Preferences, &'static str), cx: &mut Context<Workspace>) -> AnyElement {
    const W: f32 = 172.;
    const GAP: f32 = 10.;
    let chip = |text: &'static str, on: bool| div().px(px(9.)).h(px(24.)).rounded_full().flex().items_center()
        .bg(if on { crate::theme::ca(crate::theme::craft::N1000, 0.07) } else { Theme::bg_body() })
        .text_size(px(12.)).font_weight(FontWeight::MEDIUM).text_color(Theme::text_caption()).child(text);
    let items = cards.iter().map(|&(action, model, label, version, line, tag)| {
        let on = model == current;
        div().id(action).w(px(W)).h(px(172.)).flex_none().p(px(14.)).rounded(px(20.)).bg(general::card_bg())
            .border_2().border_color(if on { Theme::text() } else { gpui::transparent_black() })
            .flex().flex_col().active(|d| d.bg(Theme::fill_pressed()))
            .child(div().flex().items_center().justify_between().child(chip(version, false))
                .when(on, |d| d.child(div().size(px(22.)).rounded_full().bg(Theme::text()).flex().items_center().justify_center()
                    .child(ui::icon("check", 13., Theme::bg_body())))))
            .child(div().mt(px(14.)).text_size(px(16.)).line_height(px(21.)).font_weight(FontWeight::SEMIBOLD).text_color(Theme::text()).child(label))
            .child(div().mt(px(4.)).flex_1().text_size(px(14.)).line_height(px(19.)).text_color(Theme::text_caption()).child(tr(line.0, line.1)))
            .child(div().flex().child(chip(tr(tag.0, tag.1), false)))
            .on_click(cx.listener(move |w, _, _, cx| if !on { super::glass::haptic(); w.set_pref(move |p| pick(p, model), cx); }))
    });
    let width = 20. + cards.len() as f32 * W + cards.len().saturating_sub(1) as f32 * GAP;
    div().w_full().flex().flex_col()
        .child(roles::header(title))
        .child(div().id(id).w_full().overflow_x_scroll().restrict_scroll_to_axis()
            .child(div().w(px(width)).flex_none().px(px(10.)).flex().gap(px(GAP)).children(items)))
        .into_any_element()
}

pub fn menu_action(action: &str, w: &mut Workspace, cx: &mut Context<Workspace>) -> bool {
    if action.starts_with("text-") { return text::menu_action(action, w, cx); }
    if action.starts_with("tavern-") { return writing::menu_action(action, w, cx); }
    if action.starts_with("image-") { return image::menu_action(action, w, cx) || prompts::menu_action(action, w, cx); }
    if action.starts_with("speech-role-lang-") { return roles::menu_action(action, w, cx); }
    if !action.starts_with(MODEL_ACTION) { return false; }
    if let Some(&(_, id, ..)) = MODELS.iter().find(|m| m.0 == action) && prefs().speech_model != id {
        w.set_pref(|p| p.speech_model = id.into(), cx);
    }
    true
}

#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    static kSecClass: *const AnyObject;
    static kSecClassGenericPassword: *const AnyObject;
    static kSecAttrService: *const AnyObject;
    static kSecAttrAccount: *const AnyObject;
    static kSecAttrAccessible: *const AnyObject;
    static kSecAttrAccessibleWhenUnlockedThisDeviceOnly: *const AnyObject;
    static kSecReturnData: *const AnyObject;
    static kSecValueData: *const AnyObject;
    fn SecItemCopyMatching(query: *const AnyObject, result: *mut *const AnyObject) -> i32;
    fn SecItemAdd(attributes: *const AnyObject, result: *mut *const AnyObject) -> i32;
    fn SecItemUpdate(query: *const AnyObject, attributes: *const AnyObject) -> i32;
    fn SecItemDelete(query: *const AnyObject) -> i32;
}

pub(super) const ITEM_NOT_FOUND: i32 = -25300;

/// 钥匙串里的一项：服务名和账户名。
#[derive(Clone, Copy)]
pub(super) struct Secret<'a> { pub service: &'a str, pub account: &'a str }
const ELEVENLABS: Secret = Secret { service: "com.s1dashu.quietmd.elevenlabs", account: "api-key" };

unsafe fn key_query(item: Secret) -> *mut AnyObject {
    unsafe {
        let query: *mut AnyObject = msg_send![class!(NSMutableDictionary), dictionary];
        let _: () = msg_send![query, setObject: kSecClassGenericPassword, forKey: kSecClass];
        let _: () = msg_send![query, setObject: ns_string(item.service), forKey: kSecAttrService];
        let _: () = msg_send![query, setObject: ns_string(item.account), forKey: kSecAttrAccount];
        query
    }
}

/// 存没存着：`None` 是钥匙串这会儿查不了。
pub(super) fn key_saved(item: Secret) -> Option<bool> {
    let status = unsafe { SecItemCopyMatching(key_query(item), null_mut()) };
    if !matches!(status, 0 | ITEM_NOT_FOUND) { log::warn!("Keychain lookup for {} returned {status}", item.service); }
    match status { 0 => Some(true), ITEM_NOT_FOUND => Some(false), _ => None }
}

pub fn refresh_key_status() { KEY_SAVED.with(|saved| saved.set(key_saved(ELEVENLABS))); }

pub(crate) fn read_key() -> Option<String> { read_secret(ELEVENLABS) }

pub(super) fn read_secret(item: Secret) -> Option<String> {
    unsafe {
        let query = key_query(item);
        let yes: *mut AnyObject = msg_send![class!(NSNumber), numberWithBool: true];
        let _: () = msg_send![query, setObject: yes, forKey: kSecReturnData];
        let mut result = std::ptr::null();
        if SecItemCopyMatching(query, &mut result) != 0 { return None; }
        let data = objc2::rc::Retained::from_raw(result.cast_mut())?;
        let len: usize = msg_send![&*data, length];
        let bytes: *const std::ffi::c_void = msg_send![&*data, bytes];
        if len == 0 || bytes.is_null() { return None; }
        std::str::from_utf8(std::slice::from_raw_parts(bytes.cast(), len)).ok().map(str::to_owned)
    }
}

/// 存一项（已有就改），返回 `OSStatus`。
pub(super) fn save_secret(item: Secret, key: &str) -> i32 {
    unsafe {
        let query = key_query(item);
        let data: *mut AnyObject = msg_send![class!(NSData), dataWithBytes: key.as_ptr().cast::<std::ffi::c_void>(), length: key.len()];
        let changes: *mut AnyObject = msg_send![class!(NSDictionary), dictionaryWithObject: data, forKey: kSecValueData];
        let status = SecItemUpdate(query, changes);
        if status != ITEM_NOT_FOUND { return status; }
        let _: () = msg_send![query, addEntriesFromDictionary: changes];
        let _: () = msg_send![query, setObject: kSecAttrAccessibleWhenUnlockedThisDeviceOnly, forKey: kSecAttrAccessible];
        SecItemAdd(query, null_mut())
    }
}

pub(super) fn delete_secret(item: Secret) -> i32 { unsafe { SecItemDelete(key_query(item)) } }

fn key_result(status: i32, saved: bool) {
    let ok = status == 0 || !saved && status == ITEM_NOT_FOUND;
    if ok { KEY_SAVED.with(|state| state.set(Some(saved))); roles::key_changed(); }
    else { log::warn!("ElevenLabs Keychain update returned {status}"); }
    menu::dispatch(move |w, _, cx| {
        if ok { cx.notify(); } else {
            let text = if saved { tr("无法保存 API Key", "Could not save API Key") } else { tr("无法移除 API Key", "Could not remove API Key") };
            w.show_toast_kind(ui::ToastKind::Error, text, cx);
        }
    });
}

fn edit_key() -> bool {
    let body = tr("API Key 仅保存在此设备的钥匙串中。", "The API Key is stored in this device’s Keychain.");
    let remove = (KEY_SAVED.with(Cell::get) == Some(true)).then(|| tr("移除 API Key", "Remove API Key"));
    prompt("ElevenLabs API Key", Some(body), "API Key", "", Field::Secret, remove, |value| match value {
        Some(key) if !valid_key(&key) => menu::dispatch(|w, _, cx| w.show_toast_kind(ui::ToastKind::Error, tr("请输入有效的 API Key", "Enter a valid API Key"), cx)),
        Some(key) => key_result(save_secret(ELEVENLABS, &key), true),
        None => key_result(delete_secret(ELEVENLABS), false),
    })
}

pub(super) fn valid_key(key: &str) -> bool { !key.is_empty() && key.bytes().all(|b| b.is_ascii_graphic()) }

/// 对话框里输入框的种类：密钥（圆点遮住）、网址（网址键盘），都不自动大写、不自动更正；或者普通的一行字。
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Field { Text, Search, Secret, Url }

pub(super) fn prompt(title: &str, body: Option<&str>, placeholder: &str, value: &str, field: Field, remove: Option<&str>, complete: impl Fn(Option<String>) + 'static) -> bool {
    unsafe {
        let controller = super::files::root_controller();
        if controller.is_null() { return false; }
        let presented: *mut AnyObject = msg_send![controller, presentedViewController];
        if !presented.is_null() { return false; }

        let message = body.map_or(null_mut(), |s| ns_string(s));
        let alert: *mut AnyObject = msg_send![class!(UIAlertController), alertControllerWithTitle: ns_string(title), message: message, preferredStyle: 1isize];
        let kind = field;
        let configure = block2::RcBlock::new(move |field: *mut AnyObject| {
            let _: () = msg_send![field, setPlaceholder: ns_string(placeholder)];
            let _: () = msg_send![field, setText: ns_string(value)];
            let _: () = msg_send![field, setSecureTextEntry: kind == Field::Secret];
            if matches!(kind, Field::Secret | Field::Url) {
                // UIKeyboardTypeURL = 3；不自动大写、不自动更正、不查拼写。这几个输入特性在输入框上是转发实现的，
                // objc2 调试构建的方法检查找不到，直接走 objc_msgSend。
                let set: unsafe extern "C" fn(*mut AnyObject, objc2::runtime::Sel, isize) = std::mem::transmute(objc2::ffi::objc_msgSend as *const ());
                if kind == Field::Url { set(field, objc2::sel!(setKeyboardType:), 3); }
                set(field, objc2::sel!(setAutocapitalizationType:), 0);
                set(field, objc2::sel!(setAutocorrectionType:), 1);
                set(field, objc2::sel!(setSpellCheckingType:), 1);
            }
        });
        let _: () = msg_send![alert, addTextFieldWithConfigurationHandler: &*configure];
        let fields: *mut AnyObject = msg_send![alert, textFields];
        let field: *mut AnyObject = msg_send![fields, firstObject];
        let add = |title: &str, style: isize, handler: Option<&block2::Block<dyn Fn(*mut AnyObject)>>| {
            let action: *mut AnyObject = msg_send![class!(UIAlertAction), actionWithTitle: ns_string(title), style: style, handler: handler];
            let _: () = msg_send![alert, addAction: action];
            action
        };
        add(tr("取消", "Cancel"), 1, None);
        let complete = std::rc::Rc::new(complete);
        if let Some(label) = remove {
            let run = complete.clone();
            let remove = block2::RcBlock::new(move |_: *mut AnyObject| run(None));
            add(label, 2, Some(&remove));
        }
        // 输入框由 alert 持有，保存回调执行时仍然有效。不回填已经保存的 Key。
        let save = block2::RcBlock::new(move |_: *mut AnyObject| {
            let text: *mut AnyObject = msg_send![field, text];
            let chars: *const std::ffi::c_char = msg_send![text, UTF8String];
            let value = if chars.is_null() { String::new() } else { CStr::from_ptr(chars).to_string_lossy().trim().to_owned() };
            complete(Some(value));
        });
        let ok = add(if kind == Field::Search { tr("搜索", "Search") } else { tr("保存", "Save") }, 0, Some(&save));
        let _: () = msg_send![alert, setPreferredAction: ok];
        let _: () = msg_send![alert, setOverrideUserInterfaceStyle: if crate::theme::dark() { 2isize } else { 1isize }];
        let none: Option<&block2::Block<dyn Fn()>> = None;
        let _: () = msg_send![controller, presentViewController: alert, animated: true, completion: none];
        true
    }
}

unsafe fn ns_string(s: &str) -> *mut AnyObject {
    let c = CString::new(s).unwrap_or_default();
    unsafe { msg_send![class!(NSString), stringWithUTF8String: c.as_ptr()] }
}
