//! 文档里配图：「/」菜单「配图」→ 原生玻璃卡片（比例、图像描述、「优化描述」开关，`imagine_card.rs`）→ 发出后在打「/」的那一行插一个图片块
//! （那一行只剩「/」就用它，不然排在它后面），指向 `assets/imagine/<id>.png`。文件写好之前图片块里画占位（`placeholder`），写好了图片从占位下面淡出来。
//! 同名的 `.json` 记着这次的原文、当时的画风和写到的人物（标签原文，删改了也看得到）、实际发出的提示词和负面词、模型、尺寸、种子。
//! 提示词由 `fdl_ai::imagine::assemble` 拼：画风、人物（分角色提示词）、原文，再加设置里的质量词。
//! 「优化描述」开着时，发送先让快速模型整理（卡片上发送钮转圈，关闭就是取消），整理好弹确认页（`imagine_confirm.rs`）：
//! 改过、点「生成」才插块排队；关掉回到卡片，描述还在。整理的结果（`Plan`）记进 `.json`，「重新生成」照它拼，不再问模型。
//! 整理失败不会退回用原文生成（那会花一次没确认过的生图）：提示一下，留在卡片上。
//! 点按重试照记录原样再发；「重新生成」按原文和现在的画风、人物重新拼一次，换个种子。
//!
//! 任务按 id 记，不按块：生成完只写文件、不改文档，所以换篇文档、撤销重做、复制块都不用另外处理，文件在不在就是做完没有。
//! NovelAI 同一个 Key 一次只出一张（同时发的回 429），所以排队，一次只发一个；排着的和在生成的占位一个样子。
//! 块动作栏的「配图」（`open_passage`）给选中的块配一张：卡片上写的是可不填的补充要求，一定先整理（带前后文，人物是候选，模型挑画面里有的），图插在选中的最后一块下面。
//! 助手的 `generate_image`（`margin_write.rs`）也走这条队列：`reserve` 先记下（插进文档的由助手那头插块），`start` 排进队，`on_done` 等它画完或失败；
//! 画在对话里的图不进文档，由对话页照 `status` 画（`margin.rs`）。
//! 长按生成的图（`touch.rs` 的 `lift_block` 先问 `press`）出系统菜单：生成中是「取消生成」，生成好了是「视图」（同封面的全屏看图）、「重新生成」「查看提示词」「编辑提示词」（照记录摊开改，改完按改好的重新生成）。
use std::{cell::{Cell, RefCell}, collections::{HashMap, HashSet, VecDeque}, f32::consts::TAU, path::PathBuf, time::{Duration, Instant, SystemTime, UNIX_EPOCH}};

use fdl_ai::imagine::{Cast, Character, Passage, Plan};
use futures::{StreamExt, channel::oneshot};
use gpui::{prelude::FluentBuilder, *};
use serde::{Deserialize, Serialize};

use crate::{app::Workspace, editor::{core::{EditKind, Pos, Selection}, view::Editor}, model::{Align, Block, BlockId, BlockKind, RichText}, settings::tr, store::Store, theme::{self, Theme}, ui};
use super::{assistant::{self, image}, background, menu::{self, Item}};

#[path = "imagine_card.rs"]
pub mod card;
#[path = "imagine_confirm.rs"]
mod confirm;

pub const SEND: &str = "imagine-send";
pub const CLOSE: &str = "imagine-close";
pub const OPTIMIZE: &str = "imagine-optimize";
const CANCEL: &str = "imagine-cancel";
const PROMPT: &str = "imagine-prompt";
const AGAIN: &str = "imagine-again";
const EDIT: &str = "imagine-edit";
const VIEW: &str = "imagine-view";
const REMOVE: &str = "imagine-remove";
const RETRY: &str = "imagine-retry";
/// 上下文菜单的键：`imagine:<id>`。
const KEY: &str = "imagine:";
const PREFIX: &str = "assets/imagine/";
/// 生成好了：占位先盖着不动（等图解码出来），再淡掉。
const HOLD: f32 = 0.35;
const FADE: f32 = 0.55;

/// 一次生成的记录（`assets/imagine/<id>.json`）。
#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(default)]
struct Record {
    input: String, prompt: String, negative: String, model: String, width: u32, height: u32, seed: u32, created: u64,
    /// 当时的画风（名字、标签）和写到的人物（名字、外貌标签），没有就是空的。
    style: String, style_tags: String, people: Vec<(String, String)>,
    /// 和 `people` 一一对应的排除词；老的记录没有，按空的算。
    people_negative: Vec<String>,
    /// 整理过描述（「优化描述」）：确认后的画面标签、各人物这一幕的（名字, 提示词, 排除）、画面额外排除的。没整理过 `planned` 是假。
    planned: bool, plan_scene: String, plan_cast: Vec<(String, String, String)>, plan_negative: String,
    /// 给选中的块配的图：模型挑的出场人物（名字）。别的（和老的记录）是 `None`，出场的人按原文里写到的算。
    cast: Option<Vec<String>>,
}

/// 出场的人：记着名字的按名字找现在的预设（删掉了的就没有），没记的按原文 `mentioned`。
fn cast_of<'a>(cast: &Option<Vec<String>>, input: &str, all: &'a [Character]) -> Vec<&'a Character> {
    match cast { Some(names) => names.iter().filter_map(|n| all.iter().find(|c| c.name == *n)).collect(), None => fdl_ai::imagine::mentioned(all, input) }
}

impl Record {
    /// 记着的整理结果（没整理过是 `None`）。
    fn plan(&self) -> Option<Plan> {
        self.planned.then(|| Plan { scene: self.plan_scene.clone(), characters: self.plan_cast.iter().map(|(name, tags, negative)| Cast { name: name.clone(), tags: tags.clone(), negative: negative.clone() }).collect(), negative: self.plan_negative.clone() })
    }
    /// 按原文和现在的设置拼一份（模型、画风、人物照现在的，种子新取）。
    fn fresh(input: String, (width, height): (u32, u32), plan: Option<&Plan>, cast: Option<Vec<String>>) -> Record {
        let (model, _) = image::current();
        let lib = assistant::prompts::lib();
        let style = lib.current_style(crate::settings::prefs().english);
        let p = fdl_ai::imagine::assemble(style.as_ref(), &cast_of(&cast, &input, &lib.characters), &input, plan, (lib.quality(), lib.quality_negative()));
        Record { cast, planned: plan.is_some(), plan_scene: plan.map_or_else(String::new, |p| p.scene.clone()),
            plan_cast: plan.map_or_else(Vec::new, |p| p.characters.iter().map(|c| (c.name.clone(), c.tags.clone(), c.negative.clone())).collect()), plan_negative: plan.map_or_else(String::new, |p| p.negative.clone()),
            prompt: p.base, negative: p.negative, model: model.into(), width, height, seed: seed(), created: now(),
            style: style.as_ref().map_or_else(String::new, |s| s.name.clone()), style_tags: style.map_or_else(String::new, |s| s.positive), people: p.characters.iter().map(|c| (c.name.clone(), c.tags.clone())).collect(),
            people_negative: p.characters.into_iter().map(|c| c.negative).collect(), input }
    }
}

