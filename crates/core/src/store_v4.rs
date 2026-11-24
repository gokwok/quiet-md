use super::*;
use std::collections::HashSet;
use std::path::Path;

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct Preview {
    title: String,
    icon: Option<String>,
    body: std::sync::Arc<str>,
    reminders: Vec<(i64, i64, bool, bool)>,
    links: Vec<(String, usize, Vec<(String, Option<String>)>)>,
    tags: Vec<(String, usize)>,
}

impl Preview {
    fn document(doc: &Document) -> Self {
        Self { title: doc.title.text.clone(), icon: doc.icon.clone(), body: doc.plain_text().into(),
            reminders: doc.reminders().iter().map(|r| (r.expire, r.notify, r.should_notify, r.whole_day)).collect(),
            links: doc.doc_links().into_iter().map(|l| (l.target, l.block, l.line)).collect(), tags: doc.tags() }
    }
    fn parsed(self) -> Parsed {
        Parsed { title: self.title, icon: self.icon, body: self.body,
            reminders: self.reminders.into_iter().map(|(expire, notify, should_notify, whole_day)| crate::model::InlineReminder { expire, notify, should_notify, whole_day }).collect(),
            links: self.links.into_iter().map(|(target, block, line)| crate::model::DocLink { target, block, line }).collect(), tags: self.tags }
    }
}

#[derive(Clone, Serialize, Deserialize)]
struct Record {
    #[serde(flatten)]
    meta: Meta,
    #[serde(default)]
    preview: Preview,
}

#[derive(Clone, Default)]
pub(super) struct State {
    pub(super) local: PathBuf,
    active: HashSet<String>,
    trash: HashSet<String>,
    metadata: HashMap<String, serde_json::Value>,
    trashed: HashMap<String, serde_json::Value>,
    previews: HashMap<String, Preview>,
    records: HashMap<String, (Option<std::time::SystemTime>, u64, Record)>,
    pending: HashMap<String, DownloadState>,
}

pub(super) fn uuid_name(id: &str) -> bool {
    uuid::Uuid::parse_str(id).is_ok_and(|u| u.hyphenated().to_string() == id)
}

fn json_files(dir: &Path) -> Result<Vec<(String, PathBuf)>> {
    let rd = match crate::file_io::read_dir(dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(e.into()),
    };
    let mut out = vec![];
    for e in rd {
        let mut p = e?.path();
        if let Some(name) = p.file_name().and_then(|s| s.to_str()).and_then(|s| s.strip_prefix('.')).and_then(|s| s.strip_suffix(".icloud")) { p = p.with_file_name(name); }
        if p.extension().is_none_or(|x| x != "json") { continue; }
        if let Some(id) = p.file_stem().and_then(|s| s.to_str()).filter(|s| uuid_name(s)) { out.push((id.to_string(), p)); }
    }
    Ok(out)
}

fn optional_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    match crate::file_io::read(path) {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes).with_context(|| format!("元数据损坏，未覆盖 {}", path.display()))?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn changed_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut value = serde_json::to_value(value)?;
    value.sort_all_objects();
    let bytes = serde_json::to_vec_pretty(&value)?;
    if crate::file_io::read(path).ok().as_deref() != Some(&bytes) { write_atomic(path, &bytes)?; }
    Ok(())
}

fn update_json(path: &Path, update: impl FnOnce(&mut serde_json::Value) -> Result<()>) -> Result<()> {
    crate::file_io::with_write(path, |actual| {
        let mut value: serde_json::Value = serde_json::from_slice(&crate::file_io::read(actual)?)?;
        anyhow::ensure!(value.is_object(), "元数据不是 JSON 对象，未覆盖");
        update(&mut value)?;
        changed_json(actual, &value)
    })
}

fn synced(meta: &Meta) -> serde_json::Value {
    let mut value = serde_json::to_value(meta).expect("Meta is JSON");
    for key in ["opened", "catalogue_closed", "reading", "path"] { value.as_object_mut().unwrap().remove(key); }
    value
}

fn local_base(root: &Path) -> PathBuf {
    #[cfg(test)]
    { root.with_extension("local-state") }
    #[cfg(not(test))]
    {
        let _ = root;
        std::env::var_os("QUIET_MD_LOCAL_DATA").map(PathBuf::from)
            .unwrap_or_else(|| dirs::data_local_dir().unwrap_or_else(std::env::temp_dir).join("quiet-md"))
    }
}

