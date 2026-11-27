//! 回复旁的「朗读」：用助手的声线（语音角色）念这条回复，没绑定声线时用语音设置里排第一的角色。
//! 向 ElevenLabs 要音频（同配音的 `voice::speak`），念过的这次打开里记着，再点不再花额度。一次只念一条：
//! 念之前停掉配音和试听，换对话、新对话、关掉页面时停下。念的是回复的纯文字（去掉 Markdown 记号），最多 5000 字。
use std::{cell::{Cell, RefCell}, collections::HashMap, rc::Rc, time::Duration};

use futures::channel::oneshot;
use gpui::*;
use objc2::{msg_send, runtime::AnyObject};

use crate::{app::Workspace, model::Block, settings::{prefs, tr}, ui};
use super::super::{assistant::{self, roles}, voice};

enum State { Loading, Playing(*mut AnyObject) }

thread_local! {
    /// 正在念（或在要音频）的那条：哪一条、在做什么、第几次（停下或换一条就加一，晚到的不要）。
    static NOW: RefCell<Option<(String, State)>> = const { RefCell::new(None) };
    static EPOCH: Cell<u64> = const { Cell::new(0) };
    /// 念过的：条目 → 音频。
    static CACHE: RefCell<HashMap<String, Rc<Vec<u8>>>> = RefCell::new(HashMap::new());
}

/// 一条回复的键：对话和第几条。
pub fn key(chat: &str, i: usize) -> String { format!("{chat}:{i}") }

/// 这一条现在：`None` 没在念，`Some(false)` 在要音频，`Some(true)` 在念。
pub fn shown(key: &str) -> Option<bool> { NOW.with(|n| n.borrow().as_ref().filter(|(k, _)| k == key).map(|(_, s)| matches!(s, State::Playing(_)))) }

pub fn stop() {
    EPOCH.with(|e| e.set(e.get() + 1));
    if let Some((_, State::Playing(p))) = NOW.with(|n| n.borrow_mut().take()) {
        unsafe {
            let _: () = msg_send![p, stop];
            let _: () = msg_send![p, release];
        }
    }
}

fn toast(w: &mut Workspace, text: &str, cx: &mut Context<Workspace>) { w.show_toast_kind(ui::ToastKind::Error, text, cx); }

/// 块里的字（连子块），一块一行。
fn plain(blocks: &[Block], out: &mut Vec<String>) {
    for b in blocks {
        let t = b.text.text.trim();
        if !t.is_empty() { out.push(t.to_owned()); }
        plain(&b.children, out);
    }
}

/// 点了朗读：在念这一条就停下，不然念它。`voice` 是助手的声线（音色标识）。
pub fn toggle(key: String, text: &str, voice: Option<&str>, w: &mut Workspace, cx: &mut Context<Workspace>) {
    if shown(&key).is_some() { stop(); return cx.notify(); }
    stop();
    let all = roles::all();
    let Some(role) = voice.and_then(|v| all.iter().find(|r| r.voice_id() == v)).or(all.first()) else {
        return toast(w, tr("先在 设置 › MarginAI › 语音 里添加角色", "Add a role in Settings › MarginAI › Voice first"), cx);
    };
    let Some(api) = assistant::read_key() else { return toast(w, tr("先在 设置 › MarginAI › 语音 里填写 API Key", "Add an API Key in Settings › MarginAI › Voice first"), cx) };
    let mut lines = vec![];
    plain(&fdl_core::markdown::parse_file(text, "", None).blocks, &mut lines);
    let text: String = lines.join("\n").chars().take(voice::MAX_CHARS).collect();
    if text.trim().is_empty() { return; }
    let epoch = EPOCH.with(Cell::get);
    if let Some(bytes) = CACHE.with(|c| c.borrow().get(&key).cloned()) { return play(key, &bytes, epoch, w, cx); }
    NOW.with(|n| *n.borrow_mut() = Some((key.clone(), State::Loading)));
    cx.notify();
    let (tx, rx) = oneshot::channel();
    let (voice, model) = (role.voice_id().to_owned(), prefs().speech_model);
    assistant::llm::runtime().spawn(async move { let _ = tx.send(voice::speak(&api, &voice, &model, "", &text).await); });
    cx.spawn(async move |this, cx| {
        let result = rx.await.unwrap_or_else(|_| Err(tr("请求中断了", "The request was interrupted").into()));
        // 要到了就记下（中途停了也记，再点不用再花一次额度）。
        let result = result.map(|(audio, ..)| { let bytes = Rc::new(audio); CACHE.with(|c| c.borrow_mut().insert(key.clone(), bytes.clone())); bytes });
        if EPOCH.with(Cell::get) != epoch { return; }
        let _ = this.update(cx, |w, cx| match result {
            Ok(bytes) => play(key, &bytes, epoch, w, cx),
            Err(e) => { NOW.with(|n| n.borrow_mut().take()); toast(w, &e, cx); cx.notify(); }
        });
    }).detach();
}

/// 念：停掉配音和试听，放完了（或停了）按钮换回来。
fn play(key: String, bytes: &[u8], epoch: u64, w: &mut Workspace, cx: &mut Context<Workspace>) {
    voice::stop();
    assistant::preview::stop();
    let Some(player) = assistant::preview::open(bytes) else { NOW.with(|n| n.borrow_mut().take()); return toast(w, tr("无法播放", "Could not play"), cx) };
    NOW.with(|n| *n.borrow_mut() = Some((key, State::Playing(player))));
    cx.notify();
    cx.spawn(async move |this, cx| loop {
        cx.background_executor().timer(Duration::from_millis(200)).await;
        if EPOCH.with(Cell::get) != epoch { return; }
        let on: bool = unsafe { msg_send![player, isPlaying] };
        if !on { stop(); let _ = this.update(cx, |_, cx| cx.notify()); return; }
    }).detach();
}

/// 拷贝旁边的朗读钮：没在念是喇叭，在要音频是转圈，在念是停止。
pub fn button(i: usize, key: String, text: String, voice: Option<String>, cx: &mut Context<Workspace>) -> AnyElement {
    let state = shown(&key);
    let color = crate::theme::Theme::text_caption();
    let glyph = match state {
        Some(false) => svg().path("editor-icons/spinner.svg").size(px(16.)).flex_none().text_color(color)
            .with_animation(("margin-speak-spin", i), Animation::new(Duration::from_millis(900)).repeat(), |s, t| s.with_transformation(Transformation::rotate(percentage(t)))).into_any_element(),
        Some(true) => svg().path("editor-icons/stop.svg").size(px(16.)).flex_none().text_color(color).into_any_element(),
        None => svg().path("editor-icons/volume-2.svg").size(px(16.)).flex_none().text_color(color).into_any_element(),
    };
    div().id(("margin-speak", i)).size(px(32.)).flex().items_center().justify_center().rounded(px(8.)).active(|s| s.bg(crate::theme::Theme::fill_pressed()))
        .child(glyph)
        .on_click(cx.listener(move |w, _, _, cx| { super::super::glass::haptic(); toggle(key.clone(), &text, voice.as_deref(), w, cx); }))
        .into_any_element()
}
