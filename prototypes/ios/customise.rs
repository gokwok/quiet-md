//! 「自定义主页」：首页上显示哪几栏、什么顺序（照 Craft，IMG_9115）。
//!
//! 设置只属于这台设备，存在系统的 `NSUserDefaults` 里，不进文档库（文档库会同步，格式也不该为界面偏好改）。
//! 面板是系统的 `UITableView`（插入式分组）放在系统的半屏面板里：勾选、拖动排序都是系统控件；改了马上存，再让主页重画。
use std::cell::{Cell, RefCell};

use objc2::runtime::{AnyClass, AnyObject, NSObject};
use objc2::{ClassType, define_class, msg_send};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use serde::{Deserialize, Serialize};

use crate::settings::tr;
use crate::theme::Theme;

/// 主页最上面那排卡片放什么：最近打开的、收藏的，或者不放。和下面「收藏」那一栏（一行行的列表）是两回事，各管各的。
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Quick { Recent, Starred, Off }

/// 可以排序、隐藏的几栏。以后有了标签，加在这里。
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Section { Starred, Folders }
const SECTIONS: [Section; 2] = [Section::Starred, Section::Folders];

#[derive(Clone, PartialEq, Serialize, Deserialize, Debug)]
#[serde(default)]
pub struct Layout {
    pub quick: Quick,
    /// 「所有文档」那一行。
    pub all: bool,
    /// 各栏的顺序和开关。
    pub sections: Vec<(Section, bool)>,
}
impl Default for Layout {
    fn default() -> Self { Layout { quick: Quick::Starred, all: true, sections: vec![(Section::Starred, false), (Section::Folders, true)] } }
}
impl Layout {
    /// 读进来的可能缺栏、重栏（旧版本、手改过）：每栏恰好一次，缺的补在最后、关着。
    fn normalized(mut self) -> Self {
        let mut seen = vec![];
        self.sections.retain(|(s, _)| if seen.contains(s) { false } else { seen.push(*s); true });
        for s in SECTIONS { if !seen.contains(&s) { self.sections.push((s, false)); } }
        self
    }
}

const KEY: &str = "quiet-md.home-layout";
thread_local! { static LAYOUT: RefCell<Option<Layout>> = const { RefCell::new(None) }; }

pub fn layout() -> Layout { LAYOUT.with(|l| l.borrow_mut().get_or_insert_with(load).clone()) }

fn load() -> Layout {
    unsafe {
        let defaults: *mut AnyObject = msg_send![AnyClass::get(c"NSUserDefaults").unwrap(), standardUserDefaults];
        let s: *mut AnyObject = msg_send![defaults, stringForKey: ns_string(KEY)];
        if s.is_null() { return Layout::default(); }
        let utf8: *const std::ffi::c_char = msg_send![s, UTF8String];
        if utf8.is_null() { return Layout::default(); }
        serde_json::from_slice::<Layout>(std::ffi::CStr::from_ptr(utf8).to_bytes()).map(Layout::normalized).unwrap_or_default()
    }
}

/// 改一处：存下来，让主页重画。
fn change(f: impl FnOnce(&mut Layout)) {
    let mut l = layout();
    f(&mut l);
    if let Ok(json) = serde_json::to_string(&l) { unsafe {
        let defaults: *mut AnyObject = msg_send![AnyClass::get(c"NSUserDefaults").unwrap(), standardUserDefaults];
        let _: () = msg_send![defaults, setObject: ns_string(&json), forKey: ns_string(KEY)];
    } }
    LAYOUT.with(|c| *c.borrow_mut() = Some(l));
    super::menu::dispatch(|_, _, cx| cx.notify());
}

