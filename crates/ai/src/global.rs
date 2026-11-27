//! MarginAI 的全局库：所有空间共用的助手素材，不跟某个空间走。目录是 `MarginAI/`，里面：
//! - `settings.json`、`presets/`：写作的设置（助手、人设、选中的预设）和预设（`tavern::library`，原来在每个空间的 `tavern/`）；
//! - `imagine.json`：图像的画风、人物和质量词（`imagine::Library`，原来也在空间的 `tavern/`）；
//! - `voices.json`、`avatars/`：语音角色和自选的头像（格式归 iOS 那头管，原来在本机偏好和主目录的 `assistant-avatars/`）；
//! - `migrated.json`：已经并进来的空间。
//!
//! 放哪是这台设备自己的选择（`Place`），记在应用主目录的 `marginai.json`：iCloud 是应用 iCloud 文件夹下的 `MarginAI/`，
//! 本机是主目录下的 `MarginAI/`。没选过时有 iCloud 就放 iCloud；选了 iCloud 但这会儿用不了，先用本机的。
//! 不会自己搬：换位置（`set_place`）时把现有的复制过去（那边已有的不盖掉），原处的不删。
//!
//! 迁移（`migrate`）：把每个空间 `tavern/` 里的预设、助手、人设、画风、人物并进来，同名同内容的合成一个，
//! 同名不同内容的改名加「（2）」，原文件不动。全局库还空着时，第一个并进来的空间（当前空间排最前）的选择带过来。
//! 空间里的文件还在从 iCloud 下载时这次跳过，下次再并；并好的空间记进 `migrated.json`，不再并第二次。
use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};

use fdl_core::file_io::{self, DownloadState};
use serde_json::{Value, json};

use crate::imagine::Library;
use crate::tavern::library::{self, Card, Settings};

/// 全局库的目录名。
pub const DIR: &str = "MarginAI";

/// 全局库放哪。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place { Local, ICloud }

impl Place {
    fn id(self) -> &'static str { match self { Place::Local => "local", Place::ICloud => "icloud" } }
}

