//! 写的工具：只作用于当前空间，对话的权限是「可编辑」时才有（只读时不在工具列表里，调了也不做）。
//!
//! - 正文：`create_document`（新建一篇，正文是方言写的 Markdown，或者从别的文档拷块）、`insert_blocks`（插到某块前后、开头或结尾）、
//!   `update_blocks`（一块改成一块或几块，空着是删掉）、`replace_text`（只换字，块的其他东西不动）；
//! - 文档库：`create_folder`、`move`（到文件夹或顶层）、`rename`、`trash`（到最近删除）、`restore`。
//!
//! `SPACE.md` 也能用读和改正文的三样（`insert_blocks`、`update_blocks`、`replace_text`）改：按 Markdown 读成块，改完写回 Markdown。
//!
//! 写的工具在主线程上做：宿主照 `Host` 落到文档库里（开着的那篇走编辑器，一步撤销、界面马上刷新；别的走文档库），
//! 做完刷新快照（`Host::refresh`），改过、新建的那篇用宿主写好的样子放回 `Shelf::library`，块标识和短号接着对得上；
//! 结果里用这次 run 的短号读出改过的块，接着改用得上。每样都记进改动日志（`changes`），可以回滚。
//! 几处一起改（`edits`）要么全改要么都不改。
//!
//! 创作：`generate_image` 照一句英文标签画一张（宿主套上助手的形象、现在的画风），给了 doc 就照 `insert_blocks` 的位置插一个图片块
//! （和插块一样记进改动日志，能回滚），不给就画在对话里。先插好块、再开始画（`Host::paint`）：位置不对不白花一次生图。
use std::sync::Arc;

use fdl_core::dialect::{self, Refs};
use fdl_core::model::{Block, BlockId, BlockKind, Document, RichText};
use serde_json::{Value, json};

use super::changes::{Changes, Op, print, print_text};
use super::tools::{self, SPACE_MD, arg_bool, arg_str, args, title, title_of};
use super::{Call, Shelf, clip};

/// 写的工具的名字。
pub const NAMES: [&str; 10] = ["create_document", "insert_blocks", "update_blocks", "replace_text", "create_folder", "move", "rename", "trash", "restore", "generate_image"];
/// `generate_image` 的画幅。
pub const RATIOS: [&str; 3] = ["1:1", "2:3", "3:2"];
pub fn is_write(name: &str) -> bool { NAMES.contains(&name) }

/// 一样东西现在在哪：上一层（`None` 顶层）、后一个（回去时排在它前面）；`local`：本机文件（文档库外面的，助手不动）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Place { pub parent: Option<String>, pub next: Option<String>, pub local: bool }

/// 要画的一张：宿主记下的标识、图片块的地址和像素宽高。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Picture { pub id: String, pub src: String, pub width: u32, pub height: u32 }

/// 宿主落到文档库里的办法（主线程上调）。出错是一句给人（和模型）看的话。
pub trait Host {
    /// 一篇现在的样子（编辑器里开着的用编辑器里的）；没有是 `None`。
    fn load(&mut self, id: &str) -> Option<Document>;
    /// 正文整篇换成 `blocks`（标题不动）。返回写好的样子（规整过、块标识照旧）和它在文档库里读回来的指纹（`changes::print`）。
    fn put_body(&mut self, id: &str, blocks: Vec<Block>) -> Result<(Document, u64), String>;
    /// `SPACE.md` 现在的原文（没有是空的）。
    fn guide(&mut self) -> String;
    fn save_guide(&mut self, text: &str) -> Result<(), String>;
    /// 新建一篇，挂在 `parent`（文件夹或文档）下面；返回标识。
    fn create(&mut self, doc: Document, parent: Option<&str>) -> Result<String, String>;
    fn create_folder(&mut self, name: &str, parent: Option<&str>) -> Result<String, String>;
    fn place(&mut self, id: &str) -> Option<Place>;
    /// 挂到 `parent` 下面（`None` 顶层），排在 `next` 前面（`None` 最后）。
    fn move_to(&mut self, id: &str, parent: Option<&str>, next: Option<&str>) -> Result<(), String>;
    fn rename(&mut self, id: &str, title: &str) -> Result<(), String>;
    /// 移到最近删除（子文档跟着）；返回真删了的那几个（子文档不单列）。
    fn trash(&mut self, ids: &[String]) -> Result<Vec<String>, String>;
    fn restore(&mut self, id: &str) -> Result<(), String>;
    /// 照文档库现在的样子刷新快照里的文档、文件夹和最近删除（正文的块不动）。
    fn refresh(&mut self, shelf: &mut Shelf);
    /// 记下要画的一张（`prompt` 是画面，`me` 时带上助手的形象），还不开始画。
    fn picture(&mut self, prompt: &str, ratio: &str, me: bool) -> Result<Picture, String>;
    /// 开始画：插进了 `doc`，或者（`None`）画在对话里。
    fn paint(&mut self, id: &str, doc: Option<&str>);
}

