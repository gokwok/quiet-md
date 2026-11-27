//! 酒馆核心：照酒馆预设拼一次请求（`tavern::assemble`），模型说完就完，不调工具。输出照预设的正则成形（`Shaper`）。
//! 角色（`{{char}}`）是用的那个助手（名字和设定），人设（`{{user}}`）是写作设置里选的。
//!
//! 素材的填法：能塞的一次塞进去，作为一条系统消息排在这次的话前面（聊天记录超出预算时从最早的丢，它挨着这次的话，最后才丢）。
//! 依次是：空间的 `SPACE.md`；空间名、文档篇数和最近改过的几篇标题；这句话附带的资源——
//! 文件夹是名字和里面的文档标题，文档是标题和正文（太长取头尾，几篇一起时每篇少给些），几块是选中的内容全文加这篇的开头一段。
use serde_json::json;

use crate::tavern::assemble::{self, Identity, Request};
use crate::tavern::macros::{Env, Macros};
use crate::tavern::preset::Preset;
use crate::tavern::shape::Shaper;

use super::{Act, Attached, Call, Core, Reply, Role, Shaped, Shelf, Turn, agent::title, clip, ends};

pub struct Tavern {
    preset: Preset,
    char: Identity,
    persona: Identity,
    model: String,
    seed: u64,
    macros: Macros,
    shaper: Shaper,
}

impl Tavern {
    pub fn new(preset: Preset, char: Identity, persona: Identity, model: String, seed: u64) -> Tavern {
        let mut macros = Macros::new(Env { user: persona.name.clone(), char: char.name.clone(), model: model.clone(), seed, ..Default::default() });
        let shaper = Shaper::new(&preset, &mut macros);
        Tavern { preset, char, persona, model, seed, macros, shaper }
    }
}

/// 备好的素材（一条系统消息的正文）。什么都没有就是空的。
pub fn material(shelf: &Shelf) -> String {
    let mut parts: Vec<String> = vec![];
    let guide = shelf.guide.trim();
    if !guide.is_empty() { parts.push(format!("用户写给你的空间说明（SPACE.md）：\n\n{}", clip(guide, 4000))); }
    if !shelf.docs.is_empty() {
        let titles: Vec<String> = shelf.recent(20).iter().map(|d| format!("《{}》", title(&d.title))).collect();
        parts.push(format!("用户在{}里和你对话。空间里有 {} 篇文档，最近改过的：{}。", super::agent::space(shelf), shelf.docs.len(), titles.join("、")));
    }
    // 几篇一起时每篇少给些，免得一条消息把预算吃光。
    let docs = shelf.attached.iter().filter(|a| matches!(a, Attached::Doc(f) if f.picked.is_none())).count().max(1);
    for a in &shelf.attached {
        parts.push(match a {
            Attached::Folder { name, docs, .. } => {
                let titles: Vec<String> = docs.iter().take(40).map(|id| format!("《{}》", title(&shelf.find(id).map(|d| d.title.clone()).unwrap_or_default()))).collect();
                format!("用户附带了文件夹「{}」，里面有 {} 篇文档：{}。", title(name), docs.len(), titles.join("、"))
            }
            Attached::Doc(f) => match &f.picked {
                Some(p) => format!("用户附带了文档《{}》里的这些内容：\n\n{}\n\n（这篇文档的开头：{}）", title(&f.title), clip(p, 6000), clip(&f.markdown, 1200)),
                None => format!("用户附带了文档《{}》，全文如下：\n\n{}", title(&f.title), ends(&f.markdown, (6000 / docs).max(1500), (2000 / docs).max(500))),
            },
        });
    }
    if parts.is_empty() { String::new() } else { format!("[资料]\n{}", parts.join("\n\n")) }
}

impl Core for Tavern {
    fn begin(&mut self, input: &str, history: &[Turn], shelf: &Shelf) -> Act {
        let mut turns: Vec<assemble::Turn> = history.iter().map(|t| assemble::Turn { role: t.role, text: t.text.clone() }).collect();
        let m = material(shelf);
        if !m.is_empty() { turns.push(assemble::Turn { role: Role::System, text: m }); }
        let a = assemble::assemble(&Request {
            preset: &self.preset, char: self.char.clone(), persona: self.persona.clone(), history: turns, input: input.to_owned(),
            model: self.model.clone(), model_context: None, extended_params: false, prefill: false, now: shelf.now, seed: self.seed,
        });
        let mut body = json!({ "messages": a.messages.iter().map(|m| m.to_json()).collect::<Vec<_>>() });
        for (k, v) in a.params.body(false) { body[k] = v; }
        Act::Ask(body)
    }

    fn reply(&mut self, reply: Reply) -> Act { Act::Finish(reply.text) }

    fn results(&mut self, _: &Shelf, _: Vec<(Call, String)>) -> Act { Act::Finish(String::new()) }

    fn settled(&self, raw: &str) -> usize { self.shaper.settled(raw) }

    fn shape(&mut self, raw: &str, id: &str, preview: bool) -> Shaped {
        // 预览用宏的副本：`{{random}}` 这些每次一样，变量不会越跑越多（同 iOS 的 `assist::pump`）。
        let mut macros = if preview { self.macros.clone() } else { std::mem::replace(&mut self.macros, Macros::new(Env::default())) };
        let stored = self.shaper.store(raw, &mut macros);
        let blocks = self.shaper.shape(&stored, id, &mut macros).blocks;
        if !preview { self.macros = macros; }
        Shaped { stored, blocks }
    }
}