enum State { Queued, Running, Failed(String) }
/// 网络那头的任务和后台任务的那一份，丢掉就停。
struct Live { ticket: background::Ticket, call: tokio::task::AbortHandle }
impl Drop for Live { fn drop(&mut self) { self.call.abort(); } }
struct Job { state: State, since: Instant, live: Option<Live> }

thread_local! {
    static JOBS: RefCell<HashMap<String, Job>> = RefCell::new(HashMap::new());
    static QUEUE: RefCell<VecDeque<String>> = const { RefCell::new(VecDeque::new()) };
    /// 刚生成好的：什么时候好的（占位还要盖一会儿再淡掉）。
    static DONE: RefCell<HashMap<String, Instant>> = RefCell::new(HashMap::new());
    /// 有没有记录（文件不在、也没在生成时，有记录的是「中断了」）。
    static KNOWN: RefCell<HashMap<String, bool>> = RefCell::new(HashMap::new());
    /// 卡片开着：哪篇文档、打「/」的那一行（给选中的块配图时是选中的最后一块），和给选中的块配图时要配的那一段。
    static COMPOSE: RefCell<Option<(Option<String>, Option<BlockId>, Option<Passage>)>> = const { RefCell::new(None) };
    /// 点重试对着的图。
    static TARGET: RefCell<Option<String>> = const { RefCell::new(None) };
    /// 上一次长按弹菜单的时刻：长按松手后系统还会补一次点按，这次点按不算「点按重试」。
    static PRESSED: Cell<Option<Instant>> = const { Cell::new(None) };
    /// 各张图在窗口里的位置（「视图」从这里放大出来；系统菜单抢在 `lift_block` 前接走长按，所以每帧记），和各张图的圆角。
    static FRAMES: RefCell<HashMap<String, Bounds<Pixels>>> = RefCell::new(HashMap::new());
    static RADII: RefCell<HashMap<String, f32>> = RefCell::new(HashMap::new());
    /// 整理描述的第几次：关掉卡片、又发了一次时加一，晚到的结果对不上就不要。还在整理时是 `Some`。
    static PLANNING: Cell<(u64, bool)> = const { Cell::new((0, false)) };
    /// 占位动画的时钟：所有占位共用，状态换了（排队 → 生成 → 淡出）光也不跳。
    static EPOCH: Instant = Instant::now();
    /// 等着这一张画完（或失败、取消）的：助手的 `generate_image`。
    static WAITERS: RefCell<HashMap<String, Vec<Box<dyn FnOnce(Result<(), String>)>>>> = RefCell::new(HashMap::new());
}

/// 这一张有了结果：告诉等着的。
fn settle(id: &str, result: Result<(), String>) {
    for f in WAITERS.with(|w| w.borrow_mut().remove(id)).unwrap_or_default() { f(result.clone()); }
}

// ---- 助手画的图

/// `generate_image` 的画幅（`1:1`、`2:3`、`3:2`）对应的尺寸键。
fn ratio_key(ratio: &str) -> &'static str { match ratio { "2:3" => "portrait", "3:2" => "landscape", _ => "square" } }

/// 记下助手要画的一张：画面原文、画幅，出场的人（助手的形象，按名字），还不开始画。返回标识、图片块的地址和像素宽高。
pub fn reserve(input: &str, ratio: &str, cast: Vec<String>) -> Option<(String, String, u32, u32)> {
    let (width, height) = image::size(ratio_key(ratio));
    let id = new_id();
    save(&id, &Record::fresh(input.to_owned(), (width, height), None, Some(cast))).then(|| (id.clone(), src_of(&id), width, height))
}

/// 开始画（排进队里）。
pub fn start(id: &str, cx: &mut App) { enqueue(id.to_owned(), cx); }

/// 画完或失败时叫一声（已经有图了马上叫）。
pub fn on_done(id: &str, f: impl FnOnce(Result<(), String>) + 'static) {
    if png(id).is_file() { return f(Ok(())); }
    WAITERS.with(|w| w.borrow_mut().entry(id.to_owned()).or_default().push(Box::new(f)));
}

/// 画在对话里的一张现在怎样。
pub enum Status { Ready(PathBuf), Busy, Failed(String), Missing }
pub fn status(id: &str) -> Status {
    if !valid(id) { return Status::Missing; }
    if png(id).is_file() { return Status::Ready(png(id)); }
    match JOBS.with(|j| j.borrow().get(id).map(|j| match &j.state { State::Failed(e) => Some(e.clone()), _ => None })) {
        Some(None) => Status::Busy,
        Some(Some(e)) => Status::Failed(e),
        None if known(id) => Status::Failed(tr("生成已中断", "Generation was interrupted").into()),
        None => Status::Missing,
    }
}
/// 记着的像素宽高。
pub fn size_of(id: &str) -> Option<(u32, u32)> { load(id).map(|r| (r.width, r.height)).filter(|(w, h)| *w > 0 && *h > 0) }
/// 对话里的图点了重试：照记录原样再排一次。
pub fn retry(id: &str, cx: &mut App) { if known(id) && !png(id).is_file() { enqueue(id.to_owned(), cx); } }