/// 工具的定义（OpenAI 的 `tools` 格式）。
pub fn defs() -> Value {
    let f = |name: &str, desc: &str, params: Value| json!({ "type": "function", "function": { "name": name, "description": desc, "parameters": params } });
    let s = |d: &str| json!({ "type": "string", "description": d });
    let ids = |d: &str| json!({ "type": "array", "items": { "type": "string" }, "description": d });
    let copy = json!({ "type": "object", "description": "从别的文档拷块（不改原文）：doc 是文档标识，blocks 是块号，不给就拷整篇",
        "properties": { "doc": s("文档标识"), "blocks": ids("块号，如 [\"b3\", \"b4\"]") }, "required": ["doc"] });
    json!([
        f("create_document", "新建一篇文档。正文用 markdown（和读出来的写法一样，新块不带块号），或者用 copy_from 从别的文档拷块。",
            json!({ "type": "object", "properties": {
                "title": s("标题"), "markdown": s("正文"), "copy_from": copy.clone(),
                "folder": s("放进哪个文件夹（标识或名字），不给放在顶层") }, "required": ["title"] })),
        f("insert_blocks", "往文档里插入新块：某块前面或后面、开头或结尾。内容用 markdown（新块不带块号），或者用 copy_from 拷块。doc 写 SPACE.md 改空间说明。",
            json!({ "type": "object", "properties": {
                "doc": s("文档标识，或 SPACE.md"), "markdown": s("要插入的内容"), "copy_from": copy,
                "position": s("before、after（都要给 block）、start 或 end（默认）"), "block": s("插在哪块前后：块号，如 b12") }, "required": ["doc"] })),
        f("update_blocks", "改写文档里的块：每处给块号和改成的 markdown。一块可以改成几块；markdown 空着就是删掉这块。块有子块的话连子块一起改写（子块带上它们的块号就保持原样）。<keep/> 原样留着就不动那块。",
            json!({ "type": "object", "properties": {
                "doc": s("文档标识，或 SPACE.md"),
                "edits": { "type": "array", "items": { "type": "object", "properties": { "block": s("块号"), "markdown": s("改成的内容，空着是删掉") }, "required": ["block", "markdown"] } } },
                "required": ["doc", "edits"] })),
        f("replace_text", "在文档里换字：old 换成 new，只改字，块的格式、样式都不动。old 是纯文字（不带 Markdown 记号），在全篇只能出现一次，出现多次时给 all=true 全换，或者把 old 写长一点。",
            json!({ "type": "object", "properties": {
                "doc": s("文档标识，或 SPACE.md"),
                "edits": { "type": "array", "items": { "type": "object", "properties": { "old": s("原来的字"), "new": s("换成的字"), "all": json!({ "type": "boolean", "description": "全部换掉" }) }, "required": ["old", "new"] } } },
                "required": ["doc", "edits"] })),
        f("create_folder", "新建文件夹。", json!({ "type": "object", "properties": { "name": s("名字"), "parent": s("放在哪个文件夹里（标识或名字），不给在顶层") }, "required": ["name"] })),
        f("move", "把文档或文件夹移到某个文件夹，或者移到顶层。", json!({ "type": "object", "properties": { "ids": ids("要移的文档或文件夹的标识"), "to": s("目标文件夹（标识或名字），不给是顶层") }, "required": ["ids"] })),
        f("rename", "给文档或文件夹改名。", json!({ "type": "object", "properties": { "id": s("文档或文件夹的标识"), "title": s("新名字") }, "required": ["id", "title"] })),
        f("trash", "把文档或文件夹移到最近删除（子文档跟着走，30 天内能恢复）。", json!({ "type": "object", "properties": { "ids": ids("标识") }, "required": ["ids"] })),
        f("restore", "从最近删除里恢复。", json!({ "type": "object", "properties": { "ids": ids("最近删除里的标识（list_trash 看）") }, "required": ["ids"] })),
        f("generate_image", "画一张图（NovelAI）。prompt 用英文的 Danbooru 标签写画面：人数、构图、动作、表情、场景、光线，逗号隔开；画风和质量词会自动加上，人物外貌不用写。给了 doc 就插进那篇（位置同 insert_blocks），不给就画在对话里。一张要等几十秒，会花用户的额度，不要一次画很多张。",
            json!({ "type": "object", "properties": {
                "prompt": s("画面的英文标签，如 1girl, sitting, reading book, window, sunlight"),
                "me": json!({ "type": "boolean", "description": "画面里有你自己时给 true，会用你的形象；不画你就不给" }),
                "ratio": s("画幅：1:1（默认）、2:3（竖）或 3:2（横）"),
                "doc": s("插进哪篇文档（标识），不给就画在对话里"),
                "position": s("before、after（都要给 block）、start 或 end（默认）"), "block": s("插在哪块前后：块号，如 b12") }, "required": ["prompt"] })),
    ])
}

