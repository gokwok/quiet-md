use super::*;
use std::path::Path;
use serde_json::{Map, Value};

/// An immutable losing snapshot. The platform owns the opaque stable key.
#[derive(Clone)]
pub struct ConflictVersion { pub key: String, pub bytes: Vec<u8>, pub resolved: bool }
pub trait ConflictVersions {
    fn versions(&self, path: &Path) -> Result<Vec<ConflictVersion>>;
    /// Resolve only these durable copies; false means a new version arrived meanwhile.
    fn finish(&self, path: &Path, copied: &[String]) -> Result<bool>;
}
#[derive(Default)]
pub struct ConflictReport { pub copies: Vec<String>, pub merged: usize, pub errors: Vec<String> }

pub(super) const CLOCKS: &str = "field_modified";
const PRIVATE: [&str; 4] = ["opened", "catalogue_closed", "reading", "path"];

pub(super) fn field_stamp(value: &Value, field: &str) -> u64 {
    value.get(CLOCKS).and_then(|c| c.get(field)).and_then(Value::as_u64)
        .unwrap_or_else(|| if value.get(field).is_some() { value.get("modified").or_else(|| value.get("deleted")).and_then(Value::as_u64).unwrap_or(0).saturating_mul(1000) } else { 0 })
}

pub(super) fn stamp_fields(value: &mut Value, fields: &[String]) {
    if fields.is_empty() { return; }
    let last = value.get(CLOCKS).and_then(Value::as_object).into_iter().flat_map(|m| m.values()).filter_map(Value::as_u64).max().unwrap_or(0);
    let time = std::time::SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis().min(u64::MAX as u128) as u64;
    let clock = time.max(last.saturating_add(1));
    if !value.get(CLOCKS).is_some_and(Value::is_object) { value[CLOCKS] = Value::Object(Map::new()); }
    for field in fields { value[CLOCKS][field] = clock.into(); }
}

/// Per-field LWW, including a clock for absent/default values. Equal clocks
/// use canonical JSON ordering, so either merge order converges deterministically.
pub fn merge_metadata(winner: &[u8], other: &[u8]) -> Result<Vec<u8>> {
    let mut a: Value = serde_json::from_slice(winner)?;
    let b: Value = serde_json::from_slice(other)?;
    if a.get("meta").is_some() || b.get("meta").is_some() {
        let left = serde_json::to_vec(a.get("meta").context("回收站元数据缺失")?)?;
        let right = serde_json::to_vec(b.get("meta").context("回收站元数据缺失")?)?;
        let meta: Value = serde_json::from_slice(&merge_metadata(&left, &right)?)?;
        a = merge_object(&a, &b)?;
        a["meta"] = meta;
        serde_json::from_value::<Trashed>(a.clone())?;
        a.sort_all_objects();
        return Ok(serde_json::to_vec_pretty(&a)?);
    }
    let mut out = merge_object(&a, &b)?;
    out.sort_all_objects();
    serde_json::from_value::<Meta>(out.clone()).context("合并后的元数据无效")?;
    Ok(serde_json::to_vec_pretty(&out)?)
}

fn merge_object(a: &Value, b: &Value) -> Result<Value> {
    let am = a.as_object().context("元数据不是 JSON 对象")?;
    let bm = b.as_object().context("元数据不是 JSON 对象")?;
    let keys: std::collections::BTreeSet<_> = am.keys().chain(bm.keys())
        .chain(am.get(CLOCKS).and_then(Value::as_object).into_iter().flat_map(|m| m.keys()))
        .chain(bm.get(CLOCKS).and_then(Value::as_object).into_iter().flat_map(|m| m.keys())).cloned().collect();
    let mut out = a.clone();
    out[CLOCKS] = Value::Object(Map::new());
    for key in keys {
        if key == CLOCKS || PRIVATE.contains(&key.as_str()) { continue; }
        let (at, bt) = (field_stamp(a, &key), field_stamp(b, &key));
        let left = a.get(&key); let right = b.get(&key);
        let canonical = |v: &Value| { let mut v = v.clone(); v.sort_all_objects(); v.to_string() };
        let use_b = bt > at || (bt == at && right.map(canonical) > left.map(canonical));
        if use_b {
            match right { Some(v) => { out[&key] = v.clone(); }, None => { out.as_object_mut().unwrap().remove(&key); } }
        }
        out[CLOCKS][&key] = at.max(bt).into();
    }
    for field in PRIVATE { out.as_object_mut().unwrap().remove(field); }
    Ok(out)
}