fn quick_rows() -> [(Quick, &'static str); 3] { [(Quick::Recent, tr("最近使用", "Recent")), (Quick::Starred, tr("收藏", "Starred")), (Quick::Off, tr("无", "None"))] }
fn section_title(s: Section) -> &'static str { match s { Section::Starred => tr("收藏", "Starred"), Section::Folders => tr("文件夹", "Folders") } }

// 三组：快速访问（单选）、固定的几行（只开关）、可排序的几栏（开关 + 拖动）。
const QUICK: isize = 0;
const FIXED: isize = 1;
const ORDER: isize = 2;
const ROW_H: f64 = 52.;

#[link(name = "UIKit", kind = "framework")]
unsafe extern "C" { static UITableViewAutomaticDimension: f64; }
thread_local! { static SOURCE: Cell<*mut AnyObject> = const { Cell::new(std::ptr::null_mut()) }; }

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "QuietMdHomeLayoutSource"]
    struct Source;

    impl Source {
        #[unsafe(method(numberOfSectionsInTableView:))]
        fn sections(&self, _table: *mut AnyObject) -> isize { 3 }

        #[unsafe(method(tableView:numberOfRowsInSection:))]
        fn rows(&self, _table: *mut AnyObject, section: isize) -> isize {
            match section { QUICK => quick_rows().len() as isize, FIXED => 1, _ => SECTIONS.len() as isize }
        }

        #[unsafe(method(tableView:cellForRowAtIndexPath:))]
        fn cell(&self, _table: *mut AnyObject, at: *mut AnyObject) -> *mut AnyObject {
            let (section, row) = unsafe { index(at) };
            let l = layout();
            unsafe {
                match section {
                    QUICK => { let (q, title) = quick_rows()[row]; make_cell(title, None, l.quick == q, row + 1 == quick_rows().len()) }
                    FIXED => make_cell(tr("所有文档", "All documents"), Some(l.all), false, true),
                    _ => { let (s, on) = l.sections[row]; make_cell(section_title(s), Some(on), false, row + 1 == l.sections.len()) }
                }
            }
        }

        #[unsafe(method(tableView:canEditRowAtIndexPath:))]
        fn can_edit(&self, _table: *mut AnyObject, at: *mut AnyObject) -> bool { unsafe { index(at).0 == ORDER } }

        #[unsafe(method(tableView:canMoveRowAtIndexPath:))]
        fn can_move(&self, _table: *mut AnyObject, at: *mut AnyObject) -> bool { unsafe { index(at).0 == ORDER } }

        #[unsafe(method(tableView:moveRowAtIndexPath:toIndexPath:))]
        fn moved(&self, _table: *mut AnyObject, from: *mut AnyObject, to: *mut AnyObject) {
            let (from, to) = unsafe { (index(from).1, index(to).1) };
            change(|l| { let s = l.sections.remove(from); l.sections.insert(to.min(l.sections.len()), s); });
        }

        /// 只在自己那一组里挪。
        #[unsafe(method(tableView:targetIndexPathForMoveFromRowAtIndexPath:toProposedIndexPath:))]
        fn target(&self, _table: *mut AnyObject, from: *mut AnyObject, proposed: *mut AnyObject) -> *mut AnyObject {
            if unsafe { index(proposed).0 } == ORDER { return proposed; }
            let last = SECTIONS.len() as isize - 1;
            let row = if unsafe { index(proposed).0 } < ORDER { 0 } else { last };
            if unsafe { index(from).0 } != ORDER { return from; }
            unsafe { msg_send![AnyClass::get(c"NSIndexPath").unwrap(), indexPathForRow: row, inSection: ORDER] }
        }

        #[unsafe(method(tableView:editingStyleForRowAtIndexPath:))]
        fn style(&self, _table: *mut AnyObject, _at: *mut AnyObject) -> isize { 0 }

        #[unsafe(method(tableView:shouldIndentWhileEditingRowAtIndexPath:))]
        fn indent(&self, _table: *mut AnyObject, _at: *mut AnyObject) -> bool { false }

        #[unsafe(method(tableView:didSelectRowAtIndexPath:))]
        fn selected(&self, table: *mut AnyObject, at: *mut AnyObject) {
            let (section, row) = unsafe { index(at) };
            match section {
                QUICK => change(|l| l.quick = quick_rows()[row].0),
                FIXED => change(|l| l.all = !l.all),
                _ => change(|l| if let Some(s) = l.sections.get_mut(row) { s.1 = !s.1 }),
            }
            unsafe {
                // 整组重载：单选那组要把勾挪过去。
                let set: *mut AnyObject = msg_send![AnyClass::get(c"NSIndexSet").unwrap(), indexSetWithIndex: section as usize];
                let _: () = msg_send![table, reloadSections: set, withRowAnimation: 5isize];
            }
        }

        #[unsafe(method(tableView:viewForHeaderInSection:))]
        fn header(&self, _table: *mut AnyObject, section: isize) -> *mut AnyObject {
            let (title, note) = match section {
                QUICK => (tr("快速访问", "Quick access"), tr("轻松回到最重要的内容。", "Get back to what matters most.")),
                FIXED => (tr("自定义主页", "Customize Home"), tr("重新排序或隐藏各栏，让主页更合你的心意。", "Reorder or hide sections to make Home your own.")),
                _ => return std::ptr::null_mut(),
            };
            // 照 Craft（截屏 2026-11-14 16.46.22）：第一组的标题离面板顶边更远一些。
            unsafe { header_view(title, note, if section == QUICK { 27. } else { 10. }) }
        }

        /// 可排序那组紧跟在上一组后面，没有标题。
        #[unsafe(method(tableView:heightForHeaderInSection:))]
        fn header_h(&self, _table: *mut AnyObject, section: isize) -> f64 { if section == ORDER { 12. } else { unsafe { UITableViewAutomaticDimension } } }

        #[unsafe(method(tableView:heightForFooterInSection:))]
        fn footer_h(&self, _table: *mut AnyObject, _section: isize) -> f64 { 8. }

        #[unsafe(method(tableView:viewForFooterInSection:))]
        fn footer(&self, _table: *mut AnyObject, _section: isize) -> *mut AnyObject { std::ptr::null_mut() }
    }
);