/// 界面上这一步怎么写。
pub fn label(shelf: &Shelf, call: &Call) -> String {
    let a = args(call);
    let doc = || match doc_arg(&a, "doc") { d if d == SPACE_MD => SPACE_MD.to_owned(), d => format!("《{}》", clip(&title_of(shelf, &d), 20)) };
    let name = |id: &str| shelf.folder(id).map(|f| format!("「{}」", clip(&f.name, 20))).unwrap_or_else(|| format!("《{}》", clip(&title_of(shelf, id), 20)));
    let many = |k: &str| { let v = list_arg(&a, k); match v.len() { 1 => name(&v[0]), n => format!("{n} 项") } };
    match call.name.as_str() {
        "create_document" => format!("新建《{}》", clip(&title(&arg_str(&a, "title")), 20)),
        "insert_blocks" => format!("在{}里插入内容", doc()),
        "update_blocks" | "replace_text" => format!("修改{}", doc()),
        "create_folder" => format!("新建文件夹「{}」", clip(&arg_str(&a, "name"), 20)),
        "move" => format!("移动{}", many("ids")),
        "rename" => format!("把{}改名为《{}》", name(&arg_str(&a, "id")), clip(&arg_str(&a, "title"), 20)),
        "trash" => format!("删除{}", many("ids")),
        "restore" => format!("恢复{}", many("ids")),
        "generate_image" => if arg_str(&a, "doc").is_empty() { "画一张图".into() } else { format!("在{}里画一张图", doc()) },
        other => other.to_owned(),
    }
}

/// 执行一次写的调用，记进 `log`，返回给模型看的文字。
pub fn run(shelf: &mut Shelf, host: &mut dyn Host, log: &mut Changes, call: &Call) -> String {
    if shelf.permission != super::Permission::Write { return "错误：这段对话是只读的，改不了文档。要改的话请用户把权限换成「可编辑」。".into(); }
    let a = args(call);
    let out = match call.name.as_str() {
        "create_document" => create_document(shelf, host, log, &a),
        "insert_blocks" => insert_blocks(shelf, host, log, &a),
        "update_blocks" => update_blocks(shelf, host, log, &a),
        "replace_text" => replace_text(shelf, host, log, &a),
        "create_folder" => create_folder(shelf, host, log, &a),
        "move" => move_items(shelf, host, log, &a),
        "rename" => rename(shelf, host, log, &a),
        "trash" => trash(shelf, host, log, &a),
        "restore" => restore(shelf, host, log, &a),
        "generate_image" => generate_image(shelf, host, log, &a),
        other => Err(format!("没有叫 {other} 的写工具。")),
    };
    out.unwrap_or_else(|e| format!("错误：{e}"))
}

type Out = Result<String, String>;

/// 文档标识；`SPACE.md` 不分大小写。
fn doc_arg(a: &Value, k: &str) -> String { match arg_str(a, k) { d if d.eq_ignore_ascii_case(SPACE_MD) => SPACE_MD.to_owned(), d => d } }
/// 一串标识（只给了一个字符串也行）。
fn list_arg(a: &Value, k: &str) -> Vec<String> {
    match a.get(k) {
        Some(Value::Array(v)) => v.iter().filter_map(|x| x.as_str().map(|s| s.trim().to_owned())).filter(|s| !s.is_empty()).collect(),
        Some(Value::String(s)) if !s.trim().is_empty() => vec![s.trim().to_owned()],
        _ => vec![],
    }
}

/// 块号（b12，带不带方括号都行）→ 这篇里的块。
fn block_arg(refs: &Refs, doc: &str, s: &str) -> Result<BlockId, String> {
    let s = s.trim().trim_start_matches('[').trim_end_matches(']');
    let (d, id) = refs.resolve(s).ok_or_else(|| format!("没有块 {s}：块号要用这次读出来的（read_document 或 search 会给）。"))?;
    if d != doc { return Err(format!("块 {s} 不在这篇里，是另一篇文档的。")); }
    Ok(id)
}

