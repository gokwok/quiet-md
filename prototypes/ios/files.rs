//! 系统的文件选择器（`UIDocumentPickerViewController`）。gpui-pre-ios 0.3.12 没有实现它，这里自己接。
//!
//! 选文件：让系统把选中的文件拷进沙盒再把路径交回来，之后和桌面一样按路径读。
//! 存文件：桌面是「先问路径、再往里写」，iOS 的沙盒外面写不了，所以反过来：`new_path` 先给一个
//! 沙盒临时目录里的路径，调用方照常写完后调 `export`，再由系统的面板把写好的东西存到使用者选的地方。
use std::cell::{Cell, RefCell};
use std::convert::Infallible;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

use gpui::{PathPromptOptions, Window};
use objc2::runtime::{AnyClass, AnyObject, NSObject};
use objc2::{ClassType, define_class, msg_send};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

// `UTType` 在这个框架里，别处没有引用它。
#[link(name = "UniformTypeIdentifiers", kind = "framework")]
unsafe extern "C" {}
#[link(name = "UIKit", kind = "framework")]
unsafe extern "C" { fn UIImageJPEGRepresentation(image: *mut AnyObject, quality: f64) -> *mut AnyObject; }
// 照片选择器（`PHPickerViewController`）在这个框架里。它在自己的进程里读图库，不用申请相册权限。
#[link(name = "PhotosUI", kind = "framework")]
unsafe extern "C" {}

/// 选择器的答复：和上游 `oneshot::Receiver` 一样的用法（`.await` 得到 `Result<anyhow::Result<T>, _>`），
/// 只在主线程上传递。
pub struct Reply<T>(Rc<RefCell<(Option<T>, Option<Waker>)>>);
impl<T> Reply<T> {
    pub(super) fn pending() -> Self { Reply(Rc::new(RefCell::new((None, None)))) }
    pub(super) fn ready(value: T) -> Self { Reply(Rc::new(RefCell::new((Some(value), None)))) }
    pub(super) fn handle(&self) -> Self { Reply(self.0.clone()) }
    pub(super) fn send(&self, value: T) {
        let waker = { let mut slot = self.0.borrow_mut(); slot.0 = Some(value); slot.1.take() };
        if let Some(waker) = waker { waker.wake(); }
    }
}
impl<T> Future for Reply<T> {
    type Output = Result<anyhow::Result<T>, Infallible>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut slot = self.0.borrow_mut();
        match slot.0.take() {
            Some(value) => Poll::Ready(Ok(Ok(value))),
            None => { slot.1 = Some(cx.waker().clone()); Poll::Pending }
        }
    }
}

/// 开着的那个选择器在等什么：选文件的答复（`folders` 时拿到的是原地的文件夹），或存完后要清掉的临时目录。
enum Pending { Open { reply: Reply<Option<Vec<PathBuf>>>, folders: bool }, Export(PathBuf) }

thread_local! { static HOST: Cell<*mut AnyObject> = const { Cell::new(std::ptr::null_mut()) }; }
thread_local! { static PENDING: RefCell<Option<Pending>> = const { RefCell::new(None) }; }
thread_local! { static DELEGATE: Cell<*mut AnyObject> = const { Cell::new(std::ptr::null_mut()) }; }

