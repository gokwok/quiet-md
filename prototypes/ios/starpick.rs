//! 首页「收藏」一栏「…」里的「收藏文档」：系统的半屏面板里列出文档（最近打开的在前），上面一个搜索框，
//! 点一行就收藏或取消收藏，行尾的星跟着变。照 Craft：收藏在首页、在「收藏」页里马上看得到，面板不关。
use std::cell::{Cell, RefCell};

use objc2::runtime::{AnyClass, AnyObject, NSObject};
use objc2::{ClassType, define_class, msg_send};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};

use crate::app::Workspace;
use crate::settings::{display_title, tr};

/// 面板里的一篇：id、标题、收藏着没有。
#[derive(Clone)]
struct Doc { id: String, title: String, starred: bool }
thread_local! { static DOCS: RefCell<Vec<Doc>> = const { RefCell::new(Vec::new()) }; }
// 搜索框里的字（小写），和照它筛出来的那几篇在 `DOCS` 里的位置。
thread_local! { static QUERY: RefCell<String> = const { RefCell::new(String::new()) }; }
thread_local! { static SHOWN: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) }; }
thread_local! { static SOURCE: Cell<*mut AnyObject> = const { Cell::new(std::ptr::null_mut()) }; }
// 开着的那张表（搜索时重载）；面板收起后它被放掉，这里只在面板开着时用。
thread_local! { static TABLE: Cell<*mut AnyObject> = const { Cell::new(std::ptr::null_mut()) }; }
const ROW_H: f64 = 52.;

fn refilter() {
    let q = QUERY.with(|q| q.borrow().clone());
    let shown = DOCS.with(|d| d.borrow().iter().enumerate().filter(|(_, d)| q.is_empty() || d.title.to_lowercase().contains(&q)).map(|(i, _)| i).collect());
    SHOWN.with(|s| *s.borrow_mut() = shown);
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "QuietMdStarPickSource"]
    struct Source;

    impl Source {
        #[unsafe(method(tableView:numberOfRowsInSection:))]
        fn rows(&self, _table: *mut AnyObject, _section: isize) -> isize { SHOWN.with(|s| s.borrow().len()) as isize }

        #[unsafe(method(tableView:cellForRowAtIndexPath:))]
        fn cell(&self, _table: *mut AnyObject, at: *mut AnyObject) -> *mut AnyObject {
            let row: isize = unsafe { msg_send![at, row] };
            let doc = SHOWN.with(|s| s.borrow().get(row as usize).copied()).and_then(|i| DOCS.with(|d| d.borrow().get(i).cloned()));
            unsafe { make_cell(doc.as_ref()) }
        }

        #[unsafe(method(tableView:didSelectRowAtIndexPath:))]
        fn selected(&self, table: *mut AnyObject, at: *mut AnyObject) {
            let row: isize = unsafe { msg_send![at, row] };
            let Some(i) = SHOWN.with(|s| s.borrow().get(row as usize).copied()) else { return };
            let Some((id, on)) = DOCS.with(|d| d.borrow_mut().get_mut(i).map(|d| { d.starred = !d.starred; (d.id.clone(), d.starred) })) else { return };
            super::glass::haptic();
            super::menu::dispatch(move |w, _, cx| set_starred(w, &id, on, cx));
            unsafe {
                let rows: *mut AnyObject = msg_send![AnyClass::get(c"NSArray").unwrap(), arrayWithObject: at];
                // 5 = UITableViewRowAnimationNone：只换行尾的星。
                let _: () = msg_send![table, reloadRowsAtIndexPaths: rows, withRowAnimation: 5isize];
            }
        }

        #[unsafe(method(searchBar:textDidChange:))]
        fn searched(&self, _bar: *mut AnyObject, text: *mut AnyObject) {
            let q = unsafe { ns_str(text) }.to_lowercase();
            QUERY.with(|s| *s.borrow_mut() = q);
            refilter();
            let table = TABLE.with(|t| t.get());
            if !table.is_null() { unsafe { let _: () = msg_send![table, reloadData]; } }
        }

        #[unsafe(method(searchBarSearchButtonClicked:))]
        fn search_done(&self, bar: *mut AnyObject) { unsafe { let _: bool = msg_send![bar, resignFirstResponder]; } }

        /// 往下滑列表时收起键盘。
        #[unsafe(method(scrollViewWillBeginDragging:))]
        fn dragging(&self, table: *mut AnyObject) {
            unsafe { let _: bool = msg_send![table, endEditing: true]; }
        }
    }
);