fn dir() -> PathBuf { Store::assets_dir().join("imagine") }
fn valid(id: &str) -> bool { !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric()) }
/// 图片块的地址是不是生成出来的图，是的话取 id。
pub fn id_of(src: &str) -> Option<&str> { src.strip_prefix(PREFIX)?.strip_suffix(".png").filter(|id| valid(id)) }
fn src_of(id: &str) -> String { format!("{PREFIX}{id}.png") }
fn png(id: &str) -> PathBuf { dir().join(format!("{id}.png")) }
fn load(id: &str) -> Option<Record> { serde_json::from_slice(&std::fs::read(dir().join(format!("{id}.json"))).ok()?).ok() }
fn save(id: &str, r: &Record) -> bool {
    let ok = std::fs::create_dir_all(dir()).is_ok() && serde_json::to_vec_pretty(r).ok().is_some_and(|b| crate::store::write_file(&dir().join(format!("{id}.json")), &b).is_ok());
    KNOWN.with(|k| k.borrow_mut().insert(id.to_owned(), ok));
    ok
}
fn known(id: &str) -> bool { KNOWN.with(|k| *k.borrow_mut().entry(id.to_owned()).or_insert_with(|| dir().join(format!("{id}.json")).is_file())) }
fn new_id() -> String { uuid::Uuid::new_v4().simple().to_string()[..16].to_owned() }
fn seed() -> u32 { uuid::Uuid::new_v4().as_u128() as u32 }
fn now() -> u64 { SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()) }
fn toast(text: &'static str) { menu::dispatch(move |w, _, cx| w.show_toast_kind(ui::ToastKind::Error, text, cx)); }
/// 在当前的编辑器上重画（发起生成的那个可能已经换掉了）。
fn refresh() { menu::dispatch(|w, _, cx| if let Some(editor) = w.editor.clone() { editor.update(cx, |_, cx| cx.notify()); }); }

// ---- 卡片

/// 卡片在这篇文档里开着：底部的铅笔、块工具条让开，点正文不放光标（`assist::active` 也算上它）。
pub fn composing() -> bool { COMPOSE.with(|c| c.borrow().as_ref().is_some_and(|(doc, ..)| *doc == super::assist::showing())) }

/// 「/」菜单里点了「配图」：「/」已经删掉了，`anchor` 是打「/」的那一行。没有 Key 时不开，提示去设置。
pub fn open(ed: &mut Editor, anchor: Option<BlockId>, window: &mut Window, cx: &mut Context<Editor>) {
    if image::key().is_none() { return toast(tr("请先在设置里添加 NovelAI Key", "Add a NovelAI key in Settings first")); }
    if super::assist::active() { return; }
    begin(ed, anchor, None, window, cx);
}

fn begin(ed: &mut Editor, anchor: Option<BlockId>, passage: Option<Passage>, window: &mut Window, cx: &mut Context<Editor>) {
    card::set_passage(passage.is_some());
    COMPOSE.with(|c| *c.borrow_mut() = Some((super::assist::showing(), anchor, passage)));
    ed.finish_block_selection(window, cx);
    // 键盘交给卡片里的输入框。
    window.blur(cx);
    cx.notify();
}

/// 块工具条的「配图」：给选中的块配一张，插在选中的最后一块下面。卡片上写的是可不填的补充要求；
/// 发出一定先让快速模型整理（带上前后文，人物在写到的里面挑），确认了才生成。
pub fn open_passage(ed: &mut Editor, window: &mut Window, cx: &mut Context<Editor>) {
    if image::key().is_none() { return toast(tr("请先在设置里添加 NovelAI Key", "Add a NovelAI key in Settings first")); }
    if assistant::text::quick().is_none() { return toast(tr("先在 设置 › MarginAI › 模型 里选择模型", "Choose a model in Settings › MarginAI › Models first")); }
    if super::assist::active() { return; }
    let Some((passage, anchor)) = gather(ed) else { return toast(tr("选中的块里没有可配图的文字", "There is no text to illustrate in the selection")) };
    begin(ed, Some(anchor), Some(passage), window, cx);
}

/// 前后文各取多少：块数、字数（离选中的近的优先）。
const BEFORE: (usize, usize) = (6, 800);
const AFTER: (usize, usize) = (3, 400);

fn cut(s: &str, n: usize) -> String { s.chars().take(n).collect() }

/// 一块写成给模型看的一行：标题 `# `、列表 `- `，引用、高亮块只取字；图只要说明，表格、代码在选中的里给个开头、在前后文里只占个记号（代码不占）。
/// 没字的、分割线这些不算。
fn line(b: &Block, focus: bool) -> Option<String> {
    let text = b.text.text.trim();
    let s = match &b.kind {
        BlockKind::Heading { level, .. } => format!("{} {text}", "#".repeat(*level as usize)),
        BlockKind::Bullet | BlockKind::Ordered { .. } | BlockKind::Todo { .. } => format!("- {text}"),
        BlockKind::Code { .. } => if focus { format!("[code]\n{}", cut(text, 300)) } else { return None },
        BlockKind::Image { caption, .. } => return caption.as_deref().map(str::trim).filter(|c| !c.is_empty()).map(|c| format!("[image: {c}]")),
        BlockKind::Table(t) => return Some(if focus {
            let rows = t.rows.iter().take(2).map(|r| r.iter().map(|c| c.blocks.iter().map(|b| b.text.text.trim()).filter(|t| !t.is_empty()).collect::<Vec<_>>().join(" ")).filter(|t| !t.is_empty()).collect::<Vec<_>>().join(" | ")).collect::<Vec<_>>().join("\n");
            format!("[table]\n{}", cut(&rows, 200))
        } else { "[table]".into() }),
        BlockKind::Text | BlockKind::Quote | BlockKind::Callout { .. } | BlockKind::Page { .. } => text.to_owned(),
        _ => return None,
    };
    (!text.is_empty()).then_some(s)
}

/// 前后文到这里为止：分割线、一二级标题（换了一节）。
fn boundary(b: &Block) -> bool { matches!(b.kind, BlockKind::Divider { .. } | BlockKind::Heading { level: 1 | 2, .. }) }

/// 选中的块要配的那一段，和插图要排在哪一块后面（选中的最后一块）。选中的里没有字就是 `None`。
/// 这一段太长时留开头和结尾；前文、后文往外各取几块，碰到分割线或一二级标题停；小节是往上最近的标题。
fn gather(ed: &Editor) -> Option<(Passage, BlockId)> {
    let core = &ed.core;
    let picked = core.block_sel_ids();
    // 选中的最后一块是空段落时排在它前面（`insert_generated` 会拿图换掉空段落，选中的块不该被换掉）。
    let blank = |id: &BlockId| core.block(*id).is_some_and(|b| b.kind == BlockKind::Text && b.text.text.is_empty() && b.children.is_empty());
    let order = core.block_order(false);
    let anchor = order.iter().rfind(|id| picked.contains(id) && !blank(id)).or_else(|| order.iter().rfind(|id| picked.contains(id))).copied()?;
    fn walk<'a>(b: &'a Block, out: &mut Vec<&'a Block>) { out.push(b); if !b.kind.is_page() { b.children.iter().for_each(|c| walk(c, out)); } }
    let mut blocks = vec![];
    for id in &picked { if let Some(b) = core.block(*id) { walk(b, &mut blocks); } }
    let focus = blocks.iter().filter_map(|b| line(b, true)).collect::<Vec<_>>().join("\n");
    if focus.chars().filter(|c| !c.is_whitespace()).count() < 2 { return None; }
    let n = focus.chars().count();
    let focus = if n > 1500 { format!("{}\n[…]\n{}", cut(&focus, 600), focus.chars().skip(n - 900).collect::<String>()) } else { focus };
    let inside: HashSet<BlockId> = blocks.iter().map(|b| b.id).collect();
    let order: Vec<&Block> = core.block_order(true).into_iter().filter_map(|id| core.block(id)).collect();
    let first = order.iter().position(|b| inside.contains(&b.id)).unwrap_or(0);
    let last = order.iter().rposition(|b| inside.contains(&b.id)).map_or(order.len(), |i| i + 1);
    // 往外取：块数、字数到了就停；第一块就超了字数时截它靠近选中的那一头。
    let take = |it: &mut dyn Iterator<Item = &&Block>, (blocks, chars): (usize, usize), tail: bool| {
        let mut out: Vec<String> = vec![];
        let mut used = 0;
        for b in it {
            if boundary(b) || out.len() >= blocks { break; }
            let Some(l) = line(b, false) else { continue };
            let len = l.chars().count();
            if used + len > chars {
                if out.is_empty() { out.push(if tail { l.chars().skip(len - chars).collect() } else { cut(&l, chars) }); }
                break;
            }
            used += len;
            out.push(l);
        }
        out
    };
    let mut before = take(&mut order[..first].iter().rev(), BEFORE, true);
    before.reverse();
    let after = take(&mut order[last.min(order.len())..].iter(), AFTER, false);
    let section = order[..first].iter().rev().find(|b| matches!(b.kind, BlockKind::Heading { .. }) && !b.text.text.trim().is_empty()).map_or_else(String::new, |b| b.text.text.trim().to_owned());
    Some((Passage { title: core.doc.title.text.trim().to_owned(), section, before: before.join("\n"), focus, after: after.join("\n"), direction: String::new() }, anchor))
}

/// 记进 `.json` 的原文：补充要求（有的话）一行，加上这一段压成一行、过长截断。
fn summary(p: &Passage) -> String {
    let flat = p.focus.split_whitespace().collect::<Vec<_>>().join(" ");
    let flat = if flat.chars().count() > 120 { format!("{}…", cut(&flat, 120)) } else { flat };
    if p.direction.trim().is_empty() { flat } else { format!("{}\n{flat}", p.direction.trim()) }
}

/// 给一段配图的候选人物：这一段（和补充要求）里写到的，再加只在前后文里写到的，一共最多五个。
fn candidates<'a>(all: &'a [Character], p: &Passage) -> (Vec<&'a Character>, Vec<&'a Character>) {
    let focus = fdl_ai::imagine::mentioned(all, &format!("{}\n{}", p.focus, p.direction));
    let nearby = fdl_ai::imagine::mentioned(all, &format!("{}\n{}", p.before, p.after)).into_iter().filter(|c| !focus.iter().any(|f| f.name == c.name)).take(5 - focus.len()).collect();
    (focus, nearby)
}