/// 找块：它所在的那一排和位置（子块、表格格子里的都算）。
fn locate(bs: &mut Vec<Block>, id: BlockId) -> Option<(&mut Vec<Block>, usize)> {
    if let Some(i) = bs.iter().position(|b| b.id == id) { return Some((bs, i)); }
    for b in bs.iter_mut() {
        if let Some(r) = locate(&mut b.children, id) { return Some(r); }
        if let BlockKind::Table(t) = &mut b.kind {
            for row in &mut t.rows { for cell in row { if let Some(r) = locate(&mut cell.blocks, id) { return Some(r); } } }
        }
    }
    None
}
fn find(bs: &[Block], id: BlockId) -> Option<&Block> {
    for b in bs {
        if b.id == id { return Some(b); }
        if let Some(x) = find(&b.children, id) { return Some(x); }
        if let BlockKind::Table(t) = &b.kind { for row in &t.rows { for cell in row { if let Some(x) = find(&cell.blocks, id) { return Some(x); } } } }
    }
    None
}
fn holds(b: &Block, id: BlockId) -> bool { find(std::slice::from_ref(b), id).is_some() }

/// 正文的纯文字（快照里的 `Doc::text`）。
fn text_of(doc: &Document) -> String {
    let mut s = String::new();
    doc.for_each_block(|b| { let t = b.text.plain_text(); if !t.trim().is_empty() { s += t.trim(); s.push('\n'); } });
    s
}

/// 方言写回出错，照这次是在插新内容（`insert`）还是改一块说清楚。
fn dialect_error(e: dialect::Error, insert: bool) -> String {
    match e {
        dialect::Error::Unknown(s) | dialect::Error::Gone(s) if insert => format!("新内容里不要带块号（{s}）：块号只用来指位置；要拷贝已有的块用 copy_from。"),
        dialect::Error::Gone(s) => format!("块 {s} 不在要改的这块里：markdown 里只能带这块和它子块的块号，别处的块不要带块号（要拷贝用 insert_blocks 的 copy_from）。"),
        e => e.to_string() + "。",
    }
}

/// 要放进去的块：`markdown`（方言）或 `copy_from`（拷的块换新标识），只能给一样。
fn content(shelf: &Shelf, a: &Value) -> Result<Vec<Block>, String> {
    let md = a.get("markdown").and_then(Value::as_str).unwrap_or_default();
    let copy = a.get("copy_from").filter(|c| !c.is_null() && c.as_str() != Some(""));
    match (md.trim().is_empty(), copy) {
        (false, Some(_)) => Err("markdown 和 copy_from 只能给一样。".into()),
        (false, None) => dialect::write("", md, &[], &mut shelf.refs.lock().unwrap()).map_err(|e| dialect_error(e, true)),
        (true, Some(c)) => {
            let src = match c { Value::String(s) => s.trim().to_owned(), c => doc_arg(c, "doc") };
            let doc = if src == SPACE_MD { shelf.space_doc() } else { tools::load(shelf, &src)? };
            let picks = list_arg(c, "blocks");
            if picks.is_empty() { return Ok(doc.blocks.iter().map(Block::fresh_ids).collect()); }
            let refs = shelf.refs.lock().unwrap();
            picks.iter().map(|p| {
                let id = block_arg(&refs, &src, p)?;
                find(&doc.blocks, id).map(Block::fresh_ids).ok_or_else(|| format!("块 {p} 已经不在那篇里了，重新读一遍。"))
            }).collect()
        }
        (true, None) => Ok(vec![]),
    }
}

/// 读出这几块（带短号），给模型接着用。
fn report(shelf: &Shelf, id: &str, doc: &Document, touched: &[BlockId]) -> String {
    let mut refs = shelf.refs.lock().unwrap();
    let parts: Vec<String> = touched.iter().filter_map(|t| find(&doc.blocks, *t)).map(|b| dialect::read(id, std::slice::from_ref(b), &mut refs)).collect();
    clip(&parts.join("\n"), 4000)
}

