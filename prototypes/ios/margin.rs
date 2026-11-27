//! 助手（MarginAI）的专属页：照 Craft、Notion 的 AI 页，一整页对话，从下面升上来，盖住当前页。
//!
//! 每句话附带资源（`fdl_ai::run::Resource`：文件夹、文档、文档里的几块），写在输入卡片最上面一排，可以删、可以 @ 搜着加。
//! 默认带什么看从哪儿进来：文档页底部的玻璃钮带这篇文档，文档里选中块后底部工具条的「助手」带这几块，
//! 文件夹页左下角的 ✦ 带这个文件夹，主页的 ✦ 什么都不带（整个空间本来就是范围）。从别处进来开一段新对话；从同一处进来接着上次那段。
//!
//! 用哪个助手：这段对话自己选的 > 空间的默认助手 > 内置的 MarginAI（`fdl_ai::assistants::Assistants::resolve`）。
//! 卡片左下的胶囊是助手（头像和名字），菜单里换助手、临时换核心（酒馆 / Agent）；旁边是权限（只读 / 可编辑，按对话记，默认照助手的；
//! 酒馆不调工具，权限灰着）。用户说了第一句以后助手和核心就定了（换的话开新对话），权限随时能改。
//! 新对话先显示助手的开场白（有的话），说第一句时它才记进对话。
//!
//! 一次 run 由选定的核心驱动（`fdl_ai::run`）：酒馆一次请求，Agent 带文档工具循环（可编辑时还有写的工具）。宿主在这里：素材快照（`Shelf`：
//! 空间的 `SPACE.md`、助手的名字和设定、附带的资源）在主线程上备好，整个 run 在 `assistant::llm` 的运行时上跑（网络、执行工具、成形），
//! 每一步经 channel 交回主线程给界面。一次只有一个 run；关掉页面、切到后台照样跑完（`background`）。
//! 读的工具在运行时上照快照做；写的工具带着快照交回主线程（`Up::Write`），照 `margin_write.rs` 落到编辑器和文档库里，
//! 带着刷新过的快照回去。改了什么记在这一轮回复上（`run::changes`），回复下面是「改了 N 篇 · 回滚」；
//! 回滚前要是有哪篇之后又被手动改过，先用系统提示框问一次。
//! 对话记录（连同改动日志）存在空间的 `assistant/chats/`（`run::chat`）。回复可以拷贝。
use std::{cell::{Cell, RefCell}, collections::{HashMap, HashSet}, path::PathBuf, time::{Duration, Instant}};

use fdl_ai::assistants::Assistant;
use fdl_ai::run::{self, Act, Attached, Call, Core, Identity, Kind, Permission, Reply, Resource, Role, Shelf, changes::Changes, chat::{self, Chat, Item}, tools, write};
use fdl_ai::tavern::library::{self, Settings};
use futures::{StreamExt, channel::{mpsc, oneshot}};
use gpui::{prelude::FluentBuilder, *};

use crate::{app::{View, Workspace}, model::{Block, BlockKind}, settings::tr, store::Store, theme::Theme, ui};
use super::{assistant::{self, list, llm::Event, roles}, menu};

#[path = "margin_card.rs"]
pub mod card;
#[path = "margin_write.rs"]
mod host;
#[path = "margin_speak.rs"]
mod speak;

pub const SEND: &str = "margin-send";
const CLOSE: &str = "margin-close";
const NEW: &str = "margin-new";
const DELETE: &str = "margin-delete";
pub const OPEN_HOME: &str = "margin-open-home";
pub const OPEN_DOC: &str = "margin-open-doc";
/// 历史菜单里一段对话一项：标识按位置编号（菜单项的标识是静态的）。
const CHATS: [&str; 12] = ["margin-chat-0", "margin-chat-1", "margin-chat-2", "margin-chat-3", "margin-chat-4", "margin-chat-5", "margin-chat-6", "margin-chat-7", "margin-chat-8", "margin-chat-9", "margin-chat-10", "margin-chat-11"];
/// 资源胶囊的「移除」：按位置编号。最多带这么多样。
const REMOVE: [&str; 8] = ["margin-res-0", "margin-res-1", "margin-res-2", "margin-res-3", "margin-res-4", "margin-res-5", "margin-res-6", "margin-res-7"];
/// 助手菜单里的一项：`<前缀><标识>`。
const WHO: &str = "margin-who:";
const CORE_TAVERN: &str = "margin-core-tavern";
const CORE_AGENT: &str = "margin-core-agent";
const PERM_READ: &str = "margin-perm-read";
const PERM_WRITE: &str = "margin-perm-write";
const UNDO_OK: &str = "margin-undo-ok";
const UNDO_CANCEL: &str = "margin-undo-cancel";
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
    /// 这一轮改了什么（写的工具记的）。
    changes: Changes,
    /// 这一轮画在对话里的图（`imagine` 的记录标识）。
    images: Vec<String>,
}

