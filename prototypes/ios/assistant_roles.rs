//! ElevenLabs 音色列表和本机保存的语音角色。
use std::{cell::{Cell, RefCell}, collections::HashSet, fmt::Write, path::{Path, PathBuf}, ptr::null_mut};

use gpui::{prelude::FluentBuilder, *};
use objc2::{ClassType, class, define_class, msg_send, runtime::{AnyObject, NSObject}};
use serde::{Deserialize, Serialize};

use crate::{app::Workspace, input::TextField, settings::{Page, prefs, row_chevron, row_ink, tr}, theme::Theme, ui};
use super::{general, menu, ns_string};

#[derive(Clone, Deserialize, Serialize)]
pub struct Role {
    voice_id: String,
    voice_name: String,
    name: String,
    #[serde(default)]
    avatar: Option<String>,
}

#[derive(Clone, Deserialize)]
struct Voice { voice_id: String, name: String }

#[derive(Deserialize)]
struct VoicePage { voices: Vec<Voice>, has_more: bool, next_page_token: Option<String> }

enum LoadError { Key, Network, Http(isize), Response }
enum Catalog { Idle, Loading, Ready(Vec<Voice>), Failed(LoadError) }

thread_local! {
    static CATALOG: RefCell<Catalog> = const { RefCell::new(Catalog::Idle) };
    static REQUEST: Cell<u64> = const { Cell::new(0) };
    static SEARCH: RefCell<Option<Entity<TextField>>> = const { RefCell::new(None) };
    static EDITING: RefCell<String> = const { RefCell::new(String::new()) };
}

pub(super) fn key_changed() {
    REQUEST.with(|r| r.set(r.get() + 1));
    CATALOG.with(|c| *c.borrow_mut() = Catalog::Idle);
}

pub fn is_detail(page: Page) -> bool { matches!(page, Page::AssistantVoices | Page::AssistantRole) }

pub(super) fn section(roles: &[Role], cx: &mut Context<Workspace>) -> AnyElement {
    let available = super::KEY_SAVED.with(Cell::get) == Some(true);
    let mut rows: Vec<_> = roles.iter().map(|role| {
        let id = role.voice_id.clone();
        item(SharedString::from(format!("speech-role-{id}")), role.name.clone(), Some("ElevenLabs".into()), Some(avatar(role, 34.)), ui::icon("chevron-right", 14., row_chevron()))
            .on_click(cx.listener(move |w, _, _, cx| open_role(w, id.clone(), cx))).into_any_element()
    }).collect();
    rows.push(item("speech-add-role", tr("添加角色", "Add Role"),
        (!available).then(|| tr("先配置 ElevenLabs API Key", "Set up an ElevenLabs API Key first").into()), None,
        ui::icon("chevron-right", 14., row_chevron()))
        .when(available, |d| d.on_click(cx.listener(|w, _, _, cx| open_catalog(w, cx))))
        .into_any_element());
    general::titled(Some(tr("语音角色", "Voice Roles")), rows).into_any_element()
}

fn item(id: impl Into<ElementId>, name: impl Into<SharedString>, subtitle: Option<SharedString>, icon: Option<AnyElement>, control: impl IntoElement) -> Stateful<Div> {
    div().id(id).w_full().min_h(px(54.)).px(px(18.)).py(px(11.)).flex().items_center().gap(px(12.)).active(|d| d.bg(Theme::fill_pressed()))
        .children(icon)
        .child(div().flex_1().min_w_0().flex().flex_col().gap(px(2.))
            .child(div().truncate().text_size(px(17.)).line_height(px(22.)).text_color(row_ink()).child(name.into()))
            .children(subtitle.map(|s| div().truncate().text_size(px(14.)).line_height(px(19.)).text_color(Theme::text_caption()).child(s))))
        .child(div().flex_none().flex().items_center().child(control))
}

fn open_catalog(w: &mut Workspace, cx: &mut Context<Workspace>) {
    super::super::glass::haptic();
    SEARCH.with(|search| {
        let mut search = search.borrow_mut();
        let placeholder = tr("搜索音色", "Search voices");
        if let Some(field) = search.as_ref() {
            field.update(cx, |f, cx| { f.set_placeholder(placeholder, cx); f.set_text("", cx); });
        } else {
            let field = cx.new(|cx| TextField::new(placeholder, cx));
            cx.observe(&field, |_, _, cx| cx.notify()).detach();
            *search = Some(field);
        }
    });
    w.open_settings(Page::AssistantVoices, cx);
    fetch(cx);
}

