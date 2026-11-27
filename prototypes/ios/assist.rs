//! 助手：照酒馆核心（`fdl_ai::tavern`）的预设，让文本模型接着文档往下写。
//! 入口两个：「/」菜单的第一项（锚点是打「/」的那一行），选中块底部工具条里「配音」后面那一格（锚点是选中的最后一块）。
//! 一张原生玻璃卡片（`assist_card.rs`）写要求、切预设、助手和人设；发出后卡片收起，底部一块玻璃写着在想还是在写，
//! 写出来的先作为预览排在锚点下面（和正文同一套排版，没进文档），写完了玻璃换成「插入 / 重写 / 丢弃」。
//! 插入经过 `Core::insert_generated`，一步撤销；同时在文档库的 `assets/gen/` 记下这次的要求和原文（还原历史用）。
//!
//! 一次只有一个助手。它记着是哪篇文档：换篇文档再回来，预览照样在（锚点找不到了就排在文末）；切到后台也接着写（`background`）。
//! 网络和成形都在 `llm` 的运行时上：每收到一段，对「已经稳了的前缀」成形一次，交回主线程；主线程按字数匀速放出来
//! （每秒 30 字，积压的 0.4 秒内追平），新出现的块淡入。
use std::{cell::{Cell, RefCell}, path::PathBuf, time::{Duration, Instant}};

use fdl_ai::tavern::{assemble::{Identity, Request, assemble}, history, library::{self, Settings}, macros::{Env, Macros}, record::{self, Record, hash}, shape::Shaper};
use futures::{StreamExt, channel::mpsc};
use gpui::{prelude::FluentBuilder, *};

use crate::{app::Workspace, editor::view::Editor, model::{Block, BlockId}, settings::tr, store::Store, theme::Theme, ui};
use super::{assistant::{self, llm::Event}, menu};

#[path = "assist_card.rs"]
pub mod card;

pub const SEND: &str = "assist-send";
pub const CLOSE: &str = "assist-close";
const STOP: &str = "assist-stop";
const INSERT: &str = "assist-insert";
const AGAIN: &str = "assist-again";
const DISCARD: &str = "assist-discard";

#[derive(Clone, PartialEq)]
enum Phase { Composing, Thinking, Writing, Done, Failed(String) }

/// 正在进行的一次：网络那头的任务和后台任务的那一份，丢掉就停。
struct Live { ticket: super::background::Ticket, call: tokio::task::AbortHandle }
impl Drop for Live { fn drop(&mut self) { self.call.abort(); } }

struct Job {
    /// 哪篇文档（`Workspace::current`）。
    doc: Option<String>,
    anchor: Option<BlockId>,
    phase: Phase,
    /// 这次的要求（输入框里的原样），重写时再发一遍。
    input: String,
    gen_id: String,
    turn: u64,
    /// 预览：已经成形的块（块的 id 按位置沿用，淡入只在新出现时放一次）和它们一共多少字。
    blocks: Vec<Block>,
    total: usize,
    shown: f32,
    tick: Option<Instant>,
    /// 写完时记下的：存下来的原文、要求（跑过正则和宏）、预设名、参数。
    record: Option<Record>,
    live: Option<Live>,
    since: Instant,
    /// 跟着写（`follow`）：还跟不跟，和上一帧看到的列表位置（对不上就是手指动过）。
    follow: bool,
    seen: Option<ListOffset>,
    pushed: Option<Instant>,
}

/// 丢弃、重写、失败时撤下来的预览：不进文档，原处淡掉、让出地方，然后就没了。
struct Ghost { doc: Option<String>, anchor: Option<BlockId>, blocks: Vec<Block>, at: Instant }
/// 先淡出，再收起高度（两段叠着一点）。
const GHOST_FADE: f32 = 0.18;
const GHOST_HOLD: f32 = 0.1;
const GHOST_FOLD: f32 = 0.3;
/// 还没走完的那份预览这一帧的样子：不透明度、剩多高（0..1）。
#[derive(Clone, Copy)]
pub struct Fading { pub opacity: f32, pub height: f32 }

