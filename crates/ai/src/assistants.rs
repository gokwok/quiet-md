//! MarginAI 的助手：有名字、头像、设定和开场白，由一个核心驱动（酒馆或 Agent），带默认权限、模型、形象和声线。
//! 存在全局库（`global`）的 `assistants.json`，所有空间共用；读得宽松：坏掉的条目跳过，缺的字段按默认。
//! 总有一个内置的「MarginAI」（标识 `marginai`，Agent 核心）：能改不能删，排在最前。
//!
//! 用哪个助手：对话自己选的 > 空间的默认助手（`space.json` 的 `assistant`）> 内置的 MarginAI（`resolve`）。
//! 迁移（`migrate`）：写作设置里的每个助手卡片（`Settings::chars`）变成一个酒馆核心的助手（预设是写作设置选中的那份），
//! 同名同设定的合成一个；并过的卡片记下标识，不再并第二次（之后从别的空间并进来的卡片下次照样并）。
use std::collections::BTreeSet;
use std::io;
use std::path::Path;

use fdl_core::file_io::{self, DownloadState};
use serde_json::{Map, Value, json};

use crate::tavern::library::{self, Settings};
use crate::tavern::preset::text;

/// 内置助手的标识。
pub const BUILTIN: &str = "marginai";
const FILE: &str = "assistants.json";

/// 核心的种类。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind { Tavern, Agent }

impl Kind {
    pub fn id(self) -> &'static str { match self { Kind::Tavern => "tavern", Kind::Agent => "agent" } }
    fn parse(s: &str) -> Option<Kind> { match s { "tavern" => Some(Kind::Tavern), "agent" => Some(Kind::Agent), _ => None } }
}

/// 驱动助手的核心：种类和它的配置（酒馆是 `preset`，Agent 是 `rounds`）。两种的配置放在同一个对象里，换种类时另一种的留着；
/// 不认识的种类按 Agent 跑，原样存回去。
#[derive(Clone, Debug, PartialEq)]
pub struct Core { kind: String, pub config: Map<String, Value> }

impl Core {
    pub fn tavern(preset: &str) -> Core { let mut c = Core { kind: Kind::Tavern.id().into(), config: Map::new() }; c.set_preset(preset); c }
    pub fn agent() -> Core { Core { kind: Kind::Agent.id().into(), config: Map::new() } }
    pub fn kind(&self) -> Kind { Kind::parse(&self.kind).unwrap_or(Kind::Agent) }
    pub fn set_kind(&mut self, kind: Kind) { self.kind = kind.id().into(); }
    /// 酒馆的预设（全局库 `presets/` 里的标识）；空的是内置的「默认」。
    pub fn preset(&self) -> &str { self.config.get("preset").and_then(Value::as_str).unwrap_or_default() }
    pub fn set_preset(&mut self, id: &str) { self.config.insert("preset".into(), id.into()); }
    /// Agent 最多调几轮工具。
    pub fn rounds(&self) -> usize { self.config.get("rounds").and_then(Value::as_u64).map_or(crate::run::agent::ROUNDS, |n| (n as usize).clamp(1, 32)) }
    pub fn set_rounds(&mut self, n: usize) { self.config.insert("rounds".into(), n.clamp(1, 32).into()); }
}

/// 助手能不能改文档：默认只读，问了才改。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Permission { #[default] Read, Write }

impl Permission {
    pub fn id(self) -> &'static str { match self { Permission::Read => "read", Permission::Write => "write" } }
}

/// 指定的模型：哪个服务商（这台设备的偏好里的标识）的哪个模型，选的时候的显示名。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Model { pub provider: String, pub model: String, pub name: String }

#[derive(Clone, Debug, PartialEq)]
pub struct Assistant {
    pub id: String,
    pub name: String,
    /// 头像：和语音角色同一套标识（自带的 `builtin-NN`，或全局库 `avatars/` 里的照片）。
    pub avatar: Option<String>,
    /// 设定。
    pub description: String,
    /// 开场白，空的是没有。
    pub greeting: String,
    pub core: Core,
    pub permission: Permission,
    /// `None` 跟着全局的主模型。
    pub model: Option<Model>,
    /// 形象：图像库（`imagine.json`）里的人物标识。
    pub character: Option<String>,
    /// 声线：语音角色的音色标识（`voices.json`）。
    pub voice: Option<String>,
}

impl Assistant {
    pub fn new(id: &str, name: &str, core: Core) -> Assistant {
        Assistant { id: id.into(), name: name.into(), avatar: None, description: String::new(), greeting: String::new(), core, permission: Permission::Read, model: None, character: None, voice: None }
    }
    /// 内置的 MarginAI。
    pub fn builtin() -> Assistant {
        Assistant { description: "你是 MarginAI，住在用户笔记旁边的助手。先读再答，说话简洁；拿不准的地方直说。".into(), ..Assistant::new(BUILTIN, "MarginAI", Core::agent()) }
    }
    pub fn is_builtin(&self) -> bool { self.id == BUILTIN }

    fn read(v: &Value) -> Option<Assistant> {
        let id = text(v, "id").filter(|s| !s.is_empty())?;
        let opt = |k: &str| text(v, k).filter(|s| !s.is_empty());
        let core = v.get("core").map_or_else(Core::agent, |c| Core {
            kind: text(c, "kind").filter(|s| !s.is_empty()).unwrap_or_else(|| Kind::Agent.id().into()),
            config: c.get("config").and_then(Value::as_object).cloned().unwrap_or_default(),
        });
        let model = v.get("model").filter(|m| m.is_object()).and_then(|m| {
            let model = text(m, "model").filter(|s| !s.is_empty())?;
            Some(Model { provider: text(m, "provider").unwrap_or_default(), model, name: text(m, "name").unwrap_or_default() })
        });
        Some(Assistant {
            name: text(v, "name").unwrap_or_default(), avatar: opt("avatar"), description: text(v, "description").unwrap_or_default(), greeting: text(v, "greeting").unwrap_or_default(),
            core, permission: if text(v, "permission").as_deref() == Some("write") { Permission::Write } else { Permission::Read },
            model, character: opt("character"), voice: opt("voice"), id,
        })
    }