struct Page {
    chat: Chat,
    /// 卡片上现在带着的资源：下一句话附带这些（发完还留着，接着问同一样东西）。
    resources: Vec<Resource>,
    /// 进来时默认带的（新对话用它；从同一处进来接着上次那段）。
    entry: Vec<Resource>,
    /// 空间的默认助手（进来时读一次）。
    space: Option<String>,
    live: Option<Live>,
    /// 显示用的块，按条目位置记（读出来的对话按 Markdown 成形一次）。
    shaped: HashMap<usize, Vec<Block>>,
    /// 展开了步骤的条目。
    expanded: HashSet<usize>,
    /// 最近的对话（历史菜单），打开历史菜单前算好。
    recent: Vec<(String, String)>,
    /// 展开了改动列表的条目。
    listed: HashSet<usize>,
    /// 要回滚的那条，之后又被手动改过的几篇：先问一次。
    asking: Option<(usize, Vec<String>)>,
}

thread_local! {
    static PAGE: RefCell<Option<Page>> = const { RefCell::new(None) };
    static OPEN: Cell<bool> = const { Cell::new(false) };
    /// 开 / 关的动画从哪一刻起（`None`：第一帧还没画）。
    static MOTION: Cell<Option<(bool, Option<Instant>)>> = const { Cell::new(None) };
    static TURN: Cell<u64> = const { Cell::new(0) };
    static SCROLL: ScrollHandle = ScrollHandle::new();
    static FOLLOW: Cell<bool> = const { Cell::new(true) };
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

/// 这段对话用的助手：自己选的 > 空间的默认助手（`space`）> 内置的。
fn who(c: &Chat, space: Option<&str>) -> Assistant { list::all().resolve(&[c.assistant.as_deref(), space]).clone() }
/// 用哪个核心：这段临时换的，不然是助手自己的。
fn core_of(c: &Chat, a: &Assistant) -> Kind { c.core.unwrap_or(a.core.kind()) }

/// 资源胶囊上的字和 SF Symbol。
fn label(w: &Workspace, r: &Resource) -> (String, &'static str) {
    let title = |id: &str| w.entries.iter().find(|e| e.id == id).map(|e| title_of(&e.title));
    match r {
        Resource::Folder { id } => (title(id).unwrap_or_else(|| tr("文件夹", "Folder").into()), "folder"),
        Resource::Document { id } => (title(id).unwrap_or_else(|| tr("这篇文档", "This document").into()), "doc.text"),
        Resource::Blocks { blocks, .. } => (crate::localized_format!("选中的 {} 块", "{} selected blocks", blocks.len().max(1)), "text.quote"),
    }
}

/// 从主页进来什么都不带，从文件夹页进来带这个文件夹。
pub fn open_space(w: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let entry = match (w.view, w.folder_current.clone()) { (View::Folder, Some(id)) => vec![Resource::Folder { id }], _ => vec![] };
    show(w, entry, window, cx);
}

/// 从文档页进来：带这篇文档。
pub fn open_doc(w: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let Some(id) = w.current.clone().filter(|_| w.view == View::Doc) else { return open_space(w, window, cx) };
    show(w, vec![Resource::Document { id }], window, cx);
}

/// 文档里选中了块（编辑器那边取好块的标识和 Markdown）：带这几块。
pub fn open_blocks(w: &mut Workspace, blocks: Vec<String>, window: &mut Window, cx: &mut Context<Workspace>) {
    let Some(doc) = w.current.clone() else { return };
    let text = w.editor.as_ref().map(|e| picked(&e.read(cx).core.doc, &blocks)).unwrap_or_default();
    show(w, vec![Resource::Blocks { doc, blocks, text }], window, cx);
}

fn show(_: &mut Workspace, entry: Vec<Resource>, _: &mut Window, cx: &mut Context<Workspace>) {
    super::glass::haptic();
    let space = fdl_core::spaces::current().and_then(|s| s.assistant);
    PAGE.with(|p| {
        let mut p = p.borrow_mut();
        // 从同一处进来（选中的块也一样）：接着上次那段；不然开一段新的（正在跑的那段先停下存好）。
        let same = p.as_ref().is_some_and(|p| p.entry == entry);
        if same { if let Some(p) = p.as_mut() { p.space = space; } } else {
            if let Some(p) = p.as_mut() { save_live(p); }
            *p = Some(Page { chat: fresh(space.as_deref()), resources: entry.clone(), entry, space, live: None, shaped: HashMap::new(), expanded: HashSet::new(), recent: vec![], listed: HashSet::new(), asking: None });
        }
    });
    let fresh = PAGE.with(|p| p.borrow().as_ref().is_some_and(|p| !p.chat.started()));
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
    speak::stop();
    if let Some(live) = p.live.take() {
        live.call.abort();
        p.chat.items.push(Item { role: Role::Assistant, text: live.stored, steps: live.steps, at: now(), model: live.model, stopped: true, changes: live.changes, images: live.images, ..Default::default() });
        let _ = chat::save(&dir(), &p.chat);
    }
}

fn close(cx: &mut Context<Workspace>) {
    card::blur();
    speak::stop();
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
    let with = |f: &mut dyn FnMut(&mut Page)| PAGE.with(|p| if let Some(p) = p.borrow_mut().as_mut() { f(p); });
    match id {
        OPEN_HOME => open_space(w, window, cx),
        OPEN_DOC => open_doc(w, window, cx),
        CLOSE => close(cx),
        SEND => if busy() { stop(); cx.notify(); } else { send(w, card::take(), cx) },
        card::MENTION => card::insert_at(),
        NEW => {
            super::glass::haptic();
            with(&mut |p| {
                save_live(p);
                p.chat = fresh(p.space.as_deref());
                p.resources = p.entry.clone();
                (p.shaped, p.expanded, p.listed, p.asking) = (HashMap::new(), HashSet::new(), HashSet::new(), None);
            });
            card::focus();
            cx.notify();
        }
        DELETE => {
            with(&mut |p| {
                save_live(p);
                let _ = chat::delete(&dir(), &p.chat.id);
                p.chat = fresh(p.space.as_deref());
                p.resources = p.entry.clone();
                (p.shaped, p.expanded, p.listed, p.asking) = (HashMap::new(), HashSet::new(), HashSet::new(), None);
            });
            refresh_recent();
            cx.notify();
        }
        _ if let Some(i) = REMOVE.iter().position(|r| *r == id) => {
            super::glass::haptic();
            with(&mut |p| if i < p.resources.len() { p.resources.remove(i); });
            cx.notify();
        }
        _ if let Some(a) = id.strip_prefix(WHO) => {
            with(&mut |p| if !p.chat.started() {
                p.chat.assistant = Some(a.to_owned());
                // 换了助手：核心回到它自己的，权限照它的默认。
                p.chat.core = None;
                p.chat.permission = who(&p.chat, p.space.as_deref()).permission;
            });
            cx.notify();
        }
        CORE_TAVERN | CORE_AGENT => {
            let k = if id == CORE_AGENT { Kind::Agent } else { Kind::Tavern };
            with(&mut |p| if !p.chat.started() { let own = who(&p.chat, p.space.as_deref()).core.kind(); p.chat.core = (k != own).then_some(k); });
            cx.notify();
        }
        UNDO_OK => {
            if let Some(i) = PAGE.with(|p| p.borrow().as_ref().and_then(|p| p.asking.as_ref().map(|a| a.0))) { undo(w, i, window, cx); }
        }
        UNDO_CANCEL => { with(&mut |p| p.asking = None); cx.notify(); }
        PERM_READ | PERM_WRITE => {
            let to = if id == PERM_WRITE { Permission::Write } else { Permission::Read };
            with(&mut |p| { p.chat.permission = to; if p.chat.started() { let _ = chat::save(&dir(), &p.chat); } });
            cx.notify();
        }
        _ if let Some(i) = CHATS.iter().position(|c| *c == id) => {
            let target = PAGE.with(|p| p.borrow().as_ref().and_then(|p| p.recent.get(i).map(|(id, _)| id.clone())));
            if let Some(c) = target.and_then(|id| chat::load(&dir(), &id)) {
                with(&mut |p| {
                    save_live(p);
                    p.resources = c.last_resources().unwrap_or_default().iter().take(REMOVE.len()).cloned().collect();
                    p.chat = c.clone();
                    (p.shaped, p.expanded, p.listed, p.asking) = (HashMap::new(), HashSet::new(), HashSet::new(), None);
                });
                FOLLOW.with(|f| f.set(true));
                cx.notify();
            }
        }
        _ => return false,
    }
    true
}

/// @ 搜索里挑了一样：带上它，拿掉输入框里的 `@…`。
fn attach(r: Resource, cx: &mut Context<Workspace>) {
    super::glass::haptic();
    PAGE.with(|p| if let Some(p) = p.borrow_mut().as_mut() && !p.resources.contains(&r) && p.resources.len() < REMOVE.len() { p.resources.push(r); });
    card::clear_mention();
    cx.notify();
}

/// 素材快照：文档库里每篇的标题、所在、标签、链接和正文（纯文字），文件夹，最近删除，读正文的办法（后台线程上只读地读文件，
/// 编辑器里开着的那篇用编辑器里的），空间的 `SPACE.md`，助手，附带的资源（文档是 Markdown）。
fn shelf(w: &Workspace, resources: &[Resource], a: &Assistant, permission: Permission, cx: &App) -> Shelf {
    let (docs, dirs, trash) = host::catalog(w);
    let reader = w.store.reader();
    let library = run::Library::new(move |id| reader.load(id).ok());
    // 文档：编辑器里开着就用编辑器里的（可能还没存）；已经删了就没有。
    let open = w.editor.as_ref().map(|e| e.read(cx)).and_then(|e| Some((w.current.clone()?, e.core.doc.clone())));
    if let Some((id, doc)) = &open { library.put(id, doc.clone()); }
    let focus = |id: &str, picked: Option<String>| {
        let doc = open.as_ref().filter(|(o, _)| o == id).map(|(_, d)| d.clone()).or_else(|| w.store.load(id).ok())?;
        Some(Attached::Doc(run::Focus { id: id.to_owned(), title: doc.title.text.clone(), markdown: fdl_core::markdown::export(&doc, &Default::default()).text, picked }))
    };
    let attached = resources.iter().filter_map(|r| match r {
        Resource::Folder { id } => {
            let f = w.entries.iter().find(|e| e.id == *id && e.meta.folder.is_some())?;
            Some(Attached::Folder { id: id.clone(), name: f.title.clone(), docs: inside(w, id) })
        }
        Resource::Document { id } => focus(id, None),
        Resource::Blocks { doc, text, .. } => focus(doc, Some(text.clone())),
    }).collect();
    let space = fdl_core::spaces::current().and_then(|s| s.name.clone()).unwrap_or_default();
    let guide = fdl_ai::space_md::read(&Store::root_dir()).unwrap_or_default();
    Shelf {
        space, guide, assistant: Identity { name: a.name.clone(), description: a.description.clone() }, docs, folders: dirs, trash, library,
        attached, permission, now: Some(fdl_core::time::civil(fdl_core::time::now())), ..Default::default()
    }
}

/// 文件夹里（含子文件夹、子文档）的文档，最近改过的在前。
fn inside(w: &Workspace, folder: &str) -> Vec<String> {
    let parent: HashMap<&str, &str> = w.entries.iter().filter_map(|e| Some((e.id.as_str(), e.meta.parent.as_deref()?))).collect();
    let under = |id: &str| { let mut at = parent.get(id).copied(); for _ in 0..64 { match at { Some(p) if p == folder => return true, Some(p) => at = parent.get(p).copied(), None => return false } } false };
    let mut docs: Vec<_> = w.entries.iter().filter(|e| e.meta.folder.is_none() && under(&e.id)).collect();
    docs.sort_by_key(|e| std::cmp::Reverse(e.meta.modified));
    docs.into_iter().map(|e| e.id.clone()).collect()
}

/// 新开一段对话：助手跟着空间的默认，权限照它的默认。
fn fresh(space: Option<&str>) -> Chat {
    let mut c = Chat::new(new_id(), now());
    c.permission = who(&c, space).permission;
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
/// `Write`：一次写的调用交给主线程做，带着快照过去、带着刷新过的快照和结果回来。
enum Up { Thinking, Step(String), Preview { stored: String, blocks: Vec<Block> }, Done { stored: String, blocks: Vec<Block> }, Failed(String), Write { shelf: Shelf, call: Call, reply: oneshot::Sender<(Shelf, String)> } }

fn send(w: &mut Workspace, input: String, cx: &mut Context<Workspace>) {
    let input = input.trim().to_owned();
    if input.is_empty() { return; }
    let Some((c, resources, space)) = PAGE.with(|p| p.borrow().as_ref().map(|p| (p.chat.clone(), p.resources.clone(), p.space.clone()))) else { return };
    let a = who(&c, space.as_deref());
    let Some((base, key, model, effort)) = assistant::text::for_model(a.model.as_ref()) else { return toast(tr("先在 设置 › 模型 里选择模型", "Choose a model in Settings › Models first").into()) };
    let shelf = shelf(w, &resources, &a, c.permission, cx);
    let mut history = c.history();
    // 开场白这次才记进对话：第一次请求里也要有它。
    if c.items.is_empty() && !a.greeting.trim().is_empty() { history.insert(0, run::Turn { role: Role::Assistant, text: a.greeting.trim().to_owned() }); }
    let seed = fdl_core::time::now_ms();
    let core: Box<dyn Core> = match core_of(&c, &a) {
        Kind::Agent => Box::new(run::agent::Agent::new(a.core.rounds())),
        Kind::Tavern => {
            let d = super::assist::dir();
            let preset = match library::load(&d, a.core.preset()) { Ok(p) => p, Err(e) => return toast(e) };
            let pe = Settings::load(&d).persona().clone();
            Box::new(run::tavern::Tavern::new(preset, shelf.assistant.clone(), Identity { name: pe.name, description: pe.description }, model.clone(), seed))
        }
    };
    let turn = TURN.with(|t| { t.set(t.get() + 1); t.get() });
    let gen_id = new_id();
    let (tx, mut rx) = mpsc::unbounded::<Up>();
    let call = assistant::llm::runtime().spawn(drive(core, shelf, input.clone(), history, (base, key, model.clone(), effort), gen_id, tx));
    let ticket = super::background::begin(super::background::Kind::Assistant, "MarginAI".into(), tr("助手正在想", "Assistant thinking"), 2, move || halted(turn));
    PAGE.with(|p| if let Some(p) = p.borrow_mut().as_mut() {
        // 说第一句时开场白才记进对话（之前只是显示）；助手也定下来。
        if !p.chat.started() {
            p.chat.assistant = Some(a.id.clone());
            if p.chat.items.is_empty() && !a.greeting.trim().is_empty() { p.chat.items.push(Item { role: Role::Assistant, text: a.greeting.trim().to_owned(), at: p.chat.created, ..Default::default() }); }
        }
        p.chat.items.push(Item { role: Role::User, text: input, resources, at: now(), ..Default::default() });
        p.live = Some(Live { turn, call: call.abort_handle(), _ticket: ticket, steps: vec![], blocks: vec![], stored: String::new(), model, changes: Changes::default(), images: vec![] });
        let _ = chat::save(&dir(), &p.chat);
    });
    card::blur();
    FOLLOW.with(|f| f.set(true));
    cx.notify();
    cx.spawn(async move |this, cx| {
        while let Some(u) = rx.next().await {
            if let Up::Write { shelf, call, reply } = u { menu::dispatch(move |w, window, cx| write_now(w, window, cx, turn, shelf, call, reply)); continue; }
            let end = matches!(u, Up::Done { .. } | Up::Failed(_));
            let fresh = PAGE.with(|p| p.borrow_mut().as_mut().is_some_and(|p| p.live.as_ref().is_some_and(|l| l.turn == turn) && { apply(p, u); true }));
            if !fresh { return; }
            let _ = this.update(cx, |_, cx| cx.notify());
            if end { refresh_recent(); return; }
        }
    }).detach();
}

/// 在运行时上：照核心的吩咐一步步做，直到说完。
async fn drive(mut core: Box<dyn Core>, mut shelf: Shelf, input: String, history: Vec<run::Turn>, (base, key, model, effort): (String, String, String, Option<String>), id: String, tx: mpsc::UnboundedSender<Up>) {
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
                    let out = if write::is_write(&c.name) {
                        let (reply, back) = oneshot::channel();
                        if !send(Up::Write { shelf: shelf.clone(), call: c.clone(), reply }) { return; }
                        match back.await { Ok((s, out)) => { shelf = s; out } Err(_) => tr("错误：没写成（应用没在前台）。", "Error: not written (the app is not in front).").into() }
                    } else { tools::run(&shelf, &c) };
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
            p.chat.items.push(Item { role: Role::Assistant, text: stored, steps: live.steps, at: now(), model: live.model, changes: live.changes, images: live.images, ..Default::default() });
            p.shaped.insert(p.chat.items.len() - 1, blocks);
            let _ = chat::save(&dir(), &p.chat);
        }
        Up::Write { .. } => {}
        Up::Failed(e) => {
            let live = p.live.take().unwrap();
            live._ticket.fail();
            p.chat.items.push(Item { role: Role::Assistant, text: live.stored, steps: live.steps, at: now(), model: live.model, failed: Some(e), changes: live.changes, images: live.images, ..Default::default() });
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
        p.resources = p.chat.items[at].resources.clone();
        // 出错前已经改了的，记到重问的那一轮上（照样能回滚）。
        let (mut carry, mut images) = (Changes::default(), vec![]);
        for i in &p.chat.items[at..] { if !i.changes.undone { carry.ops.extend(i.changes.ops.iter().cloned()); } images.extend(i.images.iter().cloned()); }
        p.chat.items.truncate(at);
        p.shaped.retain(|k, _| *k < at);
        Some((input, carry, images))
    });
    if let Some((input, carry, images)) = input {
        send(w, input, cx);
        PAGE.with(|p| if let Some(l) = p.borrow_mut().as_mut().and_then(|p| p.live.as_mut()) { (l.changes, l.images) = (carry, images); });
    }
}

