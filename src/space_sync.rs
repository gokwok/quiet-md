//! iCloud moves the files. A small watcher refreshes this window's view of them.
use crate::app::Workspace;
use crate::shared_store::SharedStore;
use crate::store::{ConflictReport, DownloadState, Store};
use gpui::*;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

type Snapshot = HashMap<PathBuf, (u64, Option<SystemTime>, DownloadState)>;
#[derive(Default)]
struct SyncState { ready: bool, running: bool, root: Option<PathBuf>, snapshot: Snapshot, last_error: Option<String>, pending: Option<ConflictReport> }
impl Global for SyncState {}

pub fn init(cx: &mut App) {
    cx.set_global(SyncState::default());
    cx.defer(|cx| { cx.global_mut::<SyncState>().ready = true; start(cx); });
    // Also catches ordinary folder changes when Spotlight does not index a
    // chosen location. This compares file attributes, not document contents.
    cx.spawn(async |cx| loop {
        cx.background_executor().timer(Duration::from_secs(2)).await;
        #[cfg(not(target_os = "ios"))]
        if cx.update(start).is_err() { break; }
        #[cfg(target_os = "ios")]
        cx.update(start);
    }).detach();
}

pub fn start(cx: &mut App) {
    if !cx.has_global::<SyncState>() || !cx.has_global::<SharedStore>() || !cx.global::<SyncState>().ready { return; }
    if !cx.windows().is_empty() && let Some(report) = cx.global_mut::<SyncState>().pending.take() { publish(report, &[], true, cx); }
    if cx.global::<SyncState>().running { return; }
    let root = Store::root_dir();
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    let notified = { let _ = crate::apple_files::watch(&root); crate::apple_files::watch_changed() };
    #[cfg(not(any(target_os = "macos", target_os = "ios")))]
    let notified = false;
    let state = cx.global_mut::<SyncState>();
    if state.root.as_ref() != Some(&root) { state.root = Some(root.clone()); state.snapshot.clear(); }
    let previous = state.snapshot.clone(); state.running = true;
    let mut worker = cx.global::<SharedStore>().tree_writer();
    let folder = root.clone();
    let task = cx.background_executor().spawn(async move { scan(&mut worker, &folder, &previous, notified) });
    cx.spawn(async move |cx| {
        let (snapshot, report, documents, changed) = task.await;
        let _ = cx.update(|cx| {
            cx.global_mut::<SyncState>().running = false;
            if root != Store::root_dir() { start(cx); return; }
            cx.global_mut::<SyncState>().snapshot = snapshot;
            publish(report, &documents, changed, cx);
        });
    }).detach();
}

fn scan(store: &mut Store, root: &Path, previous: &Snapshot, notified: bool) -> (Snapshot, ConflictReport, Vec<String>, bool) {
    let mut snapshot = Snapshot::new();
    let mut documents = vec![];
    let mut report = ConflictReport::default();
    store.list();
    for directory in ["docs", "trash"] {
        let Ok(entries) = fdl_core::file_io::read_dir(root.join(directory)) else { continue; };
        for entry in entries.flatten() {
            let mut path = entry.path();
            if let Some(name) = path.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_prefix('.')).and_then(|n| n.strip_suffix(".icloud")) { path = path.with_file_name(name); }
            let metadata = fdl_core::file_io::metadata(&path).ok();
            let stamp = (metadata.as_ref().map_or(0, |m| m.len()), metadata.and_then(|m| m.modified().ok()));
            let state = match previous.get(&path).filter(|old| !notified && old.0 == stamp.0 && old.1 == stamp.1 && old.2.ready()) {
                Some(old) => old.2.clone(), None => fdl_core::file_io::download_state(&path),
            };
            // Text and metadata are small: request all of them. Images are
            // requested by the picture loader only when they become visible.
            if previous.is_empty() || !state.ready() || !previous.contains_key(&path) { let _ = fdl_core::file_io::request_download(&path); }
            let changed = previous.get(&path) != Some(&(stamp.0, stamp.1, state.clone()));
            if directory == "docs" && changed && state.ready() && matches!(path.extension().and_then(|e| e.to_str()), Some("xml" | "md"))
                && let Some(id) = path.file_stem().and_then(|s| s.to_str()).filter(|id| uuid::Uuid::parse_str(id).is_ok())
                && matches!(store.download_state(id), DownloadState::Ready | DownloadState::Missing) {
                match store.refresh_external(id) { Ok(()) => documents.push(id.to_string()), Err(e) => report.errors.push(e.to_string()) }
            }
            snapshot.insert(path, (stamp.0, stamp.1, state));
        }
    }
    let changed = notified || &snapshot != previous;
    if changed {
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        let conflicts = store.resolve_conflicts(&crate::apple_files::Versions);
        #[cfg(not(any(target_os = "macos", target_os = "ios")))]
        let conflicts = store.resolve_conflicts(&NoVersions);
        report.copies = conflicts.copies; report.merged = conflicts.merged; report.errors.extend(conflicts.errors);
    }
    (snapshot, report, documents, changed)
}

