//! 设置 › 助手设置 › 语音。模型是应用偏好，API Key 只在设备钥匙串里保存。
use std::{cell::Cell, ffi::{CStr, CString}, ptr::null_mut};

use gpui::*;
use objc2::{class, msg_send, runtime::AnyObject};

use crate::{app::Workspace, settings::{prefs, row_ink, tr}, theme::Theme, ui};
use super::{general, menu};

#[path = "assistant_roles.rs"]
pub mod roles;

pub const DEFAULT_MODEL: &str = "eleven_v4";
const MODEL_ACTION: &str = "speech-model-";
const MODELS: &[(&str, &str, &str)] = &[
    ("speech-model-v4", DEFAULT_MODEL, "Eleven v4"),
    ("speech-model-v4-turbo", "eleven_v4_turbo", "Eleven v4 Turbo"),
    ("speech-model-v3", "eleven_v3", "Eleven v3"),
    ("speech-model-v3-conversational", "eleven_v3_conversational", "Eleven v3 Conversational"),
    ("speech-model-multilingual-v2", "eleven_multilingual_v2", "Eleven Multilingual v2"),
    ("speech-model-flash-v2-5", "eleven_flash_v2_5", "Eleven Flash v2.5"),
    ("speech-model-turbo-v2-5", "eleven_turbo_v2_5", "Eleven Turbo v2.5"),
    ("speech-model-flash-v2", "eleven_flash_v2", "Eleven Flash v2"),
    ("speech-model-turbo-v2", "eleven_turbo_v2", "Eleven Turbo v2"),
];

thread_local! { static KEY_SAVED: Cell<Option<bool>> = const { Cell::new(None) }; }

pub fn voice(cx: &mut Context<Workspace>) -> AnyElement {
    let saved = KEY_SAVED.with(Cell::get);
    let status = match saved {
        Some(true) => tr("API Key 已保存", "API Key saved"),
        Some(false) => tr("API Key 未配置", "API Key not configured"),
        None => tr("钥匙串暂不可用", "Keychain unavailable"),
    };
    let action = if saved == Some(true) { tr("管理", "Manage") } else { tr("添加", "Add") };
    let provider = div().id("elevenlabs-provider").w_full().min_h(px(68.)).px(px(18.)).py(px(12.)).flex().items_center().gap(px(12.))
        .active(|d| d.bg(Theme::fill_pressed()))
        .child(div().size(px(30.)).flex_none().rounded(px(8.)).bg(Theme::bg_body()).flex().items_center().justify_center()
            .child(svg().path("editor-icons/volume-2.svg").size(px(17.)).text_color(Theme::text())))
        .child(div().flex_1().min_w_0().flex().flex_col().gap(px(2.))
            .child(div().text_size(px(17.)).line_height(px(22.)).text_color(row_ink()).child("ElevenLabs"))
            .child(div().text_size(px(14.)).line_height(px(19.)).text_color(Theme::text_caption()).child(status)))
        .child(div().flex_none().text_size(px(15.)).text_color(Theme::primary()).child(action))
        .on_click(cx.listener(|w, _, _, cx| {
            super::glass::haptic();
            refresh_key_status();
            if !edit_key() { w.show_toast_kind(ui::ToastKind::Error, tr("无法打开 API Key 设置", "Could not open API key settings"), cx); }
            cx.notify();
        })).into_any_element();
    let p = prefs();
    let (_, current, label) = MODELS.iter().find(|(_, id, _)| *id == p.speech_model).copied().unwrap_or(MODELS[0]);
    let items = MODELS.iter().map(|&(action, id, label)| menu::Item::new(action, label, "", 0).checked(id == current)).collect();
    div().w_full().pt(px(18.)).flex().flex_col().gap(px(32.))
        .child(general::titled(Some(tr("服务商连接", "Provider Connection")), vec![provider]))
        .child(general::titled(Some(tr("语音模型", "Speech Model")), vec![general::choice("speech-model", tr("默认模型", "Default Model"), label, items)]))
        .child(roles::section(&p.speech_roles, cx))
        .into_any_element()
}