/// 改一篇的正文（或 `SPACE.md`）：`f` 在块的拷贝上改，返回改出来的块（报给模型的）。改之前核对快照还是现在的样子。
fn edit(shelf: &mut Shelf, host: &mut dyn Host, log: &mut Changes, id: &str, verb: &str, f: impl FnOnce(&mut Vec<Block>, &mut Refs) -> Result<Vec<BlockId>, String>) -> Out {
    if id.is_empty() { return Err("没有给 doc（文档标识）。".into()); }
    let space = id == SPACE_MD;
    let (base, before, name) = if space {
        let now = host.guide();
        if now != shelf.guide {
            shelf.guide = now;
            shelf.library.forget(SPACE_MD);
            return Err("SPACE.md 刚被改过，先 read_document(\"SPACE.md\") 重新读一遍再改。".into());
        }
        (shelf.space_doc(), now, SPACE_MD.to_owned())
    } else {
        let base = tools::load(shelf, id)?;
        let now = host.load(id).ok_or_else(|| format!("《{}》现在读不出来（可能已经删了，或者还没从 iCloud 下载完）。", title_of(shelf, id)))?;
        // 和快照（或这次刚写好、读回来的样子）一样才改。
        let fp = print(&now);
        if fp != print(&base) && Some(fp) != log.after(id) {
            shelf.library.put(id, now);
            return Err(format!("《{}》刚被改过（可能是用户在编辑器里改的），先用 read_document 重新读一遍再改。", title_of(shelf, id)));
        }
        (base, fdl_core::docxml::serialize(&now), title_of(shelf, id))
    };
    let mut blocks = base.blocks.clone();
    let touched = f(&mut blocks, &mut shelf.refs.lock().unwrap())?;
    let saved = if space {
        let doc = Document { blocks, ..(*base).clone() };
        let text = fdl_core::markdown::export(&doc, &fdl_core::markdown::Options { in_place: true, ..Default::default() }).text;
        host.save_guide(&text)?;
        log.push(Op::Edit { id: SPACE_MD.into(), title: SPACE_MD.into(), before, after: print_text(&text) });
        shelf.guide = text;
        doc
    } else {
        let (doc, after) = host.put_body(id, blocks)?;
        log.push(Op::Edit { id: id.into(), title: name.clone(), before, after });
        host.refresh(shelf);
        if let Some(d) = shelf.docs.iter_mut().find(|d| d.id == id) { d.text = Arc::from(text_of(&doc)); }
        doc
    };
    shelf.library.put(id, saved.clone());
    let what = if space { SPACE_MD.to_owned() } else { format!("《{}》", title(&name)) };
    let body = report(shelf, id, &saved, &touched);
    Ok(if body.trim().is_empty() { format!("已{verb}{what}。") } else { format!("已{verb}{what}。现在这几块是：\n\n{body}") })
}

fn insert_blocks(shelf: &mut Shelf, host: &mut dyn Host, log: &mut Changes, a: &Value) -> Out {
    let id = doc_arg(a, "doc");
    let new = content(shelf, a)?;
    if new.is_empty() { return Err("没有要插入的内容：给 markdown 或 copy_from。".into()); }
    let (position, at) = position_arg(a)?;
    let ids: Vec<BlockId> = new.iter().map(|b| b.id).collect();
    edit(shelf, host, log, &id, "插入到", |blocks, refs| { put_at(blocks, refs, &id, &position, &at, new)?; Ok(ids) })
}

/// 插在哪：`position`（默认 end）和 before、after 要的块号。
fn position_arg(a: &Value) -> Result<(String, String), String> {
    let position = match arg_str(a, "position").to_lowercase() { p if p.is_empty() => "end".to_owned(), p => p };
    if !["start", "end", "before", "after"].contains(&position.as_str()) { return Err(format!("position 只能是 before、after、start 或 end，不是 {position}。")); }
    let at = arg_str(a, "block");
    if matches!(position.as_str(), "before" | "after") && at.is_empty() { return Err(format!("position 是 {position} 时要给 block（块号）。")); }
    Ok((position, at))
}

/// 照位置把新块放进正文。
fn put_at(blocks: &mut Vec<Block>, refs: &mut Refs, doc: &str, position: &str, at: &str, new: Vec<Block>) -> Result<(), String> {
    match position {
        "start" => { blocks.splice(0..0, new); }
        "end" => {
            // 只有一个空段落的新文档：换掉它。
            if blocks.len() == 1 && blocks[0].kind == BlockKind::Text && blocks[0].text.is_empty() && blocks[0].children.is_empty() { blocks.clear(); }
            blocks.extend(new);
        }
        p => {
            let target = block_arg(refs, doc, at)?;
            let (row, i) = locate(blocks, target).ok_or_else(|| format!("块 {at} 已经不在这篇里了，重新读一遍。"))?;
            let i = if p == "after" { i + 1 } else { i };
            row.splice(i..i, new);
        }
    }
    Ok(())
}

/// 画一张：先查参数，插进文档的先插块（记进日志），都成了才开始画。
fn generate_image(shelf: &mut Shelf, host: &mut dyn Host, log: &mut Changes, a: &Value) -> Out {
    let prompt = arg_str(a, "prompt").trim().to_owned();
    if prompt.is_empty() { return Err("prompt 是空的：用英文标签写画面，如 1girl, reading book, window。".into()); }
    if prompt.chars().count() > 1000 { return Err("prompt 太长了，1000 字以内。".into()); }
    let ratio = match arg_str(a, "ratio") { r if r.is_empty() => "1:1".to_owned(), r => r };
    if !RATIOS.contains(&ratio.as_str()) { return Err(format!("ratio 只能是 1:1、2:3 或 3:2，不是 {ratio}。")); }
    let me = arg_bool(a, "me");
    let id = doc_arg(a, "doc");
    if id.is_empty() {
        let pic = host.picture(&prompt, &ratio, me)?;
        host.paint(&pic.id, None);
        return Ok(format!("开始画了（{ratio}），画好会显示在对话里。"));
    }
    if id == SPACE_MD { return Err("SPACE.md 里不放图：不给 doc 就画在对话里。".into()); }
    tools::load(shelf, &id)?;
    let (position, at) = position_arg(a)?;
    if !at.is_empty() && matches!(position.as_str(), "before" | "after") { block_arg(&shelf.refs.lock().unwrap(), &id, &at)?; }
    let pic = host.picture(&prompt, &ratio, me)?;
    let mut b = Block::new(BlockKind::Image { src_kind: "src".into(), src: pic.src.clone(), width: Some(pic.width as f32), height: Some(pic.height as f32), caption: None, crop: None }, RichText::default());
    b.align = fdl_core::model::Align::Center;
    let new = b.id;
    let out = edit(shelf, host, log, &id, "插入图片到", |blocks, refs| { put_at(blocks, refs, &id, &position, &at, vec![b])?; Ok(vec![new]) })?;
    host.paint(&pic.id, Some(&id));
    Ok(out)
}

