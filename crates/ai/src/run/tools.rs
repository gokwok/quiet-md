//! 助手的能力里给 Agent 用的那几样：只读的文档工具，都只作用于当前空间。都在 run 开始时的快照（`Shelf`）上做，
//! 正文的块现读（`Shelf::document`，读过的留着），不改文档库。
//!
//! - `list_folder`：不给文件夹是空间总览（篇数、常用标签、顶层的文件夹和文档），给了列它下面的；
//! - `search`：跨文档找篇（关键词可省，只按文件夹、标签、时间筛也行），给了 `doc` 在这篇里找块，带前后几块；
//! - `read_document`：读一篇，正文用 `fdl_core::dialect` 读出、每块带短号，分页；`outline` 只给标题；`SPACE.md` 是空间说明全文；
//! - `list_tasks`：待办和行内提醒；`backlinks`：哪些文档 @ 了这篇；`list_trash`：最近删除。
//!
//! 块短号一次 run 里共用一份（`Shelf::refs`），哪个工具报的都一样。结果是给模型看的纯文字，太长就分页或截；
//! 出错也是一句话，说清楚怎么改。界面上每次调用显示成一步（`label`）。
use std::collections::HashMap;

use fdl_core::model::{Block, BlockKind, Document, Inline};
use serde_json::{Value, json};

use super::{Call, Doc, Shelf, clip};

/// 一页最多几项（文件夹、文档、待办）。
pub const ITEMS: usize = 30;
/// `read_document` 一次大约读多少字。
pub const PAGE: usize = 6000;
/// 空间说明的特殊标识。
pub const SPACE_MD: &str = "SPACE.md";

/// 工具的定义（OpenAI 的 `tools` 格式）。
pub fn defs() -> Value {
    let f = |name: &str, desc: &str, params: Value| json!({ "type": "function", "function": { "name": name, "description": desc, "parameters": params } });
    let s = |d: &str| json!({ "type": "string", "description": d });
    let n = |d: &str| json!({ "type": "integer", "description": d });
    json!([
        f("list_folder", "列出文件夹里的子文件夹和文档（每篇带标签和开头约 100 字）。不给 folder 是空间总览：篇数、常用标签、顶层的文件夹和文档。",
            json!({ "type": "object", "properties": {
                "folder": s("文件夹的标识或名字；也可以是文档标识，列它下面的子文档。不给是整个空间"),
                "depth": n("往下展开几层子文件夹，1–3，默认 1"),
                "sort": s("modified（默认，最近改的在前）、created 或 title"),
                "cursor": s("上次结果末尾给的 cursor，接着往下列") } })),
        f("search", "搜索。不给 doc：跨文档找篇，返回命中的片段；关键词可省，只按文件夹、标签、时间筛（比如「上周写了什么」）。给了 doc：在这篇里找块，带前后几块（块带短号）。",
            json!({ "type": "object", "properties": {
                "query": s("关键词，可省"),
                "doc": s("只在这篇文档里找（文档标识）"),
                "folder": s("只找这个文件夹里的（标识或名字，含子文件夹）"),
                "tag": s("只找带这个标签的（不带 #）"),
                "since": s("修改时间不早于，2026-11-20 或 2026-11-20 09:00"),
                "until": s("修改时间不晚于，格式同上；只写日期算到那天结束"),
                "title_only": json!({ "type": "boolean", "description": "只在标题里找" }),
                "sort": s("relevance（有关键词时默认）、modified（默认）、created 或 title"),
                "limit": n("最多几条，默认 10，最多 30"),
                "context": n("给了 doc 时命中块前后各带几块，默认 2") } })),
        f("read_document", "读一篇文档：标题、文件夹、标签、修改时间和正文。正文每块前有块号 [b12]（引用、以后改文档都用它），表达不了的块是 <keep ref=\"b12\"/>。长文分页。id 写 SPACE.md 读空间说明的全文。",
            json!({ "type": "object", "properties": {
                "id": s("文档标识，或 SPACE.md"),
                "from_block": s("从哪块开始：块号（b12）或第几块（0 起，上次结果末尾会说），默认从头"),
                "limit": n("最多读几块（顶层的块），默认读到约 6000 字"),
                "outline": json!({ "type": "boolean", "description": "只要标题结构（带块号）" }) }, "required": ["id"] })),
        f("list_tasks", "列出待办块和行内提醒，带所在文档和块号。",
            json!({ "type": "object", "properties": {
                "status": s("open（默认，没做完的）、done 或 all"),
                "folder": s("只看这个文件夹里的（标识或名字）"),
                "due_before": s("只要提醒时间早于这个的，2026-11-30 或 2026-11-30 18:00") } })),
        f("backlinks", "哪些文档 @ 了这篇，带那一行。",
            json!({ "type": "object", "properties": { "id": s("文档标识") }, "required": ["id"] })),
        f("list_trash", "最近删除里有什么（删了 30 天内的）。",
            json!({ "type": "object", "properties": { "limit": n("最多几项，默认 20") } })),
    ])
}

