//! 助手（MarginAI）的专属页：照 Craft、Notion 的 AI 页，一整页对话，从下面升上来，盖住当前页。
//!
//! 从哪儿进来决定助手看的范围（`fdl_ai::run::Scope`）：主页、文件夹页是整个空间，文档页底部的玻璃钮是这篇文档，
//! 文档里选中块后底部工具条的「助手」是这篇加选中的块。范围写在输入卡片左下的胶囊上，可以改成整个空间。
//! 换范围（从别处进来）就开一段新对话；同一个范围接着上次那段。
//!
//! 一次 run 由选定的核心驱动（`fdl_ai::run`）：酒馆一次请求，Agent 带只读文档工具循环。宿主在这里：素材快照（`Shelf`）
//! 在主线程上备好，整个 run 在 `assistant::llm` 的运行时上跑（网络、执行工具、成形），每一步经 channel 交回主线程给界面。
//! 一次只有一个 run；关掉页面、切到后台照样跑完（`background`）。对话记录存在空间的 `assistant/chats/`（`run::chat`）。
//!
//! 第一版不往文档里插：回复可以拷贝。
use std::{cell::{Cell, RefCell}, collections::{HashMap, HashSet}, path::PathBuf, sync::Arc, time::{Duration, Instant}};

use fdl_ai::run::{self, Act, Call, Core, Kind, Reply, Role, Scope, Shelf, chat::{self, Chat, Item}, tools};
use fdl_ai::tavern::{assemble::Identity, library::{self, Settings}};
use futures::{StreamExt, channel::mpsc};
use gpui::{prelude::FluentBuilder, *};

use crate::{app::{View, Workspace}, model::{Block, BlockKind}, settings::tr, store::Store, theme::Theme, ui};
use super::{assistant::{self, llm::Event}, menu};

#[path = "margin_card.rs"]
pub mod card;

pub const SEND: &str = "margin-send";
const CLOSE: &str = "margin-close";
const NEW: &str = "margin-new";
const DELETE: &str = "margin-delete";
pub const OPEN_HOME: &str = "margin-open-home";
pub const OPEN_DOC: &str = "margin-open-doc";
/// 历史菜单里一段对话一项：标识按位置编号（菜单项的标识是静态的）。
const CHATS: [&str; 12] = ["margin-chat-0", "margin-chat-1", "margin-chat-2", "margin-chat-3", "margin-chat-4", "margin-chat-5", "margin-chat-6", "margin-chat-7", "margin-chat-8", "margin-chat-9", "margin-chat-10", "margin-chat-11"];
const SLIDE: Duration = Duration::from_millis(320);

/// 正在跑的 run。
struct Live {
    turn: u64,
    call: tokio::task::AbortHandle,
    _ticket: super::background::Ticket,
    /// 走过的步骤；最后一步在跑（还没出文字时）。
    steps: Vec<String>,
    blocks: Vec<Block>,
    stored: String,
    model: String,
}

