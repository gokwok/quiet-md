//! Agent 核心：ReAct 式循环。每一轮把对话、工具调用和结果整段发给模型，模型要调工具就调，不调了它说的就是回答。
//! 工具是只读的文档工具（`tools`）。最多 `ROUNDS` 轮，到了就不再给工具，让它照已经读到的回答。
//!
//! 素材的填法：只给个开头，要更多自己去读。
//! - 整个空间：空间名、文档篇数和最近改过的几篇（带标识）；
//! - 一篇文档：标题、标识和开头一段；
//! - 选中的块：选中的内容全文，加这篇的标题和标识。
//!
//! 输出按 Markdown 成形（和粘贴 Markdown 同一套）。
use fdl_core::model::Block;
use serde_json::{Value, json};

use super::{Act, Call, Core, Reply, Role, Scope, Shaped, Shelf, Turn, clip, tools};

/// 最多几轮工具调用。
pub const ROUNDS: usize = 8;

pub struct Agent { scope: Scope, messages: Vec<Value>, rounds: usize }

impl Agent {
    pub fn new(scope: Scope) -> Agent { Agent { scope, messages: vec![], rounds: 0 } }

    fn body(&self) -> Value {
        let mut body = json!({ "messages": self.messages });
        if self.rounds < ROUNDS { body["tools"] = tools::defs(); }
        body
    }
}

/// 「空间「名字」」，没起名就是「空间」。
pub fn space(shelf: &Shelf) -> String { if shelf.space.trim().is_empty() { "空间".into() } else { format!("空间「{}」", shelf.space.trim()) } }

fn title(t: &str) -> &str { if t.trim().is_empty() { "无标题" } else { t.trim() } }

/// 系统提示词：身份、规矩和照范围备的素材。
pub fn system(scope: &Scope, shelf: &Shelf) -> String {
    let mut s = String::from("你是 MarginAI，Quiet.md（一个写作和笔记应用）里的助手。用户的文档在一个「空间」里。\n\
        你可以用工具列出、搜索和读取空间里的文档；需要文档里的内容时先去读，不要凭空编造文档里没有的东西。只读不写：你改不了文档。\n\
        回答用用户说话的语言，用 Markdown（标题、列表、粗体），简洁直接。提到某篇文档时写它的标题。\n");
    if let Some((y, m, d, h, min)) = shelf.now { s += &format!("现在是 {y}-{m:02}-{d:02} {h:02}:{min:02}。\n"); }
    s += "\n";
    match (scope, shelf.focus.as_ref()) {
        (Scope::Space, _) => {
            s += &format!("用户从{}的主页打开了你，问的是整个空间。空间里有 {} 篇文档", space(shelf), shelf.docs.len());
            let recent = shelf.recent(15);
            if recent.is_empty() { s += "。\n"; } else {
                s += "，最近改过的：\n";
                for d in recent { s += &format!("- 《{}》(id: {})\n", title(&d.title), d.id); }
            }
        }
        (Scope::Doc { id }, Some(f)) => {
            s += &format!("用户正在看文档《{}》(id: {id})，问的多半是这一篇。它的开头：\n\n{}\n\n要看全文用 read_document。\n", title(&f.title), clip(&f.markdown, 1500));
        }
        (Scope::Blocks { id, .. }, Some(f)) => {
            s += &format!("用户在文档《{}》(id: {id}) 里选中了下面这些内容，问的是它们：\n\n{}\n\n要看这篇的其他部分用 read_document。\n", title(&f.title), clip(f.picked.as_deref().unwrap_or_default(), 6000));
        }
        _ => {}
    }
    s
}

impl Core for Agent {
    fn begin(&mut self, input: &str, history: &[Turn], shelf: &Shelf) -> Act {
        self.messages = vec![json!({ "role": "system", "content": system(&self.scope, shelf) })];
        for t in history {
            let role = match t.role { Role::User => "user", Role::Assistant => "assistant", Role::System => "system" };
            self.messages.push(json!({ "role": role, "content": t.text }));
        }
        self.messages.push(json!({ "role": "user", "content": input }));
        self.rounds = 0;
        Act::Ask(self.body())
    }

    fn reply(&mut self, reply: Reply) -> Act {
        if reply.calls.is_empty() || self.rounds >= ROUNDS { return Act::Finish(reply.text); }
        let calls: Vec<Value> = reply.calls.iter().map(|c| json!({ "id": c.id, "type": "function", "function": { "name": c.name, "arguments": c.args } })).collect();
        let mut m = json!({ "role": "assistant", "content": reply.text, "tool_calls": calls });
        if reply.text.is_empty() { m["content"] = Value::Null; }
        self.messages.push(m);
        self.rounds += 1;
        Act::Use(reply.calls)
    }

    fn results(&mut self, _: &Shelf, results: Vec<(Call, String)>) -> Act {
        for (c, out) in results { self.messages.push(json!({ "role": "tool", "tool_call_id": c.id, "content": out })); }
        Act::Ask(self.body())
    }

    /// 稳了的前缀：到最后一个空行为止（代码块没合上的不算）。
    fn settled(&self, raw: &str) -> usize {
        let Some(cut) = raw.rfind("\n\n") else { return 0 };
        let head = &raw[..cut];
        if head.matches("```").count() % 2 == 1 { return head.rfind("```").map_or(0, |i| head[..i].rfind("\n\n").unwrap_or(0)); }
        cut
    }

    fn shape(&mut self, raw: &str, _: &str, _: bool) -> Shaped {
        let text = raw.trim();
        let blocks: Vec<Block> = if text.is_empty() { vec![] } else { fdl_core::markdown::parse_file(text, "", None).blocks };
        Shaped { stored: text.to_owned(), blocks }
    }
}