fn args(call: &Call) -> Value { serde_json::from_str(&call.args).unwrap_or(Value::Null) }
fn arg_str(v: &Value, k: &str) -> String { v.get(k).and_then(|x| x.as_str().map(str::to_owned).or_else(|| x.as_u64().map(|n| n.to_string()))).unwrap_or_default().trim().to_owned() }
fn arg_num(v: &Value, k: &str) -> Option<usize> { v.get(k).and_then(|x| x.as_u64().or_else(|| x.as_str()?.trim().parse().ok())).map(|n| n as usize) }
fn arg_bool(v: &Value, k: &str) -> bool { v.get(k).is_some_and(|x| x.as_bool().unwrap_or_else(|| x.as_str() == Some("true"))) }

fn title(t: &str) -> String { if t.trim().is_empty() { "无标题".into() } else { t.trim().to_owned() } }
fn title_of(shelf: &Shelf, id: &str) -> String { shelf.find(id).map(|d| title(&d.title)).unwrap_or_else(|| id.to_owned()) }
fn date(secs: u64) -> String { let (y, m, d, _, _) = fdl_core::time::civil(secs); format!("{y}-{m:02}-{d:02}") }
fn datetime(secs: u64) -> String { let (y, m, d, h, mi) = fdl_core::time::civil(secs); format!("{y}-{m:02}-{d:02} {h:02}:{mi:02}") }

/// 2026-11-20 或 2026-11-20 09:00（本地时间）→ unix 秒；`end`：只有日期时取那天的最后一秒。
fn parse_time(s: &str, end: bool) -> Option<u64> {
    let (d, t) = match s.trim().split_once([' ', 'T']) { Some((d, t)) => (d, Some(t.trim())), None => (s.trim(), None) };
    let mut p = d.split(['-', '/', '.']);
    let (y, m, day): (i64, u32, u32) = (p.next()?.parse().ok()?, p.next()?.parse().ok()?, p.next()?.parse().ok()?);
    if !(1..=12).contains(&m) || day == 0 || day > fdl_core::time::days_in_month(y, m) { return None; }
    let (h, mi) = match t { Some(t) => { let (h, mi) = t.split_once(':')?; (h.parse().ok()?, mi.get(..2).unwrap_or(mi).parse().ok()?) } None => (0, 0) };
    if h > 23 || mi > 59 { return None; }
    let at = fdl_core::time::local_to_unix(y, m, day, h, mi) + if t.is_none() && end { 86_399 } else { 0 };
    u64::try_from(at).ok()
}