/// 底部：卡片开着时是卡片的占位。
pub fn overlay() -> Option<AnyElement> { composing().then(card::slot) }

/// 工作区收到的动作（卡片上的按钮、长按菜单、占位上的重试）。
pub fn menu_action(id: &str, w: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) -> bool {
    if !id.starts_with("imagine-") { return false; }
    let Some(editor) = w.editor.clone() else { return true };
    match id {
        SEND => {
            if PLANNING.with(Cell::get).1 { return true; }
            let input = card::text().trim().to_owned();
            let ratio = card::ratio();
            // 给选中的块配图：写的是补充要求，可以不写；一定先整理。
            if let Some(passage) = COMPOSE.with(|c| c.borrow().as_ref().and_then(|c| c.2.clone())) {
                plan(String::new(), Some(Passage { direction: input, ..passage }), ratio, cx);
                return true;
            }
            if input.is_empty() { return true; }
            if assistant::prompts::lib().optimize { plan(input, None, ratio, cx); return true; }
            let anchor = COMPOSE.with(|c| c.borrow_mut().take()).and_then(|(_, a, _)| a);
            editor.update(cx, |ed, cx| create(ed, input, ratio, anchor, None, None, window, cx));
        }
        OPTIMIZE => {
            let on = !assistant::prompts::lib().optimize;
            if on && assistant::text::quick().is_none() {
                w.show_toast_kind(ui::ToastKind::Error, tr("先在 设置 › MarginAI › 模型 里选择模型", "Choose a model in Settings › MarginAI › Models first"), cx);
            } else { assistant::prompts::set_optimize(on); }
            card::set_optimize(assistant::prompts::lib().optimize);
        }
        CLOSE => {
            PLANNING.with(|p| p.set((p.get().0 + 1, false)));
            COMPOSE.with(|c| c.borrow_mut().take());
            editor.update(cx, |_, cx| cx.notify());
        }
        RETRY => if let Some(gid) = TARGET.with(|t| t.borrow_mut().take()) { editor.update(cx, |ed, cx| { let b = block_of(ed, &gid); again(ed, &gid, b, window, cx) }); },
        _ => {
            let Some(gid) = pressed() else { return true };
            match id {
                PROMPT => show_prompt(&gid),
                VIEW => if let Some(frame) = FRAMES.with(|f| f.borrow().get(&gid).copied()) && png(&gid).is_file() {
                    super::coveredit::view(&png(&gid), frame, RADII.with(|r| r.borrow().get(&gid).copied()).unwrap_or(0.) as f64);
                },
                EDIT => edit(&gid),
                AGAIN => editor.update(cx, |ed, cx| { let b = block_of(ed, &gid); again(ed, &gid, b, window, cx) }),
                CANCEL | REMOVE => {
                    forget(&gid);
                    super::glass::delete_haptic();
                    editor.update(cx, |ed, cx| if let Some(b) = block_of(ed, &gid) { ed.core.delete_block(b); ed.changed(window, cx); });
                    pump(cx);
                }
                _ => {}
            }
        }
    }
    true
}

