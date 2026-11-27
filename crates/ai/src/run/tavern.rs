//! 酒馆核心：照酒馆预设拼一次请求（`tavern::assemble`），模型说完就完，不调工具。输出照预设的正则成形（`Shaper`）。
//!
//! 素材的填法：能塞的一次塞进去，作为一条系统消息排在这次的话前面（聊天记录超出预算时从最早的丢，它挨着这次的话，最后才丢）。
//! - 整个空间：空间名、文档篇数和最近改过的几篇标题；
//! - 一篇文档：标题和正文（太长取头尾）；
//! - 选中的块：选中的内容全文，再加这篇的标题和开头一段。
use serde_json::json;

use crate::tavern::assemble::{self, Identity, Request};
use crate::tavern::macros::{Env, Macros};
use crate::tavern::preset::Preset;
use crate::tavern::shape::Shaper;

use super::{Act, Call, Core, Reply, Role, Scope, Shaped, Shelf, Turn, clip, ends};

pub struct Tavern {
    preset: Preset,
    char: Identity,
    persona: Identity,
    model: String,
    seed: u64,
    scope: Scope,
    macros: Macros,
    shaper: Shaper,
}

impl Tavern {
    pub fn new(preset: Preset, char: Identity, persona: Identity, model: String, seed: u64, scope: Scope) -> Tavern {
        let mut macros = Macros::new(Env { user: persona.name.clone(), char: char.name.clone(), model: model.clone(), seed, ..Default::default() });
        let shaper = Shaper::new(&preset, &mut macros);
        Tavern { preset, char, persona, model, seed, scope, macros, shaper }
    }
}

/// 照范围备好的素材（一条系统消息的正文）。没有就是空的。
pub fn material(scope: &Scope, shelf: &Shelf) -> String {
    match (scope, shelf.focus.as_ref()) {
        (Scope::Space, _) => {
            if shelf.docs.is_empty() { return String::new(); }
            let titles: Vec<String> = shelf.recent(20).iter().map(|d| format!("《{}》", if d.title.trim().is_empty() { "无标题" } else { d.title.trim() })).collect();
            format!("[资料] 用户在{}里和你对话。空间里有 {} 篇文档，最近改过的：{}。", super::agent::space(shelf), shelf.docs.len(), titles.join("、"))
        }
        (Scope::Doc { .. }, Some(f)) => format!("[资料] 用户正在看文档《{}》，全文如下：\n\n{}", f.title, ends(&f.markdown, 6000, 2000)),
        (Scope::Blocks { .. }, Some(f)) => format!("[资料] 用户在文档《{}》里选中了下面这些内容：\n\n{}\n\n（这篇文档的开头：{}）",
            f.title, clip(f.picked.as_deref().unwrap_or_default(), 6000), clip(&f.markdown, 1200)),
        _ => String::new(),
    }
}

impl Core for Tavern {
    fn begin(&mut self, input: &str, history: &[Turn], shelf: &Shelf) -> Act {
        let mut turns: Vec<assemble::Turn> = history.iter().map(|t| assemble::Turn { role: t.role, text: t.text.clone() }).collect();
        let m = material(&self.scope, shelf);
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