/// 界面上这一步怎么写。
pub fn label(shelf: &Shelf, call: &Call) -> String {
    let a = args(call);
    match call.name.as_str() {
        "list_folder" => match arg_str(&a, "folder") { f if f.is_empty() => "查看空间".into(), f => format!("查看「{}」", clip(&folder_name(shelf, &f), 20)) },
        "search" => {
            let q = arg_str(&a, "query");
            match (arg_str(&a, "doc"), q.is_empty()) {
                (d, false) if !d.is_empty() => format!("在《{}》里找「{}」", clip(&title_of(shelf, &d), 16), clip(&q, 16)),
                (_, false) => format!("搜索「{}」", clip(&q, 20)),
                _ => "筛选文档".into(),
            }
        }
        "read_document" => match arg_str(&a, "id") { id if id.eq_ignore_ascii_case(SPACE_MD) => "读取 SPACE.md".into(), id => format!("读取《{}》", clip(&title_of(shelf, &id), 20)) },
        "list_tasks" => "查看待办".into(),
        "backlinks" => format!("查看谁链接了《{}》", clip(&title_of(shelf, &arg_str(&a, "id")), 16)),
        "list_trash" => "查看最近删除".into(),
        other => other.to_owned(),
    }
}

/// 执行一次调用，返回给模型看的文字。
pub fn run(shelf: &Shelf, call: &Call) -> String {
    let a = args(call);
    let out = match call.name.as_str() {
        "list_folder" => list_folder(shelf, &a),
        "search" => search(shelf, &a),
        "read_document" => read_document(shelf, &a),
        "list_tasks" => list_tasks(shelf, &a),
        "backlinks" => backlinks(shelf, &a),
        "list_trash" => list_trash(shelf, &a),
        other => Err(format!("没有叫 {other} 的工具。能用的是 list_folder、search、read_document、list_tasks、backlinks、list_trash。")),
    };
    out.unwrap_or_else(|e| format!("错误：{e}"))
}

type Out = Result<String, String>;

// ---- 文件夹和文档

/// 文件夹：标识，或名字（一样的优先，再是含这几个字的）；也可以是一篇文档（它的子文档）。
fn resolve(shelf: &Shelf, f: &str) -> Result<String, String> {
    if shelf.folder(f).is_some() || shelf.find(f).is_some() { return Ok(f.to_owned()); }
    let l = f.to_lowercase();
    let exact = shelf.folders.iter().find(|x| x.name.trim().to_lowercase() == l);
    let near = || shelf.folders.iter().find(|x| x.name.to_lowercase().contains(&l));
    exact.or_else(near).map(|x| x.id.clone()).ok_or_else(|| format!("没有叫「{f}」的文件夹。用 list_folder（不带参数）看有哪些文件夹。"))
}

fn folder_name(shelf: &Shelf, f: &str) -> String {
    match resolve(shelf, f) { Ok(id) => shelf.folder(&id).map(|x| x.name.clone()).unwrap_or_else(|| title_of(shelf, &id)), Err(_) => f.to_owned() }
}

fn sorted<'a>(mut docs: Vec<&'a Doc>, sort: &str) -> Vec<&'a Doc> {
    match sort {
        "title" => docs.sort_by_key(|d| title(&d.title).to_lowercase()),
        "created" => docs.sort_by_key(|d| std::cmp::Reverse(d.created)),
        _ => docs.sort_by_key(|d| std::cmp::Reverse(d.modified)),
    }
    docs
}

/// 一篇在列表里的一行：标识、标题、所在、标签、修改日期，下一行是开头约 100 字。
fn doc_line(shelf: &Shelf, d: &Doc, indent: &str) -> String {
    let mut s = format!("{indent}- 《{}》 id: {}", title(&d.title), d.id);
    if let Some(p) = d.parent.as_deref().filter(|p| shelf.folder(p).is_some()) { s += &format!(" | 文件夹：{}", shelf.path(p)); }
    if let Some(p) = d.parent.as_deref().filter(|p| shelf.find(p).is_some()) { s += &format!(" | 在《{}》下面", title_of(shelf, p)); }
    if !d.tags.is_empty() { s += &format!(" | 标签：{}", d.tags.iter().map(|t| format!("#{t}")).collect::<Vec<_>>().join(" ")); }
    s += &format!(" | 修改于 {}", date(d.modified));
    let kids = shelf.docs.iter().filter(|x| x.parent.as_deref() == Some(&d.id)).count();
    if kids > 0 { s += &format!(" | 下面有 {kids} 篇子文档"); }
    let head = d.text.split_whitespace().collect::<Vec<_>>().join(" ");
    if !head.is_empty() { s += &format!("\n{indent}  {}", clip(&head, 100)); }
    s + "\n"
}