pub fn menu_action(action: &str, w: &mut Workspace, cx: &mut Context<Workspace>) -> bool {
    if !action.starts_with(MODEL_ACTION) { return false; }
    if let Some(&(_, id, _)) = MODELS.iter().find(|(pick, _, _)| *pick == action) && prefs().speech_model != id {
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

const ITEM_NOT_FOUND: i32 = -25300;

unsafe fn key_query() -> *mut AnyObject {
    unsafe {
        let query: *mut AnyObject = msg_send![class!(NSMutableDictionary), dictionary];
        let _: () = msg_send![query, setObject: kSecClassGenericPassword, forKey: kSecClass];
        let _: () = msg_send![query, setObject: ns_string("com.s1dashu.quietmd.elevenlabs"), forKey: kSecAttrService];
        let _: () = msg_send![query, setObject: ns_string("api-key"), forKey: kSecAttrAccount];
        query
    }
}

pub fn refresh_key_status() {
    let status = unsafe { SecItemCopyMatching(key_query(), null_mut()) };
    if !matches!(status, 0 | ITEM_NOT_FOUND) { log::warn!("ElevenLabs Keychain lookup returned {status}"); }
    KEY_SAVED.with(|saved| saved.set(match status { 0 => Some(true), ITEM_NOT_FOUND => Some(false), _ => None }));
}

fn read_key() -> Option<String> {
    unsafe {
        let query = key_query();
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

fn save_key(key: &str) -> i32 {
    unsafe {
        let query = key_query();
        let data: *mut AnyObject = msg_send![class!(NSData), dataWithBytes: key.as_ptr().cast::<std::ffi::c_void>(), length: key.len()];
        let changes: *mut AnyObject = msg_send![class!(NSDictionary), dictionaryWithObject: data, forKey: kSecValueData];
        let status = SecItemUpdate(query, changes);
        if status != ITEM_NOT_FOUND { return status; }
        let _: () = msg_send![query, addEntriesFromDictionary: changes];
        let _: () = msg_send![query, setObject: kSecAttrAccessibleWhenUnlockedThisDeviceOnly, forKey: kSecAttrAccessible];
        SecItemAdd(query, null_mut())
    }
}

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
    prompt("ElevenLabs API Key", Some(body), "API Key", "", true, remove, |value| match value {
        Some(key) if key.is_empty() || !key.bytes().all(|b| b.is_ascii_graphic()) => menu::dispatch(|w, _, cx| w.show_toast_kind(ui::ToastKind::Error, tr("请输入有效的 API Key", "Enter a valid API Key"), cx)),
        Some(key) => key_result(save_key(&key), true),
        None => key_result(unsafe { SecItemDelete(key_query()) }, false),
    })
}

fn prompt(title: &str, body: Option<&str>, placeholder: &str, value: &str, secure: bool, remove: Option<&str>, complete: impl Fn(Option<String>) + 'static) -> bool {
    unsafe {
        let controller = super::files::root_controller();
        if controller.is_null() { return false; }
        let presented: *mut AnyObject = msg_send![controller, presentedViewController];
        if !presented.is_null() { return false; }

        let message = body.map_or(null_mut(), |s| ns_string(s));
        let alert: *mut AnyObject = msg_send![class!(UIAlertController), alertControllerWithTitle: ns_string(title), message: message, preferredStyle: 1isize];
        let configure = block2::RcBlock::new(|field: *mut AnyObject| {
            let _: () = msg_send![field, setPlaceholder: ns_string(placeholder)];
            let _: () = msg_send![field, setText: ns_string(value)];
            let _: () = msg_send![field, setSecureTextEntry: secure];
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
        let ok = add(tr("保存", "Save"), 0, Some(&save));
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
