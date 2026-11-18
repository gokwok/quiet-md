//! 选中文字配音。选区的编辑菜单里「配音」（有几个角色时展开选角色）给那段文字带上 `voice` 标记（`Marks::voice`，
//! 存进文档），再向 ElevenLabs 要带逐字时间的音频，存在文档库的 `assets/voice/<标识>.mp3`，旁边的 `<标识>.json`
//! 记着角色、念的那段文字和每个字开始的时间。一段配音的最后一块下面一行是它：头像、角色、时长，点了播放。
//! 放的时候整段铺一层浅底，正在念的那一句深一档；文字改过、和念的对不上了就只放声音、不跟读。
use std::{cell::RefCell, collections::HashMap, path::PathBuf, time::Duration};

use base64::Engine;
use futures::channel::oneshot;
use gpui::{prelude::FluentBuilder, *};
use objc2::{msg_send, runtime::AnyObject};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{app::Workspace, editor::view::Editor, model::{Block, BlockId, Document, RichText}, settings::{prefs, tr}, store::Store, theme::Theme, ui};
use super::{assistant::{self, roles::Role}, menu};

/// 编辑菜单里选角色的那几项：`edit-voice-<第几个角色>`。
pub const PICK: &str = "edit-voice-";
const PICKS: [&str; 12] = ["edit-voice-0", "edit-voice-1", "edit-voice-2", "edit-voice-3", "edit-voice-4", "edit-voice-5",
    "edit-voice-6", "edit-voice-7", "edit-voice-8", "edit-voice-9", "edit-voice-10", "edit-voice-11"];
/// 一行配音右边「…」菜单的按钮和两项，按 `slot` 编号。
const MORE: [&str; 8] = ["voice-more-0", "voice-more-1", "voice-more-2", "voice-more-3", "voice-more-4", "voice-more-5", "voice-more-6", "voice-more-7"];
const REDO: [&str; 8] = ["voice-redo-0", "voice-redo-1", "voice-redo-2", "voice-redo-3", "voice-redo-4", "voice-redo-5", "voice-redo-6", "voice-redo-7"];
const DROP: [&str; 8] = ["voice-drop-0", "voice-drop-1", "voice-drop-2", "voice-drop-3", "voice-drop-4", "voice-drop-5", "voice-drop-6", "voice-drop-7"];
/// 一次最多念多少字（ElevenLabs 各模型的上限不同，最小的是 5000）。
const MAX_CHARS: usize = 5000;

/// `<标识>.json`：角色、模型、念的文字、每个字开始的秒数（和 `text` 的字一一对应；对不上时空着）、总长。
#[derive(Clone, Default, Deserialize, Serialize)]
struct Clip {
    voice_id: String,
    role: String,
    #[serde(default)]
    model: String,
    text: String,
    #[serde(default)]
    starts: Vec<f32>,
    #[serde(default)]
    length: f32,
}

enum Job { Running, Failed(String) }

struct Playing { clip: String, player: *mut AnyObject, length: f32, epoch: u64, follow: Vec<(BlockId, std::ops::Range<usize>)> }

thread_local! {
    static JOBS: RefCell<HashMap<String, Job>> = RefCell::new(HashMap::new());
    /// 读过的 json（`None`：没有这个文件）。
    static CLIPS: RefCell<HashMap<String, Option<Clip>>> = RefCell::new(HashMap::new());
    static LAST: RefCell<String> = const { RefCell::new(String::new()) };
    static PLAYING: RefCell<Option<Playing>> = const { RefCell::new(None) };
    static EPOCH: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    static SLOTS: RefCell<(Vec<String>, usize)> = const { RefCell::new((Vec::new(), 0)) };
}

