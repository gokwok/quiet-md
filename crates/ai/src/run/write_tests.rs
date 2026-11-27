//! 写的工具、改动日志和回滚：在临时文件夹里的真文档库上做（`Lib` 照 iOS 宿主的样子实现 `Host`）。
use std::collections::HashSet;
use std::path::PathBuf;

use fdl_core::editor::core::Core as Editor;
use fdl_core::model::{Align, Block, Document};
use fdl_core::store::Store;
use serde_json::Value;

use super::changes::{Changes, Op, print};
use super::write::{self, Host, Place};
use super::*;

/// 临时文档库（和设备上的状态一起放在临时文件夹里，用完删掉）。`open`：编辑器里开着的那篇。
struct Lib { root: PathBuf, store: Store, guide: String, open: Option<(String, Editor)> }

fn local_data() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    // 文档库的设备状态默认在用户的应用数据里：测试一律放进临时文件夹。
    ONCE.call_once(|| unsafe { std::env::set_var("QUIET_MD_LOCAL_DATA", std::env::temp_dir().join(format!("fdl-ai-write-local-{}", std::process::id()))) });
}

impl Lib {
    fn new() -> Lib {
        local_data();
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let root = std::env::temp_dir().join(format!("fdl-ai-write-{}-{}-{}", std::process::id(), fdl_core::time::now_ms(), N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)));
        let store = Store::open_at(root.clone()).unwrap();
        Lib { root, store, guide: String::new(), open: None }
    }
    fn doc(&mut self, title: &str, md: &str, parent: Option<&str>) -> String {
        let id = self.store.create_from(&fdl_core::markdown::parse_file(md, title, None)).unwrap();
        if parent.is_some() { self.store.set_parent(&id, parent).unwrap(); }
        id
    }
    /// 正文（编辑器里开着的用编辑器里的），一块一行。
    fn text(&mut self, id: &str) -> Vec<String> { self.load(id).unwrap().blocks.iter().map(|b| b.text.text.clone()).collect() }
    fn entry(&mut self, id: &str) -> Option<fdl_core::store::Entry> { self.store.list().into_iter().find(|e| e.id == id) }
    fn trashed(&mut self, id: &str) -> bool { self.store.list_trash().iter().any(|t| t.id == id) }
    /// 这一刻的快照，可以改文档。
    fn shelf(&mut self) -> Shelf {
        let reader = self.store.reader();
        let mut shelf = Shelf { library: Library::new(move |id| reader.load(id).ok()), permission: Permission::Write, guide: self.guide.clone(), ..Default::default() };
        if let Some((id, e)) = &self.open { shelf.library.put(id, e.doc.clone()); }
        self.refresh(&mut shelf);
        shelf
    }
}

impl Drop for Lib {
    fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.root); }
}

