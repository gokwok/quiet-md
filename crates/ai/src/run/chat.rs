//! 助手的对话记录：空间里的 `assistant/chats/<标识>.json`，一段对话一个文件，跟着空间走 iCloud。和核心无关：
//! 记的是谁说了什么（用户的每句话带着当时附带的资源）、助手那边走了哪几步、用的哪个助手、核心和模型、权限。
//! 下一次 run 的「之前的对话」从这里来（`history`）。读得宽松：缺的字段按默认，读不了的文件跳过。
//! 旧的对话（整段一个范围 `scope`、选中的块 `picked`、一定写了核心）读进来时，范围挂到第一句话上，核心当成这段自己选的。
use std::path::{Path, PathBuf};

use fdl_core::file_io;
use serde_json::{Value, json};

use super::{Kind, Permission, Resource, Role, Turn, clip};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Item {
    pub role: Role,
    /// 存下来的原文（助手那边是成形时存的样子）。
    pub text: String,
    /// 用户这句话附带的资源。
    pub resources: Vec<Resource>,
    /// 助手那边走过的步骤（工具调用），给人看的说法。
    pub steps: Vec<String>,
    pub at: u64,
    pub model: String,
    /// 没说完就出错了：错误的说法。
    pub failed: Option<String>,
    /// 用户中途停下的。
    pub stopped: bool,
}

/// 一段对话。`assistant`：这段自己选的助手（`None` 跟着空间的默认助手）；`core`：临时换的核心（`None` 用助手自己的）。
#[derive(Clone, Debug, PartialEq)]
pub struct Chat { pub id: String, pub title: String, pub assistant: Option<String>, pub core: Option<Kind>, pub permission: Permission, pub created: u64, pub items: Vec<Item> }


/// 文件放哪：空间根目录下。
pub fn dir(root: &Path) -> PathBuf { root.join("assistant").join("chats") }

fn text(v: &Value, k: &str) -> String { v.get(k).and_then(Value::as_str).unwrap_or_default().to_owned() }
fn strings(v: &Value, k: &str) -> Vec<String> { v.get(k).and_then(Value::as_array).map_or_else(Vec::new, |a| a.iter().filter_map(|b| b.as_str().map(str::to_owned)).collect()) }

pub fn resource_json(r: &Resource) -> Value {
    match r {
        Resource::Folder { id } => json!({ "kind": "folder", "id": id }),
        Resource::Document { id } => json!({ "kind": "document", "id": id }),
        Resource::Blocks { doc, blocks, text } => json!({ "kind": "blocks", "doc": doc, "blocks": blocks, "text": text }),
    }
}
pub fn resource_read(v: &Value) -> Option<Resource> {
    let id = text(v, "id");
    match v.get("kind").and_then(Value::as_str)? {
        "folder" if !id.is_empty() => Some(Resource::Folder { id }),
        "document" if !id.is_empty() => Some(Resource::Document { id }),
        "blocks" => Some(Resource::Blocks { doc: Some(text(v, "doc")).filter(|d| !d.is_empty())?, blocks: strings(v, "blocks"), text: text(v, "text") }),
        _ => None,
    }
}
/// 旧格式的范围：整个空间是没有资源。
fn old_scope(v: &Value, picked: String) -> Option<Resource> {
    let id = Some(text(v, "id")).filter(|d| !d.is_empty())?;
    match v.get("kind").and_then(Value::as_str)? {
        "doc" => Some(Resource::Document { id }),
        "blocks" => Some(Resource::Blocks { doc: id, blocks: strings(v, "blocks"), text: picked }),
        _ => None,
    }
}
fn role_str(r: Role) -> &'static str { match r { Role::User => "user", Role::Assistant => "assistant", Role::System => "system" } }

impl Chat {
    pub fn new(id: String, now: u64) -> Chat { Chat { id, title: String::new(), assistant: None, core: None, permission: Permission::Read, created: now, items: vec![] } }

    /// 最后一次说话的时间。
    pub fn updated(&self) -> u64 { self.items.last().map_or(self.created, |i| i.at) }

    /// 标题：没起名就取第一句话的开头。
    pub fn name(&self) -> String {
        if !self.title.trim().is_empty() { return self.title.trim().to_owned(); }
        self.items.iter().find(|i| i.role == Role::User).map(|i| clip(i.text.trim().lines().next().unwrap_or_default(), 24)).unwrap_or_default()
    }

    /// 用户说过话没有（开场白不算）。
    pub fn started(&self) -> bool { self.items.iter().any(|i| i.role == Role::User) }

    /// 最后一句话附带的资源（接着说时默认还带这些）。
    pub fn last_resources(&self) -> Option<&[Resource]> { self.items.iter().rev().find(|i| i.role == Role::User).map(|i| i.resources.as_slice()) }

    /// 下一次 run 的「之前的对话」：出错、空着的不算；停下的照样算（说到哪儿算哪儿）。
    pub fn history(&self) -> Vec<Turn> {
        self.items.iter().filter(|i| i.failed.is_none() && !i.text.trim().is_empty()).map(|i| Turn { role: i.role, text: i.text.clone() }).collect()
    }

    pub fn to_json(&self) -> Value {
        let mut v = json!({
            "id": self.id, "title": self.title, "permission": self.permission.id(), "created": self.created,
            "items": self.items.iter().map(|i| {
                let mut v = json!({ "role": role_str(i.role), "text": i.text, "at": i.at });
                if !i.resources.is_empty() { v["resources"] = i.resources.iter().map(resource_json).collect(); }
                if !i.steps.is_empty() { v["steps"] = json!(i.steps); }
                if !i.model.is_empty() { v["model"] = json!(i.model); }
                if let Some(f) = &i.failed { v["failed"] = json!(f); }
                if i.stopped { v["stopped"] = json!(true); }
                v
            }).collect::<Vec<_>>(),
        });
        if let Some(a) = &self.assistant { v["assistant"] = json!(a); }
        if let Some(k) = self.core { v["core"] = json!(k.id()); }
        v
    }

    pub fn read(v: &Value) -> Option<Chat> {
        let id = text(v, "id");
        if id.is_empty() { return None; }
        let mut items: Vec<Item> = v.get("items").and_then(Value::as_array).map_or_else(Vec::new, |a| a.iter().map(|i| Item {
            role: match i.get("role").and_then(Value::as_str) { Some("assistant") => Role::Assistant, Some("system") => Role::System, _ => Role::User },
            text: text(i, "text"),
            resources: i.get("resources").and_then(Value::as_array).map_or_else(Vec::new, |a| a.iter().filter_map(resource_read).collect()),
            steps: strings(i, "steps"),
            at: i.get("at").and_then(Value::as_u64).unwrap_or(0),
            model: text(i, "model"),
            failed: i.get("failed").and_then(Value::as_str).map(str::to_owned),
            stopped: i.get("stopped").and_then(Value::as_bool).unwrap_or(false),
        }).collect());
        // 旧格式：整段的范围挂到第一句话上。
        if let Some(r) = v.get("scope").and_then(|s| old_scope(s, text(v, "picked")))
            && let Some(first) = items.iter_mut().find(|i| i.role == Role::User && i.resources.is_empty()) { first.resources.push(r); }
        Some(Chat { id, title: text(v, "title"), assistant: Some(text(v, "assistant")).filter(|a| !a.is_empty()), core: Kind::parse(&text(v, "core")),
            permission: Permission::parse(&text(v, "permission")), created: v.get("created").and_then(Value::as_u64).unwrap_or(0), items })
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
    }).filter(|c| c.started()).collect();
    out.sort_by_key(|c| std::cmp::Reverse(c.updated()));
    out
}