fn dir() -> PathBuf { Store::assets_dir().join("voice") }
fn file(clip: &str, ext: &str) -> Option<PathBuf> {
    // 标识来自文档，只认字母数字和连字符，免得拼出别处的路径。
    (!clip.is_empty() && clip.len() <= 64 && clip.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')).then(|| dir().join(format!("{clip}.{ext}")))
}

fn clip_info(clip: &str) -> Option<Clip> {
    if let Some(c) = CLIPS.with(|c| c.borrow().get(clip).cloned()) { return c; }
    let read = file(clip, "json").and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice::<Clip>(&b).ok());
    CLIPS.with(|c| c.borrow_mut().insert(clip.to_owned(), read.clone()));
    read
}

fn save_info(clip: &str, info: &Clip) {
    CLIPS.with(|c| c.borrow_mut().insert(clip.to_owned(), Some(info.clone())));
    let Some(path) = file(clip, "json") else { return };
    let _ = std::fs::create_dir_all(dir());
    if let Ok(bytes) = serde_json::to_vec_pretty(info) && let Err(e) = std::fs::write(&path, bytes) { log::warn!("Could not save voice clip info: {e}"); }
}

/// 角色，上次用的排第一。
fn roles() -> Vec<Role> {
    let mut roles = prefs().speech_roles;
    let last = LAST.with(|l| l.borrow().clone());
    if let Some(i) = roles.iter().position(|r| r.voice_id() == last) { let r = roles.remove(i); roles.insert(0, r); }
    roles.truncate(PICKS.len());
    roles
}

/// 编辑菜单里的「配音」：一个角色时直接配，几个时是子菜单，没有角色时点了提示去添加。
pub fn menu_item() -> Option<super::editmenu::Item> {
    let title: SharedString = tr("配音", "Voice Over").into();
    let roles = roles();
    Some(match roles.len() {
        0 => ("edit-voice-none", title, vec![]),
        1 => (PICKS[0], title, vec![]),
        _ => ("edit-voice-menu", title, roles.iter().enumerate().map(|(i, r)| (PICKS[i], SharedString::from(r.name().to_owned()))).collect()),
    })
}