pub fn catalog(cx: &mut Context<Workspace>) -> AnyElement {
    let search = SEARCH.with(|s| s.borrow().clone());
    let filter = search.as_ref().map_or_else(String::new, |f| f.read(cx).text.trim().to_lowercase());
    let message = |text: &'static str| div().px(px(18.)).py(px(16.)).text_size(px(15.)).line_height(px(21.)).text_color(Theme::text_caption()).child(text).into_any_element();
    let (ready, rows) = CATALOG.with(|catalog| match &*catalog.borrow() {
        Catalog::Idle | Catalog::Loading => (false, vec![message(tr("正在获取音色…", "Fetching voices…"))]),
        Catalog::Failed(error) => (false, vec![message(match error {
            LoadError::Key | LoadError::Http(401 | 403) => tr("请检查 API Key 和音色读取权限。", "Check your API Key and its voices read permission."),
            LoadError::Http(429) => tr("请求过多，请稍后重试。", "Too many requests. Try again shortly."),
            LoadError::Network => tr("无法连接 ElevenLabs，请检查网络。", "Could not connect to ElevenLabs. Check your connection."),
            _ => tr("无法获取 ElevenLabs 音色。", "Could not load ElevenLabs voices."),
        }), general::action("speech-retry-voices", tr("重试", "Retry"), false, |_, cx| fetch(cx), cx)]),
        Catalog::Ready(voices) => {
            let p = prefs();
            let saved: HashSet<_> = p.speech_roles.iter().map(|r| r.voice_id.as_str()).collect();
            let mut rows: Vec<_> = voices.iter().filter(|v| filter.is_empty() || v.name.to_lowercase().contains(&filter)).map(|v| {
                let voice = v.clone();
                let added = saved.contains(v.voice_id.as_str());
                item(SharedString::from(format!("speech-voice-{}", v.voice_id)), v.name.clone(), None, None,
                    ui::icon(if added { "check" } else { "plus" }, 15., if added { Theme::primary() } else { row_chevron() }))
                    .on_click(cx.listener(move |w, _, window, cx| { window.blur(cx); add_role(w, &voice, cx); })).into_any_element()
            }).collect();
            if rows.is_empty() { rows.push(message(tr("没有找到音色。", "No voices found."))); }
            (true, rows)
        }
    });
    div().w_full().pt(px(18.)).flex().flex_col().gap(px(16.))
        .when(ready, |d| d.children(search.map(|f| general::card(vec![div().px(px(18.)).h(px(50.)).flex().items_center().gap(px(10.))
            .child(ui::icon("search", 17., Theme::text_caption())).child(div().flex_1().min_w_0().child(f)).into_any_element()]))))
        .child(general::card(rows)).into_any_element()
}

fn add_role(w: &mut Workspace, voice: &Voice, cx: &mut Context<Workspace>) {
    if !prefs().speech_roles.iter().any(|r| r.voice_id == voice.voice_id) {
        let role = Role { voice_id: voice.voice_id.clone(), voice_name: voice.name.clone(), name: voice.name.clone(), avatar: None };
        w.set_pref(|p| p.speech_roles.push(role), cx);
    }
    if prefs().speech_roles.iter().any(|r| r.voice_id == voice.voice_id) { open_role(w, voice.voice_id.clone(), cx); }
}

fn open_role(w: &mut Workspace, id: String, cx: &mut Context<Workspace>) {
    super::super::glass::haptic();
    EDITING.with(|r| *r.borrow_mut() = id);
    w.open_settings(Page::AssistantRole, cx);
}

pub fn detail(cx: &mut Context<Workspace>) -> AnyElement {
    let Some(role) = EDITING.with(|id| prefs().speech_roles.into_iter().find(|r| r.voice_id == *id.borrow())) else { return div().into_any_element() };
    let (name, photo, clear) = (role.clone(), role.voice_id.clone(), role.voice_id.clone());
    let mut rows = vec![
        item("speech-role-name", tr("名称", "Name"), Some(role.name.clone().into()), None, ui::icon("chevron-right", 14., row_chevron()))
            .on_click(cx.listener(move |w, _, _, cx| rename(w, &name, cx))).into_any_element(),
        item("speech-role-source", "ElevenLabs", Some(role.voice_name.clone().into()), None, div()).into_any_element(),
        general::action("speech-role-photo", tr("更换头像", "Change Avatar"), false, move |_, cx| pick_avatar(photo.clone(), cx), cx),
    ];
    if role.avatar.is_some() { rows.push(general::action("speech-role-reset", tr("恢复默认头像", "Use Default Avatar"), false, move |w, cx| set_avatar(w, &clear, None, cx), cx)); }
    div().pt(px(16.)).flex().flex_col().gap(px(24.))
        .child(div().w_full().flex().justify_center().child(avatar(&role, 80.)))
        .child(general::card(rows)).into_any_element()
}