/// Change only the title bytes: the losing body (including unknown XML) stays intact.
fn conflict_xml(bytes: &[u8]) -> Result<Vec<u8>> {
    let text = std::str::from_utf8(bytes)?;
    docxml::parse(text)?;
    let mut reader = quick_xml::Reader::from_str(text);
    let mut depth = 0;
    let mut title = false;
    let mut insertion = 0;
    loop {
        let start = reader.buffer_position() as usize;
        match reader.read_event()? {
            quick_xml::events::Event::Start(e) => { if depth == 0 && e.name().as_ref() == b"title" { title = true; } depth += 1; }
            quick_xml::events::Event::End(e) => {
                depth -= 1;
                if depth == 0 && title && e.name().as_ref() == b"title" {
                    return Ok(format!("{}（冲突副本）{}", &text[..start], &text[start..]).into_bytes());
                }
            }
            quick_xml::events::Event::Empty(e) if depth == 0 && e.name().as_ref() == b"title" => {
                let end = reader.buffer_position() as usize;
                let tag = text[start..end].trim_end_matches("/>");
                return Ok(format!("{}{tag}>（冲突副本）</title>{}", &text[..start], &text[end..]).into_bytes());
            }
            quick_xml::events::Event::Decl(_) => insertion = reader.buffer_position() as usize,
            quick_xml::events::Event::Eof => break,
            _ => {}
        }
    }
    Ok(format!("{}<title>（冲突副本）</title>\n{}", &text[..insertion], &text[insertion..]).into_bytes())
}

fn bounced(stem: &str) -> Option<&str> {
    let id = stem.get(..36)?;
    let suffix = stem.get(36..)?.strip_prefix(' ')?;
    (super::store_v4::uuid_name(id) && !suffix.is_empty() && suffix.split_whitespace().all(|s| s.parse::<u64>().is_ok_and(|n| n >= 2))).then_some(id)
}

impl Store {
    /// Preserve a losing body before the platform consumes its version.
    pub fn preserve_conflict(&mut self, original: &str, markdown: bool, key: &str, bytes: &[u8]) -> Result<(String, bool)> {
        let namespace = uuid::Uuid::parse_str(original)?;
        let provenance = format!("{original}:{}:{key}", if markdown { "md" } else { "xml" });
        let id = uuid::Uuid::new_v5(&namespace, provenance.as_bytes()).to_string();
        let content = if markdown { std::str::from_utf8(bytes)?; bytes.to_vec() } else { conflict_xml(bytes)? };
        let _lock = self.begin_write()?;
        if let Some(t) = self.trash.get(&id) {
            anyhow::ensure!(t.meta.conflict_source.as_deref() == Some(&provenance), "回收站存在同名文档，未覆盖");
            crate::file_io::read(self.trash_path(&id))?;
            return Ok((id, false));
        }
        if let Some(m) = self.index.get(&id) {
            anyhow::ensure!(m.conflict_source.as_deref() == Some(&provenance), "冲突副本标识已被占用，未覆盖已有文档");
            if self.path(&id).is_file() {
                let doc = self.load(&id)?;
                self.refresh_preview(&id, &doc)?;
                return Ok((id, false));
            }
        } else {
            let mut meta = self.index.get(original).cloned().or_else(|| self.trash.get(original).map(|t| t.meta.clone())).unwrap_or_default();
            meta.folder = None; meta.path = None; meta.opened = 0; meta.pinned = false; meta.starred = 0;
            meta.markdown = markdown; meta.conflict_source = Some(provenance);
            if meta.parent.as_ref().is_some_and(|p| self.index.get(p).is_none_or(|m| m.folder.is_none())) { meta.parent = None; }
            let title = meta.name.clone().unwrap_or_else(|| "未命名".into());
            meta.name = markdown.then(|| format!("{title}（冲突副本）"));
            meta.created = now(); meta.modified = now();
            self.index.insert(id.clone(), meta);
            // Record provenance before publishing the body. An interrupted copy
            // can then resume after reopening without being adopted as an orphan.
            self.save_index()?;
        }
        let path = self.root.join("docs").join(format!("{id}.{}", if markdown { "md" } else { "xml" }));
        crate::file_io::with_write(&path, |p| {
            match crate::file_io::read(p) {
                Ok(existing) => anyhow::ensure!(existing == content, "冲突副本文件已存在且内容不同，未覆盖"),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => write_atomic(p, &content)?,
                Err(e) => return Err(e.into()),
            }
            Ok(())
        })?;
        let doc = self.load(&id)?;
        self.refresh_preview(&id, &doc)?;
        Ok((id, true))
    }