struct Page {
    chat: Chat,
    /// 范围的说法：（胶囊上的字，SF Symbol）。
    label: (String, &'static str),
    /// 进来时的范围（胶囊菜单里可以从「整个空间」切回来）。
    entry: Scope,
    /// 进来时选中的块的 Markdown（范围是选中的块时；新开的对话记下它）。
    picked: String,
    live: Option<Live>,
    /// 显示用的块，按条目位置记（读出来的对话按 Markdown 成形一次）。
    shaped: HashMap<usize, Vec<Block>>,
    /// 展开了步骤的条目。
    expanded: HashSet<usize>,
    /// 最近的对话（历史菜单），打开历史菜单前算好。
    recent: Vec<(String, String)>,
}

thread_local! {
    static PAGE: RefCell<Option<Page>> = const { RefCell::new(None) };
    static OPEN: Cell<bool> = const { Cell::new(false) };
    /// 开 / 关的动画从哪一刻起（`None`：第一帧还没画）。
    static MOTION: Cell<Option<(bool, Option<Instant>)>> = const { Cell::new(None) };
    static TURN: Cell<u64> = const { Cell::new(0) };
    static SCROLL: ScrollHandle = ScrollHandle::new();
    static FOLLOW: Cell<bool> = const { Cell::new(true) };
    /// 选的核心（新对话用这个）。
    static CORE: Cell<Kind> = const { Cell::new(Kind::Agent) };
}

fn dir() -> PathBuf { chat::dir(&Store::root_dir()) }
fn now() -> u64 { fdl_core::time::now() }
fn new_id() -> String { uuid::Uuid::new_v4().simple().to_string()[..16].to_owned() }
fn toast(text: String) { menu::dispatch(move |w, _, cx| w.show_toast_kind(ui::ToastKind::Error, &text, cx)); }

/// 页开着（含正在滑上来、滑下去）。
pub fn open() -> bool { OPEN.with(|o| o.get()) || sliding() }
/// 正在滑：底下那页还要画，玻璃先让开。
pub fn sliding() -> bool { MOTION.with(|m| m.get()).is_some_and(|(_, at)| at.is_none_or(|t| t.elapsed() < SLIDE)) }
/// 完全盖住了：底下那页不用画（`app.rs`）。
pub fn covering() -> bool { OPEN.with(|o| o.get()) && !sliding() }
/// 正在跑。
pub fn busy() -> bool { PAGE.with(|p| p.borrow().as_ref().is_some_and(|p| p.live.is_some())) }

fn title_of(t: &str) -> String { if t.trim().is_empty() { tr("无标题", "Untitled").into() } else { t.trim().to_owned() } }

/// 范围的说法。
fn label(w: &Workspace, scope: &Scope) -> (String, &'static str) {
    let title = |id: &str| w.entries.iter().find(|e| e.id == id).map(|e| title_of(&e.title)).unwrap_or_else(|| tr("这篇文档", "This document").into());
    match scope {
        Scope::Space => (tr("整个空间", "Whole space").into(), "square.stack"),
        Scope::Doc { id } => (title(id), "doc.text"),
        Scope::Blocks { blocks, .. } => (crate::localized_format!("选中的 {} 块", "{} selected blocks", blocks.len()), "text.quote"),
    }
}

/// 从主页、文件夹页进来：整个空间。
pub fn open_space(w: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) { show(w, Scope::Space, String::new(), window, cx); }

/// 从文档页进来：这篇文档。
pub fn open_doc(w: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let Some(id) = w.current.clone().filter(|_| w.view == View::Doc) else { return open_space(w, window, cx) };
    show(w, Scope::Doc { id }, String::new(), window, cx);
}

/// 文档里选中了块（编辑器那边取好块的标识和 Markdown）。
pub fn open_blocks(w: &mut Workspace, blocks: Vec<String>, window: &mut Window, cx: &mut Context<Workspace>) {
    let Some(id) = w.current.clone() else { return };
    let picked = w.editor.as_ref().map(|e| picked(&e.read(cx).core.doc, &blocks)).unwrap_or_default();
    show(w, Scope::Blocks { id, blocks }, picked, window, cx);
}

fn show(w: &mut Workspace, scope: Scope, picked: String, _: &mut Window, cx: &mut Context<Workspace>) {
    super::glass::haptic();
    let label = label(w, &scope);
    PAGE.with(|p| {
        let mut p = p.borrow_mut();
        // 同一个范围（选中的块也一样）：接着上次那段；不然开一段新的（正在跑的那段先留着跑完）。
        let same = p.as_ref().is_some_and(|p| p.entry == scope);
        if !same || p.is_none() {
            let busy = p.as_ref().is_some_and(|p| p.live.is_some());
            if busy { save_live(p.as_mut().unwrap()); }
            *p = Some(Page { chat: fresh(&scope, &picked), label, entry: scope, picked, live: None, shaped: HashMap::new(), expanded: HashSet::new(), recent: vec![] });
        } else if let Some(p) = p.as_mut() { p.label = label; }
    });
    let fresh = PAGE.with(|p| p.borrow().as_ref().is_some_and(|p| p.chat.items.is_empty()));
    if fresh { card::focus(); }
    refresh_recent();
    OPEN.with(|o| o.set(true));
    MOTION.with(|m| m.set(Some((true, None))));
    FOLLOW.with(|f| f.set(true));
    super::shot::soft_hide();
    cx.notify();
}

/// 正在跑的被换下去（开了别的范围）：停掉，说到哪儿存到哪儿。
fn save_live(p: &mut Page) {
    if let Some(live) = p.live.take() {
        live.call.abort();
        p.chat.items.push(Item { role: Role::Assistant, text: live.stored, steps: live.steps, at: now(), model: live.model, stopped: true, ..Default::default() });
        let _ = chat::save(&dir(), &p.chat);
    }
}

fn close(cx: &mut Context<Workspace>) {
    card::blur();
    OPEN.with(|o| o.set(false));
    MOTION.with(|m| m.set(Some((false, None))));
    cx.notify();
}

fn refresh_recent() {
    let list: Vec<(String, String)> = chat::list(&dir()).into_iter().take(CHATS.len()).map(|c| (c.id.clone(), c.name())).collect();
    PAGE.with(|p| if let Some(p) = p.borrow_mut().as_mut() { p.recent = list; });
}

/// 系统菜单、玻璃钮点了。
pub fn menu_action(id: &str, w: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) -> bool {
    match id {
        OPEN_HOME => open_space(w, window, cx),
        OPEN_DOC => open_doc(w, window, cx),
        CLOSE => close(cx),
        SEND => if busy() { stop(); cx.notify(); } else { send(w, card::take(), cx) },
        NEW => {
            super::glass::haptic();
            PAGE.with(|p| if let Some(p) = p.borrow_mut().as_mut() {
                save_live(p);
                p.chat = fresh(&p.entry, &p.picked);
                (p.shaped, p.expanded) = (HashMap::new(), HashSet::new());
            });
            let entry = PAGE.with(|p| p.borrow().as_ref().map(|p| p.entry.clone()));
            if let Some(e) = entry { let l = label(w, &e); PAGE.with(|p| if let Some(p) = p.borrow_mut().as_mut() { p.label = l; }); }
            card::focus();
            cx.notify();
        }
        DELETE => {
            PAGE.with(|p| if let Some(p) = p.borrow_mut().as_mut() {
                save_live(p);
                let _ = chat::delete(&dir(), &p.chat.id);
                p.chat = fresh(&p.entry, &p.picked);
                (p.shaped, p.expanded) = (HashMap::new(), HashSet::new());
            });
            refresh_recent();
            cx.notify();
        }
        "margin-scope-space" | "margin-scope-entry" => {
            let to_space = id == "margin-scope-space";
            PAGE.with(|p| if let Some(p) = p.borrow_mut().as_mut() { p.chat.scope = if to_space { Scope::Space } else { p.entry.clone() }; });
            let scope = PAGE.with(|p| p.borrow().as_ref().map(|p| p.chat.scope.clone()));
            if let Some(s) = scope { let l = label(w, &s); PAGE.with(|p| if let Some(p) = p.borrow_mut().as_mut() { p.label = l; }); }
            cx.notify();
        }
        "margin-core-tavern" | "margin-core-agent" => {
            let k = if id == "margin-core-agent" { Kind::Agent } else { Kind::Tavern };
            CORE.with(|c| c.set(k));
            PAGE.with(|p| if let Some(p) = p.borrow_mut().as_mut() { p.chat.core = k; });
            cx.notify();
        }
        _ if let Some(i) = CHATS.iter().position(|c| *c == id) => {
            let target = PAGE.with(|p| p.borrow().as_ref().and_then(|p| p.recent.get(i).map(|(id, _)| id.clone())));
            if let Some(c) = target.and_then(|id| chat::load(&dir(), &id)) {
                let l = label(w, &c.scope);
                PAGE.with(|p| if let Some(p) = p.borrow_mut().as_mut() {
                    save_live(p);
                    CORE.with(|k| k.set(c.core));
                    p.chat = c;
                    p.label = l;
                    (p.shaped, p.expanded) = (HashMap::new(), HashSet::new());
                });
                FOLLOW.with(|f| f.set(true));
                cx.notify();
            }
        }
        _ => return false,
    }
    true
}

/// 素材快照：文档库里每篇的标题和正文（纯文字），进来的那篇的 Markdown。
fn shelf(w: &Workspace, scope: &Scope, picked: &str, cx: &App) -> Shelf {
    let folders: HashMap<&str, &str> = w.entries.iter().filter(|e| e.meta.folder.is_some()).map(|e| (e.id.as_str(), e.title.as_str())).collect();
    let docs = w.entries.iter().filter(|e| e.meta.folder.is_none()).map(|e| run::Doc {
        id: e.id.clone(), title: e.title.clone(), folder: e.meta.parent.as_deref().and_then(|p| folders.get(p)).map(|s| s.to_string()),
        modified: e.meta.modified, text: Arc::from(&*e.body),
    }).collect();
    // 进来的那篇：编辑器里开着就用编辑器里的（可能还没存）；已经删了就没有。
    let focus = scope.doc().and_then(|id| {
        let doc = w.editor.as_ref().map(|e| e.read(cx)).filter(|_| w.current.as_deref() == Some(id)).map(|e| e.core.doc.clone()).or_else(|| w.store.load(id).ok())?;
        Some(run::Focus { id: id.to_owned(), title: doc.title.text.clone(), markdown: fdl_core::markdown::export(&doc, &Default::default()).text,
            picked: matches!(scope, Scope::Blocks { .. }).then(|| picked.to_owned()) })
    });
    let space = fdl_core::spaces::current().and_then(|s| s.name.clone()).unwrap_or_default();
    Shelf { space, docs, focus, now: Some(fdl_core::time::civil(fdl_core::time::now())) }
}

/// 照进来时的范围新开一段对话。
fn fresh(entry: &Scope, picked: &str) -> Chat {
    let mut c = Chat::new(new_id(), entry.clone(), CORE.with(|c| c.get()), now());
    if matches!(entry, Scope::Blocks { .. }) { c.picked = picked.to_owned(); }
    c
}

/// 选中的块的 Markdown（进来时从编辑器里取；块的标识不跨启动）。不带标题，标题另给。
fn picked(d: &crate::model::Document, ids: &[String]) -> String {
    fn walk(bs: &[Block], ids: &HashSet<&str>, out: &mut Vec<Block>) {
        for b in bs { if ids.contains(b.id.0.to_string().as_str()) { out.push(b.clone()) } else { walk(&b.children, ids, out) } }
    }
    let ids: HashSet<&str> = ids.iter().map(String::as_str).collect();
    let mut blocks = vec![];
    walk(&d.blocks, &ids, &mut blocks);
    fdl_core::markdown::export(&crate::model::Document { title: Default::default(), icon: None, cover: None, blocks }, &Default::default()).text
}

/// 网络那头交回来的。
enum Up { Thinking, Step(String), Preview { stored: String, blocks: Vec<Block> }, Done { stored: String, blocks: Vec<Block> }, Failed(String) }

fn send(w: &mut Workspace, input: String, cx: &mut Context<Workspace>) {
    let input = input.trim().to_owned();
    if input.is_empty() { return; }
    let Some((base, key, model, effort)) = assistant::text::main() else { return toast(tr("先在 设置 › 文本 里选择模型", "Choose a model in Settings › Text first").into()) };
    let Some((scope, kind, picked, history)) = PAGE.with(|p| p.borrow().as_ref().map(|p| (p.chat.scope.clone(), p.chat.core, p.chat.picked.clone(), p.chat.history()))) else { return };
    let shelf = shelf(w, &scope, &picked, cx);
    let seed = fdl_core::time::now_ms();
    let core: Box<dyn Core> = match kind {
        Kind::Agent => Box::new(run::agent::Agent::new(scope.clone())),
        Kind::Tavern => {
            let d = super::assist::dir();
            let s = Settings::load(&d);
            let preset = match library::load(&d, &s.preset) { Ok(p) => p, Err(e) => return toast(e) };
            let (c, pe) = (s.char().clone(), s.persona().clone());
            Box::new(run::tavern::Tavern::new(preset, Identity { name: c.name, description: c.description }, Identity { name: pe.name, description: pe.description }, model.clone(), seed, scope.clone()))
        }
    };
    let turn = TURN.with(|t| { t.set(t.get() + 1); t.get() });
    let gen_id = new_id();
    let (tx, mut rx) = mpsc::unbounded::<Up>();
    let call = assistant::llm::runtime().spawn(drive(core, shelf, input.clone(), history, (base, key, model.clone(), effort), gen_id, tx));
    let ticket = super::background::begin(super::background::Kind::Assistant, "MarginAI".into(), tr("助手正在想", "Assistant thinking"), 2, move || halted(turn));
    PAGE.with(|p| if let Some(p) = p.borrow_mut().as_mut() {
        p.chat.items.push(Item { role: Role::User, text: input, at: now(), ..Default::default() });
        p.live = Some(Live { turn, call: call.abort_handle(), _ticket: ticket, steps: vec![], blocks: vec![], stored: String::new(), model });
        let _ = chat::save(&dir(), &p.chat);
    });
    card::blur();
    FOLLOW.with(|f| f.set(true));
    cx.notify();
    cx.spawn(async move |this, cx| {
        while let Some(u) = rx.next().await {
            let end = matches!(u, Up::Done { .. } | Up::Failed(_));
            let fresh = PAGE.with(|p| p.borrow_mut().as_mut().is_some_and(|p| p.live.as_ref().is_some_and(|l| l.turn == turn) && { apply(p, u); true }));
            if !fresh { return; }
            let _ = this.update(cx, |_, cx| cx.notify());
            if end { refresh_recent(); return; }
        }
    }).detach();
}

/// 在运行时上：照核心的吩咐一步步做，直到说完。
async fn drive(mut core: Box<dyn Core>, shelf: Shelf, input: String, history: Vec<run::Turn>, (base, key, model, effort): (String, String, String, Option<String>), id: String, tx: mpsc::UnboundedSender<Up>) {
    let send = |u: Up| tx.unbounded_send(u).is_ok();
    let mut act = core.begin(&input, &history, &shelf);
    loop {
        act = match act {
            Act::Ask(mut body) => {
                body["model"] = model.clone().into();
                if let Some(e) = &effort && body.get("reasoning_effort").is_none() { body["reasoning_effort"] = e.clone().into(); }
                let mut stream = assistant::llm::complete(base.clone(), key.clone(), body);
                let (mut reply, mut settled) = (Reply::default(), 0);
                while let Some(e) = stream.next().await {
                    let ok = match e {
                        Event::Thinking => send(Up::Thinking),
                        Event::Text(t) => {
                            reply.text.push_str(&t);
                            let cut = core.settled(&reply.text);
                            if cut <= settled { continue; }
                            settled = cut;
                            let s = core.shape(&reply.text[..cut], &id, true);
                            send(Up::Preview { stored: s.stored, blocks: s.blocks })
                        }
                        Event::Calls(c) => { reply.calls = c; true }
                        Event::Done => break,
                        Event::Failed(e) => { send(Up::Failed(e)); return; }
                    };
                    if !ok { return; }
                }
                // 这一轮要调工具：它边调边说的那几句不算回答，预览清掉。
                if !reply.calls.is_empty() && settled > 0 && !send(Up::Preview { stored: String::new(), blocks: vec![] }) { return; }
                core.reply(reply)
            }
            Act::Use(calls) => {
                let mut results: Vec<(Call, String)> = vec![];
                for c in calls {
                    if !send(Up::Step(tools::label(&shelf, &c))) { return; }
                    let out = tools::run(&shelf, &c);
                    results.push((c, out));
                }
                core.results(&shelf, results)
            }
            Act::Finish(text) => {
                let s = core.shape(&text, &id, false);
                let _ = send(if s.blocks.is_empty() && s.stored.trim().is_empty() { Up::Failed(tr("模型没有写出内容", "The model returned nothing").into()) } else { Up::Done { stored: s.stored, blocks: s.blocks } });
                return;
            }
        };
    }
}

fn apply(p: &mut Page, u: Up) {
    let Some(live) = p.live.as_mut() else { return };
    match u {
        Up::Thinking => {}
        Up::Step(s) => live.steps.push(s),
        Up::Preview { stored, blocks } => { live.stored = stored; live.blocks = blocks; }
        Up::Done { stored, blocks } => {
            let live = p.live.take().unwrap();
            p.chat.items.push(Item { role: Role::Assistant, text: stored, steps: live.steps, at: now(), model: live.model, ..Default::default() });
            p.shaped.insert(p.chat.items.len() - 1, blocks);
            let _ = chat::save(&dir(), &p.chat);
        }
        Up::Failed(e) => {
            let live = p.live.take().unwrap();
            live._ticket.fail();
            p.chat.items.push(Item { role: Role::Assistant, text: live.stored, steps: live.steps, at: now(), model: live.model, failed: Some(e), ..Default::default() });
            let _ = chat::save(&dir(), &p.chat);
        }
    }
}

/// 用户点了停止。
fn stop() { PAGE.with(|p| if let Some(p) = p.borrow_mut().as_mut() { save_live(p); }); }
/// 系统停掉了后台任务。
fn halted(turn: u64) {
    let mine = PAGE.with(|p| p.borrow().as_ref().is_some_and(|p| p.live.as_ref().is_some_and(|l| l.turn == turn)));
    if mine { stop(); menu::dispatch(|_, _, cx| cx.notify()); }
}

/// 出错的那条重新问一次：把它和它前面那句话拿掉，那句话重新发。
fn retry(w: &mut Workspace, cx: &mut Context<Workspace>) {
    let input = PAGE.with(|p| {
        let mut p = p.borrow_mut();
        let p = p.as_mut()?;
        if p.live.is_some() { return None; }
        let at = p.chat.items.iter().rposition(|i| i.role == Role::User)?;
        let input = p.chat.items[at].text.clone();
        p.chat.items.truncate(at);
        p.shaped.retain(|k, _| *k < at);
        Some(input)
    });
    if let Some(input) = input { send(w, input, cx); }
}

// ── 画 ──────────────────────────────────────────────────────────────

/// 盖在当前页上的助手页。没开时 `None`。
pub fn page(w: &Workspace, window: &mut Window, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    let (opening, at) = MOTION.with(|m| m.get()).unwrap_or((OPEN.with(|o| o.get()), Some(Instant::now() - SLIDE)));
    if !opening && at.is_some_and(|t| t.elapsed() >= SLIDE) { MOTION.with(|m| m.set(None)); return None; }
    if !OPEN.with(|o| o.get()) && !sliding() { return None; }
    let at = at.unwrap_or_else(|| { let t = Instant::now(); MOTION.with(|m| m.set(Some((opening, Some(t))))); t });
    let t = (at.elapsed().as_secs_f32() / SLIDE.as_secs_f32()).clamp(0., 1.);
    if t < 1. { window.request_animation_frame(); }
    // 升上来是缓出（快到位时慢下来），落下去是缓入。
    let shown = if opening { 1. - (1. - t).powi(4) } else { 1. - t * t };
    let (_, _, viewport) = super::layout();
    let lift = viewport.height * (1. - shown);
    let body = PAGE.with(|p| p.borrow().as_ref().map(|p| content(p, w, opening && t >= 1., window, cx)))?;
    Some(deferred(div().id("margin-page").occlude().absolute().left_0().right_0().top(lift).h(viewport.height).bg(Theme::bg_body())
        .rounded_t(px(if t < 1. { 38. } else { 0. })).shadow(if t < 1. { crate::theme::shadow_dialog() } else { vec![] })
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(body)).with_priority(4).into_any_element())
}

fn content(p: &Page, _: &Workspace, settled: bool, window: &mut Window, cx: &mut Context<Workspace>) -> AnyElement {
    let ink = super::glass::ink();
    let top = super::insets().top;
    let keyboard = (window.viewport_size().height - super::visible_height(window)).max(px(0.));
    let bottom = keyboard.max(super::insets().bottom) + px(8.);
    // 顶栏：左边历史，中间标题，右边新对话和关闭。玻璃钮在滑到位后才放（`app.rs` 那时才放行玻璃）。
    let history: Vec<menu::Item> = if p.recent.is_empty() { vec![menu::Item::new("margin-none", tr("还没有对话", "No chats yet"), "", 0).disabled()] } else {
        p.recent.iter().enumerate().map(|(i, (id, name))| menu::Item::new(CHATS[i], if name.is_empty() { tr("新对话", "New chat").to_owned() } else { name.clone() }, "", 0).checked(*id == p.chat.id)).collect()
    };
    let history = if p.chat.items.is_empty() { history } else { let mut h = history; h.push(menu::Item::new(DELETE, tr("删除这段对话", "Delete this chat"), "trash", 1).danger()); h };
    let name = p.chat.name();
    let title = div().flex_1().min_w_0().px(px(8.)).flex().justify_center().child(div().max_w_full().overflow_hidden().whitespace_nowrap().text_ellipsis()
        .text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).text_color(Theme::text()).child(if name.is_empty() { "MarginAI".to_owned() } else { name }));
    let glass_spot = |id: &'static str| div().id(id).relative().flex_none().size(px(44.)).rounded_full();
    let bar = div().flex_none().h(px(56.)).px(px(16.)).flex().items_center().gap(px(8.))
        .child(glass_spot("margin-history").when(settled, |d| d.child(menu::glass_button("margin-history", "clock", ink, history))))
        .child(title)
        .when(!p.chat.items.is_empty(), |d| d.child(glass_spot("margin-new").when(settled, |d| d.child(menu::glass_tap("margin-new", "square.and.pencil", ink, NEW, crate::settings::text("新对话"))))))
        .child(glass_spot("margin-close").when(settled, |d| d.child(menu::glass_tap("margin-close", "xmark", ink, CLOSE, crate::settings::text("关闭")))));