fn count_in(shelf: &Shelf, folder: &str) -> usize { shelf.docs.iter().filter(|d| shelf.inside(&d.id, folder)).count() }

fn list_folder(shelf: &Shelf, a: &Value) -> Out {
    let depth = arg_num(a, "depth").unwrap_or(1).clamp(1, 3);
    let sort = arg_str(a, "sort");
    let from: usize = match arg_str(a, "cursor") { c if c.is_empty() => 0, c => c.parse().map_err(|_| format!("cursor「{c}」不对：用上次结果末尾给的 cursor，或者不带 cursor 从头列。"))? };
    let at = match arg_str(a, "folder") { f if f.is_empty() => None, f => Some(resolve(shelf, &f)?) };
    let mut out = String::new();
    match &at {
        None => {
            out += &format!("{}：{} 篇文档，{} 个文件夹。\n", super::agent::space(shelf), shelf.docs.len(), shelf.folders.len());
            let mut tags: HashMap<&str, usize> = HashMap::new();
            for d in &shelf.docs { for t in &d.tags { *tags.entry(t).or_default() += 1; } }
            let mut tags: Vec<_> = tags.into_iter().collect();
            tags.sort_by_key(|(t, n)| (std::cmp::Reverse(*n), *t));
            if !tags.is_empty() { out += &format!("常用标签：{}\n", tags.iter().take(20).map(|(t, n)| format!("#{t}（{n}）")).collect::<Vec<_>>().join(" ")); }
            if shelf.docs.is_empty() && shelf.folders.is_empty() { return Ok(out + "空间里还什么都没有。"); }
            out += "\n顶层：\n";
        }
        Some(id) => match shelf.folder(id) {
            Some(f) => out += &format!("文件夹「{}」(id: {})：{} 篇文档（含子文件夹）。\n\n", shelf.path(&f.id), f.id, count_in(shelf, id)),
            None => out += &format!("《{}》下面的子文档：\n\n", title_of(shelf, id)),
        },
    }
    // 这一层的文件夹（先）和文档。
    let here = |p: Option<&str>| -> (Vec<&super::Folder>, Vec<&Doc>) {
        let mut fs: Vec<_> = shelf.folders.iter().filter(|f| f.parent.as_deref() == p).collect();
        fs.sort_by_key(|f| f.name.to_lowercase());
        (fs, sorted(shelf.docs.iter().filter(|d| d.parent.as_deref() == p).collect(), &sort))
    };
    let (fs, ds) = here(at.as_deref());
    let total = fs.len() + ds.len();
    if total == 0 { return Ok(out + "（这里是空的）"); }
    if from >= total { return Err(format!("cursor 超出了：这里一共只有 {total} 项。")); }
    let mut lines: Vec<String> = fs.iter().map(|f| {
        let mut s = format!("- 文件夹「{}」 id: {} | {} 篇\n", f.name, f.id, count_in(shelf, &f.id));
        // 往下展开几层：子文件夹和文档，每层最多 10 项。
        fn deeper(shelf: &Shelf, id: &str, level: usize, depth: usize, sort: &str, s: &mut String) {
            if level >= depth { return; }
            let indent = "  ".repeat(level);
            let mut fs: Vec<_> = shelf.folders.iter().filter(|f| f.parent.as_deref() == Some(id)).collect();
            fs.sort_by_key(|f| f.name.to_lowercase());
            let ds = sorted(shelf.docs.iter().filter(|d| d.parent.as_deref() == Some(id)).collect(), sort);
            for f in fs.iter().take(10) {
                *s += &format!("{indent}- 文件夹「{}」 id: {} | {} 篇\n", f.name, f.id, count_in(shelf, &f.id));
                deeper(shelf, &f.id, level + 1, depth, sort, s);
            }
            for d in ds.iter().take(10) { *s += &doc_line(shelf, d, &indent); }
            let more = fs.len().saturating_sub(10) + ds.len().saturating_sub(10);
            if more > 0 { *s += &format!("{indent}- ……还有 {more} 项，用 list_folder(folder=\"{id}\") 看\n"); }
        }
        deeper(shelf, &f.id, 1, depth, &sort, &mut s);
        s
    }).collect();
    lines.extend(ds.iter().map(|d| doc_line(shelf, d, "")));
    let to = (from + ITEMS).min(total);
    out += &lines[from..to].concat();
    if to < total { out += &format!("\n[第 {}–{to} 项，共 {total} 项。接着列用 cursor=\"{to}\"]", from + 1); }
    Ok(out)
}

