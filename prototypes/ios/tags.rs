//! 标签（照 Craft，ScreenRecording 2026-11-23 22-15-10）。标签是正文里的行内对象（`InlineKind::Tag`，打 `#名字` 加空格变成）；
//! 文档库的索引里每篇文档带着它的标签（`Entry::tags`），这里不读文档。
//!
//! - 首页「标签」一栏（`hub.rs` 画）：固定的几个标签，点了进那个标签的页。固定哪些只属于这台设备、按空间分开（键是空间的标识），存在 `NSUserDefaults` 里。
//! - 「所有标签」页：浏览页那一套（大标题、搜索），多一排横着滑的标签；点一个只看带它的文档，大标题换成 `#名字`，再点一下取消。
//! - 「管理固定标签」「替换标签」：系统的半屏面板里一张表，上面一个搜索框（和「收藏文档」同一套）。
//! - 在文档里点一个标签：系统的动作面板，转到标签、替换标签、转为文本。
use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

use gpui::{prelude::FluentBuilder, *};
use objc2::runtime::{AnyClass, AnyObject, NSObject};
use objc2::{ClassType, define_class, msg_send};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};

use crate::app::{View, Workspace};
use crate::editor::core::Pos;
use crate::settings::tr;
use crate::store::Entry;
use crate::theme::Theme;

thread_local! { static FILTER: RefCell<Option<String>> = const { RefCell::new(None) }; }
/// 标签页现在只看哪个标签；空着是「所有标签」。
pub fn filter() -> Option<String> { FILTER.with(|f| f.borrow().clone()) }
fn set_filter(name: Option<String>) { FILTER.with(|f| *f.borrow_mut() = name); }

/// 标签这个功能开没开：就是「自定义主页」里「标签」那一栏的开关。关着时打 `#名字` 不变标签，点文档里已有的标签也没有菜单；已有的标签原样留着。
pub fn enabled() -> bool { super::customise::layout().sections.iter().any(|(s, on)| *s == super::customise::Section::Tags && *on) }

/// 这些文档里的标签和各有几篇带着，按名字排（码位，和 Craft 的顺序一样）。
fn counts<'a>(docs: impl Iterator<Item = &'a Entry>) -> Vec<(String, usize)> {
    let mut all: BTreeMap<&str, usize> = BTreeMap::new();
    for e in docs { for (name, _) in &e.tags { *all.entry(name).or_default() += 1; } }
    all.into_iter().map(|(n, c)| (n.to_string(), c)).collect()
}
/// 文档库里所有的标签。
pub fn all(w: &Workspace) -> Vec<(String, usize)> { counts(w.entries.iter().filter(|e| e.meta.folder.is_none())) }
/// 标签页列出的文档：带标签的；选了一个标签时只有带它的。
pub fn entries(w: &Workspace) -> Vec<Entry> {
    let f = filter();
    w.entries.iter().filter(|e| e.meta.folder.is_none() && match &f { Some(f) => e.tags.iter().any(|(n, _)| n == f), None => !e.tags.is_empty() }).cloned().collect()
}

/// 只看一个标签时卡片标题那一行末尾的角标：这个标签在这篇里出现几次。照 Craft 是黄底的圆角方块，黄用收藏那颗星的金色。
pub fn badge(e: &Entry) -> Option<AnyElement> {
    if !super::floating() || super::home_page() != super::HomePage::Tags { return None; }
    let f = filter()?;
    let n = e.tags.iter().find(|(name, _)| *name == f)?.1;
    Some(div().flex_none().size(px(22.)).rounded(px(7.)).bg(super::star_color()).flex().items_center().justify_center()
        .text_size(px(11.)).line_height(px(22.)).font_weight(FontWeight::SEMIBOLD).text_color(crate::theme::c(crate::theme::craft::N1000)).child(n.to_string()).into_any_element())
}

/// 进标签页：`name` 是只看哪个标签。从文档里来的先回主页。
pub fn go(name: Option<String>, w: &mut Workspace, cx: &mut Context<Workspace>) {
    // 先存盘：索引里的标签是存盘时算的，刚打的标签这样才在。
    if w.view == View::Doc { w.flush(cx); }
    set_filter(name);
    super::set_home_page(super::HomePage::Tags);
    if w.view != View::Home { w.show_view(View::Home, cx); }
    cx.notify();
}

// ---- 固定的标签