impl Host for Lib {
    fn load(&mut self, id: &str) -> Option<Document> {
        if let Some((o, e)) = &self.open && o == id { return Some(e.doc.clone()); }
        self.store.load(id).ok()
    }
    fn put_body(&mut self, id: &str, blocks: Vec<Block>) -> Result<(Document, u64), String> {
        if let Some((o, e)) = &mut self.open && o == id { e.replace_body(blocks); return Ok((e.doc.clone(), print(&e.doc))); }
        let mut e = Editor::new(self.store.load(id).map_err(|e| e.to_string())?);
        e.replace_body(blocks);
        self.store.save(id, &e.doc).map_err(|e| e.to_string())?;
        Ok((e.doc.clone(), print(&self.store.load(id).map_err(|e| e.to_string())?)))
    }
    fn guide(&mut self) -> String { self.guide.clone() }
    fn save_guide(&mut self, text: &str) -> Result<(), String> { self.guide = text.into(); Ok(()) }
    fn create(&mut self, doc: Document, parent: Option<&str>) -> Result<String, String> {
        let id = self.store.create_from(&doc).map_err(|e| e.to_string())?;
        if parent.is_some() { self.store.set_parent(&id, parent).map_err(|e| e.to_string())?; }
        Ok(id)
    }
    fn create_folder(&mut self, name: &str, parent: Option<&str>) -> Result<String, String> { self.store.create_folder(name, parent).map_err(|e| e.to_string()) }
    fn place(&mut self, id: &str) -> Option<Place> {
        let all = self.store.list();
        let me = all.iter().find(|e| e.id == id)?;
        let mut sib: Vec<_> = all.iter().filter(|e| e.meta.parent == me.meta.parent).collect();
        sib.sort_by(|a, b| (a.meta.place(), &a.id).cmp(&(b.meta.place(), &b.id)));
        let at = sib.iter().position(|e| e.id == id)?;
        Some(Place { parent: me.meta.parent.clone(), next: sib.get(at + 1).map(|e| e.id.clone()), local: me.meta.local() })
    }
    fn move_to(&mut self, id: &str, parent: Option<&str>, next: Option<&str>) -> Result<(), String> { self.store.move_in_tree(id, parent, next).map_err(|e| e.to_string()) }
    fn rename(&mut self, id: &str, title: &str) -> Result<(), String> {
        if let Some((o, e)) = &mut self.open && o == id { e.receive_file_title(title); }
        self.store.rename(id, title).map(|_| ()).map_err(|e| e.to_string())
    }
    fn trash(&mut self, ids: &[String]) -> Result<Vec<String>, String> {
        let all = self.store.list();
        let up = |id: &str| all.iter().find(|e| e.id == id).and_then(|e| e.meta.parent.clone());
        let under = |id: &str, root: &str| { let mut at = up(id); while let Some(p) = at { if p == root { return true; } at = up(&p); } false };
        let mut done = vec![];
        for id in ids.iter().filter(|id| !ids.iter().any(|o| under(id, o))) {
            self.store.delete(id, None).map_err(|e| e.to_string())?;
            for e in all.iter().filter(|e| under(&e.id, id)) { self.store.delete(&e.id, Some(id)).map_err(|e| e.to_string())?; }
            done.push(id.clone());
        }
        Ok(done)
    }
    fn restore(&mut self, id: &str) -> Result<(), String> { self.store.restore(id).map(|_| ()).map_err(|e| e.to_string()) }
    fn refresh(&mut self, shelf: &mut Shelf) {
        let all = self.store.list();
        let names: std::collections::HashMap<String, String> = all.iter().filter(|e| e.meta.folder.is_some()).map(|e| (e.id.clone(), e.title.clone())).collect();
        shelf.docs = all.iter().filter(|e| e.meta.folder.is_none()).map(|e| Doc {
            id: e.id.clone(), title: e.title.clone(), folder: e.meta.parent.as_ref().and_then(|p| names.get(p).cloned()), parent: e.meta.parent.clone(),
            created: e.meta.created, modified: e.meta.modified, text: e.body.clone(), ..Default::default()
        }).collect();
        shelf.folders = all.iter().filter(|e| e.meta.folder.is_some()).map(|e| Folder { id: e.id.clone(), name: e.title.clone(), parent: e.meta.parent.clone() }).collect();
        shelf.trash = self.store.list_trash().into_iter().map(|t| Trashed { id: t.id, title: t.title, deleted: t.deleted, folder: t.folder }).collect();
    }
}

fn call(name: &str, args: Value) -> Call { Call { id: "c".into(), name: name.into(), args: args.to_string() } }
fn w(shelf: &mut Shelf, lib: &mut Lib, log: &mut Changes, name: &str, args: Value) -> String { write::run(shelf, lib, log, &call(name, args)) }
fn read(shelf: &Shelf, id: &str) -> String { tools::run(shelf, &call("read_document", serde_json::json!({ "id": id }))) }
/// 读出来的那一行的块号：`[b3] 内容` 里的 b3。
fn num(out: &str, text: &str) -> String {
    let line = out.lines().find(|l| l.contains(text)).unwrap_or_else(|| panic!("没有「{text}」：\n{out}"));
    let s = &line[line.find("[b").unwrap() + 1..];
    s[..s.find(']').unwrap()].to_owned()
}
/// 文档里没有重复的块标识。
fn unique(doc: &Document) {
    let mut seen = HashSet::new();
    doc.for_each_block(|b| assert!(seen.insert(b.id), "块标识重复：{:?}", b.id));
}

