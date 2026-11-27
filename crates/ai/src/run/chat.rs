//! 助手的对话记录：空间里的 `assistant/chats/<标识>.json`，一段对话一个文件，跟着空间走 iCloud。和核心无关：
//! 记的是谁说了什么、助手那边走了哪几步、用的哪个核心和模型。下一次 run 的「之前的对话」从这里来（`history`）。
//! 读得宽松：缺的字段按默认，读不了的文件跳过。
use std::path::{Path, PathBuf};

use fdl_core::file_io;
use serde_json::{Value, json};

use super::{Kind, Role, Scope, Turn, clip};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Item {
    pub role: Role,
    /// 存下来的原文（助手那边是成形时存的样子）。
    pub text: String,
    /// 助手那边走过的步骤（工具调用），给人看的说法。
    pub steps: Vec<String>,
    pub at: u64,
    pub model: String,
    /// 没说完就出错了：错误的说法。
    pub failed: Option<String>,
    /// 用户中途停下的。
    pub stopped: bool,
}

#[derive(Clone, Debug, PartialEq)]
/// `picked`：范围是选中的块时，进来那会儿选中的内容（Markdown）。块的标识不跨启动，所以内容记在对话里。
pub struct Chat { pub id: String, pub title: String, pub scope: Scope, pub core: Kind, pub created: u64, pub picked: String, pub items: Vec<Item> }


/// 文件放哪：空间根目录下。
pub fn dir(root: &Path) -> PathBuf { root.join("assistant").join("chats") }

fn text(v: &Value, k: &str) -> String { v.get(k).and_then(Value::as_str).unwrap_or_default().to_owned() }

fn scope_json(s: &Scope) -> Value {
    match s {
        Scope::Space => json!({ "kind": "space" }),
        Scope::Doc { id } => json!({ "kind": "doc", "id": id }),
        Scope::Blocks { id, blocks } => json!({ "kind": "blocks", "id": id, "blocks": blocks }),
    }
}
fn scope_read(v: &Value) -> Scope {
    let id = text(v, "id");
    match v.get("kind").and_then(Value::as_str) {
        Some("doc") if !id.is_empty() => Scope::Doc { id },
        Some("blocks") if !id.is_empty() => Scope::Blocks { id, blocks: v.get("blocks").and_then(Value::as_array).map_or_else(Vec::new, |a| a.iter().filter_map(|b| b.as_str().map(str::to_owned)).collect()) },
        _ => Scope::Space,
    }
}
fn role_str(r: Role) -> &'static str { match r { Role::User => "user", Role::Assistant => "assistant", Role::System => "system" } }

impl Chat {
    pub fn new(id: String, scope: Scope, core: Kind, now: u64) -> Chat { Chat { id, title: String::new(), scope, core, created: now, picked: String::new(), items: vec![] } }

    /// 最后一次说话的时间。
    pub fn updated(&self) -> u64 { self.items.last().map_or(self.created, |i| i.at) }

    /// 标题：没起名就取第一句话的开头。
    pub fn name(&self) -> String {
        if !self.title.trim().is_empty() { return self.title.trim().to_owned(); }
        self.items.iter().find(|i| i.role == Role::User).map(|i| clip(i.text.trim().lines().next().unwrap_or_default(), 24)).unwrap_or_default()
    }

    /// 下一次 run 的「之前的对话」：出错、空着的不算；停下的照样算（说到哪儿算哪儿）。
    pub fn history(&self) -> Vec<Turn> {
        self.items.iter().filter(|i| i.failed.is_none() && !i.text.trim().is_empty()).map(|i| Turn { role: i.role, text: i.text.clone() }).collect()
    }

    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id, "title": self.title, "scope": scope_json(&self.scope), "core": self.core.id(), "created": self.created,
            "items": self.items.iter().map(|i| {
                let mut v = json!({ "role": role_str(i.role), "text": i.text, "at": i.at });
                if !i.steps.is_empty() { v["steps"] = json!(i.steps); }
                if !i.model.is_empty() { v["model"] = json!(i.model); }
                if let Some(f) = &i.failed { v["failed"] = json!(f); }
                if i.stopped { v["stopped"] = json!(true); }
                v
            }).collect::<Vec<_>>(),
            "picked": self.picked,
        })
    }

    pub fn read(v: &Value) -> Option<Chat> {
        let id = text(v, "id");
        if id.is_empty() { return None; }
        let items = v.get("items").and_then(Value::as_array).map_or_else(Vec::new, |a| a.iter().map(|i| Item {
            role: match i.get("role").and_then(Value::as_str) { Some("assistant") => Role::Assistant, Some("system") => Role::System, _ => Role::User },
            text: text(i, "text"),
            steps: i.get("steps").and_then(Value::as_array).map_or_else(Vec::new, |s| s.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect()),
            at: i.get("at").and_then(Value::as_u64).unwrap_or(0),
            model: text(i, "model"),
            failed: i.get("failed").and_then(Value::as_str).map(str::to_owned),
            stopped: i.get("stopped").and_then(Value::as_bool).unwrap_or(false),
        }).collect());
        Some(Chat { id, title: text(v, "title"), scope: v.get("scope").map_or(Scope::Space, scope_read), core: Kind::read(&text(v, "core")),
            created: v.get("created").and_then(Value::as_u64).unwrap_or(0), picked: text(v, "picked"), items })
    }
}

/// 写一段对话（先写临时文件再改名）。
pub fn save(dir: &Path, chat: &Chat) -> std::io::Result<()> {
    let bytes = serde_json::to_vec_pretty(&chat.to_json()).map_err(std::io::Error::other)?;
    file_io::create_dir_all(dir)?;
    file_io::with_write(&dir.join(format!("{}.json", chat.id)), |p| {
        let tmp = p.with_extension("json.tmp");
        std::fs::write(&tmp, &bytes)?;
        Ok(std::fs::rename(&tmp, p)?)
    }).map_err(std::io::Error::other)
}

pub fn load(dir: &Path, id: &str) -> Option<Chat> {
    file_io::read(dir.join(format!("{id}.json"))).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()).and_then(|v| Chat::read(&v))
}

pub fn delete(dir: &Path, id: &str) -> std::io::Result<()> { file_io::remove_file(dir.join(format!("{id}.json"))) }

/// 所有对话，最近说过话的在前。没下载的（只有 `.icloud` 占位）跳过。
pub fn list(dir: &Path) -> Vec<Chat> {
    let Ok(entries) = file_io::read_dir(dir) else { return vec![] };
    let mut out: Vec<Chat> = entries.flatten().filter_map(|e| {
        let p = e.path();
        if p.extension().is_none_or(|x| x != "json") { return None; }
        file_io::read(&p).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()).and_then(|v| Chat::read(&v))
    }).filter(|c| !c.items.is_empty()).collect();
    out.sort_by_key(|c| std::cmp::Reverse(c.updated()));
    out
}