/// 每帧记下窗口的原生视图：选择器要从它的视图控制器上弹出来。
pub fn attach(window: &Window) {
    let Ok(handle) = HasWindowHandle::window_handle(window) else { return; };
    let RawWindowHandle::UiKit(handle) = handle.as_raw() else { return; };
    HOST.with(|h| h.set(handle.ui_view.as_ptr() as *mut AnyObject));
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "QuietMdDocumentPickerDelegate"]
    struct Delegate;

    impl Delegate {
        #[unsafe(method(documentPicker:didPickDocumentsAtURLs:))]
        fn picked(&self, _picker: *mut AnyObject, urls: *mut AnyObject) { finish(Some(urls)); }

        #[unsafe(method(documentPickerWasCancelled:))]
        fn cancelled(&self, _picker: *mut AnyObject) { finish(None); }

        /// 照片选择器关了（选了、取消或下滑收起，没选时 `results` 是空的）。它不会自己收起，要我们来。
        #[unsafe(method(picker:didFinishPicking:))]
        fn photos_picked(&self, picker: *mut AnyObject, results: *mut AnyObject) {
            unsafe {
                let _: () = msg_send![picker, dismissViewControllerAnimated: true, completion: std::ptr::null_mut::<AnyObject>()];
                let n: usize = if results.is_null() { 0 } else { msg_send![results, count] };
                if n == 0 { return deliver(None); }
                let result: *mut AnyObject = msg_send![results, objectAtIndex: 0usize];
                let provider: *mut AnyObject = msg_send![result, itemProvider];
                if provider.is_null() { return deliver(None); }
                let _ = std::fs::remove_dir_all(photo_root());
                let dir = photo_root().join(uuid::Uuid::new_v4().to_string());
                // 代理一直留着（见 `present`），后台线程上拿它的地址回主线程。
                let delegate = self as *const Delegate as usize;
                // 回调在后台线程上；系统给的临时文件在回调返回后就删，所以在这里同步拷走，再回主线程交结果。
                let loaded = block2::RcBlock::new(move |url: *mut AnyObject, _error: *mut AnyObject| {
                    let copied = (!url.is_null()).then(|| url_path(url)).flatten().and_then(|from| {
                        std::fs::create_dir_all(&dir).ok()?;
                        let ext = from.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
                        if matches!(ext.as_str(), "jpeg" | "jpg" | "png" | "gif" | "webp" | "bmp") {
                            let to = dir.join(from.file_name()?);
                            return std::fs::copy(&from, &to).ok().map(|_| to);
                        }
                        // HEIC 之类编辑器不认的格式：让系统解出来，存成 JPEG（方向也一并摆正）。
                        let image: *mut AnyObject = msg_send![AnyClass::get(c"UIImage")?, imageWithContentsOfFile: ns_string(&from.to_string_lossy())];
                        if image.is_null() { return None; }
                        let data = UIImageJPEGRepresentation(image, 0.9);
                        if data.is_null() { return None; }
                        let to = dir.join(from.file_stem()?).with_extension("jpg");
                        let written: bool = msg_send![data, writeToFile: ns_string(&to.to_string_lossy()), atomically: true];
                        written.then_some(to)
                    });
                    let arg = ns_string(&copied.map(|p| p.to_string_lossy().into_owned()).unwrap_or_default());
                    let _: () = msg_send![delegate as *mut AnyObject, performSelectorOnMainThread: objc2::sel!(photoLoaded:), withObject: arg, waitUntilDone: false];
                });
                let _: *mut AnyObject = msg_send![provider, loadFileRepresentationForTypeIdentifier: ns_string("public.image"), completionHandler: &*loaded];
            }
        }

        /// 选中的照片拷好了（主线程）。空串是没拿到。
        #[unsafe(method(photoLoaded:))]
        fn photo_loaded(&self, path: *mut AnyObject) {
            let path = unsafe { ns_path(path) }.filter(|p| !p.as_os_str().is_empty());
            deliver(path.map(|p| vec![p]));
        }
    }
);

fn photo_root() -> PathBuf { std::env::temp_dir().join("quiet-md-photos") }

unsafe fn ns_path(string: *mut AnyObject) -> Option<PathBuf> {
    unsafe {
        if string.is_null() { return None; }
        let utf8: *const std::ffi::c_char = msg_send![string, UTF8String];
        (!utf8.is_null()).then(|| PathBuf::from(std::ffi::CStr::from_ptr(utf8).to_string_lossy().into_owned()))
    }
}
unsafe fn url_path(url: *mut AnyObject) -> Option<PathBuf> { unsafe { let path: *mut AnyObject = msg_send![url, path]; ns_path(path) } }