/// 整理描述：卡片上转圈，问快速模型；回来了弹确认页。确认了才插块，关掉回到卡片。
/// 给选中的块配图时（`passage`）带上前后文，候选人物里只留模型挑进画面的，图插在选中的最后一块下面。
fn plan(input: String, passage: Option<Passage>, ratio: &'static str, cx: &mut Context<Workspace>) {
    let Some(quick) = assistant::text::quick() else {
        return toast(tr("先在 设置 › MarginAI › 模型 里选择模型", "Choose a model in Settings › MarginAI › Models first"));
    };
    let size = image::size(ratio);
    let lib = assistant::prompts::lib();
    let (cast, (system, user)): (Vec<Character>, _) = match &passage {
        None => {
            let cast = fdl_ai::imagine::mentioned(&lib.characters, &input);
            (cast.iter().map(|c| (*c).clone()).collect(), fdl_ai::imagine::plan_messages(&cast, &input, size))
        }
        Some(p) => {
            let (focus, nearby) = candidates(&lib.characters, p);
            (focus.iter().chain(&nearby).map(|c| (*c).clone()).collect(), fdl_ai::imagine::passage_messages(&focus, &nearby, p, size))
        }
    };
    let strict = passage.is_some();
    let input = passage.as_ref().map_or(input, summary);
    let turn = PLANNING.with(|p| { let n = p.get().0 + 1; p.set((n, true)); n });
    card::busy(true);
    cx.spawn(async move |this, cx| {
        let (base, key, model, effort) = quick;
        let mut events = assistant::llm::chat(base, key, model, effort, vec![("system", system), ("user", user)]);
        let mut reply = String::new();
        let mut error = None;
        while let Some(e) = events.next().await {
            match e {
                assistant::llm::Event::Text(t) => reply.push_str(&t),
                assistant::llm::Event::Thinking | assistant::llm::Event::Calls(_) => {}
                assistant::llm::Event::Done => break,
                assistant::llm::Event::Failed(e) => { error = Some(e); break }
            }
        }
        // 关掉了、又发了一次：这次的不要。
        if PLANNING.with(Cell::get) != (turn, true) { return; }
        PLANNING.with(|p| p.set((turn, false)));
        card::busy(false);
        let planned = if error.is_some() { None } else { fdl_ai::imagine::parse_plan(&reply, &cast.iter().collect::<Vec<_>>(), strict) };
        let _ = this.update(cx, |w, cx| {
            let Some(planned) = planned else {
                let why = error.map_or_else(|| tr("优化失败，请重试", "Could not optimize. Try again").to_owned(), |e| crate::localized_format!("优化失败：{}", "Could not optimize: {}", e));
                return w.show_toast_kind(ui::ToastKind::Error, &why, cx);
            };
            confirm(input, ratio, planned, &cast, strict, cx);
        });
    }).detach();
}

/// 确认页：描述（跟画面一样就不显示）、画面、每个出场人物一张标签卡（跟预设比去掉了什么、加了什么）、排除；点「生成」交出改好的 `Plan`，关掉是 `None`。
/// 人物按现在的预设比（编辑提示词时预设可能已经改过，比出来的就是这次会发出去的）；预设找不到就拿记着的整串当预设，全部算保留。
fn review(title: &str, input: &str, planned: &Plan, cast: &[fdl_ai::imagine::Character], done: impl Fn(Option<Plan>) + 'static) -> bool {
    use confirm::{Answer, Chips, Section};
    use fdl_ai::imagine::{apply, diff, tags};
    let chips = |preset: &str, now: &str| { let (removed, added) = diff(preset, now); Chips { preset: tags(preset), removed, added } };
    let mut sections = vec![];
    if input.trim() != planned.scene.trim() { sections.push(Section::Note(input.to_owned())); }
    sections.push(Section::Text { title: tr("画面", "Scene").into(), text: planned.scene.clone() });
    let mut bases = vec![];
    for c in &planned.characters {
        let preset = cast.iter().find(|p| p.name == c.name);
        let (look, negative) = preset.map_or_else(|| (c.tags.clone(), c.negative.clone()), |p| (p.look.clone(), p.negative.clone()));
        sections.push(Section::Tags { title: c.name.clone(), look: chips(&look, &c.tags), negative: chips(&negative, &c.negative) });
        bases.push((c.name.clone(), look, negative));
    }
    sections.push(Section::Text { title: tr("排除", "Exclude").into(), text: planned.negative.clone() });
    confirm::show(title, sections, move |answer| done(answer.map(|answers| {
        let mut texts = answers.iter().filter_map(|a| if let Answer::Text(t) = a { Some(t.trim().to_owned()) } else { None });
        let (scene, negative) = (texts.next().unwrap_or_default(), texts.next().unwrap_or_default());
        let characters = answers.iter().filter_map(|a| if let Answer::Tags { look, negative } = a { Some((look, negative)) } else { None })
            .zip(&bases).map(|((l, n), (name, look, neg))| Cast { name: name.clone(), tags: apply(look, &l.0, &l.1), negative: apply(neg, &n.0, &n.1) }).collect();
        Plan { scene, characters, negative }
    })))
}

/// 优化完的确认：确认了插块，关掉回到卡片。
fn confirm(input: String, ratio: &'static str, planned: Plan, cast: &[Character], passage: bool, _cx: &mut Context<Workspace>) {
    card::blur();
    let shown = review(tr("确认提示词", "Confirm Prompt"), &input.clone(), &planned, cast, move |plan| {
        let Some(plan) = plan else { card::focus(); return };
        let input = input.clone();
        menu::dispatch(move |w, window, cx| {
            let Some(editor) = w.editor.clone() else { return };
            let Some((_, anchor, _)) = COMPOSE.with(|c| c.borrow_mut().take()) else { return };
            let cast = passage.then(|| plan.characters.iter().map(|c| c.name.clone()).collect());
            editor.update(cx, |ed, cx| create(ed, input, ratio, anchor, Some(&plan), cast, window, cx));
        });
    });
    if !shown { card::focus(); toast(tr("无法打开确认页", "Could not open the confirmation")); }
}

/// 长按图的「编辑提示词」：照这张图的记录摊开（整理过的用整理结果；没整理过的画面就是原描述，人物是当时发出的提示词和排除），
/// 改完点「生成」按改好的重新生成（同「重新生成」：新 id、新种子、一步撤销）。不问模型。
fn edit(id: &str) {
    let Some(old) = load(id) else { return toast(tr("找不到这张图的记录", "The record for this image is missing")) };
    let plan = old.plan().unwrap_or_else(|| Plan {
        scene: old.input.clone(), negative: String::new(),
        characters: old.people.iter().enumerate().map(|(i, (name, tags))| Cast { name: name.clone(), tags: tags.clone(), negative: old.people_negative.get(i).cloned().unwrap_or_default() }).collect(),
    });
    let lib = assistant::prompts::lib();
    let cast: Vec<Character> = cast_of(&old.cast, &old.input, &lib.characters).into_iter().cloned().collect();
    let id = id.to_owned();
    let shown = review(tr("编辑提示词", "Edit Prompt"), &old.input, &plan, &cast, move |plan| {
        let Some(plan) = plan else { return };
        let id = id.clone();
        menu::dispatch(move |w, window, cx| {
            let Some(editor) = w.editor.clone() else { return };
            editor.update(cx, |ed, cx| { let b = block_of(ed, &id); redo(ed, &id, b, Some(plan), window, cx) });
        });
    });
    if !shown { toast(tr("无法打开编辑页", "Could not open the editor")); }
}

