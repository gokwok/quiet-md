//! Foundation file coordination. Core remains independent of Objective-C.
use fdl_core::file_io::{Access, Coordinator};
use std::cell::{Cell, RefCell};
use std::ffi::{CStr, CString};
use std::io;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
#[cfg(target_os = "macos")]
use objc::{class, msg_send, sel, sel_impl};
#[cfg(target_os = "macos")]
use objc::runtime::Object;
#[cfg(target_os = "macos")]
use block::ConcreteBlock as NativeBlock;
#[cfg(target_os = "ios")]
use objc2::{class, msg_send as send};
#[cfg(target_os = "ios")]
use objc2::runtime::AnyObject as Object;
#[cfg(target_os = "ios")]
use block2::RcBlock as NativeBlock;

#[cfg(target_os = "macos")]
macro_rules! send {
    ($object:expr, $name:ident) => { msg_send![$object, $name] };
    ($object:expr, $($name:ident : $arg:expr),+) => { msg_send![$object, $($name : $arg)+] };
}

pub fn install() { fdl_core::file_io::install(Arc::new(AppleFiles)); }
struct AppleFiles;
thread_local! { static CURRENT: Cell<*mut Object> = const { Cell::new(std::ptr::null_mut()) }; }
struct Current(*mut Object);
impl Drop for Current { fn drop(&mut self) { CURRENT.with(|c| c.set(self.0)); } }

struct Owned(*mut Object);
impl Drop for Owned { fn drop(&mut self) { unsafe { let _: () = send![self.0, release]; } } }

fn url(path: &Path) -> io::Result<*mut Object> {
    let bytes = CString::new(path.as_os_str().as_bytes()).map_err(io::Error::other)?;
    unsafe {
        let value: *mut Object = send![class!(NSURL), fileURLWithFileSystemRepresentation: bytes.as_ptr(), isDirectory: false, relativeToURL: std::ptr::null_mut::<Object>()];
        if value.is_null() { Err(io::Error::other("无法创建文件 URL")) } else { Ok(value) }
    }
}

fn path(url: *mut Object) -> io::Result<PathBuf> {
    if url.is_null() { return Err(io::Error::other("文件协调没有返回 URL")); }
    unsafe {
        let value: *const std::ffi::c_char = send![url, fileSystemRepresentation];
        if value.is_null() { return Err(io::Error::other("文件 URL 无效")); }
        Ok(PathBuf::from(std::ffi::OsString::from_vec(CStr::from_ptr(value).to_bytes().to_vec())))
    }
}

fn error(error: *mut Object) -> io::Error {
    unsafe {
        let code: isize = send![error, code];
        let domain: *mut Object = send![error, domain];
        let domain: *const std::ffi::c_char = send![domain, UTF8String];
        if !domain.is_null() && CStr::from_ptr(domain).to_bytes() == b"NSPOSIXErrorDomain" { return io::Error::from_raw_os_error(code as i32); }
        let description: *mut Object = send![error, localizedDescription];
        let text: *const std::ffi::c_char = send![description, UTF8String];
        let message = if text.is_null() { "文件协调失败".into() } else { CStr::from_ptr(text).to_string_lossy().into_owned() };
        let kind = match code { 4 | 260 => io::ErrorKind::NotFound, 257 | 513 => io::ErrorKind::PermissionDenied, _ => io::ErrorKind::Other };
        io::Error::new(kind, message)
    }
}

