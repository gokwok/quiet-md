//! 助手的一次 run：接到一句话，由选定的核心一步步驱动到说完。
//!
//! 核心不碰网络、不碰文档库（sans-IO）：宿主把这次要的素材备成快照（`Shelf`），核心拼出请求体（`Act::Ask`），
//! 宿主发出去、把模型说的交回来（`Reply`）；核心要调工具就交出调用（`Act::Use`），宿主照 `tools` 执行、把结果交回来；
//! 直到核心说完（`Act::Finish`）。宿主一路把核心报的步骤和文字放给界面看，用户随时可以停。
//!
//! 两个核心：
//! - 酒馆（`tavern`）：照酒馆预设拼一次请求，模型说完就完，不调工具；
//! - Agent（`agent`）：ReAct 式循环，带只读的文档工具（列出、搜索、读取），自己决定读什么、读几轮。
//!
//! 每次都带上：空间的 `SPACE.md`、用的那个助手（名字和设定）、这句话附带的资源（`Resource`：文件夹、文档、文档里的几块）。
//! 素材怎么填由核心自己定：酒馆把能塞的一次塞进去，Agent 只给个开头、要更多自己去读。
//! 对话记录（`chat`）和核心无关，两个核心共用。
pub mod agent;
pub mod chat;
pub mod tavern;
pub mod tools;

use std::sync::Arc;

use fdl_core::model::Block;
use serde_json::Value;

pub use crate::assistants::{Kind, Permission};
pub use crate::tavern::assemble::Identity;
pub use crate::tavern::preset::Role;

/// 一句话附带的资源：一个文件夹、一篇文档、一篇文档里的几块。默认的照从哪儿进来（文档页是这篇，选中块是这几块，
/// 文件夹页是这个文件夹，主页什么都不带：整个空间本来就是范围），也可以 @ 进来、删掉。
/// 块的标识不跨启动，选中的内容（Markdown）记在资源里。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resource { Folder { id: String }, Document { id: String }, Blocks { doc: String, blocks: Vec<String>, text: String } }

impl Resource {
    /// 是哪篇文档里的（文件夹不是）。
    pub fn doc(&self) -> Option<&str> { match self { Resource::Folder { .. } => None, Resource::Document { id } | Resource::Blocks { doc: id, .. } => Some(id) } }
}

/// 文档库里的一篇（快照）。正文是纯文字，搜索和读取用。
#[derive(Clone, Debug)]
pub struct Doc { pub id: String, pub title: String, pub folder: Option<String>, pub modified: u64, pub text: Arc<str> }

/// 附带的一篇（或里面的几块）：完整的 Markdown，选中的块（Markdown）。
#[derive(Clone, Debug, Default)]
pub struct Focus { pub id: String, pub title: String, pub markdown: String, pub picked: Option<String> }

/// 宿主照 `Resource` 备好的一样：文件夹是名字和里面的文档（标识），文档和块是 `Focus`。
#[derive(Clone, Debug)]
pub enum Attached { Folder { name: String, docs: Vec<String> }, Doc(Focus) }

/// 宿主在 run 开始时备好的素材。
#[derive(Clone, Debug, Default)]
pub struct Shelf {
    pub space: String,
    /// 空间的 `SPACE.md`（用户写给助手看的空间说明），没有是空的。
    pub guide: String,
    /// 用的那个助手：名字和设定。
    pub assistant: Identity,
    pub docs: Vec<Doc>,
    /// 这句话附带的资源（照 `Resource` 的顺序；找不到的不在）。
    pub attached: Vec<Attached>,
    /// 助手能不能改文档。这一版只记着传下来，写的工具以后接。
    pub permission: Permission,
    pub now: Option<(i64, u32, u32, u32, u32)>,
}

impl Shelf {
    pub fn find(&self, id: &str) -> Option<&Doc> { self.docs.iter().find(|d| d.id == id) }
    /// 最近改过的几篇。
    pub fn recent(&self, n: usize) -> Vec<&Doc> {
        let mut v: Vec<&Doc> = self.docs.iter().collect();
        v.sort_by_key(|d| std::cmp::Reverse(d.modified));
        v.truncate(n);
        v
    }
    /// 附带的这篇（完整的 Markdown）。
    pub fn focus(&self, id: &str) -> Option<&Focus> { self.attached.iter().find_map(|a| match a { Attached::Doc(f) if f.id == id => Some(f), _ => None }) }
    /// 助手的名字，没起名是「MarginAI」。
    pub fn name(&self) -> &str { if self.assistant.name.trim().is_empty() { "MarginAI" } else { self.assistant.name.trim() } }
}

/// 之前的对话（按顺序，不含这次的话）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Turn { pub role: Role, pub text: String }

/// 模型要调的一个工具：`args` 是它给的 JSON 原文。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Call { pub id: String, pub name: String, pub args: String }

/// 模型这一轮说完的：文字和要调的工具。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Reply { pub text: String, pub calls: Vec<Call> }

/// 核心下一步要宿主做的。
#[derive(Debug)]
pub enum Act {
    /// 发这个请求体（不含 `model`、`stream`，宿主补）。
    Ask(Value),
    /// 执行这些工具（`tools::run`），结果交回 `Core::results`。
    Use(Vec<Call>),
    /// 说完了：最终的原文（存下来的样子）。
    Finish(String),
}

/// 成形后的一段：存下来的原文和显示的块。
#[derive(Clone, Debug, Default)]
pub struct Shaped { pub stored: String, pub blocks: Vec<Block> }

/// 驱动 run 的核心。整个 run 在宿主的后台线程上跑，所以要 `Send`。
pub trait Core: Send {
    /// 开始：拼第一份请求。
    fn begin(&mut self, input: &str, history: &[Turn], shelf: &Shelf) -> Act;
    /// 模型这一轮说完了。
    fn reply(&mut self, reply: Reply) -> Act;
    /// 工具的结果（和 `Act::Use` 的顺序一样）。
    fn results(&mut self, shelf: &Shelf, results: Vec<(Call, String)>) -> Act;
    /// 流式文字里已经稳了的前缀到哪（字节）：预览只成形这一段，免得半截的格式闪一下。
    fn settled(&self, raw: &str) -> usize;
    /// 原文变成存下来的样子和显示的块。`id` 给块编号用。
    fn shape(&mut self, raw: &str, id: &str, preview: bool) -> Shaped;
}

/// 截到 `n` 个字，截了在末尾加「…」。
pub fn clip(s: &str, n: usize) -> String {
    match s.char_indices().nth(n) { Some((i, _)) => format!("{}…", &s[..i]), None => s.to_owned() }
}

/// 长文按字数取头尾：开头 `head` 字，结尾 `tail` 字，中间省掉。
pub fn ends(s: &str, head: usize, tail: usize) -> String {
    let n = s.chars().count();
    if n <= head + tail { return s.to_owned(); }
    let a: String = s.chars().take(head).collect();
    let b: String = s.chars().skip(n - tail).collect();
    format!("{a}\n\n[……中间省略 {} 字……]\n\n{b}", n - head - tail)
}

#[cfg(test)]
mod tests;