#[test]
fn write_tools_only_when_editable() {
    let mut lib = Lib::new();
    let id = lib.doc("笔记", "第一段。", None);
    let mut shelf = lib.shelf();
    // 只读：请求里只有读的六样，硬调写的也不做。
    shelf.permission = Permission::Read;
    let mut core = agent::Agent::new(4);
    let Act::Ask(body) = core.begin("改一下", &[], &shelf) else { panic!() };
    assert_eq!(body["tools"].as_array().unwrap().len(), 6);
    assert!(body["messages"][0]["content"].as_str().unwrap().contains("只读不写"));
    let mut log = Changes::default();
    let out = w(&mut shelf, &mut lib, &mut log, "replace_text", serde_json::json!({ "doc": id, "edits": [{ "old": "第一", "new": "第二" }] }));
    assert!(out.starts_with("错误：这段对话是只读的"), "{out}");
    assert!(tools::run(&shelf, &call("trash", serde_json::json!({ "ids": [id] }))).starts_with("错误：这段对话是只读的"));
    assert!(log.is_empty() && lib.text(&id) == ["第一段。"]);
    // 可编辑：十五样，提示里说了能改。
    shelf.permission = Permission::Write;
    let Act::Ask(body) = core.begin("改一下", &[], &shelf) else { panic!() };
    let names: Vec<&str> = body["tools"].as_array().unwrap().iter().map(|t| t["function"]["name"].as_str().unwrap()).collect();
    assert_eq!(names.len(), 15);
    assert!(write::NAMES.iter().all(|n| names.contains(n)));
    assert!(body["messages"][0]["content"].as_str().unwrap().contains("你可以改这个空间的文档"));
}

#[test]
fn insert_blocks_places_and_numbers_new_blocks() {
    let mut lib = Lib::new();
    let id = lib.doc("笔记", "# 标题\n\n第一段。\n\n- 甲\n  - 乙\n\n末段。", None);
    let mut shelf = lib.shelf();
    let mut log = Changes::default();
    let out = read(&shelf, &id);
    let (first, child) = (num(&out, "第一段"), num(&out, "乙"));
    let out = w(&mut shelf, &mut lib, &mut log, "insert_blocks", serde_json::json!({ "doc": id, "markdown": "插在后面。\n\n- [ ] 新待办", "position": "after", "block": first }));
    assert!(out.starts_with("已插入到《笔记》"), "{out}");
    let new = num(&out, "插在后面");
    assert!(out.contains("新待办"));
    assert_eq!(lib.text(&id), ["标题", "第一段。", "插在后面。", "新待办", "甲", "末段。"]);
    // 插在子块前面：和它同一层。开头、结尾。
    w(&mut shelf, &mut lib, &mut log, "insert_blocks", serde_json::json!({ "doc": id, "markdown": "丙", "position": "before", "block": child }));
    w(&mut shelf, &mut lib, &mut log, "insert_blocks", serde_json::json!({ "doc": id, "markdown": "开头", "position": "start" }));
    w(&mut shelf, &mut lib, &mut log, "insert_blocks", serde_json::json!({ "doc": id, "markdown": "结尾" }));
    let doc = lib.load(&id).unwrap();
    unique(&doc);
    let kids: Vec<&str> = doc.blocks[5].children.iter().map(|b| b.text.text.as_str()).collect();
    assert_eq!(kids, ["丙", "乙"]);
    assert_eq!((doc.blocks[0].text.text.as_str(), doc.blocks.last().unwrap().text.text.as_str()), ("开头", "结尾"));
    // 刚插的块号接着能用。
    let out = w(&mut shelf, &mut lib, &mut log, "update_blocks", serde_json::json!({ "doc": id, "edits": [{ "block": new, "markdown": "插在后面（改过）。" }] }));
    assert!(out.contains("插在后面（改过）。"), "{out}");
    // 出错：新内容带块号、块号不对、位置不对、什么都没给。
    for (args, want) in [
        (serde_json::json!({ "doc": id, "markdown": format!("[{first}] 第一段。") }), "新内容里不要带块号"),
        (serde_json::json!({ "doc": id, "markdown": "x", "position": "after", "block": "b999" }), "没有块 b999"),
        (serde_json::json!({ "doc": id, "markdown": "x", "position": "after" }), "要给 block"),
        (serde_json::json!({ "doc": id, "markdown": "x", "position": "middle" }), "position 只能是"),
        (serde_json::json!({ "doc": id }), "没有要插入的内容"),
        (serde_json::json!({ "doc": "nope", "markdown": "x" }), "没有标识为 nope 的文档"),
    ] {
        let out = w(&mut shelf, &mut lib, &mut log, "insert_blocks", args);
        assert!(out.starts_with("错误：") && out.contains(want), "{want}: {out}");
    }
    // 一篇改了几次，日志里还是一条。
    assert_eq!(log.ops.len(), 1);
    assert_eq!(log.count(), 1);
}