// ---- 搜索

/// 一块连同子块（和表格里）的纯文字。
fn plain(b: &Block) -> String {
    let mut s = b.text.plain_text();
    for c in &b.children { s.push('\n'); s += &plain(c); }
    if let BlockKind::Table(t) = &b.kind { for c in t.rows.iter().flatten() { for x in &c.blocks { s.push(' '); s += &plain(x); } } }
    s
}

/// 命中处前后各取一些字。
fn snippet(text: &str, ql: &str) -> Option<String> {
    let lower = text.to_lowercase();
    let at = lower.find(ql)?;
    // 小写后字节位置可能对不上原文：按字数换算。
    let ci = lower[..at].chars().count();
    let chars: Vec<char> = text.chars().collect();
    let (a, b) = (ci.saturating_sub(40), (ci + ql.chars().count() + 60).min(chars.len()));
    Some(chars[a..b].iter().collect::<String>().split_whitespace().collect::<Vec<_>>().join(" "))
}

fn search(shelf: &Shelf, a: &Value) -> Out {
    let q = arg_str(a, "query");
    let ql = q.to_lowercase();
    let limit = arg_num(a, "limit").unwrap_or(10).clamp(1, 30);
    let doc = arg_str(a, "doc");
    if !doc.is_empty() { return search_in(shelf, &doc, &ql, &q, limit, arg_num(a, "context").unwrap_or(2).min(5)); }
    let folder = match arg_str(a, "folder") { f if f.is_empty() => None, f => Some(resolve(shelf, &f)?) };
    let tag = arg_str(a, "tag").trim_start_matches('#').to_lowercase();
    let bad = |k: &str, v: &str| format!("{k}「{v}」看不懂，写成 2026-11-20 或 2026-11-20 09:00。");
    let since = match arg_str(a, "since") { s if s.is_empty() => None, s => Some(parse_time(&s, false).ok_or_else(|| bad("since", &s))?) };
    let until = match arg_str(a, "until") { s if s.is_empty() => None, s => Some(parse_time(&s, true).ok_or_else(|| bad("until", &s))?) };
    let title_only = arg_bool(a, "title_only");
    let mut hits: Vec<(usize, &Doc)> = shelf.docs.iter().filter(|d| {
        folder.as_deref().is_none_or(|f| shelf.inside(&d.id, f))
            && (tag.is_empty() || d.tags.iter().any(|t| t.to_lowercase() == tag))
            && since.is_none_or(|s| d.modified >= s) && until.is_none_or(|u| d.modified <= u)
    }).filter_map(|d| {
        if ql.is_empty() { return Some((0, d)); }
        let n = d.title.to_lowercase().matches(&ql).count() * 3 + if title_only { 0 } else { d.text.to_lowercase().matches(&ql).count() };
        (n > 0).then_some((n, d))
    }).collect();
    let sort = match arg_str(a, "sort") { s if s.is_empty() && !ql.is_empty() => "relevance".to_owned(), s => s };
    match sort.as_str() {
        "relevance" => hits.sort_by_key(|(n, d)| (std::cmp::Reverse(*n), std::cmp::Reverse(d.modified))),
        "title" => hits.sort_by_key(|(_, d)| title(&d.title).to_lowercase()),
        "created" => hits.sort_by_key(|(_, d)| std::cmp::Reverse(d.created)),
        _ => hits.sort_by_key(|(_, d)| std::cmp::Reverse(d.modified)),
    }
    let what = if q.is_empty() { "符合条件的文档".to_owned() } else { format!("含「{q}」的文档") };
    if hits.is_empty() { return Ok(format!("没有{what}。")); }
    let mut out = format!("{what}共 {} 篇{}：\n", hits.len(), if hits.len() > limit { format!("，列出前 {limit} 篇") } else { String::new() });
    for (_, d) in hits.iter().take(limit) {
        match snippet(&d.text, &ql).filter(|_| !ql.is_empty() && !title_only) {
            Some(s) => {
                let mut line = doc_line(shelf, d, "");
                if let Some(i) = line.find('\n').filter(|i| *i + 1 < line.len()) { line.truncate(i + 1); }
                out += &line;
                out += &format!("  …{s}…\n");
            }
            None => out += &doc_line(shelf, d, ""),
        }
    }
    Ok(out)
}