impl Coordinator for AppleFiles {
    fn download_state(&self, path: &Path) -> io::Result<fdl_core::file_io::DownloadState> { download_state(path) }
    fn request_download(&self, path: &Path) -> io::Result<()> { request_download(path) }
    fn access(&self, request: Access<'_>, run: &mut dyn FnMut(&Path, Option<&Path>) -> io::Result<()>) -> io::Result<()> {
        unsafe {
            let pool: *mut Object = send![class!(NSAutoreleasePool), new];
            let _pool = Owned(pool);
            let previous = CURRENT.with(Cell::get);
            let _coordinator = if previous.is_null() {
                let allocated: *mut Object = send![class!(NSFileCoordinator), alloc];
                let initialized: *mut Object = send![allocated, initWithFilePresenter: std::ptr::null_mut::<Object>()];
                if initialized.is_null() { return Err(io::Error::other("无法创建文件协调器")); }
                Some(Owned(initialized))
            } else { None };
            let coordinator = _coordinator.as_ref().map(|c| c.0).unwrap_or(previous);
            let _current = Current(previous);
            CURRENT.with(|c| c.set(coordinator));
            let (first, second) = request.paths();
            let a = url(&first)?;
            let b = second.as_deref().map(url).transpose()?;
            let action = RefCell::new(run);
            let outcome = RefCell::new(None);
            let invoke = |one: *mut Object, two: Option<*mut Object>| {
                // Rust panics must never unwind through an Objective-C block.
                *outcome.borrow_mut() = Some(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let p = path(one)?;
                    let q = two.map(path).transpose()?;
                    action.borrow_mut()(&p, q.as_deref())
                })));
            };
            let mut failure: *mut Object = std::ptr::null_mut();
            if let Some(b) = b {
                let callback = NativeBlock::new(|one: *mut Object, two: *mut Object| {
                    if matches!(request, Access::Move(_, _)) {
                        let _: () = send![coordinator, itemAtURL: one, willMoveToURL: two];
                    }
                    invoke(one, Some(two));
                    if matches!(request, Access::Move(_, _)) && outcome.borrow().as_ref().is_some_and(|r| matches!(r, Ok(Ok(())))) {
                        let _: () = send![coordinator, itemAtURL: one, didMoveToURL: two];
                    }
                });
                if matches!(request, Access::Copy(_, _)) {
                    let _: () = send![coordinator, coordinateReadingItemAtURL: a, options: 0usize, writingItemAtURL: b, options: 8usize, error: &mut failure, byAccessor: &*callback];
                } else {
                    let _: () = send![coordinator, coordinateWritingItemAtURL: a, options: 2usize, writingItemAtURL: b, options: 8usize, error: &mut failure, byAccessor: &*callback];
                }
            } else {
                let callback = NativeBlock::new(|one: *mut Object| invoke(one, None));
                match request {
                    Access::Read(_) | Access::Inspect(_) => {
                        let options = if matches!(request, Access::Inspect(_)) { 4usize } else { 0usize };
                        let _: () = send![coordinator, coordinateReadingItemAtURL: a, options: options, error: &mut failure, byAccessor: &*callback];
                    }
                    _ => {
                        // Atomic rename is still an ordinary content update, not ForReplacing.
                        let options = if matches!(request, Access::Delete(_)) { 1usize } else { 0usize };
                        let _: () = send![coordinator, coordinateWritingItemAtURL: a, options: options, error: &mut failure, byAccessor: &*callback];
                    }
                }
            }
            match outcome.into_inner() {
                Some(Ok(result)) => result,
                Some(Err(panic)) => std::panic::resume_unwind(panic),
                None if !failure.is_null() => Err(error(failure)),
                None => Err(io::Error::other("系统未授予文件访问权限")),
            }
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    #[test]
    fn foundation_coordinates_a_temporary_space() {
        let root = std::env::temp_dir().join(format!("qmd-foundation-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let a = root.join("a.txt"); let b = root.join("b.txt");
        AppleFiles.access(Access::Write(&a), &mut |p, _| std::fs::write(p, "original")).unwrap();
        AppleFiles.access(Access::Read(&a), &mut |p, _| { assert_eq!(std::fs::read_to_string(p)?, "original"); Ok(()) }).unwrap();
        AppleFiles.access(Access::Move(&a, &b), &mut |p, q| std::fs::rename(p, q.unwrap())).unwrap();
        AppleFiles.access(Access::Delete(&b), &mut |p, _| std::fs::remove_file(p)).unwrap();
        assert!(!a.exists() && !b.exists());
        std::fs::remove_dir(root).unwrap();
    }
}

/// The immutable versions are read first; cleanup is a separate coordinated write.
pub struct Versions;
fn version_key(version: *mut Object) -> anyhow::Result<String> {
    use base64::Engine;
    unsafe {
        let identifier: *mut Object = send![version, persistentIdentifier];
        let mut failure: *mut Object = std::ptr::null_mut();
        // Apple promises NSCoding, not NSSecureCoding, for this identifier.
        let data: *mut Object = send![class!(NSKeyedArchiver), archivedDataWithRootObject: identifier, requiringSecureCoding: false, error: &mut failure];
        if data.is_null() {
            if !failure.is_null() { return Err(error(failure).into()); }
            anyhow::bail!("无法记录 iCloud 冲突版本标识");
        }
        let len: usize = send![data, length];
        let bytes: *const u8 = send![data, bytes];
        anyhow::ensure!(len > 0 && !bytes.is_null(), "iCloud 冲突版本标识为空");
        Ok(base64::engine::general_purpose::STANDARD.encode(std::slice::from_raw_parts(bytes, len)))
    }
}

impl fdl_core::store::ConflictVersions for Versions {
    fn versions(&self, file: &Path) -> anyhow::Result<Vec<fdl_core::store::ConflictVersion>> {
        let sources = fdl_core::file_io::with_read(file, |actual| unsafe {
            let pool: *mut Object = send![class!(NSAutoreleasePool), new]; let _pool = Owned(pool);
            let file = url(actual)?;
            let unresolved: *mut Object = send![class!(NSFileVersion), unresolvedConflictVersionsOfItemAtURL: file];
            let others: *mut Object = send![class!(NSFileVersion), otherVersionsOfItemAtURL: file];
            let mut out = vec![];
            let mut seen = std::collections::HashSet::new();
            for (versions, cleanup_only) in [(unresolved, false), (others, true)] {
                let count: usize = if versions.is_null() { 0 } else { send![versions, count] };
                for i in 0..count {
                    let version: *mut Object = send![versions, objectAtIndex: i];
                    if cleanup_only {
                        let conflict: bool = send![version, isConflict];
                        let resolved: bool = send![version, isResolved];
                        if !conflict || !resolved { continue; }
                    }
                    let key = version_key(version)?;
                    if !seen.insert(key.clone()) { continue; }
                    let source: *mut Object = send![version, URL];
                    out.push((key, path(source)?, cleanup_only));
                }
            }
            Ok(out)
        })?;
        // Do not hold a read claim on the live document while a losing
        // snapshot downloads: its editor must remain free to save.
        sources.into_iter().map(|(key, source, resolved)| {
            let bytes = if resolved { vec![] } else { fdl_core::store::read_bounded(&source, fdl_core::store::EXPORT_FILE_LIMIT)? };
            Ok(fdl_core::store::ConflictVersion { key, bytes, resolved })
        }).collect()
    }

    fn finish(&self, file: &Path, copied: &[String]) -> anyhow::Result<bool> {
        if copied.is_empty() { return Ok(false); }
        fdl_core::file_io::with_write(file, |actual| unsafe {
            let pool: *mut Object = send![class!(NSAutoreleasePool), new]; let _pool = Owned(pool);
            let file = url(actual)?;
            let versions: *mut Object = send![class!(NSFileVersion), unresolvedConflictVersionsOfItemAtURL: file];
            let count: usize = if versions.is_null() { 0 } else { send![versions, count] };
            let mut handled = vec![];
            for i in 0..count {
                let version: *mut Object = send![versions, objectAtIndex: i];
                if !copied.contains(&version_key(version)?) { return Ok(false); }
                handled.push(version);
            }
            for version in handled { let _: () = send![version, setResolved: true]; }
            let mut failure: *mut Object = std::ptr::null_mut();
            let removed: bool = send![class!(NSFileVersion), removeOtherVersionsOfItemAtURL: file, error: &mut failure];
            if !removed {
                if !failure.is_null() { return Err(error(failure).into()); }
                anyhow::bail!("冲突副本已保存，iCloud 旧版本清理尚未完成");
            }
            Ok(true)
        })
    }
}

#[link(name = "Foundation", kind = "framework")]
unsafe extern "C" {
    static NSURLIsUbiquitousItemKey: *mut Object;
    static NSURLUbiquitousItemDownloadingStatusKey: *mut Object;
    static NSURLUbiquitousItemDownloadingStatusCurrent: *mut Object;
    static NSURLUbiquitousItemIsDownloadingKey: *mut Object;
    static NSURLUbiquitousItemDownloadingErrorKey: *mut Object;
    static NSURLUbiquitousItemDownloadRequestedKey: *mut Object;
    static NSMetadataQueryUbiquitousDocumentsScope: *mut Object;
    static NSMetadataQueryAccessibleUbiquitousExternalDocumentsScope: *mut Object;
    static NSMetadataQueryDidFinishGatheringNotification: *mut Object;
    static NSMetadataQueryDidUpdateNotification: *mut Object;
    static NSMetadataItemPathKey: *mut Object;
}

fn resource(url: *mut Object, key: *mut Object) -> io::Result<*mut Object> {
    unsafe {
        let mut value: *mut Object = std::ptr::null_mut();
        let mut failure: *mut Object = std::ptr::null_mut();
        let ok: bool = send![url, getResourceValue: &mut value, forKey: key, error: &mut failure];
        if !ok && !failure.is_null() { Err(error(failure)) } else { Ok(value) }
    }
}
fn truth(value: *mut Object) -> bool { !value.is_null() && unsafe { send![value, boolValue] } }
fn nsstring(text: &str) -> io::Result<*mut Object> {
    let text = CString::new(text).map_err(io::Error::other)?;
    Ok(unsafe { send![class!(NSString), stringWithUTF8String: text.as_ptr()] })
}

fn download_state(file: &Path) -> io::Result<fdl_core::file_io::DownloadState> {
    use fdl_core::file_io::DownloadState as State;
    let result = fdl_core::file_io::with_inspect(file, |actual| unsafe {
        let pool: *mut Object = send![class!(NSAutoreleasePool), new]; let _pool = Owned(pool);
        let u = url(actual)?;
        if !truth(resource(u, NSURLIsUbiquitousItemKey)?) { return Ok(if actual.exists() { State::Ready } else { State::Missing }); }
        let status = resource(u, NSURLUbiquitousItemDownloadingStatusKey)?;
        if !status.is_null() && send![status, isEqualToString: NSURLUbiquitousItemDownloadingStatusCurrent] { return Ok(State::Ready); }
        let failure = resource(u, NSURLUbiquitousItemDownloadingErrorKey)?;
        if !failure.is_null() { return Ok(State::Failed(error(failure).to_string())); }
        Ok(if truth(resource(u, NSURLUbiquitousItemIsDownloadingKey)?) { State::Downloading } else { State::Waiting })
    });
    match result {
        Ok(s) => Ok(s),
        Err(e) if e.downcast_ref::<io::Error>().is_some_and(|e| e.kind() == io::ErrorKind::NotFound) => {
            let stub = file.file_name().map(|n| file.with_file_name(format!(".{}.icloud", n.to_string_lossy())));
            Ok(if stub.is_some_and(|p| p.exists()) { State::Waiting } else { State::Missing })
        }
        Err(e) => Err(io::Error::other(e.to_string())),
    }
}

fn request_download(file: &Path) -> io::Result<()> {
    unsafe {
        let pool: *mut Object = send![class!(NSAutoreleasePool), new]; let _pool = Owned(pool);
        let u = url(file)?;
        if !truth(resource(u, NSURLIsUbiquitousItemKey)?) || truth(resource(u, NSURLUbiquitousItemDownloadRequestedKey)?) { return Ok(()); }
        let fm: *mut Object = send![class!(NSFileManager), defaultManager];
        let mut failure: *mut Object = std::ptr::null_mut();
        let ok: bool = send![fm, startDownloadingUbiquitousItemAtURL: u, error: &mut failure];
        if ok { Ok(()) } else if !failure.is_null() { Err(error(failure)) } else { Err(io::Error::other("无法开始下载")) }
    }
}

struct Watch { root: PathBuf, query: Owned, observers: Vec<Owned>, changed: Arc<std::sync::atomic::AtomicBool> }
impl Drop for Watch {
    fn drop(&mut self) { unsafe {
        let _: () = send![self.query.0, stopQuery];
        let center: *mut Object = send![class!(NSNotificationCenter), defaultCenter];
        for observer in &self.observers { let _: () = send![center, removeObserver: observer.0]; }
    } }
}
thread_local! { static WATCH: RefCell<Option<Watch>> = const { RefCell::new(None) }; }

/// Metadata notifications cover iCloud downloads as well as remote edits.
pub fn watch(root: &Path) -> io::Result<()> {
    WATCH.with(|slot| {
        if slot.borrow().as_ref().is_some_and(|w| w.root == root) { return Ok(()); }
        *slot.borrow_mut() = None;
        unsafe {
            let pool: *mut Object = send![class!(NSAutoreleasePool), new]; let _pool = Owned(pool);
            let query: *mut Object = send![class!(NSMetadataQuery), new];
            let query = Owned(query);
            let scopes = [NSMetadataQueryUbiquitousDocumentsScope, NSMetadataQueryAccessibleUbiquitousExternalDocumentsScope];
            let scopes: *mut Object = send![class!(NSArray), arrayWithObjects: scopes.as_ptr(), count: scopes.len()];
            let _: () = send![query.0, setSearchScopes: scopes];
            let folder = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
            let args = [NSMetadataItemPathKey, nsstring(&format!("{}/", folder.display()))?];
            let args: *mut Object = send![class!(NSArray), arrayWithObjects: args.as_ptr(), count: args.len()];
            let predicate: *mut Object = send![class!(NSPredicate), predicateWithFormat: nsstring("%K BEGINSWITH %@")?, argumentArray: args];
            let _: () = send![query.0, setPredicate: predicate];
            let center: *mut Object = send![class!(NSNotificationCenter), defaultCenter];
            let changed = Arc::new(std::sync::atomic::AtomicBool::new(true));
            let mut observers = vec![];
            for notification in [NSMetadataQueryDidFinishGatheringNotification, NSMetadataQueryDidUpdateNotification] {
                let flag = changed.clone();
                let callback = NativeBlock::new(move |_: *mut Object| flag.store(true, std::sync::atomic::Ordering::Relaxed));
                #[cfg(target_os = "macos")]
                let callback = callback.copy();
                let observer: *mut Object = send![center, addObserverForName: notification, object: query.0, queue: std::ptr::null_mut::<Object>(), usingBlock: &*callback];
                let observer: *mut Object = send![observer, retain];
                observers.push(Owned(observer));
            }
            let _: bool = send![query.0, startQuery];
            *slot.borrow_mut() = Some(Watch { root: root.to_path_buf(), query, observers, changed });
        }
        Ok(())
    })
}

pub fn watch_changed() -> bool {
    WATCH.with(|w| w.borrow().as_ref().is_some_and(|w| w.changed.swap(false, std::sync::atomic::Ordering::Relaxed)))
}