/// 行首带没带块号（`[b12]`，前面可以有缩进）。
fn has_ref(line: &str) -> bool {
    let t = line.trim_start();
    t.strip_prefix("[b").is_some_and(|r| { let n = r.bytes().take_while(u8::is_ascii_digit).count(); n > 0 && r[n..].starts_with(']') })
}

fn update_blocks(shelf: &mut Shelf, host: &mut dyn Host, log: &mut Changes, a: &Value) -> Out {
    let id = doc_arg(a, "doc");
    let edits: Vec<(String, String)> = a.get("edits").and_then(Value::as_array).map_or_else(Vec::new, |v| v.iter().map(|e| (arg_str(e, "block"), e.get("markdown").and_then(Value::as_str).unwrap_or_default().to_owned())).collect());
    if edits.is_empty() { return Err("edits 是空的：每处给 block（块号）和 markdown。".into()); }
    edit(shelf, host, log, &id, "修改", |blocks, refs| {
        let mut targets: Vec<BlockId> = vec![];
        for (b, _) in &edits {
            if b.is_empty() { return Err("有一处没给 block（块号）。".into()); }
            let t = block_arg(refs, &id, b)?;
            let block = find(blocks, t).ok_or_else(|| format!("块 {b} 已经不在这篇里了，重新读一遍。"))?;
            if targets.iter().any(|o| holds(block, *o) || find(blocks, *o).is_some_and(|x| holds(x, t))) {
                return Err(format!("块 {b} 和别的几处重了（同一块、或者一块是另一块的子块）：一块只改一次，改父块时子块一起写在它的 markdown 里。"));
            }
            targets.push(t);
        }
        let mut touched = vec![];
        for ((b, md), t) in edits.iter().zip(targets) {
            let (row, i) = locate(blocks, t).ok_or_else(|| format!("块 {b} 已经不在这篇里了，重新读一遍。"))?;
            if md.trim().is_empty() { row.remove(i); continue; }
            // 第一行没带块号：就是改这一块（留住它的标识和看不见的属性）。
            let mut lines: Vec<String> = md.lines().map(str::to_owned).collect();
            if let Some(line) = lines.iter_mut().find(|l| !l.trim().is_empty()) && !has_ref(line) {
                let indent = line.len() - line.trim_start().len();
                line.insert_str(indent, &format!("[{}] ", b.trim().trim_start_matches('[').trim_end_matches(']')));
            }
            let text = lines.join("\n");
            let orig = row[i].clone();
            let new = dialect::write(&id, &text, std::slice::from_ref(&orig), refs).map_err(|e| dialect_error(e, false))?;
            touched.extend(new.iter().map(|b| b.id));
            row.splice(i..i + 1, new);
        }
        Ok(touched)
    })
}