fn rename(w: &mut Workspace, role: &Role, cx: &mut Context<Workspace>) {
    let id = role.voice_id.clone();
    let title = tr("角色名称", "Role Name");
    if !super::prompt(title, None, title, &role.name, false, Some(tr("移除角色", "Remove Role")), move |value| {
        let id = id.clone();
        menu::dispatch(move |w, _, cx| match value {
            Some(name) if name.is_empty() => w.show_toast_kind(ui::ToastKind::Error, tr("请输入角色名称", "Enter a role name"), cx),
            Some(name) => w.set_pref(|p| { if let Some(role) = p.speech_roles.iter_mut().find(|r| r.voice_id == id) { role.name = name; } }, cx),
            None => remove_role(w, &id, cx),
        });
    }) { w.show_toast_kind(ui::ToastKind::Error, tr("无法打开角色设置", "Could not open role settings"), cx); }
}

fn remove_role(w: &mut Workspace, id: &str, cx: &mut Context<Workspace>) {
    let old = prefs().speech_roles.into_iter().find(|r| r.voice_id == id).and_then(|r| r.avatar);
    w.set_pref(|p| p.speech_roles.retain(|r| r.voice_id != id), cx);
    if prefs().speech_roles.iter().any(|r| r.voice_id == id) { return; }
    discard_avatar(old.as_deref());
    super::super::settings_back(w, cx);
}

fn avatar_path(id: &str) -> Option<PathBuf> {
    let id = uuid::Uuid::parse_str(id).ok()?;
    Some(crate::store::Store::home_dir().join("assistant-avatars").join(format!("{id}.png")))
}

fn avatar(role: &Role, size: f32) -> AnyElement {
    let image = role.avatar.as_deref().and_then(avatar_path).filter(|path| path.is_file());
    if let Some(path) = &image { return crate::space::icon_el(Some(path), &role.voice_id, size); }
    div().size(px(size)).flex_none().rounded(px(size * 0.24)).bg(Theme::primary_fill_1()).flex().items_center().justify_center()
        .child(svg().path("editor-icons/volume-2.svg").size(px(size * 0.52)).text_color(Theme::primary())).into_any_element()
}

fn pick_avatar(id: String, cx: &mut Context<Workspace>) {
    let pick = super::super::files::open_image();
    cx.spawn(async move |this, cx| {
        let Some(path) = pick.await.ok().and_then(Result::ok).flatten().and_then(|p| p.into_iter().next()) else { return };
        let Some(path) = super::super::crop::square(path).await.ok().and_then(Result::ok).flatten() else { return };
        let _ = this.update(cx, |w, cx| match write_avatar(&path) {
            Ok(image) => set_avatar(w, &id, Some(image), cx),
            Err(_) => w.show_toast_kind(ui::ToastKind::Error, tr("无法保存头像", "Could not save avatar"), cx),
        });
    }).detach();
}

fn write_avatar(from: &Path) -> anyhow::Result<String> {
    let id = uuid::Uuid::new_v4().to_string();
    let to = avatar_path(&id).unwrap();
    let image = image::open(from)?.thumbnail(384, 384);
    std::fs::create_dir_all(to.parent().unwrap())?;
    if let Err(err) = image.save(&to) { let _ = std::fs::remove_file(to); return Err(err.into()); }
    Ok(id)
}

fn discard_avatar(id: Option<&str>) { if let Some(path) = id.and_then(avatar_path) { let _ = std::fs::remove_file(path); } }

fn set_avatar(w: &mut Workspace, id: &str, next: Option<String>, cx: &mut Context<Workspace>) {
    let Some(old) = prefs().speech_roles.into_iter().find(|r| r.voice_id == id) else { discard_avatar(next.as_deref()); return; };
    w.set_pref(|p| { if let Some(r) = p.speech_roles.iter_mut().find(|r| r.voice_id == id) { r.avatar = next.clone(); } }, cx);
    if prefs().speech_roles.iter().any(|r| r.voice_id == id && r.avatar == next) { discard_avatar(old.avatar.as_deref()); }
    else { discard_avatar(next.as_deref()); }
}