// 当前空间的键和它固定的标签。键用空间的标识，不用文档库的路径：应用的容器路径每次安装都会变。
thread_local! { static PINNED: RefCell<Option<(String, Vec<String>)>> = const { RefCell::new(None) }; }
fn key(w: &Workspace) -> String { format!("quiet-md.pinned-tags.{}", crate::space::current(w).map_or("default", |s| s.id.as_str())) }
/// 这个空间固定在首页的标签，照固定的先后。
pub fn pinned(w: &Workspace) -> Vec<String> {
    let key = key(w);
    PINNED.with(|p| {
        let mut p = p.borrow_mut();
        if p.as_ref().is_none_or(|(k, _)| *k != key) {
            let list = unsafe {
                let defaults: *mut AnyObject = msg_send![AnyClass::get(c"NSUserDefaults").unwrap(), standardUserDefaults];
                let s: *mut AnyObject = msg_send![defaults, stringForKey: ns_string(&key)];
                serde_json::from_str::<Vec<String>>(&ns_str(s)).unwrap_or_default()
            };
            *p = Some((key, list));
        }
        p.as_ref().map(|(_, l)| l.clone()).unwrap_or_default()
    })
}
/// 固定或取消固定（面板里点的；面板是 `pinned` 读过之后才开的，键就是那时的）。
fn set_pinned(name: &str, on: bool) {
    let Some((key, mut list)) = PINNED.with(|p| p.borrow().clone()) else { return };
    list.retain(|n| n != name);
    if on { list.push(name.to_string()); }
    if let Ok(json) = serde_json::to_string(&list) { unsafe {
        let defaults: *mut AnyObject = msg_send![AnyClass::get(c"NSUserDefaults").unwrap(), standardUserDefaults];
        let _: () = msg_send![defaults, setObject: ns_string(&json), forKey: ns_string(&key)];
        // 马上落盘：应用被系统结束（或重装）时，还没写出去的偏好会丢。
        let _: bool = msg_send![defaults, synchronize];
    } }
    PINNED.with(|p| *p.borrow_mut() = Some((key, list)));
}

thread_local! { static FOLDED: Cell<bool> = const { Cell::new(false) }; }
/// 首页的标签一栏折叠着没有（只在这次运行里记着）。
pub fn folded() -> bool { FOLDED.with(|f| f.get()) }
pub fn toggle_folded() { FOLDED.with(|f| f.set(!f.get())); }
/// 首页「标签」一栏右边「…」的系统菜单（照 Craft）：管理固定标签；自定义主页、折叠 / 展开标签。
pub fn hub_items() -> Vec<super::menu::Item> {
    let folded = folded();
    vec![
        super::menu::Item::new("hub-tag-pick", tr("管理固定标签", "Manage Pinned Tags"), "tag", 0),
        super::menu::Item::new("hub-customise", tr("自定义主页", "Customize Home"), "slider.horizontal.3", 1),
        super::menu::Item::new("hub-tags-fold", if folded { tr("展开标签", "Expand tags") } else { tr("折叠标签", "Collapse tags") }, if folded { "arrow.up.left.and.arrow.down.right" } else { "arrow.down.right.and.arrow.up.left" }, 1),
    ]
}

// ---- 标签页的开头

/// 大标题（「所有标签」或 `#名字`）、搜索，下面一排横着滑的标签：最前面「≡」开所有标签的列表，然后选中的那个、带「×」，后面是列出的这些文档里别的标签和篇数。
pub fn lead(w: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let f = filter();
    let listed = entries(w);
    let chip = |i: usize, name: String, count: Option<usize>, cx: &mut Context<Workspace>| {
        let on = count.is_none();
        let ink = if on { Theme::primary() } else { Theme::text() };
        div().id(("tag-chip", i)).flex_none().h(px(32.)).px(px(12.)).rounded_full().flex().items_center().gap(px(5.)).text_size(px(15.)).whitespace_nowrap()
            .bg(if on { Theme::primary().opacity(0.12) } else { Theme::fill_hover() }).active(|d| d.opacity(0.6))
            .child(div().text_color(if on { ink } else { Theme::text_caption() }).child("#"))
            .child(div().text_color(ink).child(name.clone()))
            .children(count.map(|n| div().text_size(px(13.)).text_color(Theme::text_caption()).child(n.to_string())))
            .when(on, |d| d.child(crate::ui::icon("x", 12., ink)))
            .on_click(cx.listener(move |_, _, _, cx| { super::glass::haptic(); set_filter((!on).then(|| name.clone())); cx.notify(); }))
    };
    let mut chips: Vec<AnyElement> = f.iter().map(|n| chip(0, n.clone(), None, cx).into_any_element()).collect();
    for (i, (name, n)) in counts(listed.iter()).into_iter().filter(|(n, _)| Some(n) != f.as_ref()).enumerate() { chips.push(chip(i + 1, name, Some(n), cx).into_any_element()); }
    // 最前面一颗「≡」：所有标签的列表（带篇数、可搜索），点一个只看它。
    chips.insert(0, div().id("tag-list").flex_none().size(px(32.)).rounded_full().flex().items_center().justify_center().active(|d| d.opacity(0.6))
        .child(crate::ui::icon("list", 16., Theme::text_caption()))
        .on_click(cx.listener(|w, _, _, _| { super::glass::haptic(); open_pick(w, Mode::Filter); })).into_any_element());
    div().pt(px(super::chrome_top()))
        .child(super::large_title(f.as_ref().map_or_else(|| tr("所有标签", "All Tags").to_string(), |n| format!("#{n}"))))
        .child(super::lead_search(tr("搜索", "Search"), cx))
        .child(div().id("tag-chips").mt(px(10.)).h(px(super::CAPTION_H)).w_full().overflow_x_scroll().flex().items_center()
            .child(div().flex_none().px(px(16.)).flex().items_center().gap(px(8.)).children(chips)))
        .into_any_element()
}