/// 主线程上做一次写的调用：还是这一轮、对话还是可编辑才做（权限随时能改），改动记进这一轮。
/// 开始画图的（`generate_image`）等那张画完或失败了再回话，步骤一直停在「画一张图」上。
fn write_now(w: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>, turn: u64, mut shelf: Shelf, call: Call, reply: oneshot::Sender<(Shelf, String)>) {
    let state = PAGE.with(|p| p.borrow_mut().as_mut().and_then(|p| {
        let (permission, look) = (p.chat.permission, look(&who(&p.chat, p.space.as_deref())));
        p.live.as_mut().filter(|l| l.turn == turn).map(|l| (std::mem::take(&mut l.changes), permission, look))
    }));
    let Some((mut log, permission, look)) = state else { let _ = reply.send((shelf, tr("错误：这一轮已经停了，没有改。", "Error: this turn has stopped; nothing was changed.").into())); return };
    shelf.permission = permission;
    // 不借着 `PAGE` 做：文档库、编辑器那边可能回头来问助手页的状态。
    let mut host = host::Ios::new(w, window, cx);
    host.look = look;
    let out = write::run(&mut shelf, &mut host, &mut log, &call);
    let (pending, shown) = (host.pending.take(), std::mem::take(&mut host.shown));
    let out = if host.unbound { format!("{out}\n{}", tr("（这个助手没绑形象，画的不是固定的你，照提示词画的。）", "(This assistant has no bound look, so it was drawn from the prompt alone.)")) } else { out };
    PAGE.with(|p| if let Some(p) = p.borrow_mut().as_mut() {
        match p.live.as_mut().filter(|l| l.turn == turn) {
            Some(l) => { l.changes = log; l.images.extend(shown); }
            // 做的时候这一轮被换下去了：记到它留下的那条上。
            None => if let Some(i) = p.chat.items.iter_mut().rev().find(|i| i.role == Role::Assistant) { i.changes.ops.extend(log.ops); i.images.extend(shown); let _ = chat::save(&dir(), &p.chat); },
        }
    });
    cx.notify();
    let Some(id) = pending else { let _ = reply.send((shelf, out)); return };
    super::imagine::on_done(&id, move |r| {
        let more = match r {
            Ok(()) => tr("图已经画好了。", "The image is done.").to_owned(),
            Err(e) => crate::localized_format!("但没画成：{}。占位还在，用户点它能重试；别再调一次。", "But it failed: {}. The placeholder stays and the user can tap it to retry; do not call again.", e),
        };
        let _ = reply.send((shelf, format!("{out}\n{more}")));
        menu::dispatch(|_, _, cx| cx.notify());
    });
}

