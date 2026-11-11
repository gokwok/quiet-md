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

/// 选择器的答复：和上游 `oneshot::Receiver` 一样的用法（`.await` 得到 `Result<anyhow::Result<T>, _>`），
/// 只在主线程上传递。
pub struct Reply<T>(Rc<RefCell<(Option<T>, Option<Waker>)>>);
impl<T> Reply<T> {
    fn pending() -> Self { Reply(Rc::new(RefCell::new((None, None)))) }
    fn ready(value: T) -> Self { Reply(Rc::new(RefCell::new((Some(value), None)))) }
    fn handle(&self) -> Self { Reply(self.0.clone()) }
    fn send(&self, value: T) {
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
    }
);

/// 选择器关了：选了就把路径交回去，取消交回 `None`（调用方的 future 不能一直挂着）。
fn finish(urls: Option<*mut AnyObject>) {
    match PENDING.with(|p| p.borrow_mut().take()) {
        Some(Pending::Open { reply, folders }) => {
            let paths = urls.map(|urls| unsafe {
                let n: usize = msg_send![urls, count];
                (0..n).filter_map(|i| {
                    let url: *mut AnyObject = msg_send![urls, objectAtIndex: i];
                    // 文件夹没有拷进沙盒，要先向系统要到访问权；之后一直留着，导入是异步读的。
                    if folders { let _: bool = msg_send![url, startAccessingSecurityScopedResource]; }
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
unsafe fn root_controller() -> *mut AnyObject {
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

/// 选文件或文件夹。文件由系统拷进沙盒的临时目录；文件夹留在原地读。`images` 时只让选图片。
pub fn open(options: PathPromptOptions, images: bool) -> Reply<Option<Vec<PathBuf>>> {
    if busy() { return Reply::ready(None); }
    let folders = options.directories && !options.files;
    let shown = unsafe {
        let (Some(picker), Some(utype)) = (AnyClass::get(c"UIDocumentPickerViewController"), AnyClass::get(c"UTType")) else { return Reply::ready(None); };
        let kind: *mut AnyObject = msg_send![utype, typeWithIdentifier: ns_string(if folders { "public.folder" } else if images { "public.image" } else { "public.item" })];
        if kind.is_null() { return Reply::ready(None); }
        let kinds: *mut AnyObject = msg_send![AnyClass::get(c"NSMutableArray").unwrap(), array];
        let _: () = msg_send![kinds, addObject: kind];
        let picker: *mut AnyObject = msg_send![picker, alloc];
        let picker: *mut AnyObject = msg_send![picker, initForOpeningContentTypes: kinds, asCopy: !folders];
        if !picker.is_null() { let _: () = msg_send![picker, setAllowsMultipleSelection: options.multiple]; }
        present(picker)
    };
    if !shown { return Reply::ready(None); }
    let reply = Reply::pending();
    PENDING.with(|p| *p.borrow_mut() = Some(Pending::Open { reply: reply.handle(), folders }));
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