    // 对话：跟着最新的往下滚，用户往上翻了就不跟。
    let scroll = SCROLL.with(|s| s.clone());
    let max = scroll.max_offset().y;
    let at_bottom = -scroll.offset().y >= max - px(24.);
    if p.live.is_some() && !at_bottom && !FOLLOW.with(|f| f.get()) {} else if p.live.is_some() || FOLLOW.with(|f| f.get()) { scroll.set_offset(point(px(0.), -max)); }
    if p.live.is_none() { FOLLOW.with(|f| f.set(false)); } else { FOLLOW.with(|f| f.set(at_bottom || FOLLOW.with(|f| f.get()))); }
    let mut list = div().id("margin-list").flex_1().min_h_0().overflow_y_scroll().track_scroll(&scroll).px(px(20.)).pt(px(8.)).pb(px(card::HEIGHT + 24.)).flex().flex_col().gap(px(20.));
    for (i, item) in p.chat.items.iter().enumerate() {
        list = list.child(match item.role {
            Role::User => user(&item.text),
            _ => assistant_item(p, i, item, cx),
        });
    }
    if let Some(live) = &p.live { list = list.child(live_item(live)); }
    if p.live.is_some() { window.request_animation_frame(); }

    let chips = [
        card::Chip { title: p.label.0.clone(), glyph: p.label.1, menu: if p.entry == Scope::Space { vec![] } else {
            let entry = label_entry(p);
            vec![("margin-scope-entry", entry, p.chat.scope == p.entry), ("margin-scope-space", tr("整个空间", "Whole space").into(), p.chat.scope == Scope::Space)]
        } },
        card::Chip { title: core_name(p.chat.core).into(), glyph: match p.chat.core { Kind::Tavern => "text.bubble", Kind::Agent => "point.3.connected.trianglepath.dotted" },
            menu: vec![("margin-core-tavern", core_name(Kind::Tavern).into(), p.chat.core == Kind::Tavern), ("margin-core-agent", core_name(Kind::Agent).into(), p.chat.core == Kind::Agent)] },
    ];
    // 对话开始以后核心就定了（换核心开新对话），胶囊上没有菜单。
    let chips = if p.chat.items.is_empty() { chips } else { let [a, mut b] = chips; b.menu.clear(); [a, b] };
    div().size_full().flex().flex_col().pt(top).pb(keyboard)
        .child(bar)
        .child(list)
        .child(div().absolute().left(px(12.)).bottom(bottom).when(settled, |d| d.child(card::slot(chips, p.live.is_some()))))
        .into_any_element()
}