/// 照片选择器或「从哪里选」的那一问有了结果：交给等着选文件的那一处。
fn deliver(paths: Option<Vec<PathBuf>>) {
    match PENDING.with(|p| p.borrow_mut().take()) {
        Some(Pending::Open { reply, .. }) => reply.send(paths.filter(|p| !p.is_empty())),
        other => PENDING.with(|p| *p.borrow_mut() = other),
    }
}

/// 选择器关了：选了就把路径交回去，取消交回 `None`（调用方的 future 不能一直挂着）。
fn finish(urls: Option<*mut AnyObject>) {
    match PENDING.with(|p| p.borrow_mut().take()) {
        Some(Pending::Open { reply, folders }) => {
            let paths = urls.map(|urls| unsafe {
                let n: usize = msg_send![urls, count];
                (0..n).filter_map(|i| {
                    let url: *mut AnyObject = msg_send![urls, objectAtIndex: i];
                    // 文件夹没有拷进沙盒，要先向系统要到访问权；之后一直留着，导入是异步读的。
                    if folders {
                        let _: bool = msg_send![url, startAccessingSecurityScopedResource];
                        let retained: *mut AnyObject = msg_send![url, retain];
                        FOLDER_SCOPES.with(|s| s.borrow_mut().push(retained));
                        if let Some(path) = url_path(url) { FOLDER_BOOKMARKS.with(|b| b.borrow_mut().insert(path, capture_folder(url).map_err(|e| e.to_string()))); }
                    }
                    let path: *mut AnyObject = msg_send![url, path];
                    if path.is_null() { return None; }
                    let utf8: *const std::ffi::c_char = msg_send![path, UTF8String];
                    (!utf8.is_null()).then(|| PathBuf::from(std::ffi::CStr::from_ptr(utf8).to_string_lossy().into_owned()))
                }).collect::<Vec<_>>()
            });
            reply.send(paths.filter(|p| !p.is_empty()));
        }
        // 系统已经把文件拷走（或使用者取消了），临时的那份不留。
        Some(Pending::Export(dir)) => { let _ = std::fs::remove_dir_all(dir); }
        None => {}
    }
}

/// 窗口的根视图控制器：选择器从它上面弹出来。
pub(super) unsafe fn root_controller() -> *mut AnyObject {
    unsafe {
        let host = HOST.with(|h| h.get());
        if host.is_null() { return std::ptr::null_mut(); }
        let window: *mut AnyObject = msg_send![host, window];
        if window.is_null() { return std::ptr::null_mut(); }
        msg_send![window, rootViewController]
    }
}

/// 还有没有选择器开着。面板要是没有回调代理就关了（系统没有保证每条关闭的路都回调），
/// 这里发现它已经不在，就当作取消：否则之后每次选文件、存文件都会被当成「已经开着一个」。
fn busy() -> bool {
    if PENDING.with(|p| p.borrow().is_none()) { return false; }
    let shown = unsafe {
        let controller = root_controller();
        !controller.is_null() && { let presented: *mut AnyObject = msg_send![controller, presentedViewController]; !presented.is_null() }
    };
    if !shown { finish(None); }
    shown
}

/// 把选择器弹出来；之后它由弹出它的控制器持有，这里交出 `alloc` 得到的那一份引用。还没有窗口时返回假。
unsafe fn present(picker: *mut AnyObject) -> bool {
    unsafe {
        if picker.is_null() { return false; }
        let controller = root_controller();
        if controller.is_null() { let _: () = msg_send![picker, release]; return false; }
        let mut delegate = DELEGATE.with(|d| d.get());
        if delegate.is_null() {
            // 选择器对代理是弱引用：这一个一直留着。
            delegate = msg_send![Delegate::class(), new];
            DELEGATE.with(|d| d.set(delegate));
        }
        let _: () = msg_send![picker, setDelegate: delegate];
        // 深浅色跟应用的设置，不跟系统。
        let _: () = msg_send![picker, setOverrideUserInterfaceStyle: if crate::theme::dark() { 2isize } else { 1isize }];
        let _: () = msg_send![controller, presentViewController: picker, animated: true, completion: std::ptr::null_mut::<AnyObject>()];
        let _: () = msg_send![picker, release];
        true
    }
}

