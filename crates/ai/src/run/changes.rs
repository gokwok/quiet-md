//! 一次 run 里助手改了什么（改动日志），和回滚。记在那一轮回复上（`chat::Item::changes`），随对话记录存，重启后照样能回滚。
//!
//! 每样只记回滚要用的：改正文的记第一次改之前的整篇（DocxXML；`SPACE.md` 是原文）和改完的指纹；新建的记标识；
//! 移动的记移动前在哪（上一层和后一个）；改名的记原来的名字；删除、恢复的记标识。同一篇改几次只记一条（最早的「之前」，最新的指纹）。
//!
//! 回滚倒着做：正文整篇回到助手改之前，新建的移到最近删除，移动、改名改回去，删了的恢复，恢复的再删掉。
//! 之后有人手动改过（现在的指纹和记下的不一样）先提示一次（`edited`）。
use fdl_core::model::Document;
use serde_json::{Value, json};

use super::write::Host;
use super::tools::SPACE_MD;

#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    /// 改了正文。`before`：改之前的整篇（DocxXML；`SPACE.md` 是原文）；`after`：改完的指纹。
    Edit { id: String, title: String, before: String, after: u64 },
    /// 新建的文档或文件夹；`after`：新建时正文的指纹（文件夹是 0）。
    Create { id: String, title: String, folder: bool, after: u64 },
    /// 移动：原来挂在哪（`None` 顶层），原来后一个是谁（回去排在它前面）。
    Move { id: String, title: String, parent: Option<String>, next: Option<String> },
    /// 改名：`title` 是新名字，`was` 是原来的。
    Rename { id: String, title: String, was: String },
    Trash { id: String, title: String },
    Restore { id: String, title: String },
}

impl Op {
    pub fn id(&self) -> &str {
        match self { Op::Edit { id, .. } | Op::Create { id, .. } | Op::Move { id, .. } | Op::Rename { id, .. } | Op::Trash { id, .. } | Op::Restore { id, .. } => id }
    }
    /// 给人看的一行。
    pub fn line(&self) -> String {
        let t = |s: &str| if s.trim().is_empty() { "无标题".to_owned() } else { s.trim().to_owned() };
        match self {
            Op::Edit { id, .. } if id == SPACE_MD => "改了 SPACE.md".into(),
            Op::Edit { title, .. } => format!("改了《{}》", t(title)),
            Op::Create { title, folder: true, .. } => format!("新建了文件夹「{}」", t(title)),
            Op::Create { title, .. } => format!("新建了《{}》", t(title)),
            Op::Move { title, .. } => format!("移动了《{}》", t(title)),
            Op::Rename { title, was, .. } => format!("把《{}》改名为《{}》", t(was), t(title)),
            Op::Trash { title, .. } => format!("删除了《{}》", t(title)),
            Op::Restore { title, .. } => format!("恢复了《{}》", t(title)),
        }
    }
}

/// 一次 run 的改动。`undone`：已经回滚过了。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Changes { pub ops: Vec<Op>, pub undone: bool }

/// 正文的指纹（标题不算：改名另记）：DocxXML 的 FNV-1a。块标识不进 DocxXML，读回来一样就一样。
pub fn print(doc: &Document) -> u64 {
    let mut d = Document { title: Default::default(), ..doc.clone() };
    d.icon = None;
    d.cover = None;
    print_text(&fdl_core::docxml::serialize(&d))
}
pub fn print_text(s: &str) -> u64 { s.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3)) }

impl Changes {
    pub fn is_empty(&self) -> bool { self.ops.is_empty() }

    /// 记一样。同一篇的几次合成一条。
    pub fn push(&mut self, op: Op) {
        let id = op.id().to_owned();
        let created = self.ops.iter().any(|o| matches!(o, Op::Create { id: i, .. } if *i == id));
        match op {
            Op::Edit { after, title, .. } if created || self.ops.iter().any(|o| matches!(o, Op::Edit { id: i, .. } if *i == id)) => {
                for o in &mut self.ops {
                    match o {
                        Op::Edit { id: i, after: a, title: t, .. } | Op::Create { id: i, after: a, title: t, .. } if *i == id => { *a = after; *t = title.clone(); }
                        _ => {}
                    }
                }
            }
            Op::Rename { title, .. } if created || self.ops.iter().any(|o| matches!(o, Op::Rename { id: i, .. } if *i == id)) => {
                for o in &mut self.ops { if let Op::Rename { id: i, title: t, .. } | Op::Create { id: i, title: t, .. } = o && *i == id { *t = title.clone(); } }
            }
            // 移过的再移：回滚回到最早的地方，记第一次就够。
            Op::Move { .. } if created || self.ops.iter().any(|o| matches!(o, Op::Move { id: i, .. } if *i == id)) => {}
            op => self.ops.push(op),
        }
    }

    /// 刚改完的指纹（这次 run 里再改时用来认出有没有人动过）。
    pub fn after(&self, id: &str) -> Option<u64> {
        self.ops.iter().find_map(|o| match o { Op::Edit { id: i, after, .. } | Op::Create { id: i, after, folder: false, .. } if i == id => Some(*after), _ => None })
    }