/// 这台设备自己的记录：`<主目录>/marginai.json`。
fn device_file(home: &Path) -> PathBuf { home.join("marginai.json") }
fn device(home: &Path) -> Value { file_io::read(device_file(home)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_else(|| json!({})) }
fn set_device(home: &Path, k: &str, v: Value) -> io::Result<()> {
    let mut d = device(home);
    if !d.is_object() { d = json!({}); }
    d[k] = v;
    library::write(&device_file(home), &serde_json::to_vec_pretty(&d).map_err(io::Error::other)?)
}

/// 选过的位置；没选过是 `None`。
pub fn chosen_in(home: &Path) -> Option<Place> {
    match device(home).get("place").and_then(Value::as_str) { Some("local") => Some(Place::Local), Some("icloud") => Some(Place::ICloud), _ => None }
}

/// 实际用的位置：选的（iCloud 用不了时是本机），没选过时有 iCloud 就是 iCloud。
pub fn place_in(home: &Path, icloud: Option<&Path>) -> Place {
    match (chosen_in(home), icloud) { (Some(Place::Local), _) | (_, None) => Place::Local, _ => Place::ICloud }
}

fn root(home: &Path, icloud: Option<&Path>, place: Place) -> PathBuf {
    match (place, icloud) { (Place::ICloud, Some(c)) => c.join(DIR), _ => home.join(DIR) }
}

/// 全局库的目录（不一定已经建好）。
pub fn dir_in(home: &Path, icloud: Option<&Path>) -> PathBuf { root(home, icloud, place_in(home, icloud)) }

fn icloud() -> Option<PathBuf> { fdl_core::spaces::icloud_root() }
fn home() -> PathBuf { fdl_core::store::Store::home_dir() }
pub fn dir() -> PathBuf { dir_in(&home(), icloud().as_deref()) }
pub fn place() -> Place { place_in(&home(), icloud().as_deref()) }
/// 这台设备有没有 iCloud（没有时设置里 iCloud 那项置灰）。
pub fn icloud_available() -> bool { icloud().is_some() }

/// 换位置：把现在用的那份复制过去（那边已有的文件不盖掉），再记下选择。返回新目录。要 iCloud 但用不了时报错。
pub fn set_place_in(home: &Path, icloud: Option<&Path>, place: Place) -> io::Result<PathBuf> {
    if place == Place::ICloud && icloud.is_none() { return Err(io::Error::other("iCloud 不可用")); }
    let (from, to) = (dir_in(home, icloud), root(home, icloud, place));
    if from != to && from.is_dir() { copy_tree(&from, &to)?; }
    set_device(home, "place", place.id().into())?;
    Ok(to)
}
pub fn set_place(place: Place) -> io::Result<PathBuf> { set_place_in(&home(), icloud().as_deref(), place) }

/// 整个目录复制过去：已有的文件不盖，还没下载的（`.icloud` 占位）请求下载、这次跳过。
fn copy_tree(from: &Path, to: &Path) -> io::Result<()> {
    file_io::create_dir_all(to)?;
    for e in file_io::read_dir(from)?.flatten() {
        let (src, name) = (e.path(), e.file_name());
        let dst = to.join(&name);
        if src.is_dir() { copy_tree(&src, &dst)?; continue; }
        let name = name.to_string_lossy();
        if let Some(real) = name.strip_prefix('.').and_then(|n| n.strip_suffix(".icloud")) { let _ = file_io::request_download(&from.join(real)); continue; }
        if name.ends_with(".tmp") || file_io::metadata(&dst).is_ok() { continue; }
        library::write(&dst, &file_io::read(&src)?)?;
    }
    Ok(())
}

/// 这台设备本机的东西（语音角色、头像）并过没有。
pub fn device_migrated_in(home: &Path) -> bool { device(home).get("device_migrated").and_then(Value::as_bool).unwrap_or(false) }
pub fn device_migrated() -> bool { device_migrated_in(&home()) }
pub fn mark_device_migrated() -> io::Result<()> { set_device(&home(), "device_migrated", true.into()) }

/// 把一个目录里的文件复制进 `to`（同名的不盖），头像搬家用。
pub fn copy_files(from: &Path, to: &Path) -> io::Result<usize> {
    let Ok(entries) = file_io::read_dir(from) else { return Ok(0) };
    let mut n = 0;
    for e in entries.flatten() {
        let src = e.path();
        if !src.is_file() { continue; }
        let dst = to.join(e.file_name());
        if file_io::metadata(&dst).is_ok() { continue; }
        library::write(&dst, &file_io::read(&src)?)?;
        n += 1;
    }
    Ok(n)
}

// ---- 迁移 ----

/// 一个空间并进来了多少（新加的，合并掉的不算）。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Merged { pub presets: usize, pub chars: usize, pub personas: usize, pub styles: usize, pub characters: usize }

/// 文件能不能读：在（或者根本没有）就行；还在下载就请求下载、报错，这次不并。
fn ready(path: &Path) -> Result<bool, String> {
    match file_io::download_state(path) {
        DownloadState::Ready => Ok(true),
        DownloadState::Missing => Ok(false),
        DownloadState::Failed(e) => Err(format!("{} 下载失败：{e}", path.display())),
        DownloadState::Waiting | DownloadState::Downloading => { let _ = file_io::request_download(path); Err(format!("{} 还在从 iCloud 下载", path.display())) }
    }
}

/// 不和 `taken` 重名的名字：「名字（2）」「名字（3）」……
fn fresh_name(name: &str, taken: &dyn Fn(&str) -> bool) -> String {
    (2..).map(|n| format!("{name}（{n}）")).find(|c| !taken(c)).unwrap_or_default()
}
/// 不和 `taken` 重的标识：原样，或者加 `-m2`、`-m3`……
fn fresh_id(id: &str, taken: &dyn Fn(&str) -> bool) -> String {
    if !taken(id) { return id.to_owned(); }
    (2..).map(|n| format!("{id}-m{n}")).find(|c| !taken(c)).unwrap_or_default()
}

/// 并一组条目：`same` 判断内容一样（不比标识），`name`/`id` 取改名字和标识。返回 旧标识 → 全局里的标识，和新加了几条。
fn merge_list<T: Clone>(into: &mut Vec<T>, from: &[T], same: impl Fn(&T, &T) -> bool, name: impl Fn(&mut T) -> &mut String, id: impl Fn(&mut T) -> &mut String) -> (BTreeMap<String, String>, usize) {
    let (mut map, mut added) = (BTreeMap::new(), 0);
    for item in from {
        let mut item = item.clone();
        let old = id(&mut item).clone();
        if let Some(hit) = into.iter().position(|x| same(x, &item)) { let mut x = into[hit].clone(); map.insert(old, id(&mut x).clone()); continue; }
        let n = name(&mut item).clone();
        let names: BTreeSet<String> = into.iter().cloned().map(|mut x| name(&mut x).clone()).collect();
        if names.contains(&n) { *name(&mut item) = fresh_name(&n, &|c| names.contains(c)); }
        let ids: BTreeSet<String> = into.iter().cloned().map(|mut x| id(&mut x).clone()).collect();
        let new = fresh_id(&old, &|c| ids.contains(c));
        *id(&mut item) = new.clone();
        map.insert(old, new);
        into.push(item);
        added += 1;
    }
    (map, added)
}

/// 把一个空间的 `tavern/` 并进全局库 `global`。还有文件在下载时报错，什么都不写（下次再来）。
pub fn merge_space(global: &Path, tavern: &Path) -> Result<Merged, String> {
    let mut m = Merged::default();
    let (settings_path, imagine_path) = (tavern.join("settings.json"), tavern.join("imagine.json"));
    let has_settings = ready(&settings_path)?;
    let has_imagine = ready(&imagine_path)?;
    // 预设：先看全都下载好了没有，再一份份拷。
    let space_names = if has_settings { Settings::load(tavern).names } else { BTreeMap::new() };
    let listed = library::presets(tavern, &space_names);
    if let Some(l) = listed.iter().find(|l| !l.state.ready()) { return Err(format!("预设「{}」还没下载好", l.name)); }
    let mut preset_map = BTreeMap::new();
    for l in &listed {
        let bytes = file_io::read(library::path(tavern, &l.id)).map_err(|e| e.to_string())?;
        let same = |id: &str| file_io::read(library::path(global, id)).ok().map(|b| b == bytes);
        let mut id = l.id.clone();
        let mut n = 1;
        loop {
            match same(&id) {
                Some(true) => break,
                Some(false) => { n += 1; id = format!("{}-m{n}", l.id); }
                None => { library::write(&library::path(global, &id), &bytes).map_err(|e| e.to_string())?; m.presets += 1; break; }
            }
        }
        preset_map.insert(l.id.clone(), id);
    }
    if has_settings {
        let first = file_io::metadata(global.join("settings.json")).is_err();
        let s = Settings::load(tavern);
        let mut g = Settings::load(global);
        let same = |a: &Card, b: &Card| a.name == b.name && a.description == b.description;
        let (chars, n) = merge_list(&mut g.chars, &s.chars, same, |c| &mut c.name, |c| &mut c.id);
        m.chars = n;
        let (personas, n) = merge_list(&mut g.personas, &s.personas, same, |c| &mut c.name, |c| &mut c.id);
        m.personas = n;
        for (id, name) in &s.names { g.names.entry(preset_map.get(id).cloned().unwrap_or_else(|| id.clone())).or_insert_with(|| name.clone()); }
        if first {
            g.preset = if s.preset.is_empty() { String::new() } else { preset_map.get(&s.preset).cloned().unwrap_or_default() };
            if let Some(c) = chars.get(&s.char) { g.char = c.clone(); }
            if let Some(p) = personas.get(&s.persona) { g.persona = p.clone(); }
            g.last = s.last.clone();
        }
        g.save(global).map_err(|e| e.to_string())?;
    } else if !preset_map.is_empty() {
        // 只有预设没有设置：名字也记一下，免得另一台设备上没下载时列表里只有标识。
        let mut g = Settings::load(global);
        for (old, new) in &preset_map { if let Some(n) = space_names.get(old) { g.names.entry(new.clone()).or_insert_with(|| n.clone()); } }
        g.save(global).map_err(|e| e.to_string())?;
    }
    if has_imagine {
        let first = file_io::metadata(global.join("imagine.json")).is_err();
        let s = Library::load(tavern);
        let mut g = Library::load(global);
        let (styles, n) = merge_list(&mut g.styles, &s.styles, |a, b| (&a.name, &a.positive, &a.negative) == (&b.name, &b.positive, &b.negative), |x| &mut x.name, |x| &mut x.id);
        m.styles = n;
        let (_, n) = merge_list(&mut g.characters, &s.characters, |a, b| (&a.name, &a.aliases, &a.look, &a.negative, &a.avatar) == (&b.name, &b.aliases, &b.look, &b.negative, &b.avatar), |x| &mut x.name, |x| &mut x.id);
        m.characters = n;
        if first {
            // 内置画风的标识不在列表里，原样留着。
            g.style = styles.get(&s.style).cloned().unwrap_or_else(|| s.style.clone());
            (g.quality, g.quality_negative, g.optimize) = (s.quality.clone(), s.quality_negative.clone(), s.optimize);
        }
        g.save(global).map_err(|e| e.to_string())?;
    }
    Ok(m)
}

fn migrated_file(global: &Path) -> PathBuf { global.join("migrated.json") }
/// 已经并进来的空间。
pub fn migrated(global: &Path) -> BTreeSet<String> {
    file_io::read(migrated_file(global)).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| v.get("spaces").and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_owned)).collect())).unwrap_or_default()
}