unsafe fn ns_string(s: &str) -> *mut AnyObject {
    let c = std::ffi::CString::new(s).unwrap_or_default();
    unsafe { msg_send![AnyClass::get(c"NSString").unwrap(), stringWithUTF8String: c.as_ptr()] }
}

/// 把文件选择器弹出来（`kind` 是让选的那一类）。
unsafe fn present_documents(kind: &str, multiple: bool, copy: bool) -> bool {
    unsafe {
        let (Some(picker), Some(utype)) = (AnyClass::get(c"UIDocumentPickerViewController"), AnyClass::get(c"UTType")) else { return false; };
        let kind: *mut AnyObject = msg_send![utype, typeWithIdentifier: ns_string(kind)];
        if kind.is_null() { return false; }
        let kinds: *mut AnyObject = msg_send![AnyClass::get(c"NSMutableArray").unwrap(), array];
        let _: () = msg_send![kinds, addObject: kind];
        let picker: *mut AnyObject = msg_send![picker, alloc];
        let picker: *mut AnyObject = msg_send![picker, initForOpeningContentTypes: kinds, asCopy: copy];
        if !picker.is_null() { let _: () = msg_send![picker, setAllowsMultipleSelection: multiple]; }
        present(picker)
    }
}

/// 把照片选择器弹出来：只列图片，选一张。`compatible`：系统能转的先转成通用的格式；仍是 HEIC 的在拿到后再转（见 `photos_picked`）。
unsafe fn present_photos() -> bool {
    unsafe {
        let (Some(config), Some(picker), Some(filter)) = (AnyClass::get(c"PHPickerConfiguration"), AnyClass::get(c"PHPickerViewController"), AnyClass::get(c"PHPickerFilter")) else { return false; };
        let config: *mut AnyObject = msg_send![config, alloc];
        let config: *mut AnyObject = msg_send![config, init];
        if config.is_null() { return false; }
        let images: *mut AnyObject = msg_send![filter, imagesFilter];
        let _: () = msg_send![config, setFilter: images];
        let _: () = msg_send![config, setPreferredAssetRepresentationMode: 1isize];
        let picker: *mut AnyObject = msg_send![picker, alloc];
        let picker: *mut AnyObject = msg_send![picker, initWithConfiguration: config];
        let _: () = msg_send![config, release];
        present(picker)
    }
}

/// 选文件或文件夹。文件由系统拷进沙盒的临时目录；文件夹留在原地读。
pub fn open(options: PathPromptOptions) -> Reply<Option<Vec<PathBuf>>> {
    if busy() { return Reply::ready(None); }
    let folders = options.directories && !options.files;
    if !unsafe { present_documents(if folders { "public.folder" } else { "public.item" }, options.multiple, !folders) } { return Reply::ready(None); }
    let reply = Reply::pending();
    PENDING.with(|p| *p.borrow_mut() = Some(Pending::Open { reply: reply.handle(), folders }));
    reply
}