fn toast(text: &'static str) { menu::dispatch(move |w, _, cx| w.show_toast_kind(ui::ToastKind::Error, text, cx)); }

/// 选了角色：给选区带上新的配音标识，开始生成。
pub fn start(ed: &mut Editor, id: &str, window: &mut Window, cx: &mut Context<Editor>) {
    let roles = roles();
    let Some(role) = id.strip_prefix(PICK).and_then(|n| n.parse::<usize>().ok()).and_then(|n| roles.get(n)) else {
        return toast(tr("先在 设置 › 语音 里添加角色", "Add a role in Settings › Voice first"));
    };
    if assistant::read_key().is_none() { return toast(tr("先在 设置 › 语音 里填写 API Key", "Add an API Key in Settings › Voice first")); }
    let clip = uuid::Uuid::new_v4().simple().to_string()[..16].to_owned();
    if !ed.core.set_voice(&clip) { return; }
    let text = ed.core.doc.voice_text(&clip);
    if text.chars().count() > MAX_CHARS {
        ed.core.undo();
        ed.changed(window, cx);
        return toast(tr("选中的文字太长，一次最多 5000 字", "The selection is too long (5,000 characters at most)"));
    }
    // 光标放到这段末尾：选区收起，编辑菜单跟着收起。
    let end = ed.core.doc.voiced(&clip).last().map(|(b, r)| crate::editor::core::Pos::new(*b, r.end));
    if let Some(end) = end { ed.core.sel = crate::editor::core::Selection::caret(end); }
    ed.changed(window, cx);
    LAST.with(|l| *l.borrow_mut() = role.voice_id().to_owned());
    let info = Clip { voice_id: role.voice_id().to_owned(), role: role.name().to_owned(), model: prefs().speech_model, text, ..Default::default() };
    save_info(&clip, &info);
    generate(clip, info, cx);
}

/// 向 ElevenLabs 要音频：念完整段要几秒，这期间那一行写着「生成中」；好了就放一遍。
fn generate(clip: String, mut info: Clip, cx: &mut Context<Editor>) {
    stop();
    JOBS.with(|j| j.borrow_mut().insert(clip.clone(), Job::Running));
    cx.notify();
    let Some(key) = assistant::read_key() else {
        JOBS.with(|j| j.borrow_mut().insert(clip, Job::Failed(tr("没有 API Key", "No API Key").into())));
        return;
    };
    let (tx, rx) = oneshot::channel();
    let (voice, model, text) = (info.voice_id.clone(), info.model.clone(), info.text.clone());
    assistant::llm::runtime().spawn(async move { let _ = tx.send(speak(&key, &voice, &model, &text).await); });
    cx.spawn(async move |this, cx| {
        let result = rx.await.unwrap_or_else(|_| Err(tr("请求中断了", "The request was interrupted").into()));
        let result = result.and_then(|(audio, starts, length)| {
            let path = file(&clip, "mp3").ok_or_else(|| tr("无法保存音频", "Could not save the audio").to_owned())?;
            std::fs::create_dir_all(dir()).and_then(|_| std::fs::write(path, audio)).map_err(|_| tr("无法保存音频", "Could not save the audio").to_owned())?;
            info.starts = starts;
            info.length = length;
            save_info(&clip, &info);
            Ok(())
        });
        let _ = this.update(cx, |_, cx| {
            match result {
                Ok(()) => { JOBS.with(|j| j.borrow_mut().remove(&clip)); play(clip, cx); }
                Err(e) => { log::warn!("Voice over failed: {e}"); JOBS.with(|j| j.borrow_mut().insert(clip, Job::Failed(e))); }
            }
            cx.notify();
        });
    }).detach();
}

/// 音频、每个字开始的秒数、总长。
async fn speak(key: &str, voice: &str, model: &str, text: &str) -> Result<(Vec<u8>, Vec<f32>, f32), String> {
    let failed = |_| tr("无法连接 ElevenLabs", "Could not reach ElevenLabs").to_owned();
    let http = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(120)).build().map_err(|e| e.to_string())?;
    let url = format!("https://api.elevenlabs.io/v1/text-to-speech/{voice}/with-timestamps?output_format=mp3_44100_128");
    let model = if model.is_empty() { assistant::DEFAULT_MODEL } else { model };
    let response = http.post(url).header("xi-api-key", key).header("content-type", "application/json")
        .body(json!({ "text": text, "model_id": model }).to_string()).send().await.map_err(failed)?;
    let status = response.status();
    let body = response.bytes().await.map_err(failed)?;
    let v: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    if !status.is_success() {
        // `{"detail": {"message": …}}` 或 `{"detail": "…"}`。
        let said = ["/detail/message", "/detail"].iter().find_map(|k| v.pointer(k).and_then(Value::as_str)).map(str::to_owned);
        return Err(said.unwrap_or_else(|| match status.as_u16() { 401 => tr("API Key 无效", "The API Key is invalid").into(), code => format!("HTTP {code}") }));
    }
    let audio = v.get("audio_base64").and_then(Value::as_str).and_then(|s| base64::engine::general_purpose::STANDARD.decode(s).ok())
        .ok_or_else(|| tr("ElevenLabs 没有返回音频", "ElevenLabs returned no audio").to_owned())?;
    let floats = |k: &str| v.pointer(k).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_f64).map(|f| f as f32).collect::<Vec<_>>()).unwrap_or_default();
    let chars = v.pointer("/alignment/characters").and_then(Value::as_array).map_or(0, Vec::len);
    let starts = floats("/alignment/character_start_times_seconds");
    let length = floats("/alignment/character_end_times_seconds").last().copied().unwrap_or(0.);
    // 时间按原文的字给；数目对不上就不跟读。
    let starts = if chars == text.chars().count() && starts.len() == chars { starts } else { vec![] };
    Ok((audio, starts, length))
}

