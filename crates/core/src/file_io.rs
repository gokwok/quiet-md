//! Synchronous file access seam. Platforms coordinate access; core owns the bytes.
use anyhow::{Context, Result};
use std::cell::RefCell;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

#[derive(Clone, Copy, Debug)]
pub enum Access<'a> {
    Read(&'a Path),
    Inspect(&'a Path),
    Write(&'a Path),
    Delete(&'a Path),
    Move(&'a Path, &'a Path),
    Copy(&'a Path, &'a Path),
}

impl Access<'_> {
    pub fn paths(self) -> (PathBuf, Option<PathBuf>) {
        match self {
            Self::Read(p) | Self::Inspect(p) | Self::Write(p) | Self::Delete(p) => (p.to_path_buf(), None),
            Self::Move(a, b) | Self::Copy(a, b) => (a.to_path_buf(), Some(b.to_path_buf())),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum DownloadState { #[default] Ready, Waiting, Downloading, Missing, Failed(String) }
impl DownloadState { pub fn ready(&self) -> bool { matches!(self, Self::Ready) } }

fn local_state(path: &Path) -> io::Result<DownloadState> {
    if path.try_exists()? { return Ok(DownloadState::Ready); }
    let placeholder = path.file_name().map(|n| path.with_file_name(format!(".{}.icloud", n.to_string_lossy())));
    Ok(if placeholder.is_some_and(|p| p.is_file()) { DownloadState::Waiting } else { DownloadState::Missing })
}

/// Invoke `run` exactly once, synchronously, with the URLs granted by the OS.
/// Return an error without invoking it when access cannot be granted.
pub trait Coordinator: Send + Sync {
    fn download_state(&self, path: &Path) -> io::Result<DownloadState> { local_state(path) }
    fn request_download(&self, _: &Path) -> io::Result<()> { Ok(()) }
    fn access(&self, request: Access<'_>, run: &mut dyn FnMut(&Path, Option<&Path>) -> io::Result<()>) -> io::Result<()>;
}

static COORDINATOR: OnceLock<Arc<dyn Coordinator>> = OnceLock::new();

/// Installed before opening any space. Without it all operations use std::fs.
pub fn install(coordinator: Arc<dyn Coordinator>) -> bool { COORDINATOR.set(coordinator).is_ok() }

thread_local! {
    static ACTIVE: RefCell<Vec<(PathBuf, bool)>> = const { RefCell::new(Vec::new()) };
    #[cfg(test)]
    static TEST_COORDINATOR: RefCell<Option<Arc<dyn Coordinator>>> = const { RefCell::new(None) };
}

fn backend() -> Option<Arc<dyn Coordinator>> {
    let coordinator = COORDINATOR.get().cloned();
    #[cfg(test)]
    let coordinator = TEST_COORDINATOR.with(|c| c.borrow().clone()).or(coordinator);
    coordinator
}

pub fn download_state(path: &Path) -> DownloadState {
    let result = match backend() { Some(c) => c.download_state(path), None => local_state(path) };
    result.unwrap_or_else(|e| DownloadState::Failed(e.to_string()))
}

pub fn request_download(path: &Path) -> io::Result<()> { match backend() { Some(c) => c.request_download(path), None => Ok(()) } }

pub fn with_inspect<T>(path: &Path, action: impl FnOnce(&Path) -> Result<T>) -> Result<T> {
    coordinated(Access::Inspect(path), |p, _| action(p))
}

struct Scope(usize);
impl Drop for Scope { fn drop(&mut self) { ACTIVE.with(|a| a.borrow_mut().truncate(self.0)); } }

fn coordinated<T>(request: Access<'_>, action: impl FnOnce(&Path, Option<&Path>) -> Result<T>) -> Result<T> {
    let (first, second) = request.paths();
    let write = !matches!(request, Access::Read(_) | Access::Inspect(_));
    let held = ACTIVE.with(|a| {
        let a = a.borrow();
        a.iter().any(|(p, w)| p == &first && (!write || *w)) && second.as_ref().is_none_or(|p| a.iter().any(|(q, w)| q == p && *w))
    });
    if held { return action(&first, second.as_deref()); }
    let coordinator = backend();
    let Some(coordinator) = coordinator else { return action(&first, second.as_deref()); };
    let mut action = Some(action);
    let mut output = None;
    let coordination = coordinator.access(request, &mut |p, q| {
        let action = action.take().ok_or_else(|| io::Error::other("文件协调重复执行"))?;
        let _scope = ACTIVE.with(|a| {
            let mut a = a.borrow_mut(); let n = a.len();
            a.push((p.to_path_buf(), write));
            if let Some(q) = q { a.push((q.to_path_buf(), true)); }
            Scope(n)
        });
        let result = action(p, q);
        let ok = result.is_ok();
        output = Some(result);
        if ok { Ok(()) } else { Err(io::Error::other("文件操作失败")) }
    });
    if let Some(result) = output { let value = result?; coordination?; return Ok(value); }
    coordination?;
    anyhow::bail!("文件协调没有执行操作")
}

fn io_error(error: anyhow::Error) -> io::Error {
    match error.downcast::<io::Error>() { Ok(e) => e, Err(e) => io::Error::other(e.to_string()) }
}

/// Read/check/write within one coordinator claim (in particular, conflict checks).
pub fn with_read<T>(path: &Path, action: impl FnOnce(&Path) -> Result<T>) -> Result<T> {
    coordinated(Access::Read(path), |p, _| action(p))
}

pub fn with_write<T>(path: &Path, action: impl FnOnce(&Path) -> Result<T>) -> Result<T> {
    coordinated(Access::Write(path), |p, _| action(p))
}

pub fn read(path: impl AsRef<Path>) -> io::Result<Vec<u8>> {
    coordinated(Access::Read(path.as_ref()), |p, _| Ok(std::fs::read(p)?)).map_err(io_error)
}

pub fn read_to_string(path: impl AsRef<Path>) -> io::Result<String> {
    coordinated(Access::Read(path.as_ref()), |p, _| Ok(std::fs::read_to_string(p)?)).map_err(io_error)
}

pub fn metadata(path: impl AsRef<Path>) -> io::Result<std::fs::Metadata> {
    coordinated(Access::Inspect(path.as_ref()), |p, _| Ok(std::fs::metadata(p)?)).map_err(io_error)
}

pub fn read_dir(path: impl AsRef<Path>) -> io::Result<std::vec::IntoIter<io::Result<std::fs::DirEntry>>> {
    coordinated(Access::Inspect(path.as_ref()), |p, _| Ok(std::fs::read_dir(p)?.collect::<Vec<_>>().into_iter())).map_err(io_error)
}

pub fn remove_file(path: impl AsRef<Path>) -> io::Result<()> {
    coordinated(Access::Delete(path.as_ref()), |p, _| Ok(std::fs::remove_file(p)?)).map_err(io_error)
}

pub fn rename(from: impl AsRef<Path>, to: impl AsRef<Path>) -> io::Result<()> {
    coordinated(Access::Move(from.as_ref(), to.as_ref()), |p, q| {
        let q = q.context("协调没有返回目标路径")?;
        if q.try_exists()? { return Err(io::Error::new(io::ErrorKind::AlreadyExists, "目标已存在，未覆盖").into()); }
        Ok(std::fs::rename(p, q)?)
    }).map_err(io_error)
}

pub fn copy(from: impl AsRef<Path>, to: impl AsRef<Path>) -> io::Result<u64> {
    coordinated(Access::Copy(from.as_ref(), to.as_ref()), |p, q| Ok(std::fs::copy(p, q.context("协调没有返回目标路径")?)?)).map_err(io_error)
}

pub fn create_dir_all(path: impl AsRef<Path>) -> io::Result<()> {
    let path = path.as_ref();
    if path.is_dir() { return Ok(()); }
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) { create_dir_all(parent)?; }
    coordinated(Access::Write(path), |p, _| match std::fs::create_dir(p) {
        Ok(()) => Ok(()), Err(e) if e.kind() == io::ErrorKind::AlreadyExists && p.is_dir() => Ok(()), Err(e) => Err(e.into()),
    }).map_err(io_error)
}

#[cfg(test)]
pub(crate) fn testing<T>(coordinator: Arc<dyn Coordinator>, run: impl FnOnce() -> T) -> T {
    struct Reset(Option<Arc<dyn Coordinator>>);
    impl Drop for Reset { fn drop(&mut self) { TEST_COORDINATOR.with(|c| *c.borrow_mut() = self.0.take()); } }
    let _reset = Reset(TEST_COORDINATOR.with(|c| c.replace(Some(coordinator))));
    run()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Fake { events: Mutex<Vec<String>>, deny: Mutex<Option<PathBuf>>, remote: Mutex<Option<(PathBuf, String)>> }
    impl Coordinator for Fake {
        fn access(&self, request: Access<'_>, run: &mut dyn FnMut(&Path, Option<&Path>) -> io::Result<()>) -> io::Result<()> {
            let (p, q) = request.paths();
            self.events.lock().unwrap().push(format!("{request:?}"));
            if self.deny.lock().unwrap().as_ref() == Some(&p) { return Err(io::Error::new(io::ErrorKind::PermissionDenied, "denied")); }
            if matches!(request, Access::Write(_)) {
                let mut remote = self.remote.lock().unwrap();
                if remote.as_ref().is_some_and(|(path, _)| *path == p) {
                    let (_, bytes) = remote.take().unwrap(); std::fs::write(&p, bytes)?;
                }
            }
            run(&p, q.as_deref())
        }
    }

    #[test]
    fn store_coordinates_reads_moves_deletes_and_conflict_checks() {
        let root = std::env::temp_dir().join(format!("qmd-coordination-{}", uuid::Uuid::new_v4()));
        let fake = Arc::new(Fake::default());
        testing(fake.clone(), || {
            let mut s = Store::open_at(root.clone()).unwrap();
            let (id, mut doc) = s.create().unwrap();
            let path = root.join("docs").join(format!("{id}.xml"));
            s.load_for_edit(&id, false).unwrap();
            let remote = "<title>remote</title>\n<p>keep</p>\n";
            *fake.remote.lock().unwrap() = Some((path.clone(), remote.into()));
            doc.title = crate::model::RichText::plain("local");
            assert!(s.save(&id, &doc).is_err());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), remote);
            *fake.deny.lock().unwrap() = Some(path.clone());
            assert!(s.load(&id).is_err());
            assert!(s.save(&id, &doc).is_err());
            *fake.deny.lock().unwrap() = None;
            s.delete(&id, None).unwrap();
            s.purge(&id).unwrap();
            let events = fake.events.lock().unwrap();
            for kind in ["Read(", "Inspect(", "Write(", "Move(", "Delete("] { assert!(events.iter().any(|e| e.starts_with(kind)), "{kind}"); }
        });
        std::fs::remove_dir_all(&root).unwrap();
        std::fs::remove_dir_all(root.with_extension("local-state")).unwrap();
    }

    #[test]
    fn access_uses_the_granted_path_and_denials_do_not_run_the_action() {
        struct Redirect(PathBuf);
        impl Coordinator for Redirect {
            fn access(&self, _: Access<'_>, run: &mut dyn FnMut(&Path, Option<&Path>) -> io::Result<()>) -> io::Result<()> { run(&self.0, None) }
        }
        let path = std::env::temp_dir().join(format!("qmd-granted-{}", uuid::Uuid::new_v4()));
        std::fs::write(&path, "granted").unwrap();
        testing(Arc::new(Redirect(path.clone())), || assert_eq!(read_to_string("not-the-granted-path").unwrap(), "granted"));
        let fake = Arc::new(Fake::default());
        *fake.deny.lock().unwrap() = Some(path.clone());
        testing(fake, || assert!(with_write(&path, |_| -> Result<()> { panic!("must not run") }).is_err()));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "granted");
        std::fs::remove_file(path).unwrap();
    }
}
