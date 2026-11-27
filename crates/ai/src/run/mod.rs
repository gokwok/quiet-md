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
//! 素材怎么填由核心自己定：同一个范围（`Scope`），酒馆把能塞的一次塞进去，Agent 只给个开头、要更多自己去读。
//! 对话记录（`chat`）和核心无关，两个核心共用。
pub mod agent;
pub mod chat;
pub mod tavern;
pub mod tools;

use std::sync::Arc;

use fdl_core::model::Block;
use serde_json::Value;

pub use crate::tavern::preset::Role;

/// 助手看的范围，由从哪儿进来决定：主页是整个空间，文档页是这篇文档，文档里选了块是这篇加选中的块。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scope { Space, Doc { id: String }, Blocks { id: String, blocks: Vec<String> } }

impl Scope {
    pub fn doc(&self) -> Option<&str> { match self { Scope::Space => None, Scope::Doc { id } | Scope::Blocks { id, .. } => Some(id) } }
}

/// 文档库里的一篇（快照）。正文是纯文字，搜索和读取用。
#[derive(Clone, Debug)]
pub struct Doc { pub id: String, pub title: String, pub folder: Option<String>, pub modified: u64, pub text: Arc<str> }

/// 进来的那篇：完整的 Markdown，选中的块（Markdown）。
#[derive(Clone, Debug, Default)]
pub struct Focus { pub id: String, pub title: String, pub markdown: String, pub picked: Option<String> }

/// 宿主在 run 开始时备好的素材。
#[derive(Clone, Debug, Default)]
pub struct Shelf { pub space: String, pub docs: Vec<Doc>, pub focus: Option<Focus>, pub now: Option<(i64, u32, u32, u32, u32)> }

impl Shelf {
    pub fn find(&self, id: &str) -> Option<&Doc> { self.docs.iter().find(|d| d.id == id) }
    /// 最近改过的几篇。
    pub fn recent(&self, n: usize) -> Vec<&Doc> {
        let mut v: Vec<&Doc> = self.docs.iter().collect();
        v.sort_by_key(|d| std::cmp::Reverse(d.modified));
        v.truncate(n);
        v
    }
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

/// 核心有哪几种。`id` 存在对话记录里。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Kind { #[default] Tavern, Agent }

impl Kind {
    pub const ALL: [Kind; 2] = [Kind::Tavern, Kind::Agent];
    pub fn id(self) -> &'static str { match self { Kind::Tavern => "tavern", Kind::Agent => "agent" } }
    pub fn read(s: &str) -> Kind { if s == "agent" { Kind::Agent } else { Kind::Tavern } }
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