fn replace_text(shelf: &mut Shelf, host: &mut dyn Host, log: &mut Changes, a: &Value) -> Out {
    let id = doc_arg(a, "doc");
    let edits: Vec<(String, String, bool)> = a.get("edits").and_then(Value::as_array).map_or_else(Vec::new, |v| v.iter().map(|e| (e.get("old").and_then(Value::as_str).unwrap_or_default().to_owned(), e.get("new").and_then(Value::as_str).unwrap_or_default().to_owned(), arg_bool(e, "all"))).collect());
    if edits.is_empty() { return Err("edits 是空的：每处给 old 和 new。".into()); }
    let mut count = 0;
    let r = edit(shelf, host, log, &id, "修改", |blocks, refs| {
        fn walk(bs: &mut [Block], f: &mut dyn FnMut(&mut Block)) {
            for b in bs {
                f(b);
                walk(&mut b.children, f);
                if let BlockKind::Table(t) = &mut b.kind { for row in &mut t.rows { for cell in row { walk(&mut cell.blocks, f); } } }
            }
        }
        let mut touched: Vec<BlockId> = vec![];
        for (old, new, all) in &edits {
            if old.is_empty() { return Err("old 是空的。".into()); }
            let mut hits: Vec<BlockId> = vec![];
            walk(blocks, &mut |b| { for _ in b.text.text.matches(old.as_str()) { hits.push(b.id); } });
            if hits.is_empty() { return Err(format!("没找到「{}」。old 要写纯文字（不带 Markdown 记号、块号），也不能跨块；先 read_document 或 search 看看原文。", clip(old, 40))); }
            if hits.len() > 1 && !all {
                let mut where_: Vec<String> = hits.iter().map(|h| refs.mark(&id, *h)).collect();
                where_.dedup();
                return Err(format!("「{}」出现了 {} 次（在 {}）：全换给 all=true，只换一处就把 old 写长一点。", clip(old, 40), hits.len(), where_.join("、")));
            }
            walk(blocks, &mut |b| {
                if !b.text.text.contains(old.as_str()) { return; }
                let starts: Vec<usize> = b.text.text.match_indices(old.as_str()).map(|(i, _)| i).collect();
                for s in starts.into_iter().rev() {
                    let mut marks = b.text.marks_at(s);
                    marks.inline = None;
                    let mut ins = RichText::default();
                    ins.push(new, marks);
                    b.text.replace(s..s + old.len(), &ins);
                    count += 1;
                }
                if !touched.contains(&b.id) { touched.push(b.id); }
            });
        }
        Ok(touched)
    });
    r.map(|s| s.replacen("。", &format!("（换了 {count} 处）。"), 1))
}

fn create_document(shelf: &mut Shelf, host: &mut dyn Host, log: &mut Changes, a: &Value) -> Out {
    let name = arg_str(a, "title");
    let parent = match arg_str(a, "folder") { f if f.is_empty() => None, f => Some(tools::resolve(shelf, &f)?) };
    let mut blocks = content(shelf, a)?;
    if blocks.is_empty() { blocks.push(Block::text("")); }
    let doc = Document { title: RichText::plain(name.clone()), icon: None, cover: None, blocks };
    let id = host.create(doc.clone(), parent.as_deref())?;
    let after = host.load(&id).map_or_else(|| print(&doc), |d| print(&d));
    log.push(Op::Create { id: id.clone(), title: name.clone(), folder: false, after });
    host.refresh(shelf);
    if let Some(d) = shelf.docs.iter_mut().find(|d| d.id == id) { d.text = Arc::from(text_of(&doc)); }
    shelf.library.put(&id, doc.clone());
    let place = parent.map(|p| format!("，在「{}」里", shelf.folder(&p).map(|f| shelf.path(&f.id)).unwrap_or_else(|| title_of(shelf, &p)))).unwrap_or_default();
    let all: Vec<BlockId> = doc.blocks.iter().map(|b| b.id).collect();
    let body = report(shelf, &id, &doc, &all);
    Ok(format!("已新建《{}》(id: {id}){place}。{}", title(&name), if body.trim().is_empty() { String::new() } else { format!("正文：\n\n{body}") }))
}

/// 目标文件夹（只能是文件夹）。
fn folder_arg(shelf: &Shelf, f: &str) -> Result<String, String> {
    let id = tools::resolve(shelf, f)?;
    if shelf.folder(&id).is_none() { return Err(format!("「{f}」是文档，不是文件夹。")); }
    Ok(id)
}

fn create_folder(shelf: &mut Shelf, host: &mut dyn Host, log: &mut Changes, a: &Value) -> Out {
    let name = arg_str(a, "name");
    if name.is_empty() { return Err("没有给文件夹的名字。".into()); }
    let parent = match arg_str(a, "parent") { p if p.is_empty() => None, p => Some(folder_arg(shelf, &p)?) };
    let id = host.create_folder(&name, parent.as_deref())?;
    log.push(Op::Create { id: id.clone(), title: name.clone(), folder: true, after: 0 });
    host.refresh(shelf);
    Ok(format!("已新建文件夹「{name}」(id: {id})。"))
}

/// 文档或文件夹的名字（给人看）。
fn name_of(shelf: &Shelf, id: &str) -> String { shelf.folder(id).map(|f| format!("文件夹「{}」", f.name)).unwrap_or_else(|| format!("《{}》", title_of(shelf, id))) }
fn plain_name(shelf: &Shelf, id: &str) -> String { shelf.folder(id).map(|f| f.name.clone()).or_else(|| shelf.find(id).map(|d| d.title.clone())).unwrap_or_default() }
/// 文档库里有的、能动的（不是本机文件）。
fn movable(shelf: &Shelf, host: &mut dyn Host, id: &str) -> Result<Place, String> {
    if shelf.find(id).is_none() && shelf.folder(id).is_none() { return Err(format!("没有标识为 {id} 的文档或文件夹。")); }
    let p = host.place(id).ok_or_else(|| format!("{} 现在找不到了。", name_of(shelf, id)))?;
    if p.local { return Err(format!("{} 是本机文件（在文档库外面），助手不移动、不改名、不删除它。", name_of(shelf, id))); }
    Ok(p)
}