/// 助手的形象：图像库里那个人物的名字（`imagine` 照名字找人物）。
fn look(a: &Assistant) -> Option<String> {
    let id = a.character.as_deref()?;
    assistant::prompts::lib().characters.into_iter().find(|c| c.id == id).map(|c| c.name)
}

/// 点了「回滚」：之后有人手动改过就先问一次，不然直接回滚。
fn ask_undo(w: &mut Workspace, i: usize, window: &mut Window, cx: &mut Context<Workspace>) {
    if busy() { return; }
    let Some(changes) = PAGE.with(|p| p.borrow().as_ref().and_then(|p| p.chat.items.get(i).map(|x| x.changes.clone()))) else { return };
    if changes.undone || changes.is_empty() { return; }
    super::glass::haptic();
    let edited = changes.edited(&mut host::Ios::new(w, window, cx));
    if edited.is_empty() { return undo(w, i, window, cx); }
    PAGE.with(|p| if let Some(p) = p.borrow_mut().as_mut() { p.asking = Some((i, edited)); });
    cx.notify();
}

/// 回滚第 `i` 条回复改的东西，记成已回滚。
fn undo(w: &mut Workspace, i: usize, window: &mut Window, cx: &mut Context<Workspace>) {
    let Some(mut changes) = PAGE.with(|p| p.borrow_mut().as_mut().and_then(|p| { p.asking = None; p.chat.items.get(i).map(|x| x.changes.clone()) })) else { return };
    let problems = changes.undo(&mut host::Ios::new(w, window, cx));
    PAGE.with(|p| if let Some(p) = p.borrow_mut().as_mut() && let Some(x) = p.chat.items.get_mut(i) { x.changes = changes; let _ = chat::save(&dir(), &p.chat); });
    if problems.is_empty() { w.show_toast(tr("已回滚", "Reverted"), cx); }
    else { w.show_toast_kind(ui::ToastKind::Error, &crate::localized_format!("有几样没能回滚：{}", "Some changes could not be reverted: {}", problems.join("；")), cx); }
    cx.notify();
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

fn content(p: &Page, w: &Workspace, settled: bool, window: &mut Window, cx: &mut Context<Workspace>) -> AnyElement {
    let ink = super::glass::ink();
    let top_inset = super::insets().top;
    let keyboard = (window.viewport_size().height - super::visible_height(window)).max(px(0.));
    let bottom = keyboard.max(super::insets().bottom) + px(8.);
    // 顶栏：左边历史，中间标题，右边新对话和关闭。玻璃钮在滑到位后才放（`app.rs` 那时才放行玻璃）。
    let history: Vec<menu::Item> = if p.recent.is_empty() { vec![menu::Item::new("margin-none", tr("还没有对话", "No chats yet"), "", 0).disabled()] } else {
        p.recent.iter().enumerate().map(|(i, (id, name))| menu::Item::new(CHATS[i], if name.is_empty() { tr("新对话", "New chat").to_owned() } else { name.clone() }, "", 0).checked(*id == p.chat.id)).collect()
    };
    let history = if p.chat.items.is_empty() { history } else { let mut h = history; h.push(menu::Item::new(DELETE, tr("删除这段对话", "Delete this chat"), "trash", 1).danger()); h };
    let a = who(&p.chat, p.space.as_deref());
    let name = p.chat.name();
    let title = div().flex_1().min_w_0().px(px(8.)).flex().justify_center().child(div().max_w_full().overflow_hidden().whitespace_nowrap().text_ellipsis()
        .text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).text_color(Theme::text()).child(if name.is_empty() { a.name.clone() } else { name }));
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
    // 还没开口：先显示助手的开场白（说第一句时才记进对话）。
    if p.chat.items.is_empty() && !a.greeting.trim().is_empty() {
        list = list.child(render_blocks(&fdl_core::markdown::parse_file(a.greeting.trim(), "", None).blocks));
    }
    // 对话里的图最宽到列表的宽。
    let wide = (f32::from(super::layout().2.width) - 40.).max(120.);
    for (i, item) in p.chat.items.iter().enumerate() {
        list = list.child(match item.role {
            Role::User => user(w, item),
            _ => assistant_item(p, i, item, &a, wide, cx),
        });
    }
    if let Some(live) = &p.live { list = list.child(live_item(live, wide)); }
    if p.live.is_some() { window.request_animation_frame(); }

    let top: Vec<card::Chip> = p.resources.iter().take(REMOVE.len()).enumerate().map(|(i, r)| {
        let (title, glyph) = label(w, r);
        card::Chip { title, glyph, picture: None, menu: vec![menu::Item::new(REMOVE[i], tr("移除", "Remove"), "minus.circle", 0)], off: false }
    }).collect();
    let chips = [who_chip(&p.chat, &a), perm_chip(p.chat.permission, core_of(&p.chat, &a) == Kind::Tavern)];
    // 要回滚的那条之后又被手动改过：系统提示框问一次。
    if let Some((_, titles)) = &p.asking && settled {
        let body = crate::localized_format!("{} 在助手改完之后又被改过，回滚会把这些修改一起撤掉。", "{} changed after the assistant edited it. Reverting also discards those edits.", titles.join("、"));
        super::alert::ask(tr("之后又改过", "Edited since"), &body, tr("仍然回滚", "Revert anyway"), true, None, (UNDO_OK, UNDO_CANCEL));
    }
    let mention = card::mention().map(|q| mention_list(w, p, &q, bottom + px(card::HEIGHT + 8.), cx));
    div().size_full().flex().flex_col().pt(top_inset).pb(keyboard)
        .child(bar)
        .child(list)
        .children(mention.filter(|_| settled))
        .child(div().absolute().left(px(12.)).bottom(bottom).when(settled, |d| d.child(card::slot(top, chips, p.live.is_some()))))
        .into_any_element()
}