// ---- 在文档里点了一个标签

/// 系统的动作面板：转到标签、替换标签、转为文本。
pub fn tapped(name: String, at: Pos) {
    if !enabled() { return; }
    unsafe {
        let (Some(class), Some(item)) = (AnyClass::get(c"UIAlertController"), AnyClass::get(c"UIAlertAction")) else { return };
        let root = super::files::root_controller();
        if root.is_null() { return; }
        let busy: *mut AnyObject = msg_send![root, presentedViewController];
        if !busy.is_null() { return; }
        super::glass::haptic();
        // 0 = UIAlertControllerStyleActionSheet
        let sheet: *mut AnyObject = msg_send![class, alertControllerWithTitle: ns_string(&format!("#{name}")), message: std::ptr::null_mut::<AnyObject>(), preferredStyle: 0isize];
        let add = |title: &str, style: isize, run: Box<dyn Fn()>| {
            let handler = block2::RcBlock::new(move |_action: *mut AnyObject| run());
            let action: *mut AnyObject = msg_send![item, actionWithTitle: ns_string(title), style: style, handler: &*handler];
            let _: () = msg_send![sheet, addAction: action];
        };
        let n = name.clone();
        add(tr("转到标签", "Go to Tag"), 0, Box::new(move || { let n = n.clone(); super::menu::dispatch(move |w, _, cx| go(Some(n), w, cx)); }));
        add(tr("替换标签", "Replace Tag"), 0, Box::new(move || super::menu::dispatch(move |w, _, _| open_pick(w, Mode::Replace(at)))));
        add(tr("转为文本", "Convert to Text"), 0, Box::new(move || super::menu::dispatch(move |w, window, cx| edit(w, window, cx, move |core| { core.tag_to_text(at); }))));
        // 1 = Cancel
        add(crate::settings::text("取消"), 1, Box::new(|| {}));
        let _: () = msg_send![sheet, setOverrideUserInterfaceStyle: if crate::theme::dark() { 2isize } else { 1isize }];
        let _: () = msg_send![root, presentViewController: sheet, animated: true, completion: std::ptr::null_mut::<AnyObject>()];
    }
}

/// 改开着的文档：Core 方法自己记撤销点，这里只管之后的 `changed`。只读的文档不改。
fn edit(w: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>, f: impl FnOnce(&mut crate::editor::core::Core)) {
    if w.view != View::Doc { return; }
    if let Some(editor) = w.editor.clone() { editor.update(cx, |e, cx| if !e.read_only { f(&mut e.core); e.changed(window, cx); }); }
}

// ---- 选标签的面板

/// 面板在做什么：固定 / 取消固定（点了不关）；把文档里 `Pos` 上的标签换成选的那个；标签页只看选的那个（后两种点了就关）。
#[derive(Clone, Copy, PartialEq)]
pub enum Mode { Pin, Replace(Pos), Filter }
#[derive(Clone)]
struct Row { name: String, count: usize, on: bool }
thread_local! { static MODE: Cell<Mode> = const { Cell::new(Mode::Pin) }; }
thread_local! { static ROWS: RefCell<Vec<Row>> = const { RefCell::new(Vec::new()) }; }
// 搜索框里的字，和照它筛出来的那几行；替换时搜的字是个没有的标签名，就在最前面多一行用它。
thread_local! { static QUERY: RefCell<String> = const { RefCell::new(String::new()) }; }
thread_local! { static SHOWN: RefCell<Vec<Row>> = const { RefCell::new(Vec::new()) }; }
thread_local! { static SOURCE: Cell<*mut AnyObject> = const { Cell::new(std::ptr::null_mut()) }; }
thread_local! { static TABLE: Cell<*mut AnyObject> = const { Cell::new(std::ptr::null_mut()) }; }
const ROW_H: f64 = 48.;