/// 一个空间这次并的结果：并好了（并了多少），或者为什么这次跳过（下次再并）。
pub type Outcome = (String, Result<Merged, String>);

/// 把这些空间（标识，空间根目录）里还没并过的并进 `global`，按给的顺序。并好的记进 `migrated.json`；返回这次试过的空间。
pub fn migrate_in(global: &Path, spaces: &[(String, PathBuf)]) -> io::Result<Vec<Outcome>> {
    let mut done = migrated(global);
    let mut out = vec![];
    for (id, root) in spaces {
        if id.is_empty() || done.contains(id) { continue; }
        let r = merge_space(global, &root.join("tavern"));
        if r.is_ok() { done.insert(id.clone()); }
        out.push((id.clone(), r));
    }
    if out.iter().any(|(_, r)| r.is_ok()) {
        library::write(&migrated_file(global), &serde_json::to_vec_pretty(&json!({ "spaces": done })).map_err(io::Error::other)?)?;
    }
    Ok(out)
}

/// 把这台设备知道的空间都并进全局库（当前空间排最前，它的选择优先）。启动时调一次就行。
pub fn migrate() -> io::Result<Vec<Outcome>> {
    let current = fdl_core::spaces::current().map(|s| s.id);
    let mut spaces: Vec<_> = fdl_core::spaces::list().into_iter().filter(|s| s.available).collect();
    spaces.sort_by_key(|s| Some(&s.id) != current.as_ref());
    migrate_in(&dir(), &spaces.into_iter().map(|s| (s.id, s.root)).collect::<Vec<_>>())
}

#[cfg(test)]
mod tests;