fn publish(report: ConflictReport, documents: &[String], changed: bool, cx: &mut App) {
    if cx.windows().is_empty() {
        if !report.copies.is_empty() || !report.errors.is_empty() { cx.global_mut::<SyncState>().pending = Some(report); }
        return;
    }
    let error = report.errors.first().cloned();
    let show_error = error.is_some() && cx.global::<SyncState>().last_error != error;
    cx.global_mut::<SyncState>().last_error = error.clone();
    if !changed && report.copies.is_empty() && report.merged == 0 && !show_error { return; }
    let handles: Vec<_> = cx.windows().into_iter().filter_map(|h| h.downcast::<Workspace>()).collect();
    let mut blocked = HashSet::new();
    for h in &handles {
        if let Ok(w) = h.read(cx) && w.has_unsaved_changes() && let Some(id) = &w.current { blocked.insert(id.clone()); }
    }
    let store = cx.global::<SharedStore>().clone();
    let mut remote = HashMap::new(); let mut conflicts = HashSet::new();
    for id in documents {
        if !store.externally_changed(id).unwrap_or(false) { continue; }
        if blocked.contains(id) { conflicts.insert(id.clone()); }
        else if let Ok(doc) = store.load_for_edit(id, false) { remote.insert(id.clone(), doc); }
    }
    for h in handles {
        let _ = h.update(cx, |w, _, cx| {
            w.entries = w.store.list(); w.sort(); w.sync_links(cx); w.invalidate_previews();
            if let Some(doc) = w.current.as_ref().and_then(|id| remote.get(id)) { w.receive_external_document(doc, cx); }
            if w.current.as_ref().is_some_and(|id| conflicts.contains(id)) {
                w.show_toast_kind(crate::ui::ToastKind::Error, crate::settings::tr("其他设备修改了这篇文档。当前编辑已保留，保存时可另存副本。", "This document changed on another device. Your edits are kept; save a copy to keep both."), cx);
            } else if show_error {
                w.show_toast_kind(crate::ui::ToastKind::Error,
                    &crate::localized_format!("同步暂未完成，原文件已保留：{}", "Sync is not finished; the original files are preserved: {}", error.as_deref().unwrap_or_default()), cx);
            } else if !report.copies.is_empty() {
                w.show_toast(&crate::localized_format!("已保留 {} 份冲突副本", "Kept {} conflict copies", report.copies.len()), cx);
            }
            cx.notify();
        });
    }
}

pub fn download_label(state: &DownloadState) -> Option<&'static str> {
    use crate::settings::tr;
    match state {
        DownloadState::Ready => None,
        DownloadState::Waiting => Some(tr("等待下载", "Waiting to download")),
        DownloadState::Downloading => Some(tr("下载中…", "Downloading…")),
        DownloadState::Missing => Some(tr("文件暂不可用", "File unavailable")),
        DownloadState::Failed(_) => Some(tr("下载失败", "Download failed")),
    }
}

#[cfg(not(any(target_os = "macos", target_os = "ios")))]
struct NoVersions;
#[cfg(not(any(target_os = "macos", target_os = "ios")))]
impl crate::store::ConflictVersions for NoVersions {
    fn versions(&self, _: &Path) -> anyhow::Result<Vec<crate::store::ConflictVersion>> { Ok(vec![]) }
    fn finish(&self, _: &Path, _: &[String]) -> anyhow::Result<bool> { Ok(true) }
}