/// 一篇里找块：命中的顶层块前后各带 `n` 块，挨着的并成一段，用方言读出（带块号）。
fn search_in(shelf: &Shelf, id: &str, ql: &str, q: &str, limit: usize, n: usize) -> Out {
    if ql.is_empty() { return Err("在一篇文档里找要给 query。想看整篇用 read_document。".into()); }
    let doc = load(shelf, id)?;
    let hits: Vec<usize> = doc.blocks.iter().enumerate().filter(|(_, b)| plain(b).to_lowercase().contains(ql)).map(|(i, _)| i).collect();
    if hits.is_empty() { return Ok(format!("《{}》里没有「{q}」。", title_of(shelf, id))); }
    let mut spans: Vec<(usize, usize)> = vec![];
    for &i in hits.iter() {
        let (a, b) = (i.saturating_sub(n), (i + n + 1).min(doc.blocks.len()));
        match spans.last_mut() { Some(last) if a <= last.1 => last.1 = b, _ => spans.push((a, b)) }
    }
    let mut out = format!("《{}》(id: {id}) 里有 {} 块含「{q}」{}：\n", title_of(shelf, id), hits.len(), if spans.len() > limit { format!("，列出前 {limit} 处") } else { String::new() });
    let mut refs = shelf.refs.lock().unwrap();
    for (a, b) in spans.into_iter().take(limit) {
        out += &format!("\n--- 第 {}–{b} 块 ---\n{}\n", a + 1, fdl_core::dialect::read(id, &doc.blocks[a..b], &mut refs));
    }
    Ok(out)
}

// ---- 读一篇

fn load(shelf: &Shelf, id: &str) -> Result<std::sync::Arc<Document>, String> {
    if shelf.find(id).is_none() { return Err(format!("没有标识为 {id} 的文档。先用 search 或 list_folder 找到标识。")); }
    shelf.document(id).ok_or_else(|| format!("《{}》现在读不出来（可能还没从 iCloud 下载完），先看别的。", title_of(shelf, id)))
}

/// 这块（或它的子块）是 `target`。
fn holds(b: &Block, target: fdl_core::model::BlockId) -> bool { b.id == target || b.children.iter().any(|c| holds(c, target)) }