/// 选一张图片（插入图片、上传封面）：先问从哪里选，「照片图库」是系统的照片选择器，「文件」是只列图片的文件选择器。
pub fn open_image() -> Reply<Option<Vec<PathBuf>>> {
    if busy() { return Reply::ready(None); }
    let tr = crate::settings::tr;
    let shown = unsafe {
        let controller = root_controller();
        let (Some(alert), Some(action)) = (AnyClass::get(c"UIAlertController"), AnyClass::get(c"UIAlertAction")) else { return Reply::ready(None); };
        if controller.is_null() { return Reply::ready(None); }
        let none = std::ptr::null_mut::<AnyObject>();
        let alert: *mut AnyObject = msg_send![alert, alertControllerWithTitle: none, message: none, preferredStyle: 0isize];
        let add = |title: &str, style: isize, run: fn() -> bool| {
            // 这一问收起后才轮到这里：弹不出下一个面板（或点的是取消）就当作没选。
            let handler = block2::RcBlock::new(move |_action: *mut AnyObject| { if !run() { deliver(None); } });
            let item: *mut AnyObject = msg_send![action, actionWithTitle: ns_string(title), style: style, handler: &*handler];
            let _: () = msg_send![alert, addAction: item];
        };
        add(tr("照片图库", "Photo Library"), 0, || present_photos());
        add(tr("文件", "Files"), 0, || present_documents("public.image", false, true));
        add(tr("取消", "Cancel"), 1, || false);
        let _: () = msg_send![alert, setOverrideUserInterfaceStyle: if crate::theme::dark() { 2isize } else { 1isize }];
        // 宽屏上动作面板是气泡，要有个出处：窗口正中。
        let bubble: *mut AnyObject = msg_send![alert, popoverPresentationController];
        let host = HOST.with(|h| h.get());
        if !bubble.is_null() && !host.is_null() {
            let bounds: objc2_core_foundation::CGRect = msg_send![host, bounds];
            let middle = objc2_core_foundation::CGRect::new(objc2_core_foundation::CGPoint::new(bounds.size.width / 2., bounds.size.height / 2.), objc2_core_foundation::CGSize::new(1., 1.));
            let _: () = msg_send![bubble, setSourceView: host];
            let _: () = msg_send![bubble, setSourceRect: middle];
            let _: () = msg_send![bubble, setPermittedArrowDirections: 0usize];
        }
        let _: () = msg_send![controller, presentViewController: alert, animated: true, completion: none];
        true
    };
    if !shown { return Reply::ready(None); }
    let reply = Reply::pending();
    PENDING.with(|p| *p.borrow_mut() = Some(Pending::Open { reply: reply.handle(), folders: false }));
    reply
}

fn export_root() -> PathBuf { std::env::temp_dir().join("quiet-md-export") }

/// 存文件的第一步：沙盒临时目录里的一个路径，文件名照 `name`。调用方写完后调 `export`。
pub fn new_path(name: Option<&str>) -> Reply<Option<PathBuf>> {
    let name = name.map(|n| n.replace(['/', ':', '\\'], "_")).filter(|n| !n.is_empty()).unwrap_or_else(|| "untitled".into());
    // 上次开着面板时应用被结束，会留下没清的那份。
    if !busy() { let _ = std::fs::remove_dir_all(export_root()); }
    let dir = export_root().join(uuid::Uuid::new_v4().to_string());
    Reply::ready(std::fs::create_dir_all(&dir).ok().map(|_| dir.join(name)))
}

/// 存文件的第二步：`path` 所在的那个临时目录里写好的东西（文档，加上它旁边的附件目录），
/// 交给系统的面板存到使用者选的地方。不是 `new_path` 给出的路径不管。
pub fn export(path: &Path) {
    let Some(dir) = path.parent().filter(|d| d.parent() == Some(export_root().as_path())) else { return; };
    let entries: Vec<PathBuf> = std::fs::read_dir(dir).map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).collect()).unwrap_or_default();
    if entries.is_empty() || busy() { let _ = std::fs::remove_dir_all(dir); return; }
    let shown = unsafe {
        let Some(picker) = AnyClass::get(c"UIDocumentPickerViewController") else { return; };
        let urls: *mut AnyObject = msg_send![AnyClass::get(c"NSMutableArray").unwrap(), array];
        for entry in &entries {
            let url: *mut AnyObject = msg_send![AnyClass::get(c"NSURL").unwrap(), fileURLWithPath: ns_string(&entry.to_string_lossy())];
            if !url.is_null() { let _: () = msg_send![urls, addObject: url]; }
        }
        let picker: *mut AnyObject = msg_send![picker, alloc];
        let picker: *mut AnyObject = msg_send![picker, initForExportingURLs: urls, asCopy: true];
        present(picker)
    };
    if shown { PENDING.with(|p| *p.borrow_mut() = Some(Pending::Export(dir.to_path_buf()))); } else { let _ = std::fs::remove_dir_all(dir); }
}