/// 发出：先记下来，再插图片块（宽高按要的像素写，占位就是成图的形状），排进队里。`plan` 是确认过的整理结果。
/// 记着出场人物的（`cast`，给选中的块配的图）插在 `anchor` 下面，不碰那一块。
fn create(ed: &mut Editor, input: String, ratio: &str, anchor: Option<BlockId>, plan: Option<&Plan>, cast: Option<Vec<String>>, window: &mut Window, cx: &mut Context<Editor>) {
    let (width, height) = image::size(ratio);
    let id = new_id();
    let below = cast.is_some();
    let r = Record::fresh(input, (width, height), plan, cast);
    if !save(&id, &r) { return toast(tr("无法保存生成记录", "Could not save the image record")); }
    let mut b = Block::new(BlockKind::Image { src_kind: "src".into(), src: src_of(&id), width: Some(width as f32), height: Some(height as f32), caption: None, crop: None }, RichText::default());
    b.align = Align::Center;
    if below {
        ed.core.insert_generated(anchor, vec![b]);
        // 光标回到那一块末尾（停在图上会出图片的工具条）。
        if let Some(a) = anchor.filter(|a| ed.core.path(*a).is_some()) {
            let end = ed.core.text_of(a).map_or(0, |t| t.text.len());
            ed.core.sel = Selection::caret(Pos::new(a, end));
        }
        ed.changed(window, cx);
        return enqueue(id, cx);
    }
    // 光标放到那一行末尾：`insert_block` 在光标所在的是空段落时替换它，不然排在它后面。
    if let Some(a) = anchor.filter(|a| ed.core.path(*a).is_some()) {
        let end = ed.core.text_of(a).map_or(0, |t| t.text.len());
        ed.core.sel = Selection::caret(Pos::new(a, end));
    }
    let nid = ed.core.insert_block(b);
    let p = ed.core.ensure_paragraph_after(nid);
    ed.core.sel = Selection::caret(Pos::new(p, 0));
    ed.changed(window, cx);
    enqueue(id, cx);
}

/// 重新生成：生成好的按原文和现在的设置（模型、画风、人物）重新拼（整理过的照记着的整理结果，不再问模型），换一个新 id、新种子，块改指向新图，一步撤销能回到原来那张；
/// 失败了、中断了的就用原来的记录再排一次。
fn again(ed: &mut Editor, id: &str, block: Option<BlockId>, window: &mut Window, cx: &mut Context<Editor>) {
    if !png(id).is_file() {
        if !known(id) { return toast(tr("找不到这张图的记录", "The record for this image is missing")); }
        return enqueue(id.to_owned(), cx);
    }
    redo(ed, id, block, None, window, cx);
}

/// 换一张：按原文和 `plan`（没给就用记着的整理结果）重新拼，新 id、新种子，块改指向新图，一步撤销能回到原来那张。
fn redo(ed: &mut Editor, id: &str, block: Option<BlockId>, plan: Option<Plan>, window: &mut Window, cx: &mut Context<Editor>) {
    let (Some(old), Some(block)) = (load(id), block.filter(|b| ed.core.path(*b).is_some())) else { return toast(tr("找不到这张图的记录", "The record for this image is missing")) };
    let nid = new_id();
    let r = Record::fresh(old.input.clone(), (old.width, old.height), plan.or_else(|| old.plan()).as_ref(), old.cast.clone());
    if !save(&nid, &r) { return toast(tr("无法保存生成记录", "Could not save the image record")); }
    ed.core.checkpoint(EditKind::Other);
    if let Some(b) = ed.core.block_mut(block) && let BlockKind::Image { src, .. } = &mut b.kind { *src = src_of(&nid); }
    ed.changed(window, cx);
    enqueue(nid, cx);
}

// ---- 队列

fn enqueue(id: String, cx: &mut App) {
    JOBS.with(|j| j.borrow_mut().insert(id.clone(), Job { state: State::Queued, since: Instant::now(), live: None }));
    QUEUE.with(|q| { let mut q = q.borrow_mut(); q.retain(|x| *x != id); q.push_back(id); });
    refresh();
    pump(cx);
}

/// 不要了（取消、删除）：停掉请求，排着的拿出来。
fn forget(id: &str) {
    drop(JOBS.with(|j| j.borrow_mut().remove(id)));
    QUEUE.with(|q| q.borrow_mut().retain(|x| x != id));
    settle(id, Err(tr("用户取消了这张图", "The user cancelled this image").into()));
}

fn fail(id: &str, error: String) {
    settle(id, Err(error.clone()));
    let live = JOBS.with(|j| j.borrow_mut().get_mut(id).map(|job| { job.state = State::Failed(error); job.live.take() }));
    if let Some(Some(live)) = live { live.ticket.fail(); }
}

/// 系统停掉了后台任务：这一张按失败处理，点它重试。
fn halted(id: String) {
    fail(&id, tr("后台生成被停止了", "Background generation was stopped").into());
    refresh();
    menu::dispatch(|_, _, cx| pump(cx));
}

/// 没有在生成的就从队头拿一张发出去。
fn pump(cx: &mut App) {
    if JOBS.with(|j| j.borrow().values().any(|j| matches!(j.state, State::Running))) { return; }
    let Some(id) = QUEUE.with(|q| q.borrow_mut().pop_front()) else { return };
    if !JOBS.with(|j| j.borrow().get(&id).is_some_and(|j| matches!(j.state, State::Queued))) { return pump(cx); }
    let Some(r) = load(&id) else { fail(&id, tr("找不到这张图的记录", "The record for this image is missing").into()); refresh(); return pump(cx) };
    let Some(key) = image::key() else { fail(&id, tr("先在 设置 › 图像 里添加 API Key", "Add an API Key in Settings › Image first").into()); refresh(); return pump(cx) };
    let since = Instant::now();
    let ticket = background::begin(background::Kind::Image, String::new(), tr("正在生成", "Generating"), 1, { let id = id.clone(); move || halted(id) });
    let (tx, rx) = oneshot::channel();
    let request = image::Request { prompt: r.prompt, negative: r.negative, characters: r.people.into_iter().enumerate().map(|(i, (_, tags))| (tags, r.people_negative.get(i).cloned().unwrap_or_default())).collect(), seed: Some(r.seed) };
    let (model, size) = (r.model, (r.width, r.height));
    let url = image::endpoint();
    let call = assistant::llm::runtime().spawn(async move { let _ = tx.send(image::generate(&url, &key, &model, size, &request).await); });
    JOBS.with(|j| if let Some(job) = j.borrow_mut().get_mut(&id) { *job = Job { state: State::Running, since, live: Some(Live { ticket, call: call.abort_handle() }) }; });
    cx.spawn(async move |cx| {
        let result = rx.await.unwrap_or_else(|_| Err(tr("请求中断了", "The request was interrupted").into()));
        // 中途取消了（或者又排了一次）的不要了。
        let current = JOBS.with(|j| j.borrow().get(&id).is_some_and(|j| matches!(j.state, State::Running) && j.since == since));
        if current {
            let written = result.and_then(|bytes| {
                let _ = std::fs::create_dir_all(dir());
                crate::store::write_file(&png(&id), &bytes).map_err(|_| tr("无法保存图片", "Could not save the image").to_owned())
            });
            match written {
                Ok(()) => { DONE.with(|d| d.borrow_mut().insert(id.clone(), Instant::now())); drop(JOBS.with(|j| j.borrow_mut().remove(&id))); settle(&id, Ok(())); }
                Err(e) => fail(&id, e),
            }
            refresh();
        }
        let _ = cx.update(pump);
    }).detach();
}