/// 助手那颗：头像和名字；菜单里是所有助手和「核心」（酒馆 / Agent）。用户说过话以后都定了（灰着）。
fn who_chip(c: &Chat, a: &Assistant) -> card::Chip {
    let locked = c.started();
    let all = list::all();
    let mut items: Vec<menu::Item> = all.list.iter().map(|x| {
        let it = menu::Item::new(list::key(WHO, &x.id), x.name.clone(), "person.crop.circle", 0).checked(x.id == a.id).picture(roles::avatar_picture(x.avatar.as_deref()));
        if locked && x.id != a.id { it.disabled() } else { it }
    }).collect();
    let k = core_of(c, a);
    let kinds = [Kind::Tavern, Kind::Agent].map(|x| { let it = menu::Item::new(if x == Kind::Agent { CORE_AGENT } else { CORE_TAVERN }, core_name(x), "", 0).checked(x == k); if locked && x != k { it.disabled() } else { it } });
    items.push(menu::Item::new("margin-core", tr("核心", "Core"), "cpu", 1).sub(core_name(k)).kids(kinds.to_vec()));
    card::Chip { title: a.name.clone(), glyph: "person.crop.circle", picture: roles::avatar_picture(a.avatar.as_deref()), menu: items, off: false }
}

/// 权限那颗：只读 / 可编辑。酒馆核心不调工具，改不了文档：灰着。
fn perm_chip(p: Permission, tavern: bool) -> card::Chip {
    let (title, glyph) = match p { Permission::Read => (tr("只读", "Read only"), "eye"), Permission::Write => (tr("可编辑", "Can edit"), "pencil") };
    card::Chip { title: title.into(), glyph, picture: None, menu: vec![
        menu::Item::new(PERM_READ, tr("只读", "Read only"), "eye", 0).checked(p == Permission::Read),
        menu::Item::new(PERM_WRITE, tr("可编辑", "Can edit"), "pencil", 0).checked(p == Permission::Write),
    ], off: tavern }
}