// The picker grants folder access for this process; bookmarks restore it next launch.
thread_local! {
    static FOLDER_BOOKMARKS: RefCell<std::collections::HashMap<PathBuf, Result<String, String>>> = RefCell::default();
    static FOLDER_SCOPES: RefCell<Vec<*mut AnyObject>> = const { RefCell::new(Vec::new()) };
}
fn bookmark_file() -> PathBuf {
    std::env::var_os("QUIET_MD_LOCAL_DATA").map(PathBuf::from)
        .unwrap_or_else(|| dirs::data_local_dir().unwrap_or_else(std::env::temp_dir).join("quiet-md")).join("space-bookmarks.json")
}

unsafe fn capture_folder(url: *mut AnyObject) -> anyhow::Result<String> {
    use base64::Engine;
    unsafe {
        let mut failure: *mut AnyObject = std::ptr::null_mut();
        let data: *mut AnyObject = msg_send![url, bookmarkDataWithOptions: 0usize, includingResourceValuesForKeys: std::ptr::null_mut::<AnyObject>(), relativeToURL: std::ptr::null_mut::<AnyObject>(), error: &mut failure];
        anyhow::ensure!(!data.is_null(), "无法保存文件夹访问权限，请重新选择文件夹");
        let len: usize = msg_send![data, length]; let bytes: *const u8 = msg_send![data, bytes];
        anyhow::ensure!(!bytes.is_null(), "文件夹书签为空");
        Ok(base64::engine::general_purpose::STANDARD.encode(std::slice::from_raw_parts(bytes, len)))
    }
}

pub fn remember_space(id: &str, root: &Path) -> anyhow::Result<()> {
    let Some(bookmark) = FOLDER_BOOKMARKS.with(|b| b.borrow().get(root).cloned()) else { return Ok(()); };
    let bookmark = bookmark.map_err(anyhow::Error::msg)?;
    let file = bookmark_file();
    std::fs::create_dir_all(file.parent().unwrap())?;
    let mut all: std::collections::HashMap<String, String> = match std::fs::read(&file) {
        Ok(bytes) => serde_json::from_slice(&bytes)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Default::default(),
        Err(e) => return Err(e.into()),
    };
    all.insert(id.to_string(), bookmark);
    crate::store::write_file(&file, &serde_json::to_vec_pretty(&all)?)
}

pub fn restore_space_bookmarks() {
    use base64::Engine;
    let file = bookmark_file();
    let Some(mut all) = std::fs::read(&file).ok().and_then(|b| serde_json::from_slice::<std::collections::HashMap<String, String>>(&b).ok()) else { return; };
    let registered: std::collections::HashSet<_> = fdl_core::spaces::list().into_iter().map(|s| s.id).collect();
    let mut changed = false;
    for (id, bookmark) in &mut all {
        if !registered.contains(id) { continue; }
        let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(&*bookmark) else { continue; };
        unsafe {
            let data: *mut AnyObject = msg_send![objc2::class!(NSData), dataWithBytes: bytes.as_ptr(), length: bytes.len()];
            let mut stale = objc2::runtime::Bool::NO; let mut failure: *mut AnyObject = std::ptr::null_mut();
            let url: *mut AnyObject = msg_send![objc2::class!(NSURL), URLByResolvingBookmarkData: data, options: 256usize, relativeToURL: std::ptr::null_mut::<AnyObject>(), bookmarkDataIsStale: &mut stale, error: &mut failure];
            if url.is_null() { continue; }
            let _: bool = msg_send![url, startAccessingSecurityScopedResource];
            let retained: *mut AnyObject = msg_send![url, retain];
            FOLDER_SCOPES.with(|s| s.borrow_mut().push(retained));
            if let Some(path) = url_path(url) { let _ = fdl_core::spaces::relocate(id, &path); }
            if stale.as_bool() && let Ok(fresh) = capture_folder(url) { *bookmark = fresh; changed = true; }
        }
    }
    if changed { let _ = serde_json::to_vec_pretty(&all).map(|bytes| crate::store::write_file(&file, &bytes)); }
}