fn label_entry(p: &Page) -> String { match &p.entry { Scope::Blocks { blocks, .. } => crate::localized_format!("选中的 {} 块", "{} selected blocks", blocks.len()), _ => tr("这篇文档", "This document").into() } }
fn core_name(k: Kind) -> &'static str { match k { Kind::Tavern => tr("酒馆", "Tavern"), Kind::Agent => "Agent" } }

fn user(text: &str) -> AnyElement {
    div().w_full().flex().justify_end().child(div().max_w(relative(0.82)).px(px(14.)).py(px(9.)).rounded(px(20.)).bg(super::search_fill())
        .text_size(px(16.)).line_height(px(23.)).text_color(Theme::text()).child(text.to_owned())).into_any_element()
}

/// 步骤那一行：「› 3 步」，点开逐条列出。
fn steps_row(i: usize, steps: &[String], open: bool, cx: &mut Context<Workspace>) -> AnyElement {
    let head = div().id(("margin-steps", i)).flex().items_center().gap(px(4.)).text_size(px(14.)).text_color(Theme::text_caption())
        .child(ui::icon(if open { "chevron-down" } else { "chevron-right" }, 12., Theme::text_caption()))
        .child(crate::localized_format!("{} 步", "{} steps", steps.len()))
        .on_click(cx.listener(move |_, _, _, cx| { PAGE.with(|p| if let Some(p) = p.borrow_mut().as_mut() { if !p.expanded.remove(&i) { p.expanded.insert(i); } }); cx.notify(); }));
    div().flex().flex_col().gap(px(4.)).child(head)
        .when(open, |d| d.children(steps.iter().map(|s| div().pl(px(16.)).text_size(px(14.)).line_height(px(20.)).text_color(Theme::text_caption()).child(s.clone()))))
        .into_any_element()
}