/// @ 搜索：标题里有这些字的文件夹和文档（空着是最近改过的），点一个带上。浮在卡片上面。
fn mention_list(w: &Workspace, p: &Page, q: &str, bottom: Pixels, cx: &mut Context<Workspace>) -> AnyElement {
    let q = q.to_lowercase();
    let res = |e: &crate::store::Entry| if e.meta.folder.is_some() { Resource::Folder { id: e.id.clone() } } else { Resource::Document { id: e.id.clone() } };
    let mut hits: Vec<_> = w.entries.iter().filter(|e| (q.is_empty() || e.title.to_lowercase().contains(&q)) && !p.resources.contains(&res(e))).collect();
    hits.sort_by_key(|e| std::cmp::Reverse(e.meta.modified));
    hits.truncate(5);
    let mut col = ui::popover().id("margin-mention").occlude().absolute().left(px(12.)).right(px(12.)).bottom(bottom).py(px(6.)).flex().flex_col()
        .on_mouse_down(MouseButton::Left, |_, window, cx| { cx.stop_propagation(); window.prevent_default(); });
    if hits.is_empty() {
        col = col.child(div().px(px(16.)).py(px(10.)).text_size(px(15.)).text_color(Theme::text_caption()).child(tr("没有找到", "No matches")));
    }
    for (i, e) in hits.into_iter().enumerate() {
        let (r, folder) = (res(e), e.meta.folder.is_some());
        col = col.child(div().id(("margin-mention-row", i)).h(px(44.)).px(px(16.)).flex().items_center().gap(px(10.)).active(|d| d.bg(Theme::fill_pressed()))
            .child(ui::icon(if folder { "folder" } else { "doc" }, 16., Theme::text_caption()))
            .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(px(16.)).text_color(Theme::text()).child(title_of(&e.title)))
            .on_click(cx.listener(move |_, _, _, cx| attach(r.clone(), cx))));
    }
    col.into_any_element()
}