#[test]
fn update_blocks_rewrites_keeps_and_deletes() {
    let mut lib = Lib::new();
    let id = lib.doc("笔记", "第一段。\n\n第二段。\n\n第三段。\n\n- 父\n  - 子", None);
    // 第一段居中：改了字也还是居中。
    let mut d = lib.store.load(&id).unwrap();
    d.blocks[0].align = Align::Center;
    lib.store.save(&id, &d).unwrap();
    let mut shelf = lib.shelf();
    let mut log = Changes::default();
    let out = read(&shelf, &id);
    let (b1, b2, b3, parent, kid) = (num(&out, "第一段"), num(&out, "第二段"), num(&out, "第三段"), num(&out, "父"), num(&out, "子"));
    let out = w(&mut shelf, &mut lib, &mut log, "update_blocks", serde_json::json!({ "doc": id, "edits": [
        { "block": b1, "markdown": "第一段，改过。" },
        { "block": b2, "markdown": "## 拆成标题\n\n和一段。" },
        { "block": b3, "markdown": "" },
    ] }));
    assert!(out.starts_with("已修改《笔记》"), "{out}");
    let doc = lib.load(&id).unwrap();
    unique(&doc);
    assert_eq!(lib.text(&id), ["第一段，改过。", "拆成标题", "和一段。", "父"]);
    assert_eq!(doc.blocks[0].align, Align::Center);
    // 同一块再改一次，用结果里的块号。
    let again = num(&out, "第一段，改过");
    assert_eq!(again, b1);
    w(&mut shelf, &mut lib, &mut log, "update_blocks", serde_json::json!({ "doc": id, "edits": [{ "block": again, "markdown": "第一段，又改了。" }] }));
    assert_eq!(lib.text(&id)[0], "第一段，又改了。");
    // 父块连子块一起写：子块带着块号就原样留着。
    let out = w(&mut shelf, &mut lib, &mut log, "update_blocks", serde_json::json!({ "doc": id, "edits": [{ "block": parent, "markdown": format!("- 父（改）\n  [{kid}] - 子") }] }));
    assert!(!out.starts_with("错误"), "{out}");
    let doc = lib.load(&id).unwrap();
    assert_eq!((doc.blocks[3].text.text.as_str(), doc.blocks[3].children[0].text.text.as_str()), ("父（改）", "子"));
    // 出错（整个调用都不改）：同一块两次、父块和子块、别处的块号、块号不对。
    let before = lib.text(&id);
    let out = read(&shelf, &id);
    let (b1, parent, kid) = (num(&out, "又改了"), num(&out, "父（改）"), num(&out, "子"));
    for (edits, want) in [
        (serde_json::json!([{ "block": b1, "markdown": "a" }, { "block": b1, "markdown": "b" }]), "重了"),
        (serde_json::json!([{ "block": parent, "markdown": "a" }, { "block": kid, "markdown": "b" }]), "重了"),
        (serde_json::json!([{ "block": b1, "markdown": format!("a\n\n[{kid}] 子") }]), "不在要改的这块里"),
        (serde_json::json!([{ "block": "b999", "markdown": "a" }]), "没有块 b999"),
        (serde_json::json!([]), "edits 是空的"),
    ] {
        let out = w(&mut shelf, &mut lib, &mut log, "update_blocks", serde_json::json!({ "doc": id, "edits": edits }));
        assert!(out.starts_with("错误：") && out.contains(want), "{want}: {out}");
    }
    assert_eq!(lib.text(&id), before);
}

#[test]
fn replace_text_changes_only_the_words() {
    let mut lib = Lib::new();
    let id = lib.doc("笔记", "**导出**功能做完了。\n\n导出要再测。\n\n同步也要测。", None);
    let mut shelf = lib.shelf();
    let mut log = Changes::default();
    let out = w(&mut shelf, &mut lib, &mut log, "replace_text", serde_json::json!({ "doc": id, "edits": [{ "old": "同步", "new": "iCloud 同步" }] }));
    assert!(out.contains("换了 1 处") && out.contains("iCloud 同步也要测"), "{out}");
    // 出现两次：不给 all 不换，给了全换，粗体还在。
    let out = w(&mut shelf, &mut lib, &mut log, "replace_text", serde_json::json!({ "doc": id, "edits": [{ "old": "导出", "new": "导入" }] }));
    assert!(out.starts_with("错误：") && out.contains("出现了 2 次"), "{out}");
    let out = w(&mut shelf, &mut lib, &mut log, "replace_text", serde_json::json!({ "doc": id, "edits": [{ "old": "导出", "new": "导入", "all": true }] }));
    assert!(out.contains("换了 2 处"), "{out}");
    let doc = lib.load(&id).unwrap();
    assert_eq!(doc.blocks[0].text.text, "导入功能做完了。");
    assert!(doc.blocks[0].text.runs[0].marks.bold && doc.blocks[0].text.runs[0].len == "导入".len());
    // 带 Markdown 记号、跨块、空的：找不到。
    for (old, want) in [("**导入**", "没找到"), ("了。\n导入", "没找到"), ("", "old 是空的")] {
        let out = w(&mut shelf, &mut lib, &mut log, "replace_text", serde_json::json!({ "doc": id, "edits": [{ "old": old, "new": "x" }] }));
        assert!(out.starts_with("错误：") && out.contains(want), "{want}: {out}");
    }
}