fn assistant_item(p: &Page, i: usize, item: &Item, cx: &mut Context<Workspace>) -> AnyElement {
    let blocks = p.shaped.get(&i).cloned().unwrap_or_else(|| if item.text.trim().is_empty() { vec![] } else { fdl_core::markdown::parse_file(&item.text, "", None).blocks });
    let mut col = div().w_full().flex().flex_col().gap(px(10.));
    if !item.steps.is_empty() { col = col.child(steps_row(i, &item.steps, p.expanded.contains(&i), cx)); }
    if !blocks.is_empty() { col = col.child(render_blocks(&blocks)); }
    if let Some(e) = &item.failed {
        col = col.child(div().flex().items_center().gap(px(12.)).text_size(px(14.)).line_height(px(20.))
            .child(div().flex_1().min_w_0().text_color(Theme::danger()).child(e.clone()))
            .when(i + 1 == p.chat.items.len(), |d| d.child(div().id("margin-retry").flex_none().text_color(Theme::primary()).child(tr("重试", "Retry"))
                .on_click(cx.listener(|w, _, _, cx| retry(w, cx))))));
    } else if item.stopped {
        col = col.child(div().text_size(px(14.)).text_color(Theme::text_caption()).child(tr("已停止", "Stopped")));
    }
    if item.failed.is_none() && !item.text.trim().is_empty() {
        let text = item.text.clone();
        col = col.child(div().flex().child(div().id(("margin-copy", i)).size(px(32.)).ml(px(-6.)).flex().items_center().justify_center().rounded(px(8.)).active(|s| s.bg(Theme::fill_pressed()))
            .child(ui::icon("copy", 16., Theme::text_caption()))
            .on_click(cx.listener(move |w, _, _, cx| { super::glass::haptic(); cx.write_to_clipboard(ClipboardItem::new_string(text.clone())); w.show_toast(tr("已拷贝", "Copied"), cx); }))));
    }
    col.into_any_element()
}