fn move_items(shelf: &mut Shelf, host: &mut dyn Host, log: &mut Changes, a: &Value) -> Out {
    let ids = list_arg(a, "ids");
    if ids.is_empty() { return Err("没有给 ids。".into()); }
    let to = match arg_str(a, "to") { t if t.is_empty() || t == "顶层" || t.eq_ignore_ascii_case("root") || t.eq_ignore_ascii_case("top") => None, t => Some(folder_arg(shelf, &t)?) };
    let dest = to.as_deref().map_or_else(|| "顶层".to_owned(), |t| format!("「{}」", shelf.path(t)));
    let (mut done, mut lines) = (0, vec![]);
    for id in &ids {
        let r = movable(shelf, host, id).and_then(|p| {
            if p.parent == to { return Ok(format!("{} 本来就在{dest}", name_of(shelf, id))); }
            if to.as_deref().is_some_and(|t| t == id || shelf.inside(t, id)) { return Err(format!("{} 不能移到它自己里面", name_of(shelf, id))); }
            host.move_to(id, to.as_deref(), None)?;
            log.push(Op::Move { id: id.clone(), title: plain_name(shelf, id), parent: p.parent, next: p.next });
            done += 1;
            Ok(format!("{} 已移到{dest}", name_of(shelf, id)))
        });
        lines.push(r.unwrap_or_else(|e| format!("没移：{e}")));
    }
    if done > 0 { host.refresh(shelf); }
    if done == 0 && lines.iter().all(|l| l.starts_with("没移")) { return Err(lines.join("；")); }
    Ok(lines.join("\n"))
}

fn rename(shelf: &mut Shelf, host: &mut dyn Host, log: &mut Changes, a: &Value) -> Out {
    let (id, new) = (arg_str(a, "id"), arg_str(a, "title"));
    if new.is_empty() { return Err("新名字是空的。".into()); }
    movable(shelf, host, &id)?;
    let was = plain_name(shelf, &id);
    if was.trim() == new { return Ok(format!("名字本来就是「{new}」。")); }
    let before = name_of(shelf, &id);
    host.rename(&id, &new)?;
    log.push(Op::Rename { id: id.clone(), title: new.clone(), was });
    host.refresh(shelf);
    if let Some(d) = shelf.library.cached(&id) { shelf.library.put(&id, Document { title: RichText::plain(new.clone()), ..(*d).clone() }); }
    Ok(format!("已把{before}改名为「{new}」。"))
}

fn trash(shelf: &mut Shelf, host: &mut dyn Host, log: &mut Changes, a: &Value) -> Out {
    let ids = list_arg(a, "ids");
    if ids.is_empty() { return Err("没有给 ids。".into()); }
    let (mut ok, mut lines) = (vec![], vec![]);
    for id in &ids { match movable(shelf, host, id) { Ok(_) => ok.push(id.clone()), Err(e) => lines.push(format!("没删：{e}")) } }
    if ok.is_empty() { return Err(lines.join("；")); }
    let names: Vec<(String, String, String)> = ok.iter().map(|id| (id.clone(), plain_name(shelf, id), name_of(shelf, id))).collect();
    let gone = host.trash(&ok)?;
    let mut out = vec![];
    for (id, title, name) in names {
        if gone.contains(&id) { log.push(Op::Trash { id, title }); out.push(format!("{name} 已移到最近删除（30 天内能恢复）")); }
        else { out.push(format!("{name} 跟着上一层一起删了")); }
    }
    out.extend(lines);
    let lines = out;
    host.refresh(shelf);
    Ok(lines.join("\n"))
}

fn restore(shelf: &mut Shelf, host: &mut dyn Host, log: &mut Changes, a: &Value) -> Out {
    let ids = list_arg(a, "ids");
    if ids.is_empty() { return Err("没有给 ids。".into()); }
    let (mut done, mut lines) = (0, vec![]);
    for id in &ids {
        let Some(t) = shelf.trash.iter().find(|t| t.id == *id).cloned() else { lines.push(format!("没恢复：最近删除里没有 {id}（用 list_trash 看）")); continue };
        match host.restore(id) {
            Ok(()) => { log.push(Op::Restore { id: id.clone(), title: t.title.clone() }); done += 1; lines.push(format!("《{}》已恢复", title(&t.title))); }
            Err(e) => lines.push(format!("没恢复《{}》：{e}", title(&t.title))),
        }
    }
    if done > 0 { host.refresh(shelf); }
    if done == 0 { return Err(lines.join("；")); }
    Ok(lines.join("\n"))
}
