//! 空间: several independent libraries, each a folder of its own (照 Craft).
//!
//! The app's home (`Store::home_dir`, `~/Documents/Quiet.md` or
//! `QUIET_MD_HOME`) keeps the app's own files and `spaces.json`, the list of
//! spaces this device knows. A library already at the home is the first
//! space. A space describes itself in `space.json` (name, icon) inside its
//! folder, so the name travels with the folder; `spaces.json` only says
//! where the folders are and which one is open. Spaces created here live in
//! `<home>/spaces/<名字>/`; a space can also be any folder picked by hand.
use crate::store::Store;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// One space as the UI shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Space {
    pub id: String,
    pub root: PathBuf,
    /// Its own name; `None` until it is given one (the UI shows a default).
    pub name: Option<String>,
    /// The icon image inside the folder, if one was set.
    pub icon: Option<PathBuf>,
    /// The first space: it cannot be deleted (Craft's default space).
    pub primary: bool,
    /// The folder is there and holds a library; a moved or deleted folder
    /// stays listed (greyed) until it is removed.
    pub available: bool,
}

/// `space.json` inside a space's folder.
#[derive(Serialize, Deserialize, Default, Clone)]
struct SpaceFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    /// File name of the icon image, next to `space.json`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    icon: Option<String>,
    #[serde(default)]
    created: u64,
}

/// `spaces.json` in the home: this device's list.
#[derive(Serialize, Deserialize, Default, Clone)]
struct Registry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    current: Option<String>,
    #[serde(default)]
    spaces: Vec<Entry>,
}

#[derive(Serialize, Deserialize, Clone)]
struct Entry {
    id: String,
    /// Relative to the home when the folder is inside it (on iOS the
    /// sandbox moves with every install), absolute otherwise.
    path: String,
}

const REGISTRY: &str = "spaces.json";
const SPACE_FILE: &str = "space.json";
const ICON_STEM: &str = "space-icon";

/// Where the device's list and new local spaces live.
pub fn home() -> PathBuf { Store::home_dir() }

fn is_library(root: &Path) -> bool { root.join("format.json").is_file() }

fn resolve(home: &Path, path: &str) -> PathBuf {
    let p = Path::new(path);
    if path.is_empty() { home.to_path_buf() } else if p.is_absolute() { p.to_path_buf() } else { home.join(p) }
}

fn relative(home: &Path, root: &Path) -> String {
    match root.strip_prefix(home) { Ok(r) => r.to_string_lossy().into_owned(), Err(_) => root.to_string_lossy().into_owned() }
}