fn fetch(cx: &mut Context<Workspace>) {
    if CATALOG.with(|c| matches!(*c.borrow(), Catalog::Loading)) { return; }
    let Some(key) = super::read_key() else { CATALOG.with(|c| *c.borrow_mut() = Catalog::Failed(LoadError::Key)); return cx.notify(); };
    CATALOG.with(|c| *c.borrow_mut() = Catalog::Loading);
    let request = REQUEST.with(|r| { r.set(r.get() + 1); r.get() });
    cx.notify();
    cx.spawn(async move |this, cx| {
        let result = fetch_pages(&key).await;
        if REQUEST.with(Cell::get) == request {
            CATALOG.with(|c| *c.borrow_mut() = match result { Ok(voices) => Catalog::Ready(voices), Err(e) => Catalog::Failed(e) });
            let _ = this.update(cx, |_, cx| cx.notify());
        }
    }).detach();
}

async fn fetch_pages(key: &str) -> Result<Vec<Voice>, LoadError> {
    let mut voices = Vec::new();
    let mut ids = HashSet::new();
    let mut cursors = HashSet::new();
    let mut token = None;
    loop {
        let mut url = "https://api.elevenlabs.io/v2/voices?page_size=100&include_total_count=false".to_owned();
        if let Some(cursor) = token.as_deref() {
            url.push_str("&next_page_token=");
            for b in str::bytes(cursor) {
                if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { url.push(b as char); }
                else { let _ = write!(url, "%{b:02X}"); }
            }
        }
        let page = request_page(&url, key).await.unwrap().unwrap()?;
        voices.extend(page.voices.into_iter().filter(|v| !v.voice_id.is_empty() && ids.insert(v.voice_id.clone())));
        if !page.has_more { return Ok(voices); }
        token = page.next_page_token.filter(|t| !t.is_empty() && cursors.insert(t.clone()));
        if token.is_none() { return Err(LoadError::Response); }
    }
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "QuietMdElevenLabsSessionDelegate"]
    struct SessionDelegate;

    impl SessionDelegate {
        #[unsafe(method(URLSession:task:willPerformHTTPRedirection:newRequest:completionHandler:))]
        fn redirect(&self, _session: *mut AnyObject, _task: *mut AnyObject, _response: *mut AnyObject, _request: *mut AnyObject, reply: &block2::Block<dyn Fn(*mut AnyObject)>) {
            // API Key 只发往最初的 ElevenLabs URL。
            reply.call((null_mut(),));
        }
    }
);

fn request_page(url: &str, key: &str) -> super::super::files::Reply<Result<VoicePage, LoadError>> {
    use super::super::files::Reply;
    unsafe {
        let url: *mut AnyObject = msg_send![class!(NSURL), URLWithString: ns_string(url)];
        if url.is_null() { return Reply::ready(Err(LoadError::Response)); }
        let request: *mut AnyObject = msg_send![class!(NSMutableURLRequest), requestWithURL: url];
        let _: () = msg_send![request, setValue: ns_string(key), forHTTPHeaderField: ns_string("xi-api-key")];
        let _: () = msg_send![request, setTimeoutInterval: 15f64];
        let config: *mut AnyObject = msg_send![class!(NSURLSessionConfiguration), ephemeralSessionConfiguration];
        let queue: *mut AnyObject = msg_send![class!(NSOperationQueue), mainQueue];
        let delegate: objc2::rc::Retained<SessionDelegate> = msg_send![SessionDelegate::class(), new];
        let session: *mut AnyObject = msg_send![class!(NSURLSession), sessionWithConfiguration: config, delegate: &*delegate, delegateQueue: queue];
        let reply = Reply::pending();
        let handle = reply.handle();
        let complete = block2::RcBlock::new(move |data: *mut AnyObject, response: *mut AnyObject, error: *mut AnyObject| {
            let result = (|| {
                if !error.is_null() || response.is_null() { return Err(LoadError::Network); }
                let status: isize = msg_send![response, statusCode];
                if status != 200 { return Err(LoadError::Http(status)); }
                let len: usize = msg_send![data, length];
                let bytes: *const std::ffi::c_void = msg_send![data, bytes];
                if len == 0 || bytes.is_null() { return Err(LoadError::Response); }
                serde_json::from_slice(std::slice::from_raw_parts(bytes.cast(), len)).map_err(|_| LoadError::Response)
            })();
            handle.send(result);
        });
        let task: *mut AnyObject = msg_send![session, dataTaskWithRequest: request, completionHandler: &*complete];
        let _: () = msg_send![task, resume];
        let _: () = msg_send![session, finishTasksAndInvalidate];
        reply
    }
}