/// 一组的标题（照 Craft，IMG_9115）：17pt 半粗的标题，下面 13pt 的说明；比单元格的字往左 8pt，靠近卡片的边。
unsafe fn header_view(title: &str, note: &str, top: f64) -> *mut AnyObject {
    unsafe {
        let view: *mut AnyObject = msg_send![AnyClass::get(c"UITableViewHeaderFooterView").unwrap(), alloc];
        let view: *mut AnyObject = msg_send![view, initWithReuseIdentifier: std::ptr::null_mut::<AnyObject>()];
        let content: *mut AnyObject = msg_send![view, contentView];
        let guide: *mut AnyObject = msg_send![content, layoutMarginsGuide];
        let label = |text: &str, size: f64, weight: f64, color: *mut AnyObject| {
            let l: *mut AnyObject = msg_send![AnyClass::get(c"UILabel").unwrap(), new];
            let font: *mut AnyObject = msg_send![AnyClass::get(c"UIFont").unwrap(), systemFontOfSize: size, weight: weight];
            let _: () = msg_send![l, setFont: font];
            let _: () = msg_send![l, setTextColor: color];
            let _: () = msg_send![l, setNumberOfLines: 0isize];
            let _: () = msg_send![l, setText: ns_string(text)];
            let _: () = msg_send![l, setTranslatesAutoresizingMaskIntoConstraints: false];
            let _: () = msg_send![content, addSubview: l];
            let _: () = msg_send![l, release];
            l
        };
        let color = AnyClass::get(c"UIColor").unwrap();
        let (ink, sub): (*mut AnyObject, *mut AnyObject) = (msg_send![color, labelColor], msg_send![color, secondaryLabelColor]);
        // 0.3 ≈ UIFontWeightSemibold。
        let t = label(title, 17., 0.3, ink);
        let n = label(note, 13., 0., sub);
        let pin = |a: *mut AnyObject, b: *mut AnyObject, constant: f64| { let c: *mut AnyObject = msg_send![a, constraintEqualToAnchor: b, constant: constant]; let _: () = msg_send![c, setActive: true]; };
        let at_most = |a: *mut AnyObject, b: *mut AnyObject| { let c: *mut AnyObject = msg_send![a, constraintLessThanOrEqualToAnchor: b]; let _: () = msg_send![c, setActive: true]; };
        for l in [t, n] {
            pin(msg_send![l, leadingAnchor], msg_send![guide, leadingAnchor], -8.);
            at_most(msg_send![l, trailingAnchor], msg_send![guide, trailingAnchor]);
        }
        pin(msg_send![t, topAnchor], msg_send![content, topAnchor], top);
        pin(msg_send![n, topAnchor], msg_send![t, bottomAnchor], 5.);
        pin(msg_send![content, bottomAnchor], msg_send![n, bottomAnchor], 12.);
        msg_send![view, autorelease]
    }
}

