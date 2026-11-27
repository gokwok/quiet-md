//! 助手写的工具在手机上怎么落地（`fdl_ai::run::write::Host`），主线程上：
//! 编辑器里开着的那篇走编辑器（`Core::replace_body`，一步撤销、马上看得到；阅读模式也让它写进去），别的走文档库；
//! 新建、移动、改名、删除、恢复和界面上的操作一样走 `Workspace` / `SharedStore`，做完刷新列表。回滚也走这里。
//! `generate_image` 走文档里配图的那条队列（`imagine`）：`me` 时带上助手的形象（`look`，图像库里人物的名字），开始画的那张记在 `pending`（对话页等它画完再回话），
//! 画在对话里的记在 `shown`。
use std::collections::HashMap;
use std::sync::Arc;

use fdl_ai::run::{self, Shelf, changes::print, write::{Host, Picture, Place}};
use gpui::{Context, Window};

use crate::{app::Workspace, editor::core::Core, model::{Block, Document}, settings::{diagnostic, tr}, store::Store};

pub struct Ios<'a, 'b> {
    pub w: &'a mut Workspace, pub window: &'a mut Window, pub cx: &'a mut Context<'b, Workspace>,
    pub look: Option<String>, pub pending: Option<String>, pub shown: Vec<String>,
    /// 要画自己（`me`），助手却没绑形象。
    pub unbound: bool,
}

fn err(e: impl std::fmt::Display) -> String { diagnostic(&e.to_string()) }

/// 文档库里每篇的标题、所在、标签、链接和正文（纯文字），文件夹，最近删除（新删的在前）。
pub fn catalog(w: &Workspace) -> (Vec<run::Doc>, Vec<run::Folder>, Vec<run::Trashed>) {
    let folders: HashMap<&str, &str> = w.entries.iter().filter(|e| e.meta.folder.is_some()).map(|e| (e.id.as_str(), e.title.as_str())).collect();
    let docs = w.entries.iter().filter(|e| e.meta.folder.is_none()).map(|e| run::Doc {
        id: e.id.clone(), title: e.title.clone(), folder: e.meta.parent.as_deref().and_then(|p| folders.get(p)).map(|s| s.to_string()),
        parent: e.meta.parent.clone(), created: e.meta.created, modified: e.meta.modified,
        tags: e.tags.iter().map(|(t, _)| t.clone()).collect(),
        links: e.links.iter().map(|l| (l.target.clone(), l.line.iter().map(|(t, _)| t.as_str()).collect())).collect(),
        text: Arc::from(&*e.body),
    }).collect();
    let dirs = w.entries.iter().filter(|e| e.meta.folder.is_some()).map(|e| run::Folder { id: e.id.clone(), name: e.title.clone(), parent: e.meta.parent.clone() }).collect();
    let mut trash: Vec<run::Trashed> = w.trash.iter().map(|t| run::Trashed { id: t.id.clone(), title: t.title.clone(), deleted: t.deleted, folder: t.folder }).collect();
    trash.sort_by_key(|t| std::cmp::Reverse(t.deleted));
    (docs, dirs, trash)
}

impl<'a, 'b> Ios<'a, 'b> {
    pub fn new(w: &'a mut Workspace, window: &'a mut Window, cx: &'a mut Context<'b, Workspace>) -> Self { Ios { w, window, cx, look: None, pending: None, shown: vec![], unbound: false } }
}

impl Ios<'_, '_> {
    /// `id` 正开在编辑器里。
    fn open(&self, id: &str) -> Option<gpui::Entity<crate::editor::view::Editor>> { self.w.editor.clone().filter(|_| self.w.current.as_deref() == Some(id)) }
    /// 文档库变了：列表、最近删除、子页面跟着刷新。
    fn relist(&mut self) {
        self.w.entries = self.w.store.list();
        self.w.sort();
        self.w.trash = self.w.store.list_trash();
        self.w.sync_sub_pages(self.cx);
        self.cx.notify();
    }
    /// 动了文档库的结构：和界面上的操作一样记进文件记录。
    fn tracked<T>(&mut self, f: impl FnOnce(&mut Self) -> Result<T, String>) -> Result<T, String> {
        let before = self.w.store.track_file_changes();
        let r = f(self);
        self.relist();
        self.w.remember_files(before, tr("助手", "Assistant"));
        r
    }
}

impl Host for Ios<'_, '_> {
    fn load(&mut self, id: &str) -> Option<Document> {
        if let Some(ed) = self.open(id) { return Some(ed.update(self.cx, |e, _| { e.graft_pages(); e.root().doc.clone() })); }
        self.w.store.load(id).ok()
    }