pub(super) fn bootstrap_lock(root: &Path) -> Result<WriteLock> {
    use std::hash::{Hash, Hasher};
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    root.hash(&mut hash);
    let dir = local_base(root).join("locks");
    crate::file_io::create_dir_all(&dir)?;
    let file = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(dir.join(format!("{:016x}.lock", hash.finish())))?;
    file.lock()?;
    Ok(WriteLock(file))
}

impl Store {
    pub(super) fn v4(&self) -> bool {
        reread::<FormatMarker>(&self.root.join("format.json")).is_some_and(|m| m.format == FORMAT)
    }

    pub(super) fn init_local(&mut self) -> Result<()> {
        let id = crate::spaces::ensure_id(&self.root)?;
        // Tests and harness runs keep *all* their state in their temporary home.
        let base = local_base(&self.root);
        let local = base.join("spaces").join(id);
        crate::file_io::create_dir_all(&local)?;
        self.v4_state.borrow_mut().local = local;
        Ok(())
    }

    pub(super) fn reload_v4(&mut self) -> Result<()> {
        let mut index = HashMap::new();
        let mut trash = HashMap::new();
        let mut previews = HashMap::new();
        let mut pending = HashMap::new();
        let mut records = HashMap::new();
        for (id, path) in json_files(&self.root.join("docs"))? {
            let metadata = crate::file_io::metadata(&path).ok();
            let stamp = (metadata.as_ref().and_then(|m| m.modified().ok()), metadata.as_ref().map_or(0, |m| m.len()));
            let cached = self.v4_state.borrow().records.get(&id).filter(|r| (r.0, r.1) == stamp).map(|r| r.2.clone());
            if let Some(record) = cached {
                records.insert(id.clone(), (stamp.0, stamp.1, record.clone()));
                previews.insert(id.clone(), record.preview);
                index.insert(id, record.meta);
                continue;
            }
            let state = crate::file_io::download_state(&path);
            if !state.ready() {
                let _ = crate::file_io::request_download(&path);
                index.insert(id.clone(), self.index.get(&id).cloned().unwrap_or_default());
                if let Some(p) = self.v4_state.borrow().previews.get(&id) { previews.insert(id.clone(), p.clone()); }
                pending.insert(id, state);
                continue;
            }
            let record: Record = serde_json::from_slice(&crate::file_io::read(&path)?).with_context(|| format!("无法读取文档元数据 {}", path.display()))?;
            previews.insert(id.clone(), record.preview);
            let mut meta = record.meta;
            meta.path = None; meta.opened = 0; meta.catalogue_closed = true; meta.reading = false;
            records.insert(id.clone(), (stamp.0, stamp.1, Record { meta: meta.clone(), preview: previews[&id].clone() }));
            index.insert(id, meta);
        }
        for (id, path) in json_files(&self.root.join("trash"))? {
            if !crate::file_io::download_state(&path).ready() {
                let _ = crate::file_io::request_download(&path);
                if let Some(t) = self.trash.get(&id) { trash.insert(id, t.clone()); }
                continue;
            }
            let mut t: Trashed = serde_json::from_slice(&crate::file_io::read(&path)?)?;
            t.meta.path = None; t.meta.opened = 0; t.meta.catalogue_closed = true; t.meta.reading = false;
            trash.insert(id, t);
        }
        let state = self.v4_state.get_mut();
        state.previews = previews; state.pending = pending; state.records = records;
        let local_path = state.local.join("documents.json");
        let local: HashMap<String, Meta> = match crate::file_io::read(&local_path) {
            Ok(bytes) => serde_json::from_slice(&bytes)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => HashMap::new(),
            Err(e) => return Err(e.into()),
        };
        for (id, m) in local {
            if m.local() { index.insert(id, m); }
            else if let Some(meta) = index.get_mut(&id).or_else(|| trash.get_mut(&id).map(|t| &mut t.meta)) {
                meta.opened = m.opened; meta.catalogue_closed = m.catalogue_closed; meta.reading = m.reading;
            }
        }
        state.active = index.keys().cloned().collect();
        state.trash = trash.keys().cloned().collect();
        state.metadata = index.iter().map(|(id, m)| (id.clone(), synced(m))).collect();
        state.trashed = trash.iter().map(|(id, t)| (id.clone(), serde_json::to_value(t).expect("Trashed is JSON"))).collect();
        self.index = index; self.trash = trash;
        Ok(())
    }