#[test]
fn create_document_from_markdown_or_copies() {
    let mut lib = Lib::new();
    let folder = lib.store.create_folder("工作", None).unwrap();
    let src = lib.doc("周报", "本周完成了导出。\n\n下周做同步。", None);
    let mut shelf = lib.shelf();
    let mut log = Changes::default();
    let out = w(&mut shelf, &mut lib, &mut log, "create_document", serde_json::json!({ "title": "计划", "markdown": "# 下周\n\n- [ ] 同步", "folder": "工作" }));
    assert!(out.starts_with("已新建《计划》(id: ") && out.contains("在「工作」里") && out.contains("[b"), "{out}");
    let id = shelf.docs.iter().find(|d| d.title == "计划").unwrap().id.clone();
    assert_eq!(lib.entry(&id).unwrap().meta.parent.as_deref(), Some(folder.as_str()));
    assert_eq!(lib.text(&id), ["下周", "同步"]);
    // 新建的接着能改（块号、快照都对得上）。
    let n = num(&out, "同步");
    let out = w(&mut shelf, &mut lib, &mut log, "update_blocks", serde_json::json!({ "doc": id, "edits": [{ "block": n, "markdown": "- [ ] 同步和备份" }] }));
    assert!(!out.starts_with("错误"), "{out}");
    // 拷块：新标识，原文不动。
    let s = read(&shelf, &src);
    let out = w(&mut shelf, &mut lib, &mut log, "create_document", serde_json::json!({ "title": "摘录", "copy_from": { "doc": src, "blocks": [num(&s, "下周做同步")] } }));
    let copy = shelf.docs.iter().find(|d| d.title == "摘录").unwrap().id.clone();
    assert!(out.starts_with("已新建《摘录》"), "{out}");
    assert_eq!(lib.text(&copy), ["下周做同步。"]);
    assert_eq!(lib.text(&src), ["本周完成了导出。", "下周做同步。"]);
    // 也能拷进已有的一篇。
    w(&mut shelf, &mut lib, &mut log, "insert_blocks", serde_json::json!({ "doc": id, "copy_from": { "doc": src } }));
    let doc = lib.load(&id).unwrap();
    unique(&doc);
    assert_eq!(doc.blocks.len(), 4);
    // 新建的算新建（改过的不另记）。
    assert!(matches!(&log.ops[..], [Op::Create { .. }, Op::Create { .. }]));
    for (args, want) in [
        (serde_json::json!({ "title": "x", "markdown": "a", "copy_from": { "doc": src } }), "只能给一样"),
        (serde_json::json!({ "title": "x", "folder": "没有这个" }), "没有叫「没有这个」的文件夹"),
        (serde_json::json!({ "title": "x", "copy_from": { "doc": src, "blocks": ["b999"] } }), "没有块 b999"),
    ] {
        let out = w(&mut shelf, &mut lib, &mut log, "create_document", args);
        assert!(out.starts_with("错误：") && out.contains(want), "{want}: {out}");
    }
}