thread_local! {
    static JOB: RefCell<Option<Job>> = const { RefCell::new(None) };
    static GHOST: RefCell<Option<Ghost>> = const { RefCell::new(None) };
    static TURN: Cell<u64> = const { Cell::new(0) };
    /// 这一帧在看的文档（`frame` 每帧记下）。
    static SHOWING: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// 文档库里助手的素材：预设、助手和人设、上一次的要求。
pub fn dir() -> PathBuf { Store::root_dir().join("tavern") }

fn toast(text: String) { menu::dispatch(move |w, _, cx| w.show_toast_kind(ui::ToastKind::Error, &text, cx)); }

/// 工作区每帧调用：现在看的是哪篇文档。
pub fn frame(doc: Option<String>) { SHOWING.with(|s| *s.borrow_mut() = doc); }

/// 这一帧在看的文档。
pub fn showing() -> Option<String> { SHOWING.with(|s| s.borrow().clone()) }

fn here(job: &Job) -> bool { SHOWING.with(|s| *s.borrow() == job.doc) }

/// 助手在这篇文档里露着面（卡片、预览或底部那块玻璃）：底部的铅笔、块工具条让开。
pub fn active() -> bool { JOB.with(|j| j.borrow().as_ref().is_some_and(here)) || super::imagine::composing() }
pub fn composing() -> bool { JOB.with(|j| j.borrow().as_ref().is_some_and(|j| here(j) && j.phase == Phase::Composing)) }

/// 打开卡片。已经有一次在写（别的文档里也算）时不开，提示一下。
pub fn open(ed: &mut Editor, anchor: Option<BlockId>, window: &mut Window, cx: &mut Context<Editor>) {
    let busy = JOB.with(|j| j.borrow().as_ref().is_some_and(|j| matches!(j.phase, Phase::Thinking | Phase::Writing)));
    if busy { return toast(tr("助手还在写上一段", "The assistant is still writing").into()); }
    let doc = SHOWING.with(|s| s.borrow().clone());
    JOB.with(|j| *j.borrow_mut() = Some(Job { doc, anchor, phase: Phase::Composing, input: String::new(), gen_id: String::new(), turn: 0, blocks: vec![], total: 0, shown: 0., tick: None, record: None, live: None, since: Instant::now(), follow: true, seen: None, pushed: None }));
    // 整个退出选块（不只清掉选中：横滑、块菜单这些状态一起收）。
    ed.finish_block_selection(window, cx);
    // 键盘交给卡片里的输入框：编辑器先放开焦点，键盘上方的工具条跟着收起。
    window.blur(cx);
    cx.notify();
}

/// 工作区收到的动作（卡片和底部玻璃上的按钮）。
pub fn menu_action(id: &str, w: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) -> bool {
    if !id.starts_with("assist-") { return false; }
    // 栏刚变形（停止的位置换成了别的按钮）时的点击不算：防止想点停止却点到了别的。
    if matches!(id, INSERT | AGAIN | DISCARD) && JOB.with(|j| j.borrow().as_ref().is_some_and(|j| j.since.elapsed() < MORPH + Duration::from_millis(150))) { return true; }
    let Some(editor) = w.editor.clone() else { return true };
    let barred = JOB.with(|j| j.borrow().as_ref().is_some_and(|j| j.phase != Phase::Composing));
    match id {
        SEND => {
            let input = card::text();
            if input.trim().is_empty() { return true; }
            let mut s = Settings::load(&dir());
            s.last = input.clone();
            if let Err(e) = s.save(&dir()) { log::warn!("Could not save assistant settings: {e}"); }
            editor.update(cx, |ed, cx| start(ed, input, cx));
        }
        CLOSE | DISCARD => { if let Some(mut job) = JOB.with(|j| j.borrow_mut().take()) { ghost(&mut job); } editor.update(cx, |_, cx| cx.notify()); }
        STOP => {
            // 停下（丢掉 live 就断开请求）：写出来的留着，可以插入；什么都还没写就整个收起。字还在放时点的，直接放完。
            JOB.with(|j| {
                let mut slot = j.borrow_mut();
                let Some(job) = slot.as_mut() else { return };
                job.live = None;
                if job.blocks.is_empty() { *slot = None; } else { job.phase = Phase::Done; job.shown = job.total as f32; job.since = Instant::now(); }
            });
            editor.update(cx, |_, cx| cx.notify());
        }
        AGAIN => {
            let input = JOB.with(|j| j.borrow().as_ref().map(|j| j.input.clone()));
            if let Some(input) = input { editor.update(cx, |ed, cx| start(ed, input, cx)); }
        }
        INSERT => editor.update(cx, |ed, cx| insert(ed, window, cx)),
        _ => {}
    }
    if barred && JOB.with(|j| j.borrow().is_none()) { LEFT.with(|l| l.set(Some(Instant::now()))); }
    true
}

/// 主模型（能力优先），没选就用快速模型。
fn model() -> Option<(String, String, String, Option<String>)> { assistant::text::main() }

/// 拼好请求、发出去。
fn start(ed: &mut Editor, input: String, cx: &mut Context<Editor>) {
    let Some((base, key, model, effort)) = model() else { return toast(tr("先在 设置 › 文本 里选择模型", "Choose a model in Settings › Text first").into()) };
    let anchor = JOB.with(|j| j.borrow().as_ref().and_then(|j| j.anchor)).filter(|a| ed.core.path(*a).is_some());
    let dir = dir();
    let s = Settings::load(&dir);
    // 选中的预设还没从 iCloud 下载好、或者不见了：说出来，不拿「默认」顶上。
    let preset = match library::load(&dir, &s.preset) { Ok(p) => p, Err(e) => return toast(e) };
    let preset_name = if s.preset.is_empty() { String::new() } else { library::display_name(&preset, &s.preset) };
    let (char, persona) = (s.char().clone(), s.persona().clone());
    let turns = history::turns(&ed.core.doc, anchor, &record::dir());
    // `civil` 自己按本地时区换算。
    let now = fdl_core::time::civil(fdl_core::time::now());
    let seed = fdl_core::time::now_ms();
    let a = assemble(&Request {
        preset: &preset, char: Identity { name: char.name.clone(), description: char.description.clone() },
        persona: Identity { name: persona.name.clone(), description: persona.description.clone() },
        history: turns, input: input.clone(), model: model.clone(), model_context: None, extended_params: false, prefill: false, now: Some(now), seed,
    });
    for n in &a.notes { log::info!("Assistant: {n}"); }
    let params = a.params.body(false);
    let mut body = serde_json::json!({ "model": model, "messages": a.messages.iter().map(|m| m.to_json()).collect::<Vec<_>>() });
    for (k, v) in &params { body[k] = v.clone(); }
    if let Some(e) = effort && body.get("reasoning_effort").is_none() { body["reasoning_effort"] = e.into(); }
    let gen_id = uuid::Uuid::new_v4().simple().to_string()[..16].to_owned();
    let turn = TURN.with(|t| { t.set(t.get() + 1); t.get() });
    let record = Record { id: gen_id.clone(), request: a.request.clone(), preset: preset_name, char: char.name.clone(), persona: persona.name.clone(), model: model.clone(), params, ..Default::default() };

    // 网络和成形在 llm 的运行时上，结果经 channel 回到主线程。
    let (tx, mut rx) = mpsc::unbounded::<Update>();
    let mut macros = Macros::new(Env { user: persona.name.clone(), char: char.name.clone(), model: model.clone(), seed, ..Default::default() });
    let shaper = Shaper::new(&preset, &mut macros);
    let (tag, stream) = (gen_id.clone(), assistant::llm::complete(base, key, body));
    let call = assistant::llm::runtime().spawn(async move { pump(stream, shaper, macros, tag, tx).await });
    let ticket = super::background::begin(super::background::Kind::Assistant, char.name.clone(), tr("助手正在想", "Assistant thinking"), 2, move || halted(turn));
    JOB.with(|j| {
        let mut slot = j.borrow_mut();
        let Some(job) = slot.as_mut() else { return };
        ghost(job);
        *job = Job { doc: job.doc.clone(), anchor, phase: Phase::Thinking, input, gen_id, turn, blocks: vec![], total: 0, shown: 0., tick: None, record: Some(record), live: Some(Live { ticket, call: call.abort_handle() }), since: Instant::now(), follow: true, seen: None, pushed: None };
    });
    cx.notify();
    cx.spawn(async move |_, _| {
        while let Some(u) = rx.next().await {
            let fresh = JOB.with(|j| j.borrow().as_ref().is_some_and(|j| j.turn == turn && j.live.is_some()));
            if !fresh { return; }
            let end = matches!(u, Update::Done { .. } | Update::Failed(_));
            JOB.with(|j| if let Some(job) = j.borrow_mut().as_mut() { apply(job, u); });
            refresh();
            if end { return; }
        }
    }).detach();
}

/// 网络那头交回来的。
enum Update {
    Thinking,
    /// 稳了的前缀：存下来的样子和成形出来的块。中途停下时插入的就是它。
    Preview { stored: String, blocks: Vec<Block> },
    /// 写完了：存下来的原文和整段重新成形的块。
    Done { stored: String, blocks: Vec<Block> },
    Failed(String),
}

/// 在运行时上：收流、成形。只在稳了的前缀变长时重新成形；结束时整段再成形一次（预览那次可能少了收尾的一点）。
async fn pump(mut stream: mpsc::UnboundedReceiver<Event>, shaper: Shaper, mut macros: Macros, gen_id: String, tx: mpsc::UnboundedSender<Update>) {
    let (mut raw, mut settled) = (String::new(), 0);
    while let Some(e) = stream.next().await {
        let ok = match e {
            Event::Thinking => tx.unbounded_send(Update::Thinking).is_ok(),
            Event::Calls(_) => true,
            Event::Text(t) => {
                raw.push_str(&t);
                let cut = shaper.settled(&raw);
                if cut <= settled { continue; }
                settled = cut;
                // 预览用宏的副本：`{{random}}` 这些每次一样，变量不会越跑越多。
                let stored = shaper.store(&raw[..cut], &mut macros.clone());
                let blocks = shaper.shape(&stored, &gen_id, &mut macros.clone()).blocks;
                tx.unbounded_send(Update::Preview { stored, blocks }).is_ok()
            }
            Event::Done => {
                let stored = shaper.store(&raw, &mut macros);
                let blocks = shaper.shape(&stored, &gen_id, &mut macros).blocks;
                let _ = tx.unbounded_send(if blocks.is_empty() { Update::Failed(tr("模型没有写出内容", "The model returned nothing").into()) } else { Update::Done { stored, blocks } });
                return;
            }
            Event::Failed(e) => { let _ = tx.unbounded_send(Update::Failed(e)); return; }
        };
        if !ok { return; }
    }
}

/// 块里一共多少字（含子块）。
fn chars(b: &Block) -> usize { b.text.text.chars().count() + b.children.iter().map(chars).sum::<usize>() }

fn apply(job: &mut Job, u: Update) {
    let adopt = |job: &mut Job, mut blocks: Vec<Block>| {
        // 同一位置的块沿用旧的 id：预览重画时不当成新块再淡入一次。
        for (new, old) in blocks.iter_mut().zip(&job.blocks) { new.id = old.id; }
        job.total = blocks.iter().map(chars).sum();
        job.shown = job.shown.min(job.total as f32);
        job.blocks = blocks;
    };
    match u {
        Update::Thinking => if job.phase != Phase::Writing { job.phase = Phase::Thinking; },
        Update::Preview { stored, blocks } => {
            if let Some(r) = job.record.as_mut() { r.raw = stored; }
            if job.phase == Phase::Thinking {
                job.phase = Phase::Writing;
                job.since = Instant::now();
                if let Some(l) = &job.live { l.ticket.advance(tr("助手正在写", "Assistant writing")); }
            }
            adopt(job, blocks);
        }
        Update::Done { stored, blocks } => {
            adopt(job, blocks);
            if let Some(r) = job.record.as_mut() { r.raw = stored; }
            job.phase = Phase::Done;
            job.since = Instant::now();
            job.live = None;
        }
        Update::Failed(e) => {
            log::warn!("Assistant failed: {e}");
            if let Some(l) = &job.live { l.ticket.fail(); }
            job.live = None;
            // 写了一半的不留：失败栏只给重试和关闭，留着一段插不进去的预览只会让人困惑。想留一半的，在写的时候点停止。
            ghost(job);
            job.phase = Phase::Failed(e);
            job.since = Instant::now();
        }
    }
}

/// 后台任务被系统停掉：这次作废。
fn halted(turn: u64) {
    JOB.with(|j| if let Some(job) = j.borrow_mut().as_mut().filter(|j| j.turn == turn && j.live.is_some()) {
        job.live = None;
        ghost(job);
        job.phase = Phase::Failed(tr("后台生成被停止了", "Background generation was stopped").into());
    });
    refresh();
}

fn refresh() {
    menu::dispatch(|w, _, cx| if let Some(e) = w.editor.clone() { e.update(cx, |_, cx| cx.notify()); });
}

/// 插入：写完的块放到锚点后面（锚点是空行就替换它），记下这次生成。
fn insert(ed: &mut Editor, window: &mut Window, cx: &mut Context<Editor>) {
    let Some(job) = JOB.with(|j| j.borrow_mut().take()) else { return };
    if job.blocks.is_empty() { return; }
    let anchor = job.anchor.filter(|a| ed.core.path(*a).is_some());
    let mut blocks = job.blocks;
    // 预览里的 id 可能和文档里的重了（沿用来沿用去）：换一套新的。
    for b in &mut blocks { *b = b.fresh_ids(); }
    ed.core.insert_generated(anchor, blocks);
    ed.changed(window, cx);
    if let Some(mut r) = job.record {
        r.hash = hash(&ed.core.doc.gen_text(&job.gen_id));
        r.created = (fdl_core::time::now()) as i64;
        if let Err(e) = r.save(&record::dir()) { log::warn!("Could not save the generation record: {e}"); }
    }
}

// ---- 每帧

/// 预览不要了：已经放出来的那些交给 `GHOST` 淡掉。
fn ghost(job: &mut Job) {
    let blocks = cut(&std::mem::take(&mut job.blocks), job.shown, false);
    if !blocks.is_empty() { GHOST.with(|g| *g.borrow_mut() = Some(Ghost { doc: job.doc.clone(), anchor: job.anchor, blocks, at: Instant::now() })); }
}

/// 匀速放字：每秒 30 字，积压的 0.4 秒内追平。
fn advance(job: &mut Job) -> bool {
    let now = Instant::now();
    let dt = job.tick.map_or(0., |t| now.duration_since(t).as_secs_f32()).min(0.1);
    job.tick = Some(now);
    let left = job.total as f32 - job.shown;
    if left <= 0. { return false; }
    job.shown = (job.shown + dt * (30. + left / 0.4)).min(job.total as f32);
    if job.phase == Phase::Done && job.shown >= job.total as f32 { job.since = now; }
    true
}

/// 放出来的前 `shown` 个字。`wet`：最后几个字是灰的，像墨还没干。
fn cut(blocks: &[Block], shown: f32, wet: bool) -> Vec<Block> {
    let mut budget = shown.floor() as usize;
    let mut out = vec![];
    for b in blocks {
        if budget == 0 { break; }
        let n = chars(b);
        if n <= budget { out.push(b.clone()); budget -= n; continue; }
        // 写到一半的这块：只放出前面的字，子块不放。
        let mut part = b.clone();
        let at = |n: usize| part.text.text.char_indices().nth(n).map_or(part.text.text.len(), |(i, _)| i);
        let (end, from) = (at(budget), at(budget.saturating_sub(3)));
        part.text = part.text.slice(0..end);
        if wet { part.text.update_marks(from..end, |m| m.text_color = Some(crate::model::ColorName("gray".into()))); }
        part.children.clear();
        out.push(part);
        break;
    }
    out
}

/// 预览：锚点下面（锚点没了就在最后一块下面）排出已经放出来的字。`last` 是这块是不是文档的最后一块。
/// 第二个是要不要接着出帧，第三个有就是撤下来正在淡掉的那份。
pub fn preview(id: BlockId, last: bool, exists: impl Fn(BlockId) -> bool) -> Option<(Vec<Block>, bool, Option<Fading>)> {
    let below = |anchor: Option<BlockId>| match anchor { Some(a) if exists(a) => a == id, _ => last };
    JOB.with(|j| {
        let mut slot = j.borrow_mut();
        let job = slot.as_mut().filter(|j| here(j) && !matches!(j.phase, Phase::Composing) && !j.blocks.is_empty())?;
        // 新的一份来了，旧的不再占地方。
        GHOST.with(|g| *g.borrow_mut() = None);
        if !below(job.anchor) { return None; }
        let moving = advance(job);
        Some((cut(&job.blocks, job.shown, true), moving || matches!(job.phase, Phase::Thinking | Phase::Writing), None))
    }).or_else(|| GHOST.with(|g| {
        let mut slot = g.borrow_mut();
        let t = slot.as_ref()?.at.elapsed().as_secs_f32();
        if t >= GHOST_HOLD + GHOST_FOLD { *slot = None; return None; }
        let ghost = slot.as_ref().filter(|g| SHOWING.with(|s| *s.borrow() == g.doc) && below(g.anchor))?;
        let fold = ease_out_quint()(((t - GHOST_HOLD) / GHOST_FOLD).clamp(0., 1.));
        Some((ghost.blocks.clone(), true, Some(Fading { opacity: (1. - t / GHOST_FADE).max(0.), height: 1. - fold })))
    }))
}

/// 跟着写：预览的底边落到底部那块玻璃后面，就把正文往上推，每 1/60 秒补上差距的四分之一（新块冒出来时不跳；按时间算，120Hz 下一样快）。
/// 手指一动（列表不在上一帧留下的位置）就不跟了；滚回来、预览的底边露出来，又接着跟。`top` 是列表现在的位置，
/// 返回要推多少；推完用 `pushed` 记下推到了哪里。
pub fn follow(bottom: Pixels, top: ListOffset) -> Option<Pixels> {
    JOB.with(|j| {
        let mut slot = j.borrow_mut();
        let job = slot.as_mut().filter(|j| here(j))?;
        if job.seen.is_some_and(|t| t.item_ix != top.item_ix || t.offset_in_item != top.offset_in_item) { job.follow = false; }
        job.seen = Some(top);
        // 玻璃高 70（58 加上下各 6），再空出 16。
        let limit = super::sheet_point().y - px(70. + 16.);
        if bottom <= limit { (job.follow, job.pushed) = (true, None); return None; }
        let now = Instant::now();
        let dt = job.pushed.map_or(1. / 60., |t| now.duration_since(t).as_secs_f32()).min(1. / 20.) * 60.;
        job.pushed = Some(now);
        let gap = bottom - limit;
        job.follow.then(|| (gap * (1. - 0.75f32.powf(dt))).max(px(dt)).min(gap))
    })
}
pub fn pushed(top: ListOffset) { JOB.with(|j| if let Some(job) = j.borrow_mut().as_mut() { job.seen = Some(top); }); }

/// 底部那块玻璃：在想 / 在写（带停止），写完（插入、重写、丢弃），失败（重试、关闭）。卡片打开时是卡片。
pub fn overlay() -> Option<AnyElement> {
    if leaving() { refresh(); }
    let (phase, since, revealing, shown) = JOB.with(|j| j.borrow_mut().as_mut().filter(|j| here(j)).map(|j| {
        // 预览那块可能不在屏幕上，放字也在这里推一下。
        let revealing = j.phase == Phase::Done && advance(j);
        (j.phase.clone(), j.since, revealing, j.shown)
    })).or_else(|| { BAR.with(|b| b.set(None)); None })?;
    if phase == Phase::Composing { BAR.with(|b| b.set(None)); return Some(card::slot()); }
    // 网络先写完、字还在放：栏仍是「正在写」，放完再变。
    let phase = if revealing { Phase::Writing } else { phase };
    let ink = ink(&phase, since, shown);
    Some(bar(phase, revealing, ink))
}

/// 底部那块玻璃现在的样子：宽、高、圆角，和底边在窗口里的位置。四个数各是一根弹簧，目标换了就从现在的样子接着走：
/// 输入卡收成胶囊、胶囊长成整条、点「重写」又收回来，都是同一块玻璃。
#[derive(Clone, Copy)]
struct Glass { x: [f32; 4], v: [f32; 4] }
/// 收起之后这么久，底下原来的按钮才回来：先让这块淡掉，不叠着。
const LEAVE: Duration = Duration::from_millis(140);
thread_local! { static LEFT: Cell<Option<Instant>> = const { Cell::new(None) }; }
pub fn leaving() -> bool { LEFT.with(|l| l.get()).is_some_and(|t| t.elapsed() < LEAVE) }
/// 刚让完：回来的按钮淡入，不是一下出现。
pub fn just_left() -> bool { LEFT.with(|l| l.get()).is_some_and(|t| t.elapsed() < LEAVE * 3) }

thread_local! { static BAR: Cell<Option<(Glass, Instant)>> = const { Cell::new(None) }; }
/// 各根弹簧的阻尼比和快慢。宽的略过头再回来；底边不过头，跟在落下去的键盘后面。
const SPRINGS: [(f32, f32); 4] = [(0.8, 11.), (0.9, 14.), (1., 14.), (1., 16.)];

/// 往目标走一帧，返回这一帧的样子和还在不在动。
fn glass_to(goal: [f32; 4]) -> ([f32; 4], bool) {
    let now = Instant::now();
    let (held, waiting) = (BAR.with(|b| b.get()), card::waiting());
    // 刚从卡片接过来的，从卡片的样子开始；写完停着不出帧，隔了一阵再动，也从停着的样子接着走，只是那段时间不算。
    let (mut g, idle) = match (card::hand_over(), held) {
        (Some(card), _) => (Glass { x: card, v: [0.; 4] }, f32::MAX),
        (None, Some((g, t))) => (g, now.duration_since(t).as_secs_f32()),
        (None, None) => (Glass { x: goal, v: [0.; 4] }, f32::MAX),
    };
    let mut dt = if waiting || idle > 0.3 { 0. } else { idle.min(0.05) };
    while dt > 0. {
        let h = dt.min(1. / 240.);
        for (i, (z, w)) in SPRINGS.into_iter().enumerate() {
            g.v[i] += (w * w * (goal[i] - g.x[i]) - 2. * z * w * g.v[i]) * h;
            g.x[i] += g.v[i] * h;
        }
        dt -= h;
    }
    let moving = (0..4).any(|i| (goal[i] - g.x[i]).abs() > 0.1 || g.v[i].abs() > 2.);
    if !moving { g = Glass { x: goal, v: [0.; 4] }; }
    BAR.with(|b| b.set(Some((g, now))));
    (g.x, moving)
}

/// 这一格点了交给工作区（和原生按钮走同一条路）。
fn tap(id: &'static str) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static { move |_, _, _| menu::chosen(id) }

fn bar(phase: Phase, revealing: bool, ink: Ink) -> AnyElement {
    let glass = super::glass::supported();
    let (compact, _, viewport) = super::layout();
    let full = if compact { viewport.width - px(super::SHEET_INSET * 2.) } else { px(420.) };
    let pill = px(236.);
    let wide = matches!(phase, Phase::Done | Phase::Failed(_));
    // 一块玻璃变形：从卡片收成胶囊，胶囊长成整条，或反过来。底边另外走，内容跟着整块一起。
    let floor = f32::from(super::sheet_point().y);
    let ([w, h, r, bottom], morphing) = glass_to([f32::from(if wide { full } else { pill }), 70., 24., floor]);
    let tile = |id: &'static str, ic: &'static str, label: &'static str, tint: Hsla| {
        div().id(id).flex_1().min_w_0().h(px(58.)).flex().flex_col().items_center().justify_center().gap(px(3.)).rounded(px(16.)).relative()
            .active(|s| s.bg(Theme::fill_pressed()))
            .when(glass, |d| d.child(super::glass::tile(id, (ic, 22., tint), (label.into(), tint), None)))
            .when(!glass, |d| d.child(crate::editor_ui::icon(ic, 22., tint)).child(div().text_size(px(11.)).line_height(px(14.)).text_color(tint).whitespace_nowrap().child(label)))
            .on_mouse_down(MouseButton::Left, move |_, window, cx| { cx.stop_propagation(); window.prevent_default(); if glass { super::glass::pulse(id); } })
            .on_click(tap(id))
            .with_animation(id, Animation::new(Duration::from_millis(260)).with_easing(ease_out_quint()), |d, t| d.opacity(t))
    };
    let mut row = div().id("assist-bar").occlude().relative().top(px(bottom - floor)).w(px(w)).h(px(h)).p(px(6.)).flex().items_center().gap(px(4.)).rounded(px(r))
        .when(!glass, |d| d.bg(crate::theme::raised(Theme::bg_body())).shadow(crate::theme::shadow_menu()))
        .when(glass, |d| d.child(super::glass::panel("assist-bar-glass", r)))
        // 排在玻璃后面：玻璃头一回建好时盖在卡片上面，卡片要再提上来。
        .child(canvas(move |b, _, _| card::ride(b), |_, _, _, _| {}).absolute().inset_0())
        .on_mouse_down(MouseButton::Left, |_, window, cx| { cx.stop_propagation(); window.prevent_default(); });
    let ink_color = Theme::text();
    row = match &phase {
        Phase::Thinking | Phase::Writing => {
            let label = if phase == Phase::Thinking { tr("正在想", "Thinking") } else { tr("正在写", "Writing") };
            // 左边是标记（会动的 ¶，见 `Ink`）和一句状态，右边是停止。
            let shape = ink.shape(24., mark());
            row.child(div().flex_1().min_w_0().h(px(58.)).flex().items_center().gap(px(10.)).relative()
                    .when(glass, |d| d.child(super::glass::shape_row("assist-status", shape.clone(), (label.into(), ink_color))))
                    .when(!glass, |d| d.pl(px(8.)).child(paint(shape)).child(div().text_size(px(16.)).text_color(ink_color).child(label))))
                .child(div().flex_none().w(px(72.)).child(tile(STOP, "stop", tr("停止", "Stop"), ink_color)))
        }
        Phase::Done => row
            // 插入在最右，正是停止原来的位置：写完那一下还按着停止，落到的是插入（能撤销），不是丢弃。
            .child(tile(DISCARD, "x", tr("丢弃", "Discard"), ink_color))
            .child(tile(AGAIN, "redo", tr("重写", "Rewrite"), ink_color))
            .child(tile(INSERT, "check", tr("插入", "Insert"), mark())),
        Phase::Failed(e) => row
            // 字要走原生：GPUI 画的字在玻璃底下会被模糊掉。
            .child(div().flex_1().min_w_0().h(px(58.)).relative()
                .when(glass, |d| d.child(super::glass::row("assist-status", ("editor-icons/assistant", 22., mark()), (SharedString::from(e.clone()), Theme::danger()))))
                .when(!glass, |d| d.flex().items_center().gap(px(10.)).child(crate::editor_ui::icon("assistant", 22., mark())).child(div().flex_1().min_w_0().text_size(px(15.)).text_color(Theme::danger()).overflow_hidden().whitespace_nowrap().text_ellipsis().child(SharedString::from(e.clone())))))
            .child(div().flex_none().w(px(72.)).child(tile(AGAIN, "redo", tr("重试", "Retry"), ink_color)))
            .child(div().flex_none().w(px(72.)).child(tile(DISCARD, "x", tr("关闭", "Close"), ink_color))),
        Phase::Composing => row,
    };
    let moving = morphing || revealing || matches!(phase, Phase::Thinking | Phase::Writing);
    row.when(moving, |d| d.child(canvas(|_, window, _| window.request_animation_frame(), |_, _, _, _| {}).absolute().size_0()))
        .into_any_element()
}

// ---- 标记「朱批」

/// 助手的标记「朱批」：应用图标的 ¶（`icons/margin.svg`：圆头圆心 (452,404)、半径 190，两笔 552–644、684–776，
/// 从 214 到 826）。静止是完整的 ¶；在想时两笔收进去，圆头像月亮一样盈亏；在写时圆头停在半月，两笔跟着来字往下长。
/// 每帧往目标追，换状态时形变是连续的。`k` 是圆头右半边的鼓出（-1 月牙 … 0 半月 … 1 满月），`f` 是圆头右边那块，
/// `a`、`b` 是两笔的长短。
#[derive(Clone, Copy)]
pub struct Ink { k: f32, f: f32, a: f32, b: f32 }
const STILL: Ink = Ink { k: 0., f: 1., a: 1., b: 1. };
// 上一帧的标记、来字的脉冲、已放出的字数、上一帧的时刻。
thread_local! { static INK: Cell<(Ink, f32, f32, Option<Instant>)> = const { Cell::new((STILL, 0., 0., None)) }; }

unsafe extern "C" { fn UIAccessibilityIsReduceMotionEnabled() -> bool; }

/// 这一帧的标记。`shown` 是已放出的字数：来字时两笔被推长一点。
fn ink(phase: &Phase, since: Instant, shown: f32) -> Ink {
    let (mut q, mut pulse, seen, at) = INK.with(|c| c.get());
    let now = Instant::now();
    // 隔了一阵才又画（新的一次）：从静止的 ¶ 开始变。
    let dt = match at.map(|a| now.duration_since(a).as_secs_f32()) { Some(d) if d < 0.5 => d, _ => { q = STILL; 0. } };
    if shown > seen { pulse = (pulse + 0.12 * (shown - seen)).min(1.); }
    pulse *= 0.92f32.powf(dt * 60.);
    // 减弱动态效果：不随时间摆动，只在状态之间变一次。
    let t = if unsafe { UIAccessibilityIsReduceMotionEnabled() } { 0. } else { since.elapsed().as_secs_f32() };
    let goal = match phase {
        Phase::Thinking => Ink { k: 0.2 + 0.8 * (t * std::f32::consts::TAU / 2.6).cos(), f: 0., a: 0., b: 0. },
        Phase::Writing => Ink { k: 0., f: 1., a: (0.5 + 0.22 * (t * 2.4).sin() + 0.3 * pulse).min(1.), b: (0.42 + 0.22 * (t * 2.4 + 1.2).sin() + 0.3 * pulse).min(1.) },
        _ => STILL,
    };
    // 每 1/60 秒追近 16%（在想时圆头追得紧一些，盈亏才跟得上）。
    let step = |base: f32| 1. - (1. - base).powf(dt * 60.);
    let (fast, slow) = (step(if *phase == Phase::Thinking { 0.35 } else { 0.16 }), step(0.16));
    q = Ink { k: q.k + (goal.k - q.k) * fast, f: q.f + (goal.f - q.f) * slow, a: q.a + (goal.a - q.a) * slow, b: q.b + (goal.b - q.b) * slow };
    INK.with(|c| c.set((q, pulse, shown, Some(now))));
    q
}

impl Ink {
    /// 画在 `size` 见方里：¶ 高占 78%，居中。
    pub fn shape(self, size: f32, color: Hsla) -> super::glass::Shape {
        use super::glass::Seg::*;
        let u = size * 0.78 / 612.;
        let (ox, oy) = ((size - 514. * u) / 2. - 262. * u, (size - 612. * u) / 2. - 214. * u);
        let p = |x: f32, y: f32| (ox + x * u, oy + y * u);
        let mut segs = vec![];
        // 圆头：左半圆是两段四分之一圆，右边是横向半径 k·r 的半个椭圆（k 为负就凹进去成月牙）。
        let (cx, cy, r, e, kk) = (452., 404., 190., self.k * 190., 0.5523);
        let c = |a: (f32, f32), b: (f32, f32), to: (f32, f32)| { let (a, b, to) = (p(a.0, a.1), p(b.0, b.1), p(to.0, to.1)); Cubic(a.0, a.1, b.0, b.1, to.0, to.1) };
        let top = p(cx, cy - r);
        segs.push(Move(top.0, top.1));
        segs.push(c((cx - kk * r, cy - r), (cx - r, cy - kk * r), (cx - r, cy)));
        segs.push(c((cx - r, cy + kk * r), (cx - kk * r, cy + r), (cx, cy + r)));
        segs.push(c((cx + kk * e, cy + r), (cx + e, cy + kk * r), (cx + e, cy)));
        segs.push(c((cx + e, cy - kk * r), (cx + kk * e, cy - r), (cx, cy - r)));
        segs.push(Close);
        let mut rect = |x: f32, y: f32, w: f32, h: f32| {
            if w < 1. || h < 1. { return; }
            let (a, b) = (p(x, y), p(x + w, y + h));
            segs.extend([Move(a.0, a.1), Line(b.0, a.1), Line(b.0, b.1), Line(a.0, b.1), Close]);
        };
        rect(451., 214., 193. * self.f, 380.);
        rect(552., 214., 92., 612. * self.a);
        rect(684., 214., 92., 612. * self.b);
        super::glass::Shape { size, color, segs }
    }
}

/// 没有玻璃时用 GPUI 画同一个形状。
fn paint(shape: super::glass::Shape) -> impl IntoElement {
    let size = shape.size;
    canvas(|_, _, _| (), move |b, _, window, _| {
        use super::glass::Seg::*;
        let at = |x: f32, y: f32| point(b.origin.x + px(x), b.origin.y + px(y));
        let mut path = PathBuilder::fill();
        for seg in &shape.segs {
            match *seg {
                Move(x, y) => path.move_to(at(x, y)),
                Line(x, y) => path.line_to(at(x, y)),
                Cubic(ax, ay, bx, by, x, y) => path.cubic_bezier_to(at(x, y), at(ax, ay), at(bx, by)),
                Close => path.close(),
            }
        }
        if let Ok(path) = path.build() { window.paint_path(path, shape.color); }
    }).size(px(size)).flex_none()
}

/// 底部那块玻璃从胶囊长成整条要这么久（`menu_action` 防误点）。
const MORPH: Duration = Duration::from_millis(420);

/// 助手（和配图）的颜色：设置 › 外观 › 界面强调色。
pub fn mark() -> Hsla { Theme::primary() }

/// 预览里的块淡入、写完时整体落定用的曲线。
pub fn ease_out_quint() -> impl Fn(f32) -> f32 { |t| 1. - (1. - t).powi(5) }

pub fn settings() -> Settings { Settings::load(&dir()) }
pub fn save(s: &Settings) { if let Err(e) = s.save(&dir()) { log::warn!("Could not save assistant settings: {e}"); } }