fn load_registry(home: &Path) -> Registry {
    std::fs::read(home.join(REGISTRY)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn save_registry(home: &Path, r: &Registry) -> Result<()> {
    std::fs::create_dir_all(home)?;
    crate::store::write_file(&home.join(REGISTRY), &serde_json::to_vec_pretty(r)?)
}

fn load_file(root: &Path) -> SpaceFile {
    std::fs::read(root.join(SPACE_FILE)).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn save_file(root: &Path, f: &SpaceFile) -> Result<()> {
    crate::store::write_file(&root.join(SPACE_FILE), &serde_json::to_vec_pretty(f)?)
}

/// The registry, with a library already at the home adopted as the first
/// space (an existing user's library: nothing in it is touched).
fn registry(home: &Path) -> Registry {
    let mut r = load_registry(home);
    if !r.spaces.iter().any(|e| e.path.is_empty()) && is_library(home) {
        r.spaces.insert(0, Entry { id: uuid::Uuid::new_v4().to_string(), path: String::new() });
        if r.current.is_none() { r.current = Some(r.spaces[0].id.clone()); }
        let _ = save_registry(home, &r);
    }
    r
}

fn space(home: &Path, e: &Entry, primary: bool) -> Space {
    let root = resolve(home, &e.path);
    let f = load_file(&root);
    Space {
        id: e.id.clone(),
        icon: f.icon.map(|i| root.join(i)).filter(|p| p.is_file()),
        name: f.name.filter(|n| !n.trim().is_empty()),
        available: is_library(&root),
        primary,
        root,
    }
}

/// Every space this device knows, in the order they were added.
pub fn list() -> Vec<Space> { list_in(&home()) }

pub fn list_in(home: &Path) -> Vec<Space> {
    let r = registry(home);
    r.spaces.iter().enumerate().map(|(i, e)| space(home, e, i == 0)).collect()
}

/// The space to open: the last one used, else the first that is there.
/// `None`: nothing to open (first launch, or every folder is gone).
pub fn current() -> Option<Space> { current_in(&home()) }

pub fn current_in(home: &Path) -> Option<Space> {
    let r = registry(home);
    let all = list_in(home);
    r.current.as_ref().and_then(|id| all.iter().find(|s| &s.id == id && s.available).cloned())
        .or_else(|| all.into_iter().find(|s| s.available))
}

pub fn set_current(id: &str) -> Result<()> { set_current_in(&home(), id) }

pub fn set_current_in(home: &Path, id: &str) -> Result<()> {
    let mut r = registry(home);
    anyhow::ensure!(r.spaces.iter().any(|e| e.id == id), "找不到这个空间");
    r.current = Some(id.to_string());
    save_registry(home, &r)
}

/// Where a new space would go: the home itself for the very first one (the
/// layout every library had before spaces), else `<home>/spaces/<name>`,
/// numbered when that name is taken.
fn new_root(home: &Path, name: &str) -> PathBuf {
    if load_registry(home).spaces.is_empty() && !is_library(home) { return home.to_path_buf(); }
    let clean: String = name.chars().map(|c| if matches!(c, '/' | '\\' | ':') || c.is_control() { '_' } else { c }).collect::<String>().trim().trim_start_matches('.').chars().take(60).collect();
    let base = if clean.is_empty() { "space".to_string() } else { clean };
    let dir = home.join("spaces");
    let mut root = dir.join(&base);
    let mut n = 2;
    while root.exists() { root = dir.join(format!("{base} {n}")); n += 1; }
    root
}

/// A new space called `name`. `folder`: a folder picked by hand, which must
/// be empty or already hold a library (then that library becomes the
/// space); otherwise a new folder in the home. The new space becomes current.
pub fn create(name: &str, folder: Option<&Path>) -> Result<Space> { create_in(&home(), name, folder, &|_| Ok(())) }

/// `create`, with `seed` filling the new folder before it is opened (the
/// first space gets the 开始指南 this way).
pub fn create_with(name: &str, folder: Option<&Path>, seed: &dyn Fn(&Path) -> Result<()>) -> Result<Space> { create_in(&home(), name, folder, seed) }

pub fn create_in(home: &Path, name: &str, folder: Option<&Path>, seed: &dyn Fn(&Path) -> Result<()>) -> Result<Space> {
    let mut r = registry(home);
    let root = match folder {
        Some(f) => {
            let f = f.to_path_buf();
            anyhow::ensure!(!r.spaces.iter().any(|e| resolve(home, &e.path) == f), "这个文件夹已经是一个空间");
            let empty = std::fs::read_dir(&f).map(|mut d| d.next().is_none()).unwrap_or(true);
            anyhow::ensure!(empty || is_library(&f), "请选择一个空文件夹，或已有的 Quiet.md 文档库");
            f
        }
        None => new_root(home, name),
    };
    std::fs::create_dir_all(&root).with_context(|| format!("无法创建 {}", root.display()))?;
    if !is_library(&root) { seed(&root)?; }
    // Opening writes `format.json` and `docs/`: from now on it is a library.
    Store::open_at(root.clone())?;
    let mut f = load_file(&root);
    let name = name.trim();
    if !name.is_empty() { f.name = Some(name.to_string()); }
    if f.created == 0 { f.created = crate::time::now(); }
    save_file(&root, &f)?;
    let id = uuid::Uuid::new_v4().to_string();
    r.spaces.push(Entry { id: id.clone(), path: relative(home, &root) });
    r.current = Some(id.clone());
    save_registry(home, &r)?;
    let primary = r.spaces.len() == 1;
    Ok(space(home, r.spaces.last().unwrap(), primary))
}

pub fn rename(id: &str, name: &str) -> Result<()> { rename_in(&home(), id, name) }

pub fn rename_in(home: &Path, id: &str, name: &str) -> Result<()> {
    let s = find(home, id)?;
    let mut f = load_file(&s.root);
    let name = name.trim();
    f.name = (!name.is_empty()).then(|| name.to_string());
    save_file(&s.root, &f)
}

/// Set the space's icon to an image (`bytes`, `ext` like `png`); `None`
/// goes back to the default.
pub fn set_icon(id: &str, image: Option<(&[u8], &str)>) -> Result<()> { set_icon_in(&home(), id, image) }

pub fn set_icon_in(home: &Path, id: &str, image: Option<(&[u8], &str)>) -> Result<()> {
    let s = find(home, id)?;
    let mut f = load_file(&s.root);
    let old = f.icon.take();
    if let Some((bytes, ext)) = image {
        // A new name each time: views cache images by path.
        let name = format!("{ICON_STEM}-{}.{}", &uuid::Uuid::new_v4().to_string()[..8], ext.to_ascii_lowercase());
        crate::store::write_file(&s.root.join(&name), bytes)?;
        f.icon = Some(name);
    }
    save_file(&s.root, &f)?;
    if let Some(old) = old.filter(|o| o.starts_with(ICON_STEM) && !o.contains('/')) { let _ = std::fs::remove_file(s.root.join(old)); }
    Ok(())
}

/// Take a space off this device's list; its folder is left as it is. The
/// first space stays.
pub fn remove(id: &str) -> Result<Space> { remove_in(&home(), id) }

pub fn remove_in(home: &Path, id: &str) -> Result<Space> {
    let s = find(home, id)?;
    anyhow::ensure!(!s.primary, "第一个空间不能移除");
    let mut r = registry(home);
    r.spaces.retain(|e| e.id != id);
    if r.current.as_deref() == Some(id) { r.current = r.spaces.first().map(|e| e.id.clone()); }
    save_registry(home, &r)?;
    Ok(s)
}

fn find(home: &Path, id: &str) -> Result<Space> {
    list_in(home).into_iter().find(|s| s.id == id).context("找不到这个空间")
}

/// Nothing to open: the first-space setup is due.
pub fn needs_setup() -> bool { current().is_none() }

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("qmd-spaces-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// An existing library at the home becomes the first space untouched:
    /// no `space.json` is written into it.
    #[test]
    fn existing_library_is_adopted() {
        let home = temp();
        Store::open_at(home.clone()).unwrap();
        let all = list_in(&home);
        assert_eq!(all.len(), 1);
        assert!(all[0].primary && all[0].available && all[0].root == home && all[0].name.is_none());
        assert!(!home.join(SPACE_FILE).exists());
        assert_eq!(current_in(&home).unwrap().id, all[0].id);
        // Adopted once: the id stays.
        assert_eq!(list_in(&home)[0].id, all[0].id);
    }

    /// Nothing at the home: setup is due, and the first space goes into the
    /// home itself; later ones into `spaces/`.
    #[test]
    fn first_space_lives_in_the_home() {
        let home = temp();
        assert!(current_in(&home).is_none());
        let first = create_in(&home, "我的空间", None, &|_| Ok(())).unwrap();
        assert_eq!(first.root, home);
        assert!(first.primary);
        assert_eq!(first.name.as_deref(), Some("我的空间"));
        let second = create_in(&home, "工作", None, &|_| Ok(())).unwrap();
        assert_eq!(second.root, home.join("spaces").join("工作"));
        assert!(!second.primary);
        assert_eq!(current_in(&home).unwrap().id, second.id);
        let third = create_in(&home, "工作", None, &|_| Ok(())).unwrap();
        assert_eq!(third.root, home.join("spaces").join("工作 2"));
        // Paths inside the home are stored relative to it.
        let reg = load_registry(&home);
        assert_eq!(reg.spaces[0].path, "");
        assert_eq!(reg.spaces[1].path, "spaces/工作");
    }

    /// A picked folder must be empty or a library; a library keeps its documents.
    #[test]
    fn picked_folders() {
        let home = temp();
        create_in(&home, "a", None, &|_| Ok(())).unwrap();
        let busy = temp();
        std::fs::write(busy.join("x.txt"), "x").unwrap();
        assert!(create_in(&home, "b", Some(&busy), &|_| Ok(())).is_err());
        let lib = temp();
        let mut store = Store::open_at(lib.clone()).unwrap();
        let id = store.create_from(&crate::model::Document::default()).unwrap();
        let s = create_in(&home, "lib", Some(&lib), &|_| Ok(())).unwrap();
        assert_eq!(s.root, lib);
        assert!(Store::open_at(lib.clone()).unwrap().list().iter().any(|e| e.id == id));
        assert!(create_in(&home, "again", Some(&lib), &|_| Ok(())).is_err());
        assert!(load_registry(&home).spaces[1].path.starts_with('/'));
    }

    /// Removing leaves the folder; a missing folder is listed as unavailable
    /// and never chosen as current.
    #[test]
    fn remove_and_missing() {
        let home = temp();
        let first = create_in(&home, "a", None, &|_| Ok(())).unwrap();
        let second = create_in(&home, "b", None, &|_| Ok(())).unwrap();
        assert!(remove_in(&home, &first.id).is_err());
        std::fs::rename(&second.root, home.join("moved")).unwrap();
        assert!(!list_in(&home)[1].available);
        assert_eq!(current_in(&home).unwrap().id, first.id);
        let gone = remove_in(&home, &second.id).unwrap();
        assert_eq!(gone.id, second.id);
        assert!(home.join("moved").join("format.json").exists());
        assert_eq!(list_in(&home).len(), 1);
    }

    #[test]
    fn rename_and_icon() {
        let home = temp();
        let s = create_in(&home, "a", None, &|_| Ok(())).unwrap();
        rename_in(&home, &s.id, "  新名字 ").unwrap();
        assert_eq!(list_in(&home)[0].name.as_deref(), Some("新名字"));
        set_icon_in(&home, &s.id, Some((b"png", "PNG"))).unwrap();
        let icon = list_in(&home)[0].icon.clone().unwrap();
        assert!(icon.extension().unwrap() == "png" && icon.starts_with(&s.root));
        set_icon_in(&home, &s.id, None).unwrap();
        assert!(list_in(&home)[0].icon.is_none() && !icon.exists());
    }
}