/// 收藏或取消收藏，不弹提示（面板还开着，行尾的星就是反馈）。
fn set_starred(w: &mut Workspace, id: &str, on: bool, cx: &mut gpui::Context<Workspace>) {
    let Some(e) = w.entries.iter_mut().find(|e| e.id == id) else { return };
    if (e.meta.starred > 0) == on { return; }
    if let Ok(t) = w.store.set_starred(id, on) { e.meta.starred = t; }
    cx.notify();
}

unsafe fn ns_string(s: &str) -> *mut AnyObject {
    let c = std::ffi::CString::new(s).unwrap_or_default();
    unsafe { msg_send![AnyClass::get(c"NSString").unwrap(), stringWithUTF8String: c.as_ptr()] }
}
unsafe fn ns_str(s: *mut AnyObject) -> String {
    unsafe {
        if s.is_null() { return String::new(); }
        let utf8: *const std::ffi::c_char = msg_send![s, UTF8String];
        if utf8.is_null() { String::new() } else { std::ffi::CStr::from_ptr(utf8).to_string_lossy().into_owned() }
    }
}

/// 一行：文档图标、标题，行尾的星（收藏着是实心金色，没收藏是空心的淡色）。交回自动释放的单元格。
unsafe fn make_cell(doc: Option<&Doc>) -> *mut AnyObject {
    unsafe {
        let cell: *mut AnyObject = msg_send![AnyClass::get(c"UITableViewCell").unwrap(), alloc];
        let cell: *mut AnyObject = msg_send![cell, initWithStyle: 0isize, reuseIdentifier: std::ptr::null_mut::<AnyObject>()];
        let Some(doc) = doc else { return msg_send![cell, autorelease] };
        let config: *mut AnyObject = msg_send![cell, defaultContentConfiguration];
        let _: () = msg_send![config, setText: ns_string(&doc.title)];
        let icon: *mut AnyObject = msg_send![AnyClass::get(c"UIImage").unwrap(), systemImageNamed: ns_string("doc.text")];
        let _: () = msg_send![config, setImage: icon];
        let props: *mut AnyObject = msg_send![config, imageProperties];
        let grey: *mut AnyObject = msg_send![AnyClass::get(c"UIColor").unwrap(), secondaryLabelColor];
        let _: () = msg_send![props, setTintColor: grey];
        let text: *mut AnyObject = msg_send![config, textProperties];
        let _: () = msg_send![text, setNumberOfLines: 1isize];
        let _: () = msg_send![cell, setContentConfiguration: config];
        let star: *mut AnyObject = msg_send![AnyClass::get(c"UIImage").unwrap(), systemImageNamed: ns_string(if doc.starred { "star.fill" } else { "star" })];
        let mark: *mut AnyObject = msg_send![AnyClass::get(c"UIImageView").unwrap(), alloc];
        let mark: *mut AnyObject = msg_send![mark, initWithImage: star];
        let tint = if doc.starred { super::glass::ui_color(super::star_color()) } else { msg_send![AnyClass::get(c"UIColor").unwrap(), tertiaryLabelColor] };
        let _: () = msg_send![mark, setTintColor: tint];
        let size: *mut AnyObject = msg_send![AnyClass::get(c"UIImageSymbolConfiguration").unwrap(), configurationWithPointSize: 17f64];
        let _: () = msg_send![mark, setPreferredSymbolConfiguration: size];
        let _: () = msg_send![cell, setAccessoryView: mark];
        let _: () = msg_send![mark, release];
        msg_send![cell, autorelease]
    }
}