fn refilter() {
    let q = QUERY.with(|q| q.borrow().clone());
    let name = q.trim().trim_start_matches('#').to_string();
    let low = name.to_lowercase();
    let mut shown: Vec<Row> = ROWS.with(|r| r.borrow().iter().filter(|r| low.is_empty() || r.name.to_lowercase().contains(&low)).cloned().collect());
    let fresh = !name.is_empty() && name.chars().all(fdl_core::model::tag_char) && !shown.iter().any(|r| r.name == name);
    if fresh && matches!(MODE.with(|m| m.get()), Mode::Replace(_)) { shown.insert(0, Row { name, count: 0, on: false }); }
    SHOWN.with(|s| *s.borrow_mut() = shown);
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "QuietMdTagPickSource"]
    struct Source;

    impl Source {
        #[unsafe(method(tableView:numberOfRowsInSection:))]
        fn rows(&self, _table: *mut AnyObject, _section: isize) -> isize { SHOWN.with(|s| s.borrow().len()) as isize }

        #[unsafe(method(tableView:cellForRowAtIndexPath:))]
        fn cell(&self, _table: *mut AnyObject, at: *mut AnyObject) -> *mut AnyObject {
            let row: isize = unsafe { msg_send![at, row] };
            let row = SHOWN.with(|s| s.borrow().get(row as usize).cloned());
            unsafe { make_cell(row.as_ref()) }
        }

        #[unsafe(method(tableView:didSelectRowAtIndexPath:))]
        fn selected(&self, table: *mut AnyObject, at: *mut AnyObject) {
            let i: isize = unsafe { msg_send![at, row] };
            let Some(row) = SHOWN.with(|s| s.borrow().get(i as usize).cloned()) else { return };
            super::glass::haptic();
            match MODE.with(|m| m.get()) {
                Mode::Pin => {
                    let on = !row.on;
                    set_pinned(&row.name, on);
                    for list in [&ROWS, &SHOWN] { list.with(|l| if let Some(r) = l.borrow_mut().iter_mut().find(|r| r.name == row.name) { r.on = on; }); }
                    super::menu::dispatch(|_, _, cx| cx.notify());
                    unsafe {
                        let rows: *mut AnyObject = msg_send![AnyClass::get(c"NSArray").unwrap(), arrayWithObject: at];
                        // 5 = UITableViewRowAnimationNone：只换行尾的勾。
                        let _: () = msg_send![table, reloadRowsAtIndexPaths: rows, withRowAnimation: 5isize];
                    }
                }
                mode => {
                    unsafe {
                        let root = super::files::root_controller();
                        if !root.is_null() { let _: () = msg_send![root, dismissViewControllerAnimated: true, completion: std::ptr::null_mut::<AnyObject>()]; }
                    }
                    match mode {
                        Mode::Replace(pos) => super::menu::dispatch(move |w, window, cx| edit(w, window, cx, move |core| core.set_tag(pos, &row.name))),
                        _ => super::menu::dispatch(move |_, _, cx| { set_filter(Some(row.name)); cx.notify(); }),
                    }
                }
            }
        }

        #[unsafe(method(searchBar:textDidChange:))]
        fn searched(&self, _bar: *mut AnyObject, text: *mut AnyObject) {
            QUERY.with(|s| *s.borrow_mut() = unsafe { ns_str(text) });
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

/// 一行：「#」、名字，右边是带它的文档篇数；固定着的行尾打勾。交回自动释放的单元格。
unsafe fn make_cell(row: Option<&Row>) -> *mut AnyObject {
    unsafe {
        let cell: *mut AnyObject = msg_send![AnyClass::get(c"UITableViewCell").unwrap(), alloc];
        let cell: *mut AnyObject = msg_send![cell, initWithStyle: 0isize, reuseIdentifier: std::ptr::null_mut::<AnyObject>()];
        let Some(row) = row else { return msg_send![cell, autorelease] };
        let config: *mut AnyObject = msg_send![AnyClass::get(c"UIListContentConfiguration").unwrap(), valueCellConfiguration];
        let _: () = msg_send![config, setText: ns_string(&row.name)];
        if row.count > 0 { let _: () = msg_send![config, setSecondaryText: ns_string(&row.count.to_string())]; }
        let icon: *mut AnyObject = msg_send![AnyClass::get(c"UIImage").unwrap(), systemImageNamed: ns_string("number")];
        let _: () = msg_send![config, setImage: icon];
        let props: *mut AnyObject = msg_send![config, imageProperties];
        let grey: *mut AnyObject = msg_send![AnyClass::get(c"UIColor").unwrap(), secondaryLabelColor];
        let _: () = msg_send![props, setTintColor: grey];
        let text: *mut AnyObject = msg_send![config, textProperties];
        let _: () = msg_send![text, setNumberOfLines: 1isize];
        let _: () = msg_send![cell, setContentConfiguration: config];
        // 3 = UITableViewCellAccessoryCheckmark
        let _: () = msg_send![cell, setAccessoryType: if row.on { 3isize } else { 0isize }];
        msg_send![cell, autorelease]
    }
}

/// 打开面板。已经弹着别的（或还没有窗口）时不做事。
pub fn open_pick(w: &Workspace, mode: Mode) {
    let pins = pinned(w);
    let mut rows: Vec<Row> = all(w).into_iter().map(|(name, count)| Row { on: mode == Mode::Pin && pins.contains(&name), name, count }).collect();
    // 固定着但文档里已经没有了的标签也列出来，不然没处取消固定。
    if mode == Mode::Pin { for name in &pins { if !rows.iter().any(|r| &r.name == name) { rows.push(Row { name: name.clone(), count: 0, on: true }); } } }
    MODE.with(|m| m.set(mode));
    ROWS.with(|r| *r.borrow_mut() = rows);
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
        // 2 = UITableViewStyleInsetGrouped，和「收藏文档」同一套。
        let table: *mut AnyObject = msg_send![table, initWithFrame: CGRect::new(CGPoint::new(0., 0.), CGSize::new(0., 0.)), style: 2isize];
        TABLE.with(|t| t.set(table));
        let _: () = msg_send![table, setDataSource: source];
        let _: () = msg_send![table, setDelegate: source];
        let _: () = msg_send![table, setRowHeight: ROW_H];
        // 1 = UIScrollViewKeyboardDismissModeOnDrag。
        let _: () = msg_send![table, setKeyboardDismissMode: 1isize];
        let clear: *mut AnyObject = msg_send![AnyClass::get(c"UIColor").unwrap(), clearColor];
        let _: () = msg_send![table, setBackgroundColor: clear];

        // 表头：标题、搜索框。
        let head: *mut AnyObject = msg_send![AnyClass::get(c"UIView").unwrap(), alloc];
        let head: *mut AnyObject = msg_send![head, initWithFrame: CGRect::new(CGPoint::new(0., 0.), CGSize::new(400., 98.))];
        let title: *mut AnyObject = msg_send![AnyClass::get(c"UILabel").unwrap(), alloc];
        let title: *mut AnyObject = msg_send![title, initWithFrame: CGRect::new(CGPoint::new(28., 22.), CGSize::new(344., 22.))];
        let font: *mut AnyObject = msg_send![AnyClass::get(c"UIFont").unwrap(), systemFontOfSize: 17f64, weight: 0.3f64];
        let _: () = msg_send![title, setFont: font];
        let ink: *mut AnyObject = msg_send![AnyClass::get(c"UIColor").unwrap(), labelColor];
        let _: () = msg_send![title, setTextColor: ink];
        let _: () = msg_send![title, setText: ns_string(match mode { Mode::Pin => tr("管理固定标签", "Manage Pinned Tags"), Mode::Replace(_) => tr("替换标签", "Replace Tag"), Mode::Filter => tr("所有标签", "All Tags") })];
        // 2：宽度跟着表头。
        let _: () = msg_send![title, setAutoresizingMask: 2usize];
        let _: () = msg_send![head, addSubview: title];
        let _: () = msg_send![title, release];
        let bar: *mut AnyObject = msg_send![AnyClass::get(c"UISearchBar").unwrap(), alloc];
        let bar: *mut AnyObject = msg_send![bar, initWithFrame: CGRect::new(CGPoint::new(8., 50.), CGSize::new(384., 44.))];
        // 2 = UISearchBarStyleMinimal：没有底色，透出面板的玻璃。
        let _: () = msg_send![bar, setSearchBarStyle: 2usize];
        let _: () = msg_send![bar, setPlaceholder: ns_string(tr("搜索标签", "Search tags"))];
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