/// 正在跑的那条：步骤、写出来的，还没出字时一行在做什么（淡淡地一明一暗）。
fn live_item(live: &Live) -> AnyElement {
    let mut col = div().w_full().flex().flex_col().gap(px(10.));
    let pulse = |text: String| div().text_size(px(14.)).text_color(Theme::text_caption())
        .child(text).with_animation("margin-pulse", Animation::new(Duration::from_millis(1400)).repeat(), |d, t| d.opacity(0.45 + 0.55 * (1. - (t * std::f32::consts::TAU).cos()) / 2.));
    if live.blocks.is_empty() {
        let doing = live.steps.last().map(|s| format!("{s}…")).unwrap_or_else(|| tr("正在想…", "Thinking…").into());
        if live.steps.len() > 1 { col = col.child(div().text_size(px(14.)).text_color(Theme::text_caption()).child(crate::localized_format!("已完成 {} 步", "{} steps done", live.steps.len() - 1))); }
        col = col.child(pulse(doing));
    } else {
        if !live.steps.is_empty() { col = col.child(div().text_size(px(14.)).text_color(Theme::text_caption()).child(crate::localized_format!("{} 步", "{} steps", live.steps.len()))); }
        col = col.child(render_blocks(&live.blocks));
    }
    col.into_any_element()
}

