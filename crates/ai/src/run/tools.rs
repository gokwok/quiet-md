//! 助手的能力里给 Agent 用的那几样：只读的文档工具。都在 run 开始时的快照（`Shelf`）上做，不碰文档库。
//! 结果是给模型看的纯文字，太长就截；界面上每次调用显示成一步（`label`）。
use serde_json::{Value, json};

use super::{Call, Shelf, clip};

/// 一次读多少字。
pub const PAGE: usize = 6000;

/// 工具的定义（OpenAI 的 `tools` 格式）。
pub fn defs() -> Value {
    let f = |name: &str, desc: &str, params: Value| json!({ "type": "function", "function": { "name": name, "description": desc, "parameters": params } });
    json!([
        f("list_documents", "列出空间里的文档：标识、标题、所在文件夹、修改日期，最近改过的在前。",
            json!({ "type": "object", "properties": { "folder": { "type": "string", "description": "只列这个文件夹里的（文件夹名字，可省）" }, "limit": { "type": "integer", "description": "最多几篇，默认 50" } } })),
        f("search_documents", "按关键词在标题和正文里搜索文档，返回命中的文档和命中处前后的文字。",
            json!({ "type": "object", "properties": { "query": { "type": "string", "description": "关键词" } }, "required": ["query"] })),
        f("read_document", "读一篇文档的正文。很长的分段读：offset 是从第几个字开始。",
            json!({ "type": "object", "properties": { "id": { "type": "string", "description": "文档标识" }, "offset": { "type": "integer", "description": "从第几个字开始，默认 0" } }, "required": ["id"] })),
    ])
}

fn args(call: &Call) -> Value { serde_json::from_str(&call.args).unwrap_or(Value::Null) }
fn arg_str(v: &Value, k: &str) -> String { v.get(k).and_then(Value::as_str).unwrap_or_default().trim().to_owned() }
fn arg_num(v: &Value, k: &str) -> Option<usize> { v.get(k).and_then(|x| x.as_u64().or_else(|| x.as_str()?.trim().parse().ok())).map(|n| n as usize) }

fn title_of(shelf: &Shelf, id: &str) -> String {
    shelf.find(id).map(|d| if d.title.trim().is_empty() { "无标题".to_owned() } else { d.title.clone() }).unwrap_or_else(|| id.to_owned())
}

/// 界面上这一步怎么写。
pub fn label(shelf: &Shelf, call: &Call) -> String {
    let a = args(call);
    match call.name.as_str() {
        "list_documents" => match arg_str(&a, "folder") { f if f.is_empty() => "列出文档".into(), f => format!("列出「{f}」里的文档") },
        "search_documents" => format!("搜索「{}」", clip(&arg_str(&a, "query"), 20)),
        "read_document" => format!("读取《{}》", clip(&title_of(shelf, &arg_str(&a, "id")), 20)),
        other => other.to_owned(),
    }
}

fn date(secs: u64) -> String { let (y, m, d, _, _) = fdl_core::time::civil(secs); format!("{y}-{m:02}-{d:02}") }

/// 执行一次调用，返回给模型看的文字。
pub fn run(shelf: &Shelf, call: &Call) -> String {
    let a = args(call);
    match call.name.as_str() {
        "list_documents" => {
            let folder = arg_str(&a, "folder");
            let limit = arg_num(&a, "limit").unwrap_or(50).clamp(1, 200);
            let docs: Vec<_> = shelf.recent(usize::MAX).into_iter().filter(|d| folder.is_empty() || d.folder.as_deref().is_some_and(|f| f.contains(&folder))).collect();
            if docs.is_empty() { return "没有文档。".into(); }
            let mut out = format!("共 {} 篇{}：\n", docs.len(), if docs.len() > limit { format!("，列出最近改过的 {limit} 篇") } else { String::new() });
            for d in docs.iter().take(limit) {
                out += &format!("- id: {} | 《{}》{} | 修改于 {} | {} 字\n", d.id, title_of(shelf, &d.id), d.folder.as_ref().map(|f| format!(" | 文件夹：{f}")).unwrap_or_default(), date(d.modified), d.text.chars().count());
            }
            out
        }
        "search_documents" => {
            let q = arg_str(&a, "query");
            if q.is_empty() { return "错误：没有给关键词。".into(); }
            let ql = q.to_lowercase();
            let mut hits: Vec<(usize, &super::Doc)> = shelf.docs.iter().filter_map(|d| {
                let n = d.title.to_lowercase().matches(&ql).count() * 3 + d.text.to_lowercase().matches(&ql).count();
                (n > 0).then_some((n, d))
            }).collect();
            hits.sort_by_key(|(n, d)| (std::cmp::Reverse(*n), std::cmp::Reverse(d.modified)));
            if hits.is_empty() { return format!("没有找到含「{q}」的文档。"); }
            let mut out = format!("找到 {} 篇：\n", hits.len());
            for (_, d) in hits.iter().take(10) {
                out += &format!("- id: {} | 《{}》\n", d.id, title_of(shelf, &d.id));
                if let Some(s) = snippet(&d.text, &ql) { out += &format!("  …{s}…\n"); }
            }
            out
        }
        "read_document" => {
            let id = arg_str(&a, "id");
            let Some(d) = shelf.find(&id) else { return format!("错误：没有标识为 {id} 的文档。先用 list_documents 或 search_documents 找到标识。") };
            // 进来的那篇用完整的 Markdown（标题、列表这些都在），别的用纯文字。
            let text: String = match shelf.focus.as_ref().filter(|f| f.id == id) { Some(f) => f.markdown.clone(), None => d.text.to_string() };
            let total = text.chars().count();
            let from = arg_num(&a, "offset").unwrap_or(0).min(total);
            let part: String = text.chars().skip(from).take(PAGE).collect();
            let to = from + part.chars().count();
            let mut out = format!("《{}》第 {}–{} 字，共 {total} 字：\n\n{part}", title_of(shelf, &id), from, to);
            if to < total { out += &format!("\n\n[还有 {} 字，接着读用 offset={to}]", total - to); }
            out
        }
        other => format!("错误：没有叫 {other} 的工具。"),
    }
}

/// 命中处前后各取一些字。
fn snippet(text: &str, ql: &str) -> Option<String> {
    let lower = text.to_lowercase();
    let at = lower.find(ql)?;
    // 小写后字节位置可能对不上原文：按字数换算。
    let ci = lower[..at].chars().count();
    let chars: Vec<char> = text.chars().collect();
    let (a, b) = (ci.saturating_sub(40), (ci + ql.chars().count() + 60).min(chars.len()));
    Some(chars[a..b].iter().collect::<String>().replace('\n', " "))
}