fn core_name(k: Kind) -> &'static str { match k { Kind::Tavern => tr("酒馆", "Tavern"), Kind::Agent => "Agent" } }

/// 用户的一句话：右边的气泡；附带了资源的话，气泡上面一行灰字写着带了什么。
fn user(w: &Workspace, item: &Item) -> AnyElement {
    let with = (!item.resources.is_empty()).then(|| item.resources.iter().map(|r| label(w, r).0).collect::<Vec<_>>().join(" · "));
    div().w_full().flex().flex_col().items_end().gap(px(4.))
        .children(with.map(|t| div().max_w(relative(0.82)).overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(px(13.)).text_color(Theme::text_caption()).child(format!("@ {t}"))))
        .child(div().max_w(relative(0.82)).px(px(14.)).py(px(9.)).rounded(px(20.)).bg(super::search_fill())
            .text_size(px(16.)).line_height(px(23.)).text_color(Theme::text()).child(item.text.clone())).into_any_element()
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

/// 这一轮改了文档：「改了 N 篇 · 回滚」，点前半截逐条列出改了什么；回滚过的是「已回滚」。助手在说的时候先不让回滚。
fn changes_row(i: usize, c: &Changes, open: bool, busy: bool, cx: &mut Context<Workspace>) -> AnyElement {
    let head = div().id(("margin-changes", i)).flex().items_center().gap(px(4.))
        .child(ui::icon(if open { "chevron-down" } else { "chevron-right" }, 12., Theme::text_caption()))
        .child(crate::localized_format!("改了 {} 篇", "Changed {}", c.count()))
        .on_click(cx.listener(move |_, _, _, cx| { PAGE.with(|p| if let Some(p) = p.borrow_mut().as_mut() { if !p.listed.remove(&i) { p.listed.insert(i); } }); cx.notify(); }));
    let action = if c.undone { div().child(tr("已回滚", "Reverted")).into_any_element() } else {
        div().id(("margin-undo", i)).text_color(if busy { Theme::text_caption() } else { Theme::primary() }).child(tr("回滚", "Revert"))
            .when(!busy, |d| d.on_click(cx.listener(move |w, _, window, cx| ask_undo(w, i, window, cx)))).into_any_element()
    };
    div().flex().flex_col().gap(px(4.)).text_size(px(14.)).line_height(px(20.)).text_color(Theme::text_caption())
        .child(div().flex().items_center().gap(px(6.)).child(head).child("·").child(action))
        .when(open, |d| d.children(c.lines().into_iter().map(|l| div().pl(px(16.)).child(l))))
        .into_any_element()
}

/// 助手的一条：步骤、正文、画在对话里的图、改动，底下拷贝和朗读。
fn assistant_item(p: &Page, i: usize, item: &Item, a: &Assistant, wide: f32, cx: &mut Context<Workspace>) -> AnyElement {
    let blocks = p.shaped.get(&i).cloned().unwrap_or_else(|| if item.text.trim().is_empty() { vec![] } else { fdl_core::markdown::parse_file(&item.text, "", None).blocks });
    let mut col = div().w_full().flex().flex_col().gap(px(10.));
    if !item.steps.is_empty() { col = col.child(steps_row(i, &item.steps, p.expanded.contains(&i), cx)); }
    if !blocks.is_empty() { col = col.child(render_blocks(&blocks)); }
    col = col.children(item.images.iter().map(|id| super::imagine::chat_picture(id, wide)));
    if !item.changes.is_empty() { col = col.child(changes_row(i, &item.changes, p.listed.contains(&i), p.live.is_some(), cx)); }
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
        let speak = speak::button(i, speak::key(&p.chat.id, i), text.clone(), a.voice.clone(), cx);
        col = col.child(div().flex().ml(px(-6.)).child(div().id(("margin-copy", i)).size(px(32.)).flex().items_center().justify_center().rounded(px(8.)).active(|s| s.bg(Theme::fill_pressed()))
            .child(ui::icon("copy", 16., Theme::text_caption()))
            .on_click(cx.listener(move |w, _, _, cx| { super::glass::haptic(); cx.write_to_clipboard(ClipboardItem::new_string(text.clone())); w.show_toast(tr("已拷贝", "Copied"), cx); })))
            .child(speak));
    }
    col.into_any_element()
}

/// 正在跑的那条：步骤、写出来的，还没出字时一行在做什么（淡淡地一明一暗）。
fn live_item(live: &Live, wide: f32) -> AnyElement {
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
    // 这一轮已经画在对话里的（`generate_image` 等它画完才往下，所以放在最后）。
    col.children(live.images.iter().map(|id| super::imagine::chat_picture(id, wide))).into_any_element()
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