/// 打开面板。已经开着（或还没有窗口）时不做事。
pub fn open(w: &Workspace) {
    let mut docs: Vec<(u64, Doc)> = w.entries.iter().filter(|e| e.meta.folder.is_none())
        .map(|e| (e.meta.opened.max(e.meta.modified), Doc { id: e.id.clone(), title: display_title(&e.title), starred: e.meta.starred > 0 })).collect();
    docs.sort_by(|a, b| b.0.cmp(&a.0));
    DOCS.with(|d| *d.borrow_mut() = docs.into_iter().map(|(_, d)| d).collect());
    QUERY.with(|q| q.borrow_mut().clear());
    refilter();
    unsafe {
        let root = super::files::root_controller();
        if root.is_null() { return; }
        let presented: *mut AnyObject = msg_send![root, presentedViewController];
        if !presented.is_null() { return; }
        let mut source = SOURCE.with(|s| s.get());
        if source.is_null() {
            // 表格和搜索框对代理是弱引用：这一个一直留着。
            source = msg_send![Source::class(), new];
            SOURCE.with(|s| s.set(source));
        }
        let table: *mut AnyObject = msg_send![AnyClass::get(c"UITableView").unwrap(), alloc];
        // 2 = UITableViewStyleInsetGrouped，和「自定义主页」同一套。
        let table: *mut AnyObject = msg_send![table, initWithFrame: CGRect::new(CGPoint::new(0., 0.), CGSize::new(0., 0.)), style: 2isize];
        TABLE.with(|t| t.set(table));
        let _: () = msg_send![table, setDataSource: source];
        let _: () = msg_send![table, setDelegate: source];
        let _: () = msg_send![table, setRowHeight: ROW_H];
        // 1 = UIScrollViewKeyboardDismissModeOnDrag。
        let _: () = msg_send![table, setKeyboardDismissMode: 1isize];
        let clear: *mut AnyObject = msg_send![AnyClass::get(c"UIColor").unwrap(), clearColor];
        let _: () = msg_send![table, setBackgroundColor: clear];

        // 表头：标题、一句说明（照 Craft「收藏」页那句）、搜索框。
        let head: *mut AnyObject = msg_send![AnyClass::get(c"UIView").unwrap(), alloc];
        let head: *mut AnyObject = msg_send![head, initWithFrame: CGRect::new(CGPoint::new(0., 0.), CGSize::new(400., 118.))];
        let label = |text: &str, size: f64, weight: f64, color: *mut AnyObject, y: f64, h: f64| {
            let l: *mut AnyObject = msg_send![AnyClass::get(c"UILabel").unwrap(), alloc];
            let l: *mut AnyObject = msg_send![l, initWithFrame: CGRect::new(CGPoint::new(28., y), CGSize::new(344., h))];
            let font: *mut AnyObject = msg_send![AnyClass::get(c"UIFont").unwrap(), systemFontOfSize: size, weight: weight];
            let _: () = msg_send![l, setFont: font];
            let _: () = msg_send![l, setTextColor: color];
            let _: () = msg_send![l, setText: ns_string(text)];
            // 2：宽度跟着表头。
            let _: () = msg_send![l, setAutoresizingMask: 2usize];
            let _: () = msg_send![head, addSubview: l];
            let _: () = msg_send![l, release];
        };
        let color = AnyClass::get(c"UIColor").unwrap();
        let (ink, sub): (*mut AnyObject, *mut AnyObject) = (msg_send![color, labelColor], msg_send![color, secondaryLabelColor]);
        label(tr("收藏文档", "Star documents"), 17., 0.3, ink, 22., 22.);
        label(tr("收藏你想稍后轻松查找的文档。只有你能看到你的收藏。", "Star documents you want to find easily later. Only you can see your stars."), 13., 0., sub, 46., 18.);
        let bar: *mut AnyObject = msg_send![AnyClass::get(c"UISearchBar").unwrap(), alloc];
        let bar: *mut AnyObject = msg_send![bar, initWithFrame: CGRect::new(CGPoint::new(8., 70.), CGSize::new(384., 44.))];
        // 2 = UISearchBarStyleMinimal：没有底色，透出面板的玻璃。
        let _: () = msg_send![bar, setSearchBarStyle: 2usize];
        let _: () = msg_send![bar, setPlaceholder: ns_string(tr("搜索", "Search"))];
        let _: () = msg_send![bar, setDelegate: source];
        let _: () = msg_send![bar, setAutoresizingMask: 2usize];
        let _: () = msg_send![head, addSubview: bar];
        let _: () = msg_send![bar, release];
        let _: () = msg_send![table, setTableHeaderView: head];
        let _: () = msg_send![head, release];

        let controller: *mut AnyObject = msg_send![AnyClass::get(c"UIViewController").unwrap(), new];
        let _: () = msg_send![controller, setView: table];
        let _: () = msg_send![table, release];
        let _: () = msg_send![controller, setOverrideUserInterfaceStyle: if crate::theme::dark() { 2isize } else { 1isize }];
        let sheet: *mut AnyObject = msg_send![controller, sheetPresentationController];
        if !sheet.is_null() {
            let detent = AnyClass::get(c"UISheetPresentationControllerDetent").unwrap();
            let medium: *mut AnyObject = msg_send![detent, mediumDetent];
            let large: *mut AnyObject = msg_send![detent, largeDetent];
            let list: *mut AnyObject = msg_send![AnyClass::get(c"NSMutableArray").unwrap(), array];
            let _: () = msg_send![list, addObject: medium];
            let _: () = msg_send![list, addObject: large];
            let _: () = msg_send![sheet, setDetents: list];
            let _: () = msg_send![sheet, setPrefersGrabberVisible: true];
        }
        let _: () = msg_send![root, presentViewController: controller, animated: true, completion: std::ptr::null_mut::<AnyObject>()];
        let _: () = msg_send![controller, release];
    }
}