pub fn stop() {
    EPOCH.with(|e| e.set(e.get() + 1));
    let Some(p) = PLAYING.with(|p| p.borrow_mut().take()) else { return };
    unsafe {
        let _: () = msg_send![p.player, stop];
        let _: () = msg_send![p.player, release];
    }
}

fn playing(clip: &str) -> bool { PLAYING.with(|p| p.borrow().as_ref().is_some_and(|p| p.clip == clip)) }

fn play(clip: String, cx: &mut Context<Editor>) {
    stop();
    assistant::preview::stop();
    let Some(bytes) = file(&clip, "mp3").and_then(|p| std::fs::read(p).ok()) else { return toast(tr("找不到这段配音的音频", "The audio for this voice over is missing")) };
    let Some(player) = assistant::preview::open(&bytes) else { return toast(tr("无法播放", "Could not play")) };
    let length: f64 = unsafe { msg_send![player, duration] };
    let epoch = EPOCH.with(|e| e.get());
    PLAYING.with(|p| *p.borrow_mut() = Some(Playing { clip, player, length: length.max(0.1) as f32, epoch, follow: vec![] }));
    cx.notify();
    // 跟读：每 80 毫秒看一眼放到哪了，换句子时重画。放完、换了一段、编辑器关了就停。
    cx.spawn(async move |this, cx| loop {
        cx.background_executor().timer(Duration::from_millis(80)).await;
        let Some((clip, player)) = PLAYING.with(|p| p.borrow().as_ref().filter(|p| p.epoch == epoch).map(|p| (p.clip.clone(), p.player))) else { return };
        let on: bool = unsafe { msg_send![player, isPlaying] };
        let at: f64 = unsafe { msg_send![player, currentTime] };
        let alive = this.update(cx, |ed, cx| {
            let follow = if on { follow(&ed.core.doc, &clip, at as f32) } else { vec![] };
            PLAYING.with(|p| if let Some(p) = p.borrow_mut().as_mut() { p.follow = follow; });
            cx.notify();
        }).is_ok();
        if !on || !alive { stop(); let _ = this.update(cx, |_, cx| cx.notify()); return; }
    }).detach();
}

/// 正在念的那一句在文档里的位置。文字改过（和念的对不上）就没有。
fn follow(doc: &Document, clip: &str, t: f32) -> Vec<(BlockId, std::ops::Range<usize>)> {
    let Some(info) = clip_info(clip) else { return vec![] };
    if info.starts.is_empty() || doc.voice_text(clip) != info.text { return vec![]; }
    let text = &info.text;
    let k = info.starts.partition_point(|s| *s <= t).saturating_sub(1);
    let b = text.char_indices().nth(k).map_or(text.len(), |(i, _)| i);
    // 句末：中文的句号问号叹号分号、换行；英文的 `.` 要后面是空白或结尾（`Quiet.md` 不算）。
    const END: [char; 9] = ['。', '！', '？', '!', '?', '；', ';', '\n', '…'];
    let ends = |i: usize, c: char| END.contains(&c) || c == '.' && text[i + 1..].chars().next().is_none_or(char::is_whitespace);
    let start = text[..b].char_indices().rev().find(|&(i, c)| ends(i, c)).map_or(0, |(i, c)| i + c.len_utf8());
    let start = start + (text[start..].len() - text[start..].trim_start().len());
    let end = text[b..].char_indices().find(|&(i, c)| ends(b + i, c)).map_or(text.len(), |(i, c)| b + i + c.len_utf8());
    // `voice_text` 的各段之间是一个字节的分隔。
    let mut out = vec![];
    let mut at = 0;
    for (i, (block, r)) in doc.voiced(clip).into_iter().enumerate() {
        if i > 0 { at += 1; }
        let (s, e) = (at, at + r.len());
        let (a, z) = (start.max(s), end.min(e));
        if a < z { out.push((block, r.start + a - s..r.start + z - s)); }
        at = e;
    }
    out
}