fn read_document(shelf: &Shelf, a: &Value) -> Out {
    let id = arg_str(a, "id");
    if id.is_empty() { return Err("没有给 id。".into()); }
    if id.eq_ignore_ascii_case(SPACE_MD) {
        let g = shelf.guide.trim();
        return Ok(if g.is_empty() { "这个空间还没有 SPACE.md（空间说明）。".into() } else { format!("SPACE.md（{} 字）：\n\n{g}", g.chars().count()) });
    }
    let doc = load(shelf, &id)?;
    let d = shelf.find(&id).ok_or("文档不见了")?;
    let total = doc.blocks.len();
    let mut out = format!("《{}》 id: {id}", title(&d.title));
    if let Some(p) = d.parent.as_deref().filter(|p| shelf.folder(p).is_some()) { out += &format!(" | 文件夹：{}", shelf.path(p)); }
    if !d.tags.is_empty() { out += &format!(" | 标签：{}", d.tags.iter().map(|t| format!("#{t}")).collect::<Vec<_>>().join(" ")); }
    out += &format!(" | 修改于 {} | 共 {total} 块\n", datetime(d.modified));
    let mut refs = shelf.refs.lock().unwrap();
    if arg_bool(a, "outline") {
        let mut lines = vec![];
        fn walk(bs: &[Block], doc: &str, refs: &mut fdl_core::dialect::Refs, lines: &mut Vec<String>) {
            for b in bs {
                if let BlockKind::Heading { level, .. } = b.kind { lines.push(format!("[{}] {} {}", refs.mark(doc, b.id), "#".repeat(level.clamp(1, 6) as usize), b.text.plain_text().trim())); }
                walk(&b.children, doc, refs, lines);
            }
        }
        walk(&doc.blocks, &id, &mut refs, &mut lines);
        return Ok(out + &if lines.is_empty() { "\n这篇没有标题。".to_owned() } else { format!("\n标题结构：\n{}", lines.join("\n")) });
    }
    if total == 0 { return Ok(out + "\n（正文是空的）"); }
    let from = match arg_str(a, "from_block") {
        f if f.is_empty() => 0,
        f => match f.parse::<usize>() {
            Ok(i) => i,
            Err(_) => {
                let (doc_id, block) = refs.resolve(&f).ok_or_else(|| format!("没有块 {f}：块号要用这次读出来的，或者写第几块（0 起）。"))?;
                if doc_id != id { return Err(format!("块 {f} 不在这篇里。")); }
                doc.blocks.iter().position(|b| holds(b, block)).ok_or_else(|| format!("块 {f} 已经不在这篇里了，从头读一遍。"))?
            }
        },
    };
    if from >= total { return Err(format!("from_block 超出了：这篇只有 {total} 块（0–{}）。", total - 1)); }
    // 读到约 `PAGE` 字或 `limit` 块。
    let max = arg_num(a, "limit").unwrap_or(usize::MAX).max(1);
    let mut to = from;
    let mut size = 0;
    while to < total && to - from < max && (to == from || size + plain(&doc.blocks[to]).chars().count() <= PAGE) {
        size += plain(&doc.blocks[to]).chars().count();
        to += 1;
    }
    out += &format!("第 {}–{to} 块：\n\n{}", from + 1, fdl_core::dialect::read(&id, &doc.blocks[from..to], &mut refs));
    if to < total { out += &format!("\n\n[还有 {} 块没读，接着读用 from_block={to}]", total - to); }
    Ok(out)
}

// ---- 待办、反向链接、回收站