// ---- 占位

/// 图片块（`R::image`）问这一块要不要画占位。`local` 是文件已经在了：刚生成好的盖一层占位，等一下再淡掉。
/// 不是生成出来的、或者没记录可查的，返回 `None`，照桌面的样子画。
pub fn placeholder(src: &str, local: bool, w: f32, h: f32, radius: f32) -> Option<AnyElement> {
    let id = id_of(src)?.to_owned();
    let ctx = context(&id, radius);
    let veil = if local {
        DONE.with(|d| d.borrow().get(&id).copied()).and_then(|at| {
            let t = at.elapsed().as_secs_f32();
            if t >= HOLD + FADE { DONE.with(|d| d.borrow_mut().remove(&id)); return None; }
            let k = ((t - HOLD) / FADE).clamp(0., 1.);
            Some(developing(w, h, radius).opacity(1. - k * k * (3. - 2. * k)).into_any_element())
        })
    } else {
        let failed = JOBS.with(|j| j.borrow().get(&id).map(|j| match &j.state { State::Failed(e) => Some(e.clone()), _ => None }));
        match failed {
            Some(None) => Some(developing(w, h, radius).into_any_element()),
            Some(Some(e)) => Some(stopped(id, tr("生成失败", "Generation failed"), Some(e), w, h, radius, false)),
            None if known(&id) => Some(stopped(id, tr("生成已中断", "Generation was interrupted"), None, w, h, radius, false)),
            // 不是这里生成的（文件丢了、也没有记录）：照普通图片显示「找不到」。
            None => return None,
        }
    };
    Some(div().absolute().inset_0().children(veil).child(ctx).into_any_element())
}

/// 生成中：成图形状的一块浅底，底色很轻地起伏，一道柔光从左往右慢慢扫过去，歇一下再来；正中一个淡淡的图片图标跟着呼吸。
/// 时间都从共用的时钟算（不靠元素自己的动画状态），滚出去再滚回来、换了状态，光都接得上。
pub(crate) fn developing(w: f32, h: f32, radius: f32) -> Div {
    const SWEEP: f32 = 1.8;
    const REST: f32 = 0.9;
    const BREATH: f32 = 2.7;
    let t = EPOCH.with(|e| e.elapsed().as_secs_f32());
    let p = ((t % (SWEEP + REST)) / SWEEP).min(1.);
    // 两头缓、中间快。
    let e = if p < 0.5 { 4. * p * p * p } else { 1. - (-2. * p + 2.).powi(3) / 2. };
    let breath = 0.5 - 0.5 * (t * TAU / BREATH).cos();
    let dark = theme::dark();
    let base = theme::ca(theme::craft::N1000, if dark { 0.06 } else { 0.05 } + 0.02 * breath);
    let light = if dark { hsla(0., 0., 1., 0.07) } else { hsla(0., 0., 1., 0.65) };
    let clear = light.opacity(0.);
    let band = (w * 0.45).clamp(90., 220.);
    let x = -band + (w + band) * e;
    let half = band / 2.;
    div().absolute().inset_0().rounded(px(radius)).overflow_hidden().bg(base)
        .child(div().absolute().top_0().bottom_0().left(px(x)).w(px(half)).bg(linear_gradient(90., linear_color_stop(clear, 0.), linear_color_stop(light, 1.))))
        .child(div().absolute().top_0().bottom_0().left(px(x + half)).w(px(half)).bg(linear_gradient(90., linear_color_stop(light, 0.), linear_color_stop(clear, 1.))))
        .when(w > 80. && h > 80., |d| d.child(div().absolute().inset_0().flex().items_center().justify_center()
            .child(ui::icon("image", 30., Theme::text_placeholder()).opacity(0.35 + 0.3 * breath))))
        // 动着的时候一直出帧。
        .child(canvas(|_, window, _| window.request_animation_frame(), |_, _, _, _| {}).absolute().size_0())
}

/// 停下了（失败、中断）：浅底上写一句为什么，点一下重试。
/// `chat`：画在助手对话里的（没有文档块），点一下照记录原样再排一次。
fn stopped(id: String, title: &'static str, detail: Option<String>, w: f32, h: f32, radius: f32, chat: bool) -> AnyElement {
    let roomy = w > 160. && h > 120.;
    div().absolute().inset_0().rounded(px(radius)).overflow_hidden().bg(Theme::fill_hover()).px(px(24.))
        .flex().flex_col().items_center().justify_center().gap(px(4.)).font_family(theme::UI_FONT).text_center()
        .when(roomy, |d| d.child(ui::icon("image-broken", 30., Theme::text_placeholder()).mb(px(6.))))
        .child(div().text_size(px(15.)).line_height(px(21.)).text_color(Theme::text_caption()).child(title))
        .children(detail.filter(|_| roomy).map(|e| div().max_w(px(280.)).text_size(px(13.)).line_height(px(18.)).text_color(Theme::text_placeholder()).max_h(px(36.)).overflow_hidden().child(e)))
        .child(div().mt(px(4.)).text_size(px(15.)).line_height(px(21.)).text_color(Theme::link()).child(tr("点按重试", "Tap to retry")))
        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
            cx.stop_propagation();
            window.prevent_default();
            if PRESSED.with(|p| p.take()).is_some_and(|t| t.elapsed() < Duration::from_secs(3)) { return; }
            super::glass::haptic();
            if chat { retry(&id, cx); return window.refresh(); }
            TARGET.with(|t| *t.borrow_mut() = Some(id.clone()));
            menu::chosen(RETRY);
        })
        .into_any_element()
}