/// 勾选的颜色。深色照 Craft 实测（IMG_9115 勾选圈 96,192,248），浅色用主色。
unsafe fn accent() -> *mut AnyObject {
    unsafe { super::glass::ui_color(if crate::theme::dark() { crate::theme::rgbaf(96, 192, 248, 1.) } else { Theme::primary() }) }
}

unsafe fn index(at: *mut AnyObject) -> (isize, usize) {
    unsafe { let s: isize = msg_send![at, section]; let r: isize = msg_send![at, row]; (s, r.max(0) as usize) }
}

/// 一行：`toggle` 有值时行首是勾选圈（开 / 关），否则 `checked` 时行尾一个勾（单选）。交回自动释放的单元格。
unsafe fn make_cell(title: &str, toggle: Option<bool>, checked: bool, last: bool) -> *mut AnyObject {
    unsafe {
        let cell: *mut AnyObject = msg_send![AnyClass::get(c"UITableViewCell").unwrap(), alloc];
        let cell: *mut AnyObject = msg_send![cell, initWithStyle: 0isize, reuseIdentifier: std::ptr::null_mut::<AnyObject>()];
        let config: *mut AnyObject = msg_send![cell, defaultContentConfiguration];
        let _: () = msg_send![config, setText: ns_string(title)];
        if let Some(on) = toggle {
            let name = if on { "checkmark.circle.fill" } else { "circle" };
            let image: *mut AnyObject = msg_send![AnyClass::get(c"UIImage").unwrap(), systemImageNamed: ns_string(name)];
            let _: () = msg_send![config, setImage: image];
            let props: *mut AnyObject = msg_send![config, imageProperties];
            let tint = if on { accent() } else { msg_send![AnyClass::get(c"UIColor").unwrap(), tertiaryLabelColor] };
            let _: () = msg_send![props, setTintColor: tint];
            // Craft 的勾选圈约 17pt 见方。
            let size: *mut AnyObject = msg_send![AnyClass::get(c"UIImageSymbolConfiguration").unwrap(), configurationWithPointSize: 17.5f64];
            let _: () = msg_send![props, setPreferredSymbolConfiguration: size];
            // 勾选圈和字之间照 Craft 留宽一点。
            let _: () = msg_send![config, setImageToTextPadding: 22f64];
        }
        let _: () = msg_send![cell, setContentConfiguration: config];
        // 自己画行间的细线（表格的系统线关掉了）：照 Craft，很淡，从字的左边一直通到卡片右边缘（拖动把手底下也有）。
        if !last {
            let line: *mut AnyObject = msg_send![AnyClass::get(c"UIView").unwrap(), new];
            let color = super::glass::ui_color(if crate::theme::dark() { crate::theme::rgbaf(255, 255, 255, 0.09) } else { crate::theme::rgbaf(0, 0, 0, 0.06) });
            let _: () = msg_send![line, setBackgroundColor: color];
            let _: () = msg_send![line, setTranslatesAutoresizingMaskIntoConstraints: false];
            let _: () = msg_send![cell, addSubview: line];
            let _: () = msg_send![line, release];
            // 和字的左边对齐：系统内容视图（`UIListContentView`）有字的布局参照；拿不到就用单元格的边距。
            let content: *mut AnyObject = msg_send![cell, contentView];
            let text_guide: bool = msg_send![content, respondsToSelector: objc2::sel!(textLayoutGuide)];
            let guide: *mut AnyObject = if text_guide { msg_send![content, textLayoutGuide] } else { msg_send![cell, layoutMarginsGuide] };
            let screen: *mut AnyObject = msg_send![AnyClass::get(c"UIScreen").unwrap(), mainScreen];
            let scale: f64 = msg_send![screen, scale];
            let pin = |a: *mut AnyObject, b: *mut AnyObject| { let c: *mut AnyObject = msg_send![a, constraintEqualToAnchor: b]; let _: () = msg_send![c, setActive: true]; };
            pin(msg_send![line, leadingAnchor], msg_send![guide, leadingAnchor]);
            pin(msg_send![line, trailingAnchor], msg_send![cell, trailingAnchor]);
            pin(msg_send![line, bottomAnchor], msg_send![cell, bottomAnchor]);
            let height: *mut AnyObject = msg_send![line, heightAnchor];
            let h: *mut AnyObject = msg_send![height, constraintEqualToConstant: 1. / scale.max(1.)];
            let _: () = msg_send![h, setActive: true];
        }
        // 表格一直在编辑状态（好露出拖动的把手），编辑状态下显示的是 `editingAccessoryType`。
        let accessory = if checked { 3isize } else { 0isize };
        let _: () = msg_send![cell, setAccessoryType: accessory];
        let _: () = msg_send![cell, setEditingAccessoryType: accessory];
        let _: () = msg_send![cell, setTintColor: accent()];
        msg_send![cell, autorelease]
    }
}