fn list_tasks(shelf: &Shelf, a: &Value) -> Out {
    let status = match arg_str(a, "status") { s if s.is_empty() => "open".to_owned(), s => s };
    if !matches!(status.as_str(), "open" | "done" | "all") { return Err(format!("status「{status}」不对：open、done 或 all。")); }
    let folder = match arg_str(a, "folder") { f if f.is_empty() => None, f => Some(resolve(shelf, &f)?) };
    let due = match arg_str(a, "due_before") { s if s.is_empty() => None, s => Some(parse_time(&s, false).ok_or_else(|| format!("due_before「{s}」看不懂，写成 2026-11-30 或 2026-11-30 18:00。"))? * 1000) };
    let mut refs = shelf.refs.lock().unwrap();
    let (mut out, mut n, mut more) = (String::new(), 0, 0);
    for d in sorted(shelf.docs.iter().filter(|d| folder.as_deref().is_none_or(|f| shelf.inside(&d.id, f))).collect(), "modified") {
        let Some(doc) = shelf.document(&d.id) else { continue };
        let mut lines = vec![];
        fn walk(bs: &[Block], f: &mut dyn FnMut(&Block)) { for b in bs { f(b); walk(&b.children, f); } }
        walk(&doc.blocks, &mut |b| {
            let reminders: Vec<_> = b.text.runs.iter().filter_map(|r| r.marks.inline.as_ref().and_then(Inline::reminder)).collect();
            let done = match b.kind { BlockKind::Todo { done } => Some(done), _ => None };
            if done.is_none() && reminders.is_empty() { return; }
            if done.is_some_and(|x| match status.as_str() { "open" => x, "done" => !x, _ => false }) { return; }
            if done.is_none() && status == "done" { return; }
            if due.is_some_and(|t| !reminders.iter().any(|r| r.expire < t as i64)) { return; }
            if n >= 100 { more += 1; return; }
            n += 1;
            let when: Vec<String> = reminders.iter().map(|r| if r.whole_day { date(r.expire.max(0) as u64 / 1000) } else { datetime(r.expire.max(0) as u64 / 1000) }).collect();
            let mark = match done { Some(true) => "- [x]", Some(false) => "- [ ]", None => "- 提醒" };
            let text = clip(b.text.plain_text().split_whitespace().collect::<Vec<_>>().join(" ").as_str(), 80);
            let when = if when.is_empty() { String::new() } else { format!("（提醒 {}）", when.join("、")) };
            lines.push(format!("  {mark} [{}] {text}{when}", refs.mark(&d.id, b.id)));
        });
        if !lines.is_empty() { out += &format!("《{}》 id: {}\n{}\n", title(&d.title), d.id, lines.join("\n")); }
    }
    if n == 0 { return Ok("没有符合条件的待办和提醒。".into()); }
    if more > 0 { out += &format!("\n[还有 {more} 项没列出，用 folder 或 due_before 缩小范围]"); }
    Ok(format!("共 {} 项：\n\n{out}", n + more))
}

fn backlinks(shelf: &Shelf, a: &Value) -> Out {
    let id = arg_str(a, "id");
    if shelf.find(&id).is_none() { return Err(format!("没有标识为 {id} 的文档。")); }
    let from: Vec<&Doc> = sorted(shelf.docs.iter().filter(|d| d.id != id && d.links.iter().any(|(t, _)| *t == id)).collect(), "modified");
    if from.is_empty() { return Ok(format!("没有文档链接《{}》。", title_of(shelf, &id))); }
    let mut out = format!("有 {} 篇文档链接了《{}》：\n", from.len(), title_of(shelf, &id));
    for d in from {
        out += &format!("- 《{}》 id: {}\n", title(&d.title), d.id);
        for (_, line) in d.links.iter().filter(|(t, _)| *t == id).take(3) { if !line.trim().is_empty() { out += &format!("  「{}」\n", clip(line.trim(), 80)); } }
    }
    Ok(out)
}

fn list_trash(shelf: &Shelf, a: &Value) -> Out {
    let limit = arg_num(a, "limit").unwrap_or(20).clamp(1, 100);
    if shelf.trash.is_empty() { return Ok("最近删除是空的。".into()); }
    let mut out = format!("最近删除里有 {} 项{}：\n", shelf.trash.len(), if shelf.trash.len() > limit { format!("，列出最近的 {limit} 项") } else { String::new() });
    for t in shelf.trash.iter().take(limit) {
        out += &format!("- {}「{}」 id: {} | 删除于 {}\n", if t.folder { "文件夹" } else { "文档" }, title(&t.title), t.id, datetime(t.deleted));
    }
    Ok(out + "（删掉的内容读不了；要用得请用户在「最近删除」里恢复。）")
}