#[test]
fn folders_moves_renames_trash_and_restore() {
    let mut lib = Lib::new();
    let work = lib.store.create_folder("工作", None).unwrap();
    let a = lib.doc("周报", "x", None);
    let b = lib.store.create_folder("读书", None).unwrap();
    let kid = lib.doc("子文档", "z", Some(&b));
    let mut shelf = lib.shelf();
    let mut log = Changes::default();
    let out = w(&mut shelf, &mut lib, &mut log, "create_folder", serde_json::json!({ "name": "归档", "parent": work }));
    assert!(out.starts_with("已新建文件夹「归档」"), "{out}");
    let archive = shelf.folders.iter().find(|f| f.name == "归档").unwrap().id.clone();
    assert_eq!(shelf.path(&archive), "工作 / 归档");
    let out = w(&mut shelf, &mut lib, &mut log, "move", serde_json::json!({ "ids": [a, "nope"], "to": "归档" }));
    assert!(out.contains("已移到「工作 / 归档」") && out.contains("没移：没有标识为 nope"), "{out}");
    assert_eq!(shelf.find(&a).unwrap().parent.as_deref(), Some(archive.as_str()));
    // 不能移到自己里面；文件夹只能是文件夹。
    assert!(w(&mut shelf, &mut lib, &mut log, "move", serde_json::json!({ "ids": [work], "to": archive })).contains("不能移到它自己里面"));
    assert!(w(&mut shelf, &mut lib, &mut log, "move", serde_json::json!({ "ids": [a], "to": kid })).contains("是文档，不是文件夹"));
    let out = w(&mut shelf, &mut lib, &mut log, "rename", serde_json::json!({ "id": a, "title": "第 48 周周报" }));
    assert!(out.contains("改名为「第 48 周周报」"), "{out}");
    assert_eq!(lib.entry(&a).unwrap().title, "第 48 周周报");
    assert!(w(&mut shelf, &mut lib, &mut log, "rename", serde_json::json!({ "id": a, "title": " " })).starts_with("错误：新名字是空的"));
    // 删一篇带子文档的：子文档跟着进最近删除；再恢复。
    let out = w(&mut shelf, &mut lib, &mut log, "trash", serde_json::json!({ "ids": [b, kid] }));
    assert!(out.contains("文件夹「读书」 已移到最近删除") && out.contains("《子文档》 跟着上一层一起删了"), "{out}");
    assert!(lib.trashed(&b) && lib.entry(&kid).is_none() && shelf.folder(&b).is_none() && shelf.trash.iter().any(|t| t.id == b));
    let out = w(&mut shelf, &mut lib, &mut log, "restore", serde_json::json!({ "ids": [b, "nope"] }));
    assert!(out.contains("《读书》已恢复") && out.contains("最近删除里没有 nope"), "{out}");
    assert_eq!(lib.entry(&kid).unwrap().meta.parent.as_deref(), Some(b.as_str()));
    assert!(w(&mut shelf, &mut lib, &mut log, "restore", serde_json::json!({ "ids": ["nope"] })).starts_with("错误："));
    let lines = log.lines();
    assert_eq!(lines, ["新建了文件夹「归档」", "移动了《周报》", "把《周报》改名为《第 48 周周报》", "删除了《读书》", "恢复了《读书》"]);
    assert_eq!(log.count(), 3);
}

/// 回滚：倒着一样样做回去——正文回到助手改之前，新建的进最近删除，移动、改名改回去，删的恢复，恢复的再删掉。
#[test]
fn undo_puts_everything_back() {
    let mut lib = Lib::new();
    let work = lib.store.create_folder("工作", None).unwrap();
    let a = lib.doc("周报", "第一段。\n\n第二段。", None);
    let b = lib.doc("读书", "y", None);
    let c = lib.doc("旧稿", "z", None);
    let old = lib.doc("草稿", "w", None);
    // 手排过的顺序（没手排过的按修改时间排，改了正文就会挪）。
    lib.store.initialize_manual_order().unwrap();
    let order: Vec<String> = { let mut e = lib.store.list(); e.retain(|e| e.meta.parent.is_none()); e.sort_by(|x, y| (x.meta.place(), &x.id).cmp(&(y.meta.place(), &y.id))); e.into_iter().map(|e| e.id).collect() };
    lib.store.delete(&old, None).unwrap();
    let before = lib.store.load(&a).unwrap();
    let mut shelf = lib.shelf();
    let mut log = Changes::default();
    let out = read(&shelf, &a);
    let n = num(&out, "第一段");
    w(&mut shelf, &mut lib, &mut log, "update_blocks", serde_json::json!({ "doc": a, "edits": [{ "block": n, "markdown": "全改了。" }] }));
    w(&mut shelf, &mut lib, &mut log, "replace_text", serde_json::json!({ "doc": a, "edits": [{ "old": "第二", "new": "第 2" }] }));
    w(&mut shelf, &mut lib, &mut log, "create_document", serde_json::json!({ "title": "新的", "markdown": "n" }));
    w(&mut shelf, &mut lib, &mut log, "create_folder", serde_json::json!({ "name": "新文件夹" }));
    w(&mut shelf, &mut lib, &mut log, "move", serde_json::json!({ "ids": [b], "to": work }));
    w(&mut shelf, &mut lib, &mut log, "rename", serde_json::json!({ "id": b, "title": "读书笔记" }));
    w(&mut shelf, &mut lib, &mut log, "trash", serde_json::json!({ "ids": [c] }));
    w(&mut shelf, &mut lib, &mut log, "restore", serde_json::json!({ "ids": [old] }));
    let made: Vec<String> = log.ops.iter().filter_map(|o| match o { Op::Create { id, .. } => Some(id.clone()), _ => None }).collect();
    assert_eq!(made.len(), 2);
    assert_eq!(log.count(), 6);
    // 存进对话再读出来（重启后也能回滚）。
    let mut log = Changes::read(&serde_json::from_str(&log.to_json().to_string()).unwrap());
    assert!(log.edited(&mut lib).is_empty());
    let problems = log.undo(&mut lib);
    assert!(problems.is_empty(), "{problems:?}");
    assert!(log.undone);
    assert_eq!(print(&lib.store.load(&a).unwrap()), print(&before));
    assert!(made.iter().all(|id| lib.trashed(id) && lib.entry(id).is_none()));
    let eb = lib.entry(&b).unwrap();
    assert_eq!((eb.meta.parent.as_deref(), eb.title.as_str()), (None, "读书"));
    assert!(lib.entry(&c).is_some() && !lib.trashed(&c));
    assert!(lib.trashed(&old) && lib.entry(&old).is_none());
    // 回到原来的位置（顶层的顺序也一样）。
    let now: Vec<String> = { let mut e = lib.store.list(); e.retain(|e| e.meta.parent.is_none() && e.id != old); e.sort_by(|x, y| (x.meta.place(), &x.id).cmp(&(y.meta.place(), &y.id))); e.into_iter().map(|e| e.id).collect() };
    assert_eq!(now, order.into_iter().filter(|i| *i != old).collect::<Vec<_>>());
}