/// 助手对话里的一张（`generate_image` 不给文档时）：画好了是图，点一下全屏看（那里能存到相册、分享）；
/// 在画是生成中的占位，失败了点一下重试。`w` 是可用的宽，高照成图的比例。
pub fn chat_picture(id: &str, w: f32) -> AnyElement {
    let (pw, ph) = size_of(id).unwrap_or((1, 1));
    let h = (w * ph as f32 / pw as f32).min(w * 1.5);
    let w = h * pw as f32 / ph as f32;
    const R: f32 = 12.;
    let inner = match status(id) {
        Status::Ready(path) => {
            let key = id.to_owned();
            let at = path.clone();
            div().id(SharedString::from(format!("margin-picture-{id}"))).absolute().inset_0().rounded(px(R)).overflow_hidden()
                .child(img(path).size_full().rounded(px(R)).object_fit(ObjectFit::Cover))
                .child(canvas(move |b, _, _| { FRAMES.with(|f| f.borrow_mut().insert(key, b)); }, |_, _, _, _| {}).absolute().inset_0())
                .on_click({ let id = id.to_owned(); move |_, _, _| if let Some(frame) = FRAMES.with(|f| f.borrow().get(&id).copied()) { super::glass::haptic(); super::coveredit::view(&at, frame, R as f64); } })
                .into_any_element()
        }
        Status::Busy => developing(w, h, R).into_any_element(),
        Status::Failed(e) => stopped(id.to_owned(), tr("生成失败", "Generation failed"), Some(e), w, h, R, true),
        Status::Missing => stopped(id.to_owned(), tr("找不到这张图", "Image missing"), None, w, h, R, true),
    };
    div().relative().flex_none().w(px(w)).h(px(h)).child(inner).into_any_element()
}

// ---- 长按菜单

/// 长按生成出来的图：和首页卡片同一套系统上下文菜单（`menu::context`，图浮起、其余压暗、菜单从图边上出来）。
/// 每帧按这张图现在的状态报一次菜单项：生成中只能取消，生成好了看提示词、重新生成，失败或中断时再加删除。
fn context(id: &str, radius: f32) -> AnyElement {
    RADII.with(|r| r.borrow_mut().insert(id.to_owned(), radius));
    let busy = JOBS.with(|j| j.borrow().get(id).is_some_and(|j| !matches!(j.state, State::Failed(_))));
    let prompt = Item::new(PROMPT, tr("查看提示词", "View Prompt"), "text.quote", 1);
    let edit = Item::new(EDIT, tr("编辑提示词", "Edit Prompt"), "pencil", 1);
    let (prompt, edit) = if known(id) { (prompt, edit) } else { (prompt.disabled(), edit.disabled()) };
    let items = if busy {
        vec![Item::new(CANCEL, tr("取消生成", "Cancel"), "xmark", 0).danger()]
    } else if png(id).is_file() {
        vec![Item::new(VIEW, tr("视图", "View"), "arrow.up.left.and.arrow.down.right", 0), Item::new(AGAIN, tr("重新生成", "Regenerate"), "arrow.clockwise", 1), prompt, edit]
    } else {
        vec![Item::new(AGAIN, tr("重新生成", "Regenerate"), "arrow.clockwise", 1), prompt, edit, Item::new(REMOVE, tr("删除", "Delete"), "trash", 2).danger()]
    };
    let key = id.to_owned();
    div().absolute().inset_0()
        .child(canvas(move |b, _, _| { FRAMES.with(|f| f.borrow_mut().insert(key, b)); }, |_, _, _, _| {}).absolute().inset_0())
        .child(menu::context(SharedString::from(format!("{KEY}{id}")), "", radius, items))
        .into_any_element()
}

/// 长按落在生成出来的图上（`lift_block` 先问这里）：菜单由系统弹，块不提起来、也不选词。
pub fn press(src: &str, at: Point<Pixels>, frame: Bounds<Pixels>) -> bool {
    if id_of(src).is_none() || !menu::context_at(at) { return false; }
    PRESSED.with(|p| p.set(Some(Instant::now())));
    if let Some(id) = id_of(src) { FRAMES.with(|f| f.borrow_mut().insert(id.to_owned(), frame)); }
    true
}

/// 菜单是对哪张图弹的。
fn pressed() -> Option<String> { menu::context_target().and_then(|t| t.strip_prefix(KEY).map(str::to_owned)) }

/// 这张图在文档里的块。
fn block_of(ed: &Editor, id: &str) -> Option<BlockId> {
    let src = src_of(id);
    ed.core.block_order(false).into_iter().find(|b| matches!(ed.core.block(*b).map(|x| &x.kind), Some(BlockKind::Image { src: s, .. }) if *s == src))
}

/// 查看提示词：系统的页面卡片，只读，分段写原文、人物、画风、实际发出的提示词和排除的词，最后一行模型、尺寸、种子。
/// 右上角「拷贝」拷实际发出的提示词。老的记录没有人物和画风，那两段不写。
fn show_prompt(id: &str) {
    let Some(r) = load(id) else { return toast(tr("找不到这张图的记录", "The record for this image is missing")) };
    let mut parts = vec![(tr("描述", "Description").to_owned(), r.input.clone())];
    if !r.people.is_empty() {
        let line = |(i, (n, t)): (usize, &(String, String))| match r.people_negative.get(i).filter(|x| !x.trim().is_empty()) {
            Some(x) => crate::localized_format!("{}：{}\n排除：{}", "{}: {}\nExclude: {}", n, t, x),
            None => crate::localized_format!("{}：{}", "{}: {}", n, t),
        };
        parts.push((tr("人物", "People").into(), r.people.iter().enumerate().map(line).collect::<Vec<_>>().join("\n\n")));
    }
    if r.planned { parts.push((tr("画面（优化后）", "Scene (optimized)").into(), r.plan_scene.clone())); }
    if !r.style.is_empty() { parts.push((crate::localized_format!("画风 · {}", "Style · {}", r.style), r.style_tags.clone())); }
    parts.push((tr("提示词", "Prompt").into(), r.prompt.clone()));
    parts.push((tr("排除", "Exclude").into(), r.negative.clone()));
    let meta = crate::localized_format!("{} · {} × {} · 种子 {}", "{} · {} × {} · Seed {}", image::model_name(&r.model), r.width, r.height, r.seed);
    parts.retain(|(_, t)| !t.trim().is_empty());
    parts.push((String::new(), meta));
    if !assistant::sheet::show(tr("提示词", "Prompt"), &parts, &r.prompt) { toast(tr("无法打开提示词", "Could not show the prompt")); }
}
