//! Agent 核心：ReAct 式循环。每一轮把对话、工具调用和结果整段发给模型，模型要调工具就调，不调了它说的就是回答。
//! 工具是只读的文档工具（`tools`），对话可编辑时加上写的（`write`），一轮可以调好几个。最多 `rounds` 轮（助手的核心配置，默认 `ROUNDS`），到了就不再给工具，让它照已经读到的回答。
//!
//! 系统提示词：助手的名字和设定、规矩、空间的 `SPACE.md`（太长只放开头，全文用 `read_document("SPACE.md")`）、空间概况（篇数和最近改过的几篇，带标识），
//! 再是这句话附带的资源，只给个开头、要更多自己去读：
//! - 文件夹：名字和里面的文档（带标识）；
//! - 一篇文档：标题、标识和开头一段；
//! - 文档里的几块：选中的内容全文，加这篇的标题和标识。
//!
//! 输出按 Markdown 成形（和粘贴 Markdown 同一套）。
use fdl_core::model::Block;
use serde_json::{Value, json};

use super::{Act, Attached, Call, Core, Permission, Reply, Role, Shaped, Shelf, Turn, clip, tools, write};

/// 默认最多几轮工具调用。
pub const ROUNDS: usize = 8;
/// 系统提示词里的 `SPACE.md` 最多放几个字。
pub const GUIDE: usize = 2000;

/// `write`：这次能不能改文档（对话的权限，`begin` 时定）。
pub struct Agent { limit: usize, messages: Vec<Value>, rounds: usize, write: bool }

impl Agent {
    pub fn new(limit: usize) -> Agent { Agent { limit: limit.max(1), messages: vec![], rounds: 0, write: false } }

    fn body(&self) -> Value {
        let mut body = json!({ "messages": self.messages });
        if self.rounds < self.limit {
            let mut defs = tools::defs();
            if self.write && let (Some(all), Value::Array(more)) = (defs.as_array_mut(), write::defs()) { all.extend(more); }
            body["tools"] = defs;
        }
        body
    }
}

/// 「空间「名字」」，没起名就是「空间」。
pub fn space(shelf: &Shelf) -> String { if shelf.space.trim().is_empty() { "空间".into() } else { format!("空间「{}」", shelf.space.trim()) } }

pub(super) fn title(t: &str) -> &str { if t.trim().is_empty() { "无标题" } else { t.trim() } }

/// 系统提示词：身份、设定、规矩、空间说明和照资源备的素材。
pub fn system(shelf: &Shelf) -> String {
    let mut s = format!("你叫{}，是 Quiet.md（一个写作和笔记应用）里的助手。用户的文档在一个「空间」里。\n", shelf.name());
    let desc = shelf.assistant.description.trim();
    if !desc.is_empty() { s += &format!("\n你的设定：\n{desc}\n\n"); }
    s += "你能用工具查看这个空间：list_folder（文件夹和空间总览）、search（找文档，或在一篇里找）、read_document（读一篇，或 SPACE.md）、\
        list_tasks（待办和提醒）、backlinks（谁链接了这篇）、list_trash（最近删除）。需要文档里的内容就先去查、去读，不要凭空编造文档里没有的东西；\
        互不依赖的几样可以在一轮里一起调。读出的正文每块前有块号（如 [b12]），这次对话里一直有效，提到具体某块时可以带上；\
        <keep ref=\"b12\"/> 是没法用文字表示的块（分栏、倒计时之类）。\n\
        回答用用户说话的语言，用 Markdown（标题、列表、粗体），简洁直接。提到某篇文档时写它的标题。\n";
    s += if shelf.permission == Permission::Write {
        "这段对话里你可以改这个空间的文档：insert_blocks（插入新块）、update_blocks（改写或删掉几块）、replace_text（只换字）改正文，\
        create_document 新建，create_folder、move、rename、trash、restore 管文件夹和文档；SPACE.md 也能用前三样改。\
        改之前先读，块号用读出来的；新写的块不要带块号，<keep/> 原样留着就不会动那块。只换几个字用 replace_text，整块重写用 update_blocks。\
        用户没让你改就不要改；改完简单说改了哪些。用户随时能回滚你这一轮的改动。\n"
    } else { "只读不写：你改不了文档。\n" };
    if let Some((y, m, d, h, min)) = shelf.now { s += &format!("现在是 {y}-{m:02}-{d:02} {h:02}:{min:02}。\n"); }
    let guide = shelf.guide.trim();
    if !guide.is_empty() {
        s += &format!("\n用户写给你的空间说明（SPACE.md）：\n\n{}\n", clip(guide, GUIDE));
        if guide.chars().count() > GUIDE { s += "\n（SPACE.md 太长，上面只是开头；全文用 read_document(\"SPACE.md\") 读。）\n"; }
    }
    s += &format!("\n{}里有 {} 篇文档", space(shelf), shelf.docs.len());
    let recent = shelf.recent(15);
    if recent.is_empty() { s += "。\n"; } else {
        s += "，最近改过的：\n";
        for d in recent { s += &format!("- 《{}》(id: {})\n", title(&d.title), d.id); }
    }
    if shelf.attached.is_empty() { s += "\n用户这句话没有附带资源，问的多半是整个空间。\n"; }
    for a in &shelf.attached {
        s += "\n";
        match a {
            Attached::Folder { id, name, docs } => {
                s += &format!("用户附带了文件夹「{}」(id: {id})，里面有 {} 篇文档：\n", title(name), docs.len());
                for id in docs.iter().take(30) { s += &format!("- 《{}》(id: {id})\n", title(&shelf.find(id).map(|d| d.title.clone()).unwrap_or_default())); }
                if docs.len() > 30 { s += "- ……（更多用 list_folder）\n"; }
            }
            Attached::Doc(f) => match &f.picked {
                Some(p) => s += &format!("用户附带了文档《{}》(id: {}) 里的这些内容：\n\n{}\n\n要看这篇的其他部分用 read_document。\n", title(&f.title), f.id, clip(p, 6000)),
                None => s += &format!("用户附带了文档《{}》(id: {})。它的开头：\n\n{}\n\n要看全文用 read_document。\n", title(&f.title), f.id, clip(&f.markdown, 1500)),
            },
        }
    }
    s
}

impl Core for Agent {
    fn begin(&mut self, input: &str, history: &[Turn], shelf: &Shelf) -> Act {
        self.messages = vec![json!({ "role": "system", "content": system(shelf) })];
        for t in history {
            let role = match t.role { Role::User => "user", Role::Assistant => "assistant", Role::System => "system" };
            self.messages.push(json!({ "role": role, "content": t.text }));
        }
        self.messages.push(json!({ "role": "user", "content": input }));
        self.rounds = 0;
        self.write = shelf.permission == Permission::Write;
        Act::Ask(self.body())
    }

    fn reply(&mut self, reply: Reply) -> Act {
        if reply.calls.is_empty() || self.rounds >= self.limit { return Act::Finish(reply.text); }
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