    fn merge_conflict_metadata(&mut self, path: &Path, bytes: &[u8]) -> Result<()> {
        let _lock = self.begin_write()?;
        crate::file_io::with_write(path, |p| {
            let winner = crate::file_io::read(p)?;
            let merged = merge_metadata(&winner, bytes)?;
            if merged != winner { write_atomic(p, &merged)?; }
            Ok(())
        })?;
        self.reload_v4()
    }

    /// iCloud owns version state. Preserve unresolved snapshots before asking
    /// the platform to resolve them; already-resolved versions only need cleanup.
    pub fn resolve_conflicts(&mut self, provider: &dyn ConflictVersions) -> ConflictReport {
        let mut report = ConflictReport::default();
        for directory in ["docs", "trash"] {
            let entries = match crate::file_io::read_dir(self.root.join(directory)) {
                Ok(e) => e, Err(e) => { report.errors.push(e.to_string()); continue; }
            };
            for entry in entries {
                let path = match entry { Ok(e) => e.path(), Err(e) => { report.errors.push(e.to_string()); continue; } };
                if !crate::file_io::download_state(&path).ready() { let _ = crate::file_io::request_download(&path); continue; }
                let extension = match path.extension().and_then(|s| s.to_str()) { Some(e @ ("xml" | "md" | "json")) => e, _ => continue };
                let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else { continue; };
                let result = (|| -> Result<()> {
                    if let Some(original) = bounced(stem) {
                        let bytes = crate::file_io::read(&path)?;
                        if extension == "json" {
                            self.merge_conflict_metadata(&path.with_file_name(format!("{original}.json")), &bytes)?;
                            report.merged += 1;
                        } else {
                            let digest = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_OID, &bytes);
                            let (id, created) = self.preserve_conflict(original, extension == "md", &format!("bounced:{stem}:{digest}"), &bytes)?;
                            if created { report.copies.push(id); }
                        }
                        // Do not delete a newer bounced file that arrived during preservation.
                        crate::file_io::with_write(&path, |p| {
                            anyhow::ensure!(crate::file_io::read(p)? == bytes, "冲突文件又有改动，保留待下次处理");
                            crate::file_io::remove_file(p)?; Ok(())
                        })?;
                        return Ok(());
                    }
                    if !super::store_v4::uuid_name(stem) { return Ok(()); }
                    let versions = provider.versions(&path)?;
                    let mut copied = vec![];
                    for version in versions {
                        if version.resolved { copied.push(version.key); continue; }
                        if extension == "json" { self.merge_conflict_metadata(&path, &version.bytes)?; report.merged += 1; }
                        else {
                            let (id, created) = self.preserve_conflict(stem, extension == "md", &version.key, &version.bytes)?;
                            if created { report.copies.push(id); }
                        }
                        copied.push(version.key);
                    }
                    if !copied.is_empty() { provider.finish(&path, &copied)?; }
                    Ok(())
                })();
                if let Err(e) = result { report.errors.push(format!("{}：{e}", path.file_name().unwrap().to_string_lossy())); }
            }
        }
        report
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    struct Library(PathBuf);
    impl Library {
        fn new() -> Self { Self(std::env::temp_dir().join(format!("qmd-conflicts-{}", uuid::Uuid::new_v4()))) }
        fn open(&self) -> Store { Store::open_at(self.0.clone()).unwrap() }
    }
    impl Drop for Library {
        fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); let _ = std::fs::remove_dir_all(self.0.with_extension("local-state")); }
    }
    fn doc(title: &str) -> Document { Document { title: crate::model::RichText::plain(title), ..Default::default() } }
    struct Versions { path: PathBuf, data: RefCell<Vec<ConflictVersion>>, finishes: Cell<usize>, fail_cleanup: Cell<bool> }
    impl ConflictVersions for Versions {
        fn versions(&self, path: &Path) -> Result<Vec<ConflictVersion>> { Ok(if path == self.path { self.data.borrow().clone() } else { vec![] }) }
        fn finish(&self, _: &Path, copied: &[String]) -> Result<bool> {
            for v in self.data.borrow().iter() { assert!(copied.contains(&v.key)); }
            self.finishes.set(self.finishes.get() + 1);
            if self.fail_cleanup.replace(false) {
                for version in self.data.borrow_mut().iter_mut() { version.resolved = true; }
                anyhow::bail!("quota cleanup failed after marking resolved");
            }
            self.data.borrow_mut().clear();
            Ok(true)
        }
    }
    fn provider(path: PathBuf, bytes: &[u8]) -> Versions {
        Versions { path, data: RefCell::new(vec![ConflictVersion { key: "version-one".into(), bytes: bytes.to_vec(), resolved: false }]), finishes: Cell::new(0), fail_cleanup: Cell::new(false) }
    }

    #[test]
    fn metadata_merge_uses_field_times_not_sort_numbers() {
        let a = serde_json::json!({"created":1,"modified":10,"pinned":true,"width":"wide","root_order":1,"starred":7,
            "field_modified":{"pinned":100,"width":20,"root_order":300,"starred":350}});
        let b = serde_json::json!({"created":1,"modified":10,"pinned":false,"width":"full","root_order":2,"path":"/outside", "future":"kept",
            "field_modified":{"pinned":50,"width":200,"root_order":200,"starred":400}});
        let (a, b) = (serde_json::to_vec(&a).unwrap(), serde_json::to_vec(&b).unwrap());
        let merged = merge_metadata(&a, &b).unwrap();
        assert_eq!(merged, merge_metadata(&b, &a).unwrap());
        let m: Value = serde_json::from_slice(&merged).unwrap();
        assert_eq!(m["pinned"], true); assert_eq!(m["width"], "full"); assert_eq!(m["root_order"], 1);
        assert!(m.get("starred").is_none() && m.get("path").is_none()); assert_eq!(m["future"], "kept");
        assert_eq!(merge_metadata(&merged, &merged).unwrap(), merged);
    }

    #[test]
    fn conflict_copy_keeps_original_and_every_body_byte() {
        let lib = Library::new(); let mut s = lib.open();
        let folder = s.create_folder("folder", None).unwrap();
        let id = s.create_from(&doc("winner")).unwrap(); s.set_parent(&id, Some(&folder)).unwrap();
        let original = std::fs::read(s.path(&id)).unwrap();
        let losing = include_bytes!("../tests/fixtures/all-blocks.xml");
        let (copy, created) = s.preserve_conflict(&id, false, "v1", losing).unwrap();
        assert!(created); assert_eq!(s.index[&copy].parent.as_ref(), Some(&folder));
        let content = std::fs::read_to_string(s.path(&copy)).unwrap();
        assert_eq!(content.replacen("（冲突副本）", "", 1).as_bytes(), losing);
        assert_eq!(std::fs::read(s.path(&id)).unwrap(), original);
        assert_eq!(s.preserve_conflict(&id, false, "v1", losing).unwrap(), (copy.clone(), false));
        s.delete(&copy, None).unwrap();
        assert_eq!(s.preserve_conflict(&id, false, "v1", losing).unwrap(), (copy, false));
    }

    #[test]
    fn markdown_conflict_preserves_front_matter_and_raw_syntax() {
        let lib = Library::new(); let mut s = lib.open();
        let id = s.create_markdown(&doc("MD"), "").unwrap();
        let bytes = b"---\na: 1\n---\n*  spaced  *\n\n<img custom='yes'>\n";
        let (copy, _) = s.preserve_conflict(&id, true, "md-version", bytes).unwrap();
        assert_eq!(std::fs::read(s.path(&copy)).unwrap(), bytes);
        assert_eq!(s.list().iter().find(|e| e.id == copy).unwrap().title, "MD（冲突副本）");
    }

    #[test]
    fn resolved_system_versions_only_retry_cleanup() {
        let lib = Library::new(); let mut s = lib.open();
        let id = s.create_from(&doc("winner")).unwrap();
        let p = provider(s.path(&id), b"<title>loser</title>\n<p>keep</p>\n");
        p.fail_cleanup.set(true);
        let result = s.resolve_conflicts(&p);
        assert_eq!(result.copies.len(), 1); assert_eq!(result.errors.len(), 1); assert_eq!(p.finishes.get(), 1);
        let mut reopened = lib.open();
        let again = reopened.resolve_conflicts(&p);
        assert!(again.copies.is_empty() && again.errors.is_empty()); assert_eq!(p.finishes.get(), 2);
        assert_eq!(reopened.list().len(), 2);
    }

    #[test]
    fn unusable_versions_are_never_marked_resolved() {
        let lib = Library::new(); let mut s = lib.open();
        let id = s.create_from(&doc("winner")).unwrap();
        let original = std::fs::read(s.path(&id)).unwrap();
        let p = provider(s.path(&id), &[0xff]);
        let result = s.resolve_conflicts(&p);
        assert_eq!(result.errors.len(), 1); assert_eq!(p.finishes.get(), 0);
        assert_eq!(std::fs::read(s.path(&id)).unwrap(), original);
        assert_eq!(s.list().len(), 1);
    }

    #[test]
    fn bounced_files_become_copies_and_bad_metadata_stays_for_retry() {
        let lib = Library::new(); let mut s = lib.open();
        let id = s.create_from(&doc("winner")).unwrap();
        let body = lib.0.join("docs").join(format!("{id} 2.xml"));
        std::fs::write(&body, "<title>other</title>\n<p>untouched</p>\n").unwrap();
        let metadata = lib.0.join("docs").join(format!("{id} 2.json"));
        std::fs::write(&metadata, "invalid JSON").unwrap();
        let p = provider(PathBuf::new(), b"");
        let result = s.resolve_conflicts(&p);
        assert_eq!(result.copies.len(), 1); assert_eq!(result.errors.len(), 1);
        assert!(!body.exists() && metadata.exists()); assert_eq!(s.list().len(), 2);
        let again = s.resolve_conflicts(&p); assert!(again.copies.is_empty());
    }
    #[test]
    fn interrupted_copy_resumes_before_versions_are_consumed() {
        struct Deny(PathBuf);
        impl crate::file_io::Coordinator for Deny {
            fn access(&self, r: crate::file_io::Access<'_>, f: &mut dyn FnMut(&Path, Option<&Path>) -> std::io::Result<()>) -> std::io::Result<()> {
                let (p, q) = r.paths();
                if matches!(r, crate::file_io::Access::Write(_)) && p == self.0 { return Err(std::io::Error::other("disk full")); }
                f(&p, q.as_deref())
            }
        }
        let lib = Library::new(); let mut s = lib.open();
        let id = s.create_from(&doc("winner")).unwrap();
        let p = provider(s.path(&id), b"<title>loser</title>\n<p>preserve</p>\n");
        let copy = uuid::Uuid::new_v5(&uuid::Uuid::parse_str(&id).unwrap(), format!("{id}:xml:version-one").as_bytes()).to_string();
        let target = lib.0.join("docs").join(format!("{copy}.xml"));
        let result = crate::file_io::testing(std::sync::Arc::new(Deny(target)), || s.resolve_conflicts(&p));
        assert_eq!(result.errors.len(), 1); assert_eq!(p.finishes.get(), 0);
        drop(s);
        let mut s = lib.open();
        let result = s.resolve_conflicts(&p);
        assert!(result.errors.is_empty(), "{:?}", result.errors);
        assert_eq!(result.copies.len(), 1); assert_eq!(p.finishes.get(), 1);
        assert_eq!(s.list().len(), 2);
    }

}