    fn put_body(&mut self, id: &str, blocks: Vec<Block>) -> Result<(Document, u64), String> {
        if let Some(ed) = self.open(id) {
            let window = &mut *self.window;
            let doc = ed.update(self.cx, |e, cx| {
                // 开着的子页面先收起；阅读模式下也让助手的改动进去（它的快照跟着换）。
                e.leave_pages();
                let ro = e.read_only;
                if ro { e.set_read_only(false, cx); }
                e.core.replace_body(blocks);
                e.changed(window, cx);
                if ro { e.set_read_only(true, cx); }
                e.core.doc.clone()
            });
            return Ok((doc.clone(), print(&doc)));
        }
        let mut core = Core::new(self.w.store.load(id).map_err(err)?);
        core.replace_body(blocks);
        self.w.store.save(id, &core.doc).map_err(err)?;
        let back = self.w.store.load(id).map_err(err)?;
        self.relist();
        Ok((core.doc, print(&back)))
    }

    fn guide(&mut self) -> String { fdl_ai::space_md::read(&Store::root_dir()).unwrap_or_default() }
    fn save_guide(&mut self, text: &str) -> Result<(), String> { fdl_ai::space_md::write(&Store::root_dir(), text).map_err(err) }

    fn create(&mut self, doc: Document, parent: Option<&str>) -> Result<String, String> {
        self.tracked(|s| {
            let id = s.w.store.create_from(&doc).map_err(err)?;
            if parent.is_some() { s.w.store.set_parent(&id, parent).map_err(err)?; }
            Ok(id)
        })
    }

    fn create_folder(&mut self, name: &str, parent: Option<&str>) -> Result<String, String> { self.tracked(|s| s.w.store.create_folder(name, parent).map_err(err)) }

    fn place(&mut self, id: &str) -> Option<Place> {
        let me = self.w.entries.iter().find(|e| e.id == id)?;
        let mut sib: Vec<_> = self.w.entries.iter().filter(|e| e.meta.parent == me.meta.parent && !e.meta.local()).collect();
        sib.sort_by(|a, b| (a.meta.place(), &a.id).cmp(&(b.meta.place(), &b.id)));
        let at = sib.iter().position(|e| e.id == id);
        Some(Place { parent: me.meta.parent.clone(), next: at.and_then(|i| sib.get(i + 1)).map(|e| e.id.clone()), local: me.meta.local() })
    }

    fn move_to(&mut self, id: &str, parent: Option<&str>, next: Option<&str>) -> Result<(), String> { self.tracked(|s| s.w.store.move_in_tree(id, parent, next).map_err(err)) }

    fn rename(&mut self, id: &str, title: &str) -> Result<(), String> {
        // 和侧栏改名一样：开着的先存，改名后编辑器跟着换标题。
        if self.open(id).is_some() && !self.w.flush(self.cx) { return Err(tr("开着的这篇还没存好，没改名", "The open document is not saved yet").into()); }
        self.tracked(|s| s.w.store.rename(id, title).map(|_| ()).map_err(err))?;
        if let Some(ed) = self.open(id) { ed.update(self.cx, |e, cx| e.receive_file_title(title, cx)); }
        Ok(())
    }

    fn trash(&mut self, ids: &[String]) -> Result<Vec<String>, String> {
        // 开着的那篇要删：先存好（`delete_docs` 存不上会改成先弹「保存失败」）。
        if self.w.current.as_ref().is_some_and(|c| ids.contains(c)) && !self.w.flush(self.cx) { return Err(tr("开着的这篇还没存好，没删", "The open document is not saved yet").into()); }
        self.w.delete_docs(ids, self.window, self.cx);
        self.relist();
        let gone: Vec<String> = ids.iter().filter(|id| !self.w.entries.iter().any(|e| e.id == **id) && self.w.trash.iter().any(|t| t.id == **id)).cloned().collect();
        if gone.is_empty() { return Err(tr("没能移到最近删除", "Could not move to Recently Deleted").into()); }
        Ok(gone)
    }

    fn restore(&mut self, id: &str) -> Result<(), String> { self.tracked(|s| s.w.store.restore(id).map(|_| ()).map_err(err)) }

    fn refresh(&mut self, shelf: &mut Shelf) { (shelf.docs, shelf.folders, shelf.trash) = catalog(self.w); }

    fn picture(&mut self, prompt: &str, ratio: &str, me: bool) -> Result<Picture, String> {
        if super::super::assistant::image::key().is_none() { return Err(tr("用户还没在 设置 › MarginAI › 图像 里添加 NovelAI Key，现在画不了图。", "No NovelAI key in Settings › MarginAI › Image yet, so images cannot be drawn.").into()); }
        let cast = if me { self.look.iter().cloned().collect() } else { vec![] };
        let (id, src, width, height) = super::super::imagine::reserve(prompt, ratio, cast).ok_or_else(|| tr("无法保存生成记录", "Could not save the image record").to_owned())?;
        // 要画自己却没绑形象：照样画（照提示词），回话时说一声。
        if me && self.look.is_none() { self.unbound = true; }
        Ok(Picture { id, src, width, height })
    }

    fn paint(&mut self, id: &str, doc: Option<&str>) {
        self.pending = Some(id.to_owned());
        if doc.is_none() { self.shown.push(id.to_owned()); }
        super::super::imagine::start(id, self.cx);
    }
}