/// 打开面板。已经开着（或还没有窗口）时不做事。
pub fn open() {
    unsafe {
        let root = super::files::root_controller();
        if root.is_null() { return; }
        let presented: *mut AnyObject = msg_send![root, presentedViewController];
        if !presented.is_null() { return; }
        let mut source = SOURCE.with(|s| s.get());
        if source.is_null() {
            // 表格对数据源和代理是弱引用：这一个一直留着。
            source = msg_send![Source::class(), new];
            SOURCE.with(|s| s.set(source));
        }
        let table: *mut AnyObject = msg_send![AnyClass::get(c"UITableView").unwrap(), alloc];
        // 2 = UITableViewStyleInsetGrouped。
        let table: *mut AnyObject = msg_send![table, initWithFrame: CGRect::new(CGPoint::new(0., 0.), CGSize::new(0., 0.)), style: 2isize];
        let _: () = msg_send![table, setDataSource: source];
        let _: () = msg_send![table, setDelegate: source];
        let _: () = msg_send![table, setRowHeight: ROW_H];
        // 0 = UITableViewCellSeparatorStyleNone：行间线自己画（`make_cell`）。
        let _: () = msg_send![table, setSeparatorStyle: 0isize];
        let _: () = msg_send![table, setEditing: true, animated: false];
        let _: () = msg_send![table, setAllowsSelectionDuringEditing: true];
        let _: () = msg_send![table, setEstimatedSectionHeaderHeight: 64f64];
        // 透出面板自己的玻璃。
        let clear: *mut AnyObject = msg_send![AnyClass::get(c"UIColor").unwrap(), clearColor];
        let _: () = msg_send![table, setBackgroundColor: clear];
        let controller: *mut AnyObject = msg_send![AnyClass::get(c"UIViewController").unwrap(), new];
        let _: () = msg_send![controller, setView: table];
        let _: () = msg_send![table, release];
        let _: () = msg_send![controller, setOverrideUserInterfaceStyle: if crate::theme::dark() { 2isize } else { 1isize }];
        let sheet: *mut AnyObject = msg_send![controller, sheetPresentationController];
        if !sheet.is_null() {
            let detent = AnyClass::get(c"UISheetPresentationControllerDetent").unwrap();
            // 先停在刚好放下全部内容的高度，往上还能拉满。
            // 两组标题、组间距、上下留白，实测合计约 186。
            let fit = 186. + (quick_rows().len() + 1 + SECTIONS.len()) as f64 * ROW_H;
            let resolver = block2::RcBlock::new(move |_context: *mut AnyObject| -> f64 { fit });
            let custom: *mut AnyObject = msg_send![detent, customDetentWithIdentifier: std::ptr::null_mut::<AnyObject>(), resolver: &*resolver];
            let large: *mut AnyObject = msg_send![detent, largeDetent];
            let list: *mut AnyObject = msg_send![AnyClass::get(c"NSMutableArray").unwrap(), array];
            let _: () = msg_send![list, addObject: custom];
            let _: () = msg_send![list, addObject: large];
            let _: () = msg_send![sheet, setDetents: list];
            let _: () = msg_send![sheet, setPrefersGrabberVisible: true];
        }
        let _: () = msg_send![root, presentViewController: controller, animated: true, completion: std::ptr::null_mut::<AnyObject>()];
        let _: () = msg_send![controller, release];
    }
}

unsafe fn ns_string(s: &str) -> *mut AnyObject {
    let c = std::ffi::CString::new(s).unwrap_or_default();
    unsafe { msg_send![AnyClass::get(c"NSString").unwrap(), stringWithUTF8String: c.as_ptr()] }
}