// ── 块：和文档同样的层级，简化的样子（只读） ─────────────────────────

fn render_blocks(blocks: &[Block]) -> Div {
    let mut col = div().w_full().flex().flex_col().gap(px(8.));
    let mut n = 0u32;
    for b in blocks {
        n = match b.kind { BlockKind::Ordered { seq } => seq.unwrap_or(n + 1), _ => 0 };
        col = col.child(block(b, n));
    }
    col
}

fn rich(t: &crate::model::RichText, size: f32, line: f32, color: Hsla) -> AnyElement {
    let mut highlights = vec![];
    let mut at = 0;
    for r in &t.runs {
        let m = &r.marks;
        if m.bold || m.italic || m.code || m.strike || m.link.is_some() {
            let mut s = HighlightStyle::default();
            if m.bold { s.font_weight = Some(FontWeight::SEMIBOLD); }
            if m.italic { s.font_style = Some(FontStyle::Italic); }
            if m.strike { s.strikethrough = Some(StrikethroughStyle { thickness: px(1.), color: Some(color) }); }
            if m.code { s.background_color = Some(Theme::fill_pressed()); }
            if m.link.is_some() { s.color = Some(Theme::primary()); }
            highlights.push((at..at + r.len, s));
        }
        at += r.len;
    }
    div().w_full().min_w_0().text_size(px(size)).line_height(px(line)).text_color(color)
        .child(StyledText::new(SharedString::from(t.text.clone())).with_highlights(highlights)).into_any_element()
}