    fn to_json(&self) -> Value {
        json!({
            "id": self.id, "name": self.name, "avatar": self.avatar, "description": self.description, "greeting": self.greeting,
            "core": { "kind": self.core.kind, "config": self.core.config }, "permission": self.permission.id(),
            "model": self.model.as_ref().map(|m| json!({ "provider": m.provider, "model": m.model, "name": m.name })),
            "character": self.character, "voice": self.voice,
        })
    }
}

/// 全部助手（内置的排最前）和并过的卡片。
#[derive(Clone, Debug, PartialEq)]
pub struct Assistants { pub list: Vec<Assistant>, migrated: BTreeSet<String> }

impl Default for Assistants { fn default() -> Assistants { Assistants { list: vec![Assistant::builtin()], migrated: BTreeSet::new() } } }

impl Assistants {
    pub fn load(dir: &Path) -> Assistants {
        let Some(v) = file_io::read(dir.join(FILE)).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()) else { return Assistants::default() };
        let mut list: Vec<Assistant> = v.get("assistants").and_then(Value::as_array).map_or_else(Vec::new, |a| a.iter().filter_map(Assistant::read).collect());
        let mut seen = BTreeSet::new();
        list.retain(|a| seen.insert(a.id.clone()));
        match list.iter().position(Assistant::is_builtin) { Some(0) => {} Some(i) => { let b = list.remove(i); list.insert(0, b); } None => list.insert(0, Assistant::builtin()) }
        let migrated = v.get("migrated").and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect()).unwrap_or_default();
        Assistants { list, migrated }
    }

    pub fn save(&self, dir: &Path) -> io::Result<()> {
        let v = json!({ "assistants": self.list.iter().map(Assistant::to_json).collect::<Vec<_>>(), "migrated": self.migrated });
        library::write(&dir.join(FILE), &serde_json::to_vec_pretty(&v).map_err(io::Error::other)?)
    }

    pub fn get(&self, id: &str) -> Option<&Assistant> { self.list.iter().find(|a| a.id == id) }
    pub fn get_mut(&mut self, id: &str) -> Option<&mut Assistant> { self.list.iter_mut().find(|a| a.id == id) }
    /// 照顺序找第一个还在的（对话的、空间的），都不在是内置的。
    pub fn resolve(&self, ids: &[Option<&str>]) -> &Assistant {
        ids.iter().flatten().find_map(|id| self.get(id)).unwrap_or(&self.list[0])
    }
    /// 删掉一个；内置的删不了。返回删了没有。
    pub fn remove(&mut self, id: &str) -> bool {
        let n = self.list.len();
        self.list.retain(|a| a.is_builtin() || a.id != id);
        self.list.len() != n
    }
    /// 复制一份（新标识 `new_id`、名字 `name`），排在原来那个后面。
    pub fn duplicate(&mut self, id: &str, new_id: &str, name: &str) -> bool {
        let Some(i) = self.list.iter().position(|a| a.id == id) else { return false };
        if self.get(new_id).is_some() { return false; }
        let copy = Assistant { id: new_id.into(), name: name.into(), ..self.list[i].clone() };
        self.list.insert(i + 1, copy);
        true
    }
}

fn ready(path: &Path) -> bool { matches!(file_io::download_state(path), DownloadState::Ready | DownloadState::Missing) }

/// 把写作设置里还没并过的助手卡片变成助手（见模块说明）。「/」卡片还没选过助手时，选上原来选中的那张卡片变成的助手。
/// 两份文件还在从 iCloud 下载时这次不并。返回新加了几个。
pub fn migrate(dir: &Path) -> io::Result<usize> {
    let (file, settings_file) = (dir.join(FILE), dir.join("settings.json"));
    if !ready(&file) || !ready(&settings_file) || file_io::metadata(&settings_file).is_err() { return Ok(0); }
    let mut s = Settings::load(dir);
    let mut all = Assistants::load(dir);
    let pristine = Settings::default().chars;
    let (mut added, mut picked) = (0, None);
    for c in &s.chars {
        if all.migrated.contains(&c.id) { continue; }
        all.migrated.insert(c.id.clone());
        // 没动过的内置「写作助手」不变成助手：有内置的 MarginAI 了。
        if pristine.contains(c) { continue; }
        let to = match all.list.iter().find(|a| !a.is_builtin() && a.name == c.name && a.description == c.description) {
            Some(a) => a.id.clone(),
            None => {
                let id = if all.get(&c.id).is_none() { c.id.clone() } else { (2..).map(|n| format!("{}-m{n}", c.id)).find(|x| all.get(x).is_none()).unwrap_or_default() };
                all.list.push(Assistant { description: c.description.clone(), ..Assistant::new(&id, &c.name, Core::tavern(&s.preset)) });
                added += 1;
                id
            }
        };
        if c.id == s.char { picked = Some(to); }
    }
    if all == Assistants::load(dir) { return Ok(0); }
    all.save(dir)?;
    if let Some(id) = picked.filter(|_| s.assistant.is_empty()) { s.assistant = id; s.save(dir)?; }
    Ok(added)
}

#[cfg(test)]
mod tests;