/// 排字前改一改这一块的几段：配过音的文字不放时下面一道淡线；正在放的那段铺浅底，念到的那一句深一档。
pub fn decorate(block: BlockId, text: &RichText, runs: &mut Vec<TextRun>) {
    if !text.runs.iter().any(|r| r.marks.voice.is_some()) { return; }
    let (now, current) = PLAYING.with(|p| p.borrow().as_ref().map(|p| (p.clip.clone(), p.follow.iter().filter(|(b, _)| *b == block).map(|(_, r)| r.clone()).collect::<Vec<_>>()))).unwrap_or_default();
    for r in &current { split(runs, r.start); split(runs, r.end); }
    let spans: Vec<(std::ops::Range<usize>, Option<String>)> = text.spans().map(|(r, m)| (r, m.voice.clone())).collect();
    let mut at = 0;
    for run in runs.iter_mut() {
        let range = at..at + run.len;
        at += run.len;
        let Some(voice) = spans.iter().find(|(r, _)| r.start <= range.start && range.start < r.end).and_then(|(_, v)| v.as_deref()) else { continue };
        if voice == now {
            let here = current.iter().any(|c| c.start <= range.start && range.end <= c.end);
            run.background_color = Some(if here { Theme::fill_pressed() } else { Theme::fill_hover() });
        } else if run.underline.is_none() {
            run.underline = Some(UnderlineStyle { thickness: px(1.), color: Some(Theme::text_placeholder().opacity(0.35)), wavy: false });
        }
    }
}

fn split(runs: &mut Vec<TextRun>, at: usize) {
    let mut o = 0;
    for i in 0..runs.len() {
        let len = runs[i].len;
        if o < at && at < o + len {
            let mut tail = runs[i].clone();
            runs[i].len = at - o;
            tail.len = o + len - at;
            runs.insert(i + 1, tail);
            return;
        }
        o += len;
    }
}

/// 第几个「…」按钮：同一段配音一直用同一个，最多同时 8 个。
fn slot(clip: &str) -> usize {
    SLOTS.with(|s| {
        let (list, next) = &mut *s.borrow_mut();
        if let Some(i) = list.iter().position(|c| c == clip) { return i; }
        let i = *next % MORE.len();
        *next += 1;
        if list.len() < MORE.len() { list.push(clip.to_owned()); list.len() - 1 } else { list[i] = clip.to_owned(); i }
    })
}

fn clock(secs: f32) -> String { let s = secs.max(0.).round() as u32; format!("{}:{:02}", s / 60, s % 60) }

/// 这一块下面的配音行：在这一块结束的那几段配音，一段一行。
pub fn rows(b: &Block, doc: &Document, cx: &mut Context<Editor>) -> Vec<AnyElement> {
    let mut clips: Vec<String> = vec![];
    for (_, m) in b.text.spans() { if let Some(v) = &m.voice && !clips.contains(v) { clips.push(v.clone()); } }
    clips.retain(|c| doc.voiced(c).last().is_some_and(|(id, _)| *id == b.id));
    clips.into_iter().map(|clip| row(clip, doc, cx)).collect()
}