    /// 改了几篇（文档、文件夹、SPACE.md，各算一篇）。
    pub fn count(&self) -> usize {
        let mut ids: Vec<&str> = self.ops.iter().map(Op::id).collect();
        ids.sort_unstable();
        ids.dedup();
        ids.len()
    }

    pub fn lines(&self) -> Vec<String> { self.ops.iter().map(Op::line).collect() }

    /// 之后又被改过的几篇（标题）：现在的正文和助手改完时不一样了。找不到的（删了）不算，回滚时再说。
    pub fn edited(&self, host: &mut dyn Host) -> Vec<String> {
        let mut out = vec![];
        for o in &self.ops {
            let (id, title, after) = match o { Op::Edit { id, title, after, .. } | Op::Create { id, title, after, folder: false } => (id, title, *after), _ => continue };
            let now = if id == SPACE_MD { Some(print_text(&host.guide())) } else { host.load(id).map(|d| print(&d)) };
            if now.is_some_and(|n| n != after) { out.push(if id == SPACE_MD { SPACE_MD.to_owned() } else { format!("《{}》", if title.trim().is_empty() { "无标题" } else { title.trim() }) }); }
        }
        out
    }

    /// 回滚，倒着一样样做。做不了的几样（说法）返回，其余照做；做完记成已回滚。
    pub fn undo(&mut self, host: &mut dyn Host) -> Vec<String> {
        let mut problems = vec![];
        for o in self.ops.iter().rev() {
            let r = match o {
                Op::Edit { id, before, .. } if id == SPACE_MD => host.save_guide(before),
                Op::Edit { id, before, .. } => match fdl_core::docxml::parse(before) {
                    Ok(doc) if host.load(id).is_some() => host.put_body(id, doc.blocks).map(|_| ()),
                    Ok(_) => Err("找不到了（可能已经删了）".into()),
                    Err(e) => Err(format!("记下的原文读不出来：{e}")),
                },
                Op::Create { id, .. } | Op::Restore { id, .. } => host.trash(std::slice::from_ref(id)).map(|_| ()),
                Op::Move { id, parent, next, .. } => host.move_to(id, parent.as_deref(), next.as_deref()).or_else(|_| host.move_to(id, parent.as_deref(), None)),
                Op::Rename { id, was, .. } => host.rename(id, was),
                Op::Trash { id, .. } => host.restore(id),
            };
            if let Err(e) = r { problems.push(format!("{}：{e}", o.line())); }
        }
        self.undone = true;
        problems
    }

    pub fn to_json(&self) -> Value {
        let ops: Vec<Value> = self.ops.iter().map(|o| match o {
            Op::Edit { id, title, before, after } => json!({ "op": "edit", "id": id, "title": title, "before": before, "after": after.to_string() }),
            Op::Create { id, title, folder, after } => json!({ "op": "create", "id": id, "title": title, "folder": folder, "after": after.to_string() }),
            Op::Move { id, title, parent, next } => json!({ "op": "move", "id": id, "title": title, "parent": parent, "next": next }),
            Op::Rename { id, title, was } => json!({ "op": "rename", "id": id, "title": title, "was": was }),
            Op::Trash { id, title } => json!({ "op": "trash", "id": id, "title": title }),
            Op::Restore { id, title } => json!({ "op": "restore", "id": id, "title": title }),
        }).collect();
        let mut v = json!({ "ops": ops });
        if self.undone { v["undone"] = json!(true); }
        v
    }

    /// 读得宽松：认不出的一样跳过。
    pub fn read(v: &Value) -> Changes {
        let s = |o: &Value, k: &str| o.get(k).and_then(Value::as_str).unwrap_or_default().to_owned();
        let opt = |o: &Value, k: &str| o.get(k).and_then(Value::as_str).map(str::to_owned);
        let n = |o: &Value, k: &str| o.get(k).and_then(|x| x.as_str().and_then(|t| t.parse().ok()).or_else(|| x.as_u64())).unwrap_or(0);
        let ops = v.get("ops").and_then(Value::as_array).map_or_else(Vec::new, |a| a.iter().filter_map(|o| {
            let (id, title) = (s(o, "id"), s(o, "title"));
            if id.is_empty() { return None; }
            Some(match o.get("op").and_then(Value::as_str)? {
                "edit" => Op::Edit { id, title, before: s(o, "before"), after: n(o, "after") },
                "create" => Op::Create { id, title, folder: o.get("folder").and_then(Value::as_bool).unwrap_or(false), after: n(o, "after") },
                "move" => Op::Move { id, title, parent: opt(o, "parent"), next: opt(o, "next") },
                "rename" => Op::Rename { id, title, was: s(o, "was") },
                "trash" => Op::Trash { id, title },
                "restore" => Op::Restore { id, title },
                _ => return None,
            })
        }).collect());
        Changes { ops, undone: v.get("undone").and_then(Value::as_bool).unwrap_or(false) }
    }
}