/// 之后有人手动改过：回滚前认得出来；没人动过不提示。指纹重新从文档库读也一样（XML 和 Markdown 文档都是）。
#[test]
fn undo_notices_later_hand_edits() {
    let mut lib = Lib::new();
    let a = lib.doc("周报", "第一段。", None);
    let md = lib.store.create_markdown(&fdl_core::markdown::parse_file("Markdown 文档。\n\n- 一项\n", "Note", None), "").unwrap();
    let mut shelf = lib.shelf();
    let mut log = Changes::default();
    for id in [&a, &md] {
        let out = w(&mut shelf, &mut lib, &mut log, "insert_blocks", serde_json::json!({ "doc": id, "markdown": "助手加的。" }));
        assert!(!out.starts_with("错误"), "{out}");
    }
    let mut fresh = Lib { root: lib.root.clone(), store: Store::open_at(lib.root.clone()).unwrap(), guide: String::new(), open: None };
    assert!(log.edited(&mut fresh).is_empty());
    std::mem::forget(fresh);
    // 用户又改了周报。
    let mut d = lib.store.load(&a).unwrap();
    d.blocks.push(Block::text("用户写的。"));
    lib.store.save(&a, &d).unwrap();
    assert_eq!(log.edited(&mut lib), ["《周报》"]);
    // 快照还是旧的：助手再改这篇会被拦下，让它先重新读。
    let out = w(&mut shelf, &mut lib, &mut log, "replace_text", serde_json::json!({ "doc": a, "edits": [{ "old": "第一", "new": "第 1" }] }));
    assert!(out.starts_with("错误：《周报》刚被改过"), "{out}");
    read(&shelf, &a);
    let out = w(&mut shelf, &mut lib, &mut log, "replace_text", serde_json::json!({ "doc": a, "edits": [{ "old": "第一", "new": "第 1" }] }));
    assert!(!out.starts_with("错误"), "{out}");
    // 照样能回滚（用户确认过之后）。
    assert!(log.undo(&mut lib).is_empty());
    assert_eq!(lib.text(&a), ["第一段。"]);
}

/// 编辑器里开着的那篇：走编辑器（一步撤销），回滚也走编辑器。
#[test]
fn the_open_document_goes_through_the_editor() {
    let mut lib = Lib::new();
    let a = lib.doc("周报", "第一段。", None);
    let doc = lib.store.load(&a).unwrap();
    lib.open = Some((a.clone(), Editor::new(doc)));
    let mut shelf = lib.shelf();
    let mut log = Changes::default();
    let out = w(&mut shelf, &mut lib, &mut log, "insert_blocks", serde_json::json!({ "doc": a, "markdown": "助手加的。" }));
    assert!(!out.starts_with("错误"), "{out}");
    // 文档库里还是原样（编辑器自己存），编辑器里有了。
    assert_eq!(lib.store.load(&a).unwrap().blocks.len(), 1);
    let (_, e) = lib.open.as_mut().unwrap();
    assert_eq!(e.doc.blocks.len(), 2);
    assert!(e.dirty);
    assert!(e.undo());
    assert!(e.redo());
    log.undo(&mut lib);
    let (_, e) = lib.open.as_mut().unwrap();
    assert_eq!(e.doc.blocks.iter().map(|b| b.text.text.as_str()).collect::<Vec<_>>(), ["第一段。"]);
    assert!(e.undo());
    assert_eq!(e.doc.blocks.len(), 2);
}