fn row(clip: String, doc: &Document, cx: &mut Context<Editor>) -> AnyElement {
    let info = clip_info(&clip);
    let job = JOBS.with(|j| j.borrow().get(&clip).map(|j| match j { Job::Running => None, Job::Failed(e) => Some(e.clone()) }));
    let on = playing(&clip);
    let role = info.as_ref().and_then(|i| prefs().speech_roles.into_iter().find(|r| r.voice_id() == i.voice_id));
    let face = match (&role, &info) {
        (Some(r), _) => assistant::roles::avatar(r, 20.),
        (None, Some(i)) => assistant::roles::orb(&i.voice_id, 20.).into_any_element(),
        (None, None) => assistant::roles::orb(&clip, 20.).into_any_element(),
    };
    let name: SharedString = role.as_ref().map(|r| r.name().to_owned()).or_else(|| info.as_ref().map(|i| i.role.clone())).unwrap_or_default().into();
    let has_audio = file(&clip, "mp3").is_some_and(|p| p.exists());
    let stale = info.as_ref().is_some_and(|i| doc.voice_text(&clip) != i.text);
    let status: SharedString = match &job {
        Some(None) => tr("生成中", "Generating").into(),
        Some(Some(_)) => tr("生成失败", "Failed").into(),
        None if !has_audio => tr("没有音频", "No audio").into(),
        None if on => PLAYING.with(|p| p.borrow().as_ref().map(|p| {
            let at: f64 = unsafe { msg_send![p.player, currentTime] };
            format!("{} / {}", clock(at as f32), clock(p.length))
        })).unwrap_or_default().into(),
        None if stale => tr("文字已改动", "Text changed").into(),
        None => clock(info.as_ref().map_or(0., |i| i.length)).into(),
    };
    let tap = { let clip = clip.clone(); move |ed: &mut Editor, _: &MouseDownEvent, _: &mut Window, cx: &mut Context<Editor>| {
        cx.stop_propagation();
        super::glass::haptic();
        match JOBS.with(|j| j.borrow().get(&clip).map(|j| matches!(j, Job::Running))) {
            Some(true) => {}
            Some(false) => if let Some(info) = clip_info(&clip) { generate(clip.clone(), Clip { text: ed.core.doc.voice_text(&clip), ..info }, cx) },
            None if playing(&clip) => { stop(); cx.notify(); }
            None => play(clip.clone(), cx),
        }
    } };
    let n = slot(&clip);
    let mut items = vec![menu::Item::new(REDO[n], tr("重新生成", "Regenerate"), "arrow.clockwise", 0)];
    items.push(menu::Item::new(DROP[n], tr("删除配音", "Remove Voice Over"), "trash", 1).danger());
    let caption = Theme::text_caption();
    div().id(SharedString::from(format!("voice-row-{clip}"))).mt(px(6.)).h(px(28.)).flex().items_center().gap(px(8.))
        .child(div().flex().items_center().gap(px(8.)).cursor_pointer().on_mouse_down(MouseButton::Left, cx.listener(tap))
            .child(face)
            .child(div().text_size(px(13.)).text_color(caption).child(name))
            .child(div().text_size(px(13.)).text_color(Theme::text_placeholder()).child(status))
            .when(on, |d| d.child(assistant::preview::bars("voice-bars", 11., caption)))
            .when(matches!(job, Some(None)), |d| d.opacity(0.6)))
        .child(div().relative().size(px(28.)).flex().items_center().justify_center()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(svg().path("editor-icons/ellipsis.svg").size(px(16.)).text_color(Theme::text_placeholder()))
            .child(menu::button(MORE[n], items)))
        .into_any_element()
}

/// 「…」菜单里的两项。
pub fn menu_action(id: &str, w: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) -> bool {
    let (redo, n) = match (REDO.iter().position(|k| *k == id), DROP.iter().position(|k| *k == id)) { (Some(n), _) => (true, n), (_, Some(n)) => (false, n), _ => return false };
    let Some(clip) = SLOTS.with(|s| s.borrow().0.get(n).cloned()) else { return true };
    let Some(editor) = w.editor.clone() else { return true };
    editor.update(cx, |ed, cx| {
        if redo {
            let Some(info) = clip_info(&clip) else { return };
            // 角色还在就用它现在的名字。
            generate(clip.clone(), Clip { text: ed.core.doc.voice_text(&clip), starts: vec![], length: 0., ..info }, cx);
        } else {
            if playing(&clip) { stop(); }
            ed.core.clear_voice(&clip);
            ed.changed(window, cx);
            // 撤销能把标记找回来：音频文件先留着。
        }
    });
    true
}