fn block(b: &Block, n: u32) -> AnyElement {
    let text = || rich(&b.text, 16., 26., Theme::text());
    let marker = |m: AnyElement, body: AnyElement| div().w_full().flex().items_start().child(div().flex_none().w(px(22.)).h(px(26.)).flex().items_center().child(m)).child(div().flex_1().min_w_0().child(body)).into_any_element();
    let dot = |s: &str| div().text_size(px(16.)).text_color(Theme::text_caption()).child(s.to_owned()).into_any_element();
    let el = match &b.kind {
        BlockKind::Heading { level, .. } => { let (s, l) = match level { 1 => (22., 30.), 2 => (19., 27.), _ => (17., 25.) }; div().pt(px(4.)).font_weight(FontWeight::SEMIBOLD).child(rich(&b.text, s, l, Theme::text())).into_any_element() }
        BlockKind::Bullet => marker(dot("•"), text()),
        BlockKind::Ordered { .. } => marker(dot(&format!("{n}.")), text()),
        BlockKind::Todo { done } => marker(if *done { ui::icon("todo-check", 16., Theme::text_caption()).into_any_element() } else { div().size(px(14.)).rounded(px(4.)).border_1().border_color(Theme::text_caption()).into_any_element() }, rich(&b.text, 16., 26., if *done { Theme::text_caption() } else { Theme::text() })),
        // 引用、提示块的字常在子块里（Markdown 导入的就是）：子块画在框里面。
        BlockKind::Quote => return div().w_full().flex().gap(px(12.)).child(div().flex_none().w(px(3.)).rounded(px(2.)).bg(Theme::line_border()))
            .child(div().flex_1().min_w_0().flex().flex_col().gap(px(8.)).text_color(Theme::text_caption()).when(!b.text.text.trim().is_empty(), |d| d.child(rich(&b.text, 16., 26., Theme::text_caption()))).child(render_blocks(&b.children))).into_any_element(),
        BlockKind::Code { .. } => div().w_full().p(px(12.)).rounded(px(10.)).bg(super::search_fill()).font_family(crate::theme::MONO_FONT)
            .text_size(px(14.)).line_height(px(21.)).text_color(Theme::text()).child(b.text.text.clone()).into_any_element(),
        BlockKind::Callout { emoji, .. } => return div().w_full().p(px(12.)).rounded(px(10.)).bg(super::search_fill()).flex().gap(px(8.))
            .when(!emoji.is_empty(), |d| d.child(div().flex_none().text_size(px(16.)).line_height(px(26.)).child(emoji.clone())))
            .child(div().flex_1().min_w_0().flex().flex_col().gap(px(8.)).when(!b.text.text.trim().is_empty(), |d| d.child(text())).child(render_blocks(&b.children))).into_any_element(),
        BlockKind::Divider { .. } => div().w_full().py(px(6.)).child(div().h(px(1.)).bg(Theme::line_divider())).into_any_element(),
        _ if b.text.text.trim().is_empty() && b.children.is_empty() => div().into_any_element(),
        _ => text(),
    };
    if b.children.is_empty() { return el; }
    div().w_full().flex().flex_col().gap(px(8.)).child(el).child(div().pl(px(22.)).child(render_blocks(&b.children))).into_any_element()
}