/// `SPACE.md`：按 Markdown 读成块，三样都能改，写回 Markdown；回滚回到原文（一字不差）。
#[test]
fn space_md_is_editable() {
    let mut lib = Lib::new();
    lib.guide = "# 小说\n\n第三人称，过去时。\n\n## 人物\n\n- 林：主角\n".into();
    let original = lib.guide.clone();
    let mut shelf = lib.shelf();
    let mut log = Changes::default();
    let out = read(&shelf, "space.md");
    let n = num(&out, "林：主角");
    w(&mut shelf, &mut lib, &mut log, "insert_blocks", serde_json::json!({ "doc": "SPACE.md", "markdown": "- 周：配角", "position": "after", "block": n }));
    w(&mut shelf, &mut lib, &mut log, "replace_text", serde_json::json!({ "doc": "SPACE.md", "edits": [{ "old": "过去时", "new": "现在时" }] }));
    let n = num(&read(&shelf, "SPACE.md"), "第三人称");
    w(&mut shelf, &mut lib, &mut log, "update_blocks", serde_json::json!({ "doc": "SPACE.md", "edits": [{ "block": n, "markdown": "第一人称，现在时。" }] }));
    assert_eq!(lib.guide, "# 小说\n\n第一人称，现在时。\n\n## 人物\n\n- 林：主角\n- 周：配角\n");
    assert_eq!(shelf.guide, lib.guide);
    assert_eq!(log.lines(), ["改了 SPACE.md"]);
    // 用户之后手动改了：提示；回滚回到原文。
    lib.guide.push_str("\n补一句。\n");
    assert_eq!(log.edited(&mut lib), ["SPACE.md"]);
    assert!(log.undo(&mut lib).is_empty());
    assert_eq!(lib.guide, original);
    // 模板（有空的列表项）写回来只差行尾的空格。
    let doc = fdl_core::markdown::parse_file(crate::space_md::TEMPLATE, "", None);
    let back = fdl_core::markdown::export(&doc, &fdl_core::markdown::Options { in_place: true, ..Default::default() }).text;
    let trim = |s: &str| s.lines().map(str::trim_end).collect::<Vec<_>>().join("\n");
    assert_eq!(trim(&back), trim(crate::space_md::TEMPLATE));
}

#[test]
fn change_log_round_trips_with_the_chat() {
    let mut log = Changes::default();
    log.push(Op::Edit { id: "a".into(), title: "周报".into(), before: "<doc/>".into(), after: u64::MAX });
    log.push(Op::Edit { id: "a".into(), title: "周报".into(), before: "后来的不算".into(), after: 7 });
    log.push(Op::Create { id: "n".into(), title: "新的".into(), folder: false, after: 1 });
    log.push(Op::Rename { id: "n".into(), title: "改过名".into(), was: "新的".into() });
    log.push(Op::Move { id: "b".into(), title: "读书".into(), parent: Some("f".into()), next: None });
    log.push(Op::Move { id: "b".into(), title: "读书".into(), parent: Some("g".into()), next: Some("x".into()) });
    log.push(Op::Trash { id: "c".into(), title: "旧稿".into() });
    assert_eq!(log.ops.len(), 4);
    assert_eq!(log.ops[0], Op::Edit { id: "a".into(), title: "周报".into(), before: "<doc/>".into(), after: 7 });
    assert_eq!(log.ops[1], Op::Create { id: "n".into(), title: "改过名".into(), folder: false, after: 1 });
    assert_eq!(log.ops[2], Op::Move { id: "b".into(), title: "读书".into(), parent: Some("f".into()), next: None });
    assert_eq!(Changes::read(&log.to_json()), log);
    let mut chat = chat::Chat::new("x".into(), 1);
    chat.items.push(chat::Item { role: Role::Assistant, text: "改好了".into(), changes: log.clone(), ..Default::default() });
    chat.items.push(chat::Item { role: Role::Assistant, text: "没改".into(), ..Default::default() });
    let back = chat::Chat::read(&chat.to_json()).unwrap();
    assert_eq!(back, chat);
    assert!(chat.to_json()["items"][1].get("changes").is_none());
}