    fn preview_at(&self, id: &str, meta: &Meta, trash: bool) -> Result<Preview> {
        if let Some(title) = &meta.folder { return Ok(Preview { title: title.clone(), icon: meta.folder_icon.clone(), ..Default::default() }); }
        let path = if let Some(path) = &meta.path { PathBuf::from(path) }
            else { self.root.join(if trash { "trash" } else { "docs" }).join(format!("{id}.{}", if meta.markdown { "md" } else { "xml" })) };
        let text = match crate::file_io::read_to_string(&path) {
            Ok(text) => text,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Preview::default()),
            Err(e) => return Err(e).with_context(|| format!("无法读取 {}", path.display())),
        };
        let doc = if meta.markdown { crate::markdown::parse_file(&text, meta.path.as_ref().and_then(|p| Path::new(p).file_stem()).and_then(|s| s.to_str()).or(meta.name.as_deref()).unwrap_or_default(), meta.path.as_ref().and_then(|p| Path::new(p).parent())) } else { docxml::parse(&text)? };
        Ok(Preview::document(&doc))
    }

    pub(super) fn write_index_v4(&self, index: &HashMap<String, Meta>) -> Result<()> { self.write_index_records(index, None) }

    fn write_index_records(&self, index: &HashMap<String, Meta>, adopted: Option<&str>) -> Result<()> {
        let old_local = self.v4_state.borrow().local.join("documents.json");
        let mut local: HashMap<String, Meta> = optional_json(&old_local)?.unwrap_or_default();
        // Keep UI state for trashed documents so restoring keeps this device's view.
        local.retain(|id, _| index.contains_key(id) || self.trash.contains_key(id));
        for (id, meta) in index {
            anyhow::ensure!(uuid_name(id), "文档标识不是 UUID：{id}");
            local.insert(id.clone(), if meta.local() { meta.clone() } else { Meta { opened: meta.opened, catalogue_closed: meta.catalogue_closed, reading: meta.reading, ..Default::default() } });
            if meta.local() { continue; }
            let desired = synced(meta);
            let previous = self.v4_state.borrow().metadata.get(id).cloned();
            if previous.as_ref() == Some(&desired) { continue; }
            anyhow::ensure!(!self.v4_state.borrow().pending.contains_key(id), "文档信息正在下载，请稍后再试");
            let path = self.root.join("docs").join(format!("{id}.json"));
            crate::file_io::with_write(&path, |actual| {
                let preview = if meta.folder.is_some() { self.preview_at(id, meta, false)? } else {
                    match reread::<Record>(actual) {
                        Some(r) => r.preview,
                        None => match reread::<serde_json::Value>(&self.root.join("trash").join(format!("{id}.json"))).and_then(|v| v.get("preview").cloned()) {
                            Some(p) => serde_json::from_value(p)?,
                            None => self.preview_at(id, meta, false)?,
                        }
                    }
                };
                let current: Option<serde_json::Value> = optional_json(actual)?;
                anyhow::ensure!(current.is_some() || previous.is_none(), "文档元数据已被移走，未重新创建：{id}");
                let mut value = current.unwrap_or_else(|| serde_json::json!({}));
                let object = value.as_object_mut().context("文档元数据不是 JSON 对象")?;
                let fields: HashSet<_> = desired.as_object().unwrap().keys().chain(previous.as_ref().and_then(|p| p.as_object()).into_iter().flat_map(|p| p.keys())).cloned().collect();
                let mut changed = vec![];
                for key in fields {
                    if previous.as_ref().and_then(|p| p.get(&key)) == desired.get(&key) { continue; }
                    changed.push(key.clone());
                    match desired.get(&key) { Some(v) => { object.insert(key, v.clone()); }, None => { object.remove(&key); } }
                }
                for key in ["opened", "catalogue_closed", "reading", "path"] { object.remove(key); }
                object.insert("preview".into(), serde_json::to_value(preview)?);
                if adopted == Some(id.as_str()) {
                    value[super::store_conflicts::CLOCKS] = serde_json::json!({});
                    for field in &changed { value[super::store_conflicts::CLOCKS][field] = 0.into(); }
                } else { super::store_conflicts::stamp_fields(&mut value, &changed); }
                changed_json(actual, &value)?;
                Ok(())
            })?;
            self.v4_state.borrow_mut().metadata.insert(id.clone(), desired);
        }
        let mut state = self.v4_state.borrow_mut();
        changed_json(&state.local.join("documents.json"), &local)?;
        for id in state.active.difference(&index.keys().cloned().collect()) {
            let p = self.root.join("docs").join(format!("{id}.json"));
            match crate::file_io::remove_file(p) { Ok(()) => {}, Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}, Err(e) => return Err(e.into()) }
        }
        state.active = index.keys().cloned().collect();
        state.metadata.retain(|id, _| index.contains_key(id));
        Ok(())
    }

    pub(super) fn write_trash_v4(&self, trash: &HashMap<String, Trashed>) -> Result<()> {
        crate::file_io::create_dir_all(self.root.join("trash"))?;
        for (id, t) in trash {
            anyhow::ensure!(uuid_name(id), "文档标识不是 UUID：{id}");
            let desired = serde_json::to_value(t)?;
            if self.v4_state.borrow().trashed.get(id) == Some(&desired) { continue; }
            let path = self.root.join("trash").join(format!("{id}.json"));
            crate::file_io::with_write(&path, |actual| {
                let source = optional_json::<serde_json::Value>(&self.root.join("docs").join(format!("{id}.json")))?;
                let old = optional_json::<serde_json::Value>(actual)?;
                let preview = source.as_ref().and_then(|v| v.get("preview")).or_else(|| old.as_ref().and_then(|v| v.get("preview"))).cloned()
                    .unwrap_or(serde_json::to_value(self.preview_at(id, &t.meta, true).unwrap_or_default())?);
                let mut meta = source.or_else(|| old.as_ref().and_then(|v| v.get("meta")).cloned()).unwrap_or_else(|| synced(&t.meta));
                for field in ["preview", "opened", "catalogue_closed", "reading", "path"] { meta.as_object_mut().context("回收站元数据无效")?.remove(field); }
                let mut value = old.unwrap_or_else(|| serde_json::json!({}));
                anyhow::ensure!(value.is_object(), "回收站元数据不是 JSON 对象，未覆盖");
                let deleted = serde_json::to_value(t.deleted)?; let root = serde_json::to_value(&t.root)?;
                let changed: Vec<_> = [("deleted", &deleted), ("root", &root)].into_iter().filter(|(key, v)| value.get(*key) != Some(*v)).map(|(key, _)| key.to_string()).collect();
                value["meta"] = meta; value["deleted"] = deleted; value["root"] = root; value["preview"] = preview;
                super::store_conflicts::stamp_fields(&mut value, &changed);
                changed_json(actual, &value)
            })?;
            self.v4_state.borrow_mut().trashed.insert(id.clone(), desired);
        }
        let mut state = self.v4_state.borrow_mut();
        for id in state.trash.difference(&trash.keys().cloned().collect()) {
            let p = self.root.join("trash").join(format!("{id}.json"));
            match crate::file_io::remove_file(p) { Ok(()) => {}, Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}, Err(e) => return Err(e.into()) }
        }
        state.trash = trash.keys().cloned().collect();
        state.trashed.retain(|id, _| trash.contains_key(id));
        Ok(())
    }

    pub(super) fn refresh_preview(&self, id: &str, doc: &Document) -> Result<()> {
        if !self.v4() || self.index.get(id).is_some_and(Meta::local) { return Ok(()); }
        let path = self.root.join("docs").join(format!("{id}.json"));
        update_json(&path, |v| {
            let preview = serde_json::to_value(Preview::document(doc))?;
            if v.get("preview") != Some(&preview) { v["preview"] = preview; super::store_conflicts::stamp_fields(v, &["preview".into()]); }
            Ok(())
        })
    }

    pub fn download_state(&self, id: &str) -> DownloadState {
        if let Some(state) = self.v4_state.borrow().pending.get(id) { return state.clone(); }
        if self.index.get(id).is_some_and(|m| m.folder.is_some()) { return DownloadState::Ready; }
        crate::file_io::download_state(&self.path(id))
    }

    pub(super) fn ensure_downloaded(&self, id: &str) -> Result<()> {
        match self.download_state(id) {
            DownloadState::Ready => Ok(()),
            DownloadState::Missing => anyhow::bail!("文档文件暂不可用，未读取或覆盖"),
            DownloadState::Failed(e) => anyhow::bail!("文档下载失败：{e}"),
            _ => { let _ = crate::file_io::request_download(&self.path(id)); anyhow::bail!("文档正在下载，完成后再打开") }
        }
    }

    pub fn externally_changed(&self, id: &str) -> Result<bool> {
        self.ensure_downloaded(id)?;
        let loaded = self.loaded.borrow();
        let Some(previous) = loaded.get(id) else { return Ok(false); };
        Ok(crate::file_io::read_to_string(self.path(id))? != *previous)
    }

    /// Refresh the listing after an external file event; never alters an editor's baseline.
    pub fn refresh_external(&mut self, id: &str) -> Result<()> {
        anyhow::ensure!(uuid_name(id), "文档标识不是 UUID");
        let _lock = self.begin_write()?;
        if !self.index.contains_key(id) {
            anyhow::ensure!(!self.trash.contains_key(id), "文档已在回收站，未重新创建");
            let xml = self.root.join("docs").join(format!("{id}.xml"));
            let md = self.root.join("docs").join(format!("{id}.md"));
            anyhow::ensure!(xml.is_file() || md.is_file(), "文档尚未下载");
            let markdown = !xml.is_file();
            self.index.insert(id.to_string(), Meta { created: now(), modified: now(), markdown, ..Default::default() });
            self.write_index_records(&self.index, Some(id))?;
        }
        self.ensure_downloaded(id)?;
        let meta = self.index.get(id).context("文档元数据不存在")?;
        if meta.folder.is_some() || meta.local() { return Ok(()); }
        let preview = self.preview_at(id, meta, false)?;
        let path = self.root.join("docs").join(format!("{id}.json"));
        update_json(&path, |v| {
            let preview = serde_json::to_value(preview)?;
            if v.get("preview") != Some(&preview) { v["preview"] = preview; super::store_conflicts::stamp_fields(v, &["preview".into()]); }
            Ok(())
        })
    }

    pub(super) fn list_v4(&mut self) -> Vec<Entry> {
        let _lock = match self.begin_write() { Ok(l) => Some(l), Err(e) => { self.warnings.push(e.to_string()); None } };
        let mut out = vec![];
        for (id, meta) in &self.index {
            let preview = if meta.local() {
                if self.download_state(id).ready() { self.preview_at(id, meta, false).ok() }
                else { Some(Preview { title: self.markdown_title(id), ..Default::default() }) }
            } else { Some(self.v4_state.borrow().previews.get(id).cloned().unwrap_or_default()) };
            let Some(preview) = preview else { continue; };
            let Parsed { title, icon, body, reminders, links, tags } = preview.parsed();
            out.push(Entry { download: self.download_state(id), id: id.clone(), meta: meta.clone(), title: meta.folder.clone().or_else(|| meta.name.clone()).unwrap_or(title), icon, body, reminders, links, tags });
        }
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }

    pub(super) fn adopt_orphans(&mut self) -> Result<()> {
        for e in crate::file_io::read_dir(self.root.join("docs"))? {
            let path = e?.path();
            if !matches!(path.extension().and_then(|s| s.to_str()), Some("xml" | "md")) { continue; }
            let Some(id) = path.file_stem().and_then(|s| s.to_str()).filter(|id| uuid_name(id)) else { continue; };
            if self.index.contains_key(id) || self.trash.contains_key(id) { continue; }
            if !crate::file_io::download_state(&path).ready() { let _ = crate::file_io::request_download(&path); continue; }
            if let Err(e) = self.refresh_external(id) { self.warnings.push(format!("未能收录文档 {id}，原文件仍保留：{e}")); }
        }
        Ok(())
    }

    pub(super) fn list_trash_v4(&mut self) -> Vec<TrashEntry> {
        let locked = self.begin_write();
        if let Err(e) = &locked { self.warnings.push(e.to_string()); }
        if locked.is_ok() {
            let cutoff = now().saturating_sub(TRASH_DAYS * 86_400);
            let expired: Vec<_> = self.trash.iter().filter(|(id, t)| t.deleted <= cutoff && (t.meta.folder.is_some() || self.trash_path(id).is_file()))
                .map(|(id, _)| id.clone()).collect();
            for id in expired {
                let result = if self.trash[&id].meta.folder.is_some() { Ok(()) } else { crate::file_io::remove_file(self.trash_path(&id)) };
                match result { Ok(()) => { self.trash.remove(&id); }, Err(e) => self.warnings.push(format!("无法清理过期回收站文档 {id}：{e}")) }
            }
            if let Err(e) = self.save_trash() { self.warnings.push(format!("回收站元数据保存失败：{e}")); }
        }
        let mut out = vec![];
        for (id, t) in &self.trash {
            if t.root.as_ref().is_some_and(|r| self.trash.contains_key(r)) { continue; }
            let preview: Preview = reread::<serde_json::Value>(&self.root.join("trash").join(format!("{id}.json")))
                .and_then(|v| v.get("preview").cloned()).and_then(|v| serde_json::from_value(v).ok()).unwrap_or_default();
            out.push(TrashEntry { id: id.clone(), title: t.meta.folder.clone().or_else(|| t.meta.name.clone()).unwrap_or(preview.title),
                icon: t.meta.folder_icon.clone().or(preview.icon), deleted: t.deleted, parent: t.meta.parent.clone(), folder: t.meta.folder.is_some() });
        }
        out.sort_by(|a, b| b.deleted.cmp(&a.deleted).then_with(|| a.id.cmp(&b.id)));
        out
    }

    /// Keep interrupted writes for recovery, outside the synchronized folder.
    pub(super) fn clean_sync_folder(&mut self) -> Result<()> {
        let _lock = self.begin_write()?;
        let local = self.v4_state.borrow().local.join("recovered-files");
        for dir in [&self.root, &self.root.join("docs"), &self.root.join("trash")] {
            let entries = match crate::file_io::read_dir(dir) { Ok(e) => e, Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue, Err(e) => return Err(e.into()) };
            for e in entries {
                let e = e?;
                if !e.file_type()?.is_file() { continue; }
                let name = e.file_name().to_string_lossy().into_owned();
                if !(name.ends_with(".tmp") || name.contains(".corrupt-") || matches!(name.as_str(), ".lock" | "index.json.bak" | "index.json" | "index.before-folders.json")) { continue; }
                crate::file_io::create_dir_all(&local)?;
                let destination = local.join(format!("{}-{name}", uuid::Uuid::new_v4()));
                write_atomic(&destination, &crate::file_io::read(e.path())?)?;
                crate::file_io::remove_file(e.path())?;
            }
        }
        Ok(())
    }

    pub(super) fn migrate_v4(&mut self) -> Result<()> {
        let _lock = self.begin_write()?;
        if self.v4() { return Ok(()); }
        // A failed migration leaves the old marker and indexes intact. Retrying
        // reconstructs the same sidecars; the marker is the last published file.
        for entry in crate::file_io::read_dir(self.root.join("docs"))? {
            let path = entry?.path();
            let markdown = match path.extension().and_then(|s| s.to_str()) { Some("xml") => false, Some("md") => true, _ => continue };
            let Some(id) = path.file_stem().and_then(|s| s.to_str()).filter(|s| uuid_name(s)) else { continue };
            self.index.entry(id.to_string()).or_insert_with(|| Meta { created: now(), modified: now(), markdown, ..Default::default() });
        }
        if let Ok(entries) = crate::file_io::read_dir(self.root.join("trash")) {
            for entry in entries {
                let path = entry?.path();
                let markdown = match path.extension().and_then(|s| s.to_str()) { Some("xml") => false, Some("md") => true, _ => continue };
                let Some(id) = path.file_stem().and_then(|s| s.to_str()).filter(|s| uuid_name(s)) else { continue };
                self.trash.entry(id.to_string()).or_insert_with(|| Trashed { meta: Meta { created: now(), modified: now(), markdown, ..Default::default() }, deleted: now(), root: None });
            }
        }
        self.write_index_v4(&self.index)?;
        self.write_trash_v4(&self.trash)?;
        for (id, meta) in self.index.iter().filter(|(_, m)| !m.local()) {
            let path = self.root.join("docs").join(format!("{id}.json"));
            let mut value: serde_json::Value = reread(&path).context("迁移元数据不存在")?;
            value["preview"] = serde_json::to_value(self.preview_at(id, meta, false)?)?;
            changed_json(&path, &value)?;
        }
        for rel in ["index.json", "trash/index.json"] {
            let source = self.root.join(rel);
            if source.exists() {
                let dir = self.v4_state.borrow().local.join("migration-format4").join(if rel.starts_with("trash/") { "trash" } else { "docs" });
                crate::file_io::create_dir_all(&dir)?;
                let mut backup = dir.join("index.before-format4.json");
                let bytes = crate::file_io::read(&source)?;
                if backup.exists() && crate::file_io::read(&backup)? != bytes { backup = dir.join(format!("index.before-format4-{}.json", uuid::Uuid::new_v4())); }
                if !backup.exists() { write_atomic(&backup, &bytes)?; }
            }
        }
        write_atomic(&self.root.join("format.json"), &serde_json::to_vec_pretty(&FormatMarker { format: FORMAT })?)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self { Self(std::env::temp_dir().join(format!("qmd-v4-{}", uuid::Uuid::new_v4()))) }
        fn open(&self) -> Store { Store::open_at(self.0.clone()).unwrap() }
        fn legacy(&self) -> (String, String, Vec<u8>) {
            std::fs::create_dir_all(self.0.join("docs")).unwrap();
            std::fs::create_dir_all(self.0.join("trash")).unwrap();
            let id = uuid::Uuid::new_v4().to_string();
            let folder = uuid::Uuid::new_v4().to_string();
            let meta = Meta { created: 7, modified: 9, opened: 11, reading: true, catalogue_closed: false, parent: Some(folder.clone()), pinned: true, ..Default::default() };
            let index = HashMap::from([(id.clone(), meta), (folder.clone(), Meta { folder: Some("文件夹".into()), created: 1, modified: 2, ..Default::default() })]);
            let bytes = serde_json::to_vec_pretty(&index).unwrap();
            std::fs::write(self.0.join("format.json"), b"{\"format\":3}").unwrap();
            std::fs::write(self.0.join("index.json"), &bytes).unwrap();
            std::fs::write(self.0.join("docs").join(format!("{id}.xml")), "<title>迁移原文</title>\n<p>不改正文</p>\n").unwrap();
            (id, folder, bytes)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
            let _ = std::fs::remove_dir_all(self.0.with_extension("local-state"));
        }
    }
    fn document(title: &str) -> Document { Document { title: crate::model::RichText::plain(title), ..Default::default() } }

    #[test]
    fn migration_preserves_bytes_metadata_and_private_state() {
        let dir = Temp::new();
        let (id, folder, index) = dir.legacy();
        let body = std::fs::read(dir.0.join("docs").join(format!("{id}.xml"))).unwrap();
        let mut s = dir.open();
        let e = s.list().into_iter().find(|e| e.id == id).unwrap();
        assert_eq!(e.title, "迁移原文");
        assert_eq!(e.meta.parent, Some(folder.clone()));
        assert!(e.meta.pinned && e.meta.reading && !e.meta.catalogue_closed);
        assert_eq!(e.meta.opened, 11);
        assert_eq!(std::fs::read(dir.0.join("docs").join(format!("{id}.xml"))).unwrap(), body);
        assert_eq!(std::fs::read(s.v4_state.borrow().local.join("migration-format4/docs/index.before-format4.json")).unwrap(), index);
        for key in [id, folder] {
            let json: serde_json::Value = reread(&dir.0.join("docs").join(format!("{key}.json"))).unwrap();
            for field in ["opened", "reading", "catalogue_closed", "path"] { assert!(json.get(field).is_none(), "{field}"); }
        }
        for file in ["index.json", "index.json.bak", ".lock"] { assert!(!dir.0.join(file).exists()); }
        assert!(s.v4_state.borrow().local.join(".lock").is_file());
        assert_eq!(dir.open().list().len(), 2);
    }

    #[test]
    fn interrupted_migration_keeps_legacy_index_and_can_retry() {
        let dir = Temp::new();
        let (id, _, index) = dir.legacy();
        let collision = dir.0.join("docs").join(format!("{id}.json"));
        std::fs::create_dir(&collision).unwrap();
        assert!(Store::open_at(dir.0.clone()).is_err());
        assert_eq!(reread::<FormatMarker>(&dir.0.join("format.json")).unwrap().format, 3);
        assert_eq!(std::fs::read(dir.0.join("index.json")).unwrap(), index);
        std::fs::remove_dir(&collision).unwrap();
        assert_eq!(dir.open().list().len(), 2);
    }

    #[test]
    fn local_view_changes_do_not_touch_synced_metadata() {
        let dir = Temp::new();
        let mut s = dir.open();
        let id = s.create_from(&document("page")).unwrap();
        let path = dir.0.join("docs").join(format!("{id}.json"));
        let bytes = std::fs::read(&path).unwrap();
        let stamp = std::fs::metadata(&path).unwrap().modified().unwrap();
        s.touch(&id).unwrap();
        s.set_catalogue(&id, true).unwrap();
        s.update_meta(&id, |m| m.reading = true).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), stamp);
        s.delete(&id, None).unwrap();
        s.restore(&id).unwrap();
        let m = &dir.open().list()[0].meta;
        assert!(m.reading && !m.catalogue_closed && m.opened > 0);
    }

    #[test]
    fn stale_unchanged_entries_never_replace_remote_metadata() {
        let dir = Temp::new();
        let mut s = dir.open();
        let a = s.create_from(&document("a")).unwrap();
        let b = s.create_from(&document("b")).unwrap();
        let _lock = s.begin_write().unwrap();
        let path = dir.0.join("docs").join(format!("{b}.json"));
        let mut remote: serde_json::Value = reread(&path).unwrap();
        remote["pinned"] = true.into();
        remote["future_field"] = "keep".into();
        changed_json(&path, &remote).unwrap();
        s.index.get_mut(&a).unwrap().pinned = true;
        s.save_index().unwrap();
        assert_eq!(reread::<serde_json::Value>(&path).unwrap(), remote);
        // Independent fields on the same record survive too.
        s.index.get_mut(&b).unwrap().starred = 123;
        s.save_index().unwrap();
        let merged: serde_json::Value = reread(&path).unwrap();
        assert_eq!(merged["pinned"], true);
        assert_eq!(merged["future_field"], "keep");
        assert_eq!(merged["starred"], 123);
    }

    #[test]
    fn conflict_filenames_and_partial_downloads_do_not_become_documents() {
        let dir = Temp::new();
        let mut s = dir.open();
        let id = s.create_from(&document("original")).unwrap();
        let xml = docxml::serialize(&document("conflict"));
        std::fs::write(dir.0.join("docs").join(format!("{id} 2.xml")), &xml).unwrap();
        std::fs::write(dir.0.join("docs/not-a-uuid.xml"), &xml).unwrap();
        std::fs::remove_file(s.path(&id)).unwrap();
        let rows = s.list();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title, "original");
        assert!(s.load_for_edit(&id, false).is_err());
        assert!(s.save(&id, &document("local")).is_err());
    }

    #[test]
    fn corrupted_sidecar_is_never_replaced_with_defaults() {
        let dir = Temp::new();
        let mut s = dir.open();
        let id = s.create_from(&document("original")).unwrap();
        let path = dir.0.join("docs").join(format!("{id}.json"));
        std::fs::write(&path, "broken").unwrap();
        assert!(s.set_pinned(&id, true).is_err());
        assert!(Store::open_at(dir.0.clone()).is_err());
        assert_eq!(std::fs::read_to_string(path).unwrap(), "broken");
    }

    #[test]
    fn abandoned_files_are_preserved_locally() {
        let dir = Temp::new();
        let s = dir.open();
        std::fs::write(dir.0.join("docs/interrupted.xml.tmp"), "unsaved").unwrap();
        std::fs::write(dir.0.join("index.json.corrupt-42"), "damaged").unwrap();
        let reopened = dir.open();
        assert!(!dir.0.join("docs/interrupted.xml.tmp").exists());
        assert!(!dir.0.join("index.json.corrupt-42").exists());
        let contents: HashSet<_> = std::fs::read_dir(reopened.v4_state.borrow().local.join("recovered-files")).unwrap().flatten()
            .map(|e| std::fs::read_to_string(e.path()).unwrap()).collect();
        assert!(contents.contains("unsaved") && contents.contains("damaged"));
        assert_eq!(s.v4_state.borrow().local, reopened.v4_state.borrow().local);
    }
    #[test]
    fn a_partial_download_is_listed_but_cannot_be_opened_or_saved() {
        struct Pending(PathBuf);
        impl crate::file_io::Coordinator for Pending {
            fn download_state(&self, path: &Path) -> std::io::Result<DownloadState> { Ok(if path == self.0 { DownloadState::Downloading } else if path.exists() { DownloadState::Ready } else { DownloadState::Missing }) }
            fn access(&self, r: crate::file_io::Access<'_>, f: &mut dyn FnMut(&Path, Option<&Path>) -> std::io::Result<()>) -> std::io::Result<()> { let (p, q) = r.paths(); f(&p, q.as_deref()) }
        }
        let dir = Temp::new(); let mut s = dir.open();
        let id = s.create_from(&document("cached title")).unwrap();
        let path = s.path(&id);
        std::fs::write(&path, "<title>partial</title>").unwrap();
        crate::file_io::testing(std::sync::Arc::new(Pending(path.clone())), || {
            let entries = s.list();
            assert_eq!(entries[0].title, "cached title");
            assert_eq!(entries[0].download, DownloadState::Downloading);
            assert!(s.load_for_edit(&id, false).is_err());
            assert!(s.save(&id, &document("must not overwrite")).is_err());
        });
        assert_eq!(std::fs::read_to_string(path).unwrap(), "<title>partial</title>");
    }

}
