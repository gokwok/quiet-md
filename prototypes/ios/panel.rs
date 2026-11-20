//! 原生的玻璃面板和拼它用的几样构件：样式面板（`stylepanel.rs`）的每一页都是一列 `Node`。
//!
//! 和 `card.rs` 同一条路：一张 `UIGlassEffect` 的卡片放在 `shot::overlay` 里，位置跟 GPUI 的占位（`slot`）；里面全是系统控件，
//! GPUI 画不到玻璃上面。内容用数据描述，变了就整张重排（面板很小，几十个视图）；控件点了把一个字符串交回 `stylepanel::action`，
//! 选了哪个颜色、哪一格都编在这个字符串里。尺寸照 Craft 实测（录屏 1320px 宽 = 440pt）。
use std::cell::{Cell, RefCell};

use gpui::*;
use objc2::encode::{Encoding, RefEncode};
use objc2::{ClassType, define_class, msg_send, sel};
use objc2::runtime::{AnyClass, AnyObject, NSObject};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

#[path = "style_cards.rs"]
mod style_cards;

/// 一块的填充：纯色、两色渐变（方向同 `store::Backdrop::Gradient`）、图片文件。
#[derive(Clone, PartialEq, Debug)]
pub enum Fill { Color(Hsla), Gradient(Hsla, Hsla, u8), Image(String), Tape(String) }

/// 行尾的控件。
#[derive(Clone, PartialEq, Debug)]
pub enum Trailing {
    /// 一块颜色，点了去改它。
    Chip(Fill, String),
    Switch(bool, String),
    /// 当前值和可选的几项（标题，动作）。
    Menu(String, Vec<(String, String)>),
}

/// 主面板里的一格。
#[derive(Clone, PartialEq, Debug)]
pub struct Tile { pub span: u8, pub title: String, pub art: Art, pub act: String }
#[derive(Clone, PartialEq, Debug)]
pub enum Art {
    /// 「文本颜色」：由纸色衬着当前文字色的笔尖。
    Ink { ink: Hsla, paper: Hsla },
    /// 字体：大字是字体的名字，用它自己的字体写；下面一行小字。
    Font(String, &'static str),
    /// 封面：一张缩略图在上，名字在下；没有封面时是个图标。
    Cover(Option<String>),
    /// 整格铺满（背景、纸），名字在左上角。
    Fill(Option<Fill>),
    /// 名字在左上角，右下一个图标（样式、分隔符）。
    Symbol(&'static str),
    /// 纸：整格是纸的颜色，右下角折起一个角。
    Paper { paper: Hsla, ink: Hsla },
    /// 样式：当前文档在前，其他样式露出一小段页边。
    Stack { current: Card, others: Vec<Card> },
    /// 分隔符：下面三条由细到粗的线；胶带时是一条胶带。
    Rules(Option<Fill>),
}

/// 样式的缩略页，主面板和图库共用。
#[derive(Clone, PartialEq, Debug)]
pub struct Card { pub fill: Option<Fill>, pub paper: Hsla, pub ink: Hsla, pub name: String, pub design: &'static str, pub faded: bool, pub divider: MiniDivider, pub cover: Option<String>, pub menu: Vec<MenuItem> }

/// 系统菜单里的一项（样式卡片长按、底栏「…」）：点了交出 `act`。
#[derive(Clone, PartialEq, Debug)]
pub struct MenuItem { pub title: String, pub symbol: &'static str, pub act: String, pub danger: bool }

#[derive(Clone, PartialEq, Debug)]
pub enum MiniDivider { Line, Tape(Fill), Doodle(String) }

/// 格式面板（`format.rs`）的一格：长什么样，点了交出什么，选没选中，能不能点，有没有一层浅底，占几份宽。
#[derive(Clone, PartialEq, Debug)]
pub struct Slot { pub look: Look, pub act: String, pub on: bool, pub enabled: bool, pub tile: bool, pub span: f64 }
impl Slot {
    pub fn new(look: Look, act: impl Into<String>) -> Self { Slot { look, act: act.into(), on: false, enabled: true, tile: false, span: 1. } }
    pub fn on(mut self, on: bool) -> Self { self.on = on; self }
    pub fn enabled(mut self, enabled: bool) -> Self { self.enabled = enabled; self }
    pub fn tile(mut self) -> Self { self.tile = true; self }
    pub fn span(mut self, span: f64) -> Self { self.span = span; self }
}
#[derive(Clone, PartialEq, Debug)]
pub enum Look {
    /// 字（可在前面带一个 SF Symbol），字号和字重照它预示的样子（标题大、正文小）。
    Text(String, Option<&'static str>, f64, f64),
    Symbol(&'static str),
    /// 左边一道竖线、右边字：「焦点」的样子。
    Bar(String),
    /// 彩虹圈，中间是这一块现在的颜色（没有就是底色）。
    Wheel(Option<Hsla>),
    /// 字体：大字样用它自己的字体写，下面一行小字（样张，名字，设计）。
    Font(String, String, &'static str),
}

#[derive(Clone, PartialEq, Debug)]
pub struct Strip { pub fill: Fill, pub title: String, pub picked: bool, pub act: Option<String> }

#[derive(Clone, PartialEq, Debug)]
pub struct PhotoTile { pub path: String, pub title: String, pub credit: Option<String>, pub picked: bool, pub act: String }

#[derive(Clone, PartialEq, Debug)]
pub enum Node {
    /// 主面板顶上的小横条。
    Grab,
    /// 标题栏：标题，对比度不够的警告，重置（动作），关闭（动作）。下面一条线。
    Header { title: String, warn: bool, reset: Option<String>, close: String },
    /// 一条分段：图标（SF Symbols）或文字，当前那一段是深色的胶囊。点了交出 `act` + 序号。
    /// 图标和文字都给了：选中那段图标后面跟着文字，别的段只有图标（照 Craft 的分隔符样式）。
    Segments { symbols: Vec<&'static str>, titles: Vec<String>, picked: usize, act: String },
    BackgroundModes { titles: [String; 4], picked: usize },
    /// 字体（照 Craft，截图 2026-11-20）：没选的是一颗颗圆钮，里面是用它自己的字体写的字样；选中的那颗拉长成胶囊占满余下的宽，
    /// 字样后面跟着名字，主色字、浅主色底。各项是（字样，名字，字体设计）。点了交出 `act` + 序号。
    Fonts { items: Vec<(String, String, &'static str)>, picked: Option<usize>, act: String },
    /// 色点，一排六个。点了交出 `act` + 序号；`custom` 是最后那颗彩虹圈（动作，是否选中，圈里的颜色）。
    Swatches { fills: Vec<Fill>, picked: Option<usize>, act: String, custom: Option<(String, bool, Hsla)> },
    Caption(String),
    Row { label: String, trailing: Trailing },
    /// 并排的两颗胶囊，选中的那颗字是主色。
    Choice { labels: Vec<String>, picked: usize, act: String },
    Tiles(Vec<Vec<Tile>>),
    /// 缩略图，一排三张，选中的描边。点了交出 `act` + 序号。
    Thumbs { fills: Vec<Fill>, picked: Option<usize>, act: String },
    Photos { items: Vec<PhotoTile>, library: bool },
    SearchPhotos { prompt: String, query: String },
    /// 图库底栏（照 Craft）：左边当前样式的名字和一行出处，右边一颗胶囊按钮。
    StyleFooter { name: String, note: String, action: Option<(String, &'static str)>, menu: Vec<MenuItem> },
    /// 样式的卡片。手机两列，较宽的面板三列。
    Cards { items: Vec<Card>, picked: Option<usize>, act: String },
    /// 一排带图标的方格按钮（图标，标题，动作）。
    Actions(Vec<(&'static str, String, String)>),
    /// 同样的按钮，几个平分一整行（和纸胶带的「上传」「和纸图库」）。
    Wide(Vec<(&'static str, String, String)>),
    /// 一条条胶带，下面写名字；一行 `cols` 条，选中的加蓝色描边。
    Strips { items: Vec<Strip>, cols: usize },
    /// 标题下方滚动的内容，面板高度受窗口限制。
    Scroll { nodes: Vec<Node>, max_height: f64 },
    /// 加载、搜索和出结果时保持同一高度。
    FixedScroll { nodes: Vec<Node>, height: f64 },
    /// 空着时的一句话。
    Empty(&'static str, String),
    /// 一排小色点（格式面板的颜色），一排八颗；`None` 是「无」：空圈上一道斜线。点了交出 `act` + 序号。
    Dots { fills: Vec<Option<Hsla>>, picked: Option<usize>, act: String },
    /// 一行格子（格式面板），行高 `h`；`rule`：下面一条细线。
    Cells { cells: Vec<Slot>, h: f64, rule: bool },
}

pub const PAD: f32 = 16.;
const SWATCH: f64 = 48.;
const PITCH: f64 = 64.;
const TILE: f64 = 125.;
const GAP: f64 = 8.;
/// 字体圆钮的直径和间距（Craft 截图量的：60px@1.6 ≈ 37pt，间距约 10pt）。
const FONT_DOT: f64 = 38.;
const FONT_GAP: f64 = 10.;
// 字体那一行上一次画的时候选的是哪颗（动作，序号）：换选中时从那里动过去。面板重新升起时清掉。
thread_local! { static FONT_PICK: RefCell<Option<(String, Option<usize>)>> = const { RefCell::new(None) }; }
// 二选一（沉浸式 / 淡化）上一次选的是哪颗，换选中时和字体一样从旧样子动过去。
thread_local! { static CHOICE_PICK: RefCell<Option<(String, usize)>> = const { RefCell::new(None) }; }
// 带图标和文字的分段（分隔符样式）上一次选的是哪段，换选中时胶囊从那里滑过去。
thread_local! { static SEG_PICK: RefCell<Option<(String, usize)>> = const { RefCell::new(None) }; }

fn rows(n: usize, per: usize) -> f64 { n.div_ceil(per) as f64 }
fn node_h(node: &Node, w: f64) -> f64 {
    match node {
        Node::Grab => 4.,
        Node::Header { .. } => 64.,
        Node::Segments { .. } => 13. + 36. + 10.,
        Node::BackgroundModes { .. } => 54.,
        Node::Fonts { .. } => 14. + FONT_DOT + 14.,
        Node::Swatches { fills, custom, .. } => rows(fills.len() + custom.is_some() as usize, 6) * PITCH + 4.,
        Node::Caption(_) => 30.,
        Node::Row { .. } => 50.,
        Node::Choice { .. } => 44. + 6.,
        Node::Tiles(rows) => rows.len() as f64 * (TILE + GAP) - GAP + 8.,
        Node::Thumbs { fills, .. } => rows(fills.len(), 3) * (thumb_h(w) + 5.),
        Node::Photos { items, library } => {
            let (cols, _, _, pitch) = photo_grid(w, *library);
            rows(items.len(), cols) * pitch
        }
        Node::SearchPhotos { .. } => 57.,
        Node::StyleFooter { .. } => 68.,
        Node::Cards { items, .. } => {
            let (cols, _, preview) = style_grid(w);
            rows(items.len(), cols) * (preview + 33.)
        }
        Node::Actions(_) => thumb_h(w) + 5.,
        Node::Wide(_) => 74.,
        Node::Strips { items, cols, .. } => rows(items.len(), (*cols).max(1)) * strip_h(*cols),
        Node::Scroll { nodes, max_height } => f64::from(height(nodes, w as f32)).min(*max_height),
        Node::FixedScroll { height, .. } => *height,
        Node::Empty(..) => 110.,
        Node::Cells { h, .. } => *h,
        Node::Dots { fills, .. } => rows(fills.len(), 8) * 48. + 6.,
    }
}
/// 胶带用固定高度展示纸纹，图库另留出一张衬底和名称的位置。
fn strip_preview(cols: usize) -> (f64, f64) { if cols == 1 { (51., 60.) } else { (42., 108.) } }
fn strip_h(cols: usize) -> f64 { strip_preview(cols).1 + if cols == 1 { 32. } else { 36. } }
fn thumb_h(w: f64) -> f64 { ((w - PAD as f64 * 2. - 10.) / 3. * 0.68).round() }
fn photo_grid(w: f64, library: bool) -> (usize, f64, f64, f64) {
    let cols = if library { 2 } else { 3 };
    let cw = (w - f64::from(PAD) * 2. - (cols - 1) as f64 * 8.) / cols as f64;
    let ch = (cw * if library { 0.66 } else { 0.62 }).clamp(42., 160.);
    (cols, cw, ch, ch + if library { 31. } else { 8. })
}
fn style_grid(w: f64) -> (usize, f64, f64) {
    let cols = if w < 540. { 2 } else { 3 };
    let card = (w - f64::from(PAD) * 2. - 12. * (cols - 1) as f64) / cols as f64;
    (cols, card, (card * 1.06).min(270.))
}
/// 这一列内容要多高（宽 `w` 时），占位按它留。
pub fn height(nodes: &[Node], w: f32) -> f32 { (nodes.iter().map(|n| node_h(n, f64::from(w))).sum::<f64>() + if matches!(nodes.first(), Some(Node::Grab)) { 20. } else { 14. }) as f32 }

#[derive(Clone, Copy, Default, PartialEq)]
pub struct Material { pub tint: Option<Hsla>, pub wash: Option<Hsla> }

struct Panel { view: *mut AnyObject, pull: *mut AnyObject, scroll: Option<*mut AnyObject>, frame: Bounds<Pixels>, nodes: Vec<Node>, dark: Option<bool>, material: Material, shown: bool }
thread_local! { static PANEL: RefCell<Option<Panel>> = const { RefCell::new(None) }; }
thread_local! { static SEEN: Cell<bool> = const { Cell::new(false) }; }

#[link(name = "UIKit", kind = "framework")]
unsafe extern "C" { fn UIAccessibilityIsReduceTransparencyEnabled() -> objc2::runtime::Bool; }

/// 每帧开头调用：这一帧没有占位就收起。
pub fn begin(window: &Window) {
    SEEN.with(|s| s.set(false));
    window.on_next_frame(|_, _| if !SEEN.with(|s| s.get()) { PANEL.with(|p| if let Some(panel) = p.borrow_mut().as_mut().filter(|panel| panel.shown) { unsafe {
        panel.shown = false;
        let view = panel.view as usize;
        let body = block2::RcBlock::new(move || { let _: () = msg_send![view as *mut AnyObject, setAlpha: 0f64]; });
        let done = block2::RcBlock::new(move |_finished: objc2::runtime::Bool| { let shown = PANEL.with(|p| p.borrow().as_ref().is_some_and(|p| p.shown)); if !shown { let _: () = msg_send![view as *mut AnyObject, setHidden: true]; } });
        let _: () = msg_send![AnyClass::get(c"UIView").unwrap(), animateWithDuration: 0.18f64, animations: &*body, completion: &*done];
    } }) });
}

/// 放在 GPUI 元素里的占位：面板摆在这个元素的位置上，内容是 `nodes`。
pub fn slot(nodes: Vec<Node>, material: Material) -> impl IntoElement {
    canvas(move |bounds, window, _| place(bounds, nodes.clone(), material, window), |_, _, _, _| {}).absolute().inset_0()
}

fn place(frame: Bounds<Pixels>, nodes: Vec<Node>, material: Material, window: &Window) {
    let Ok(handle) = HasWindowHandle::window_handle(window) else { return; };
    let RawWindowHandle::UiKit(handle) = handle.as_raw() else { return; };
    let parent = super::shot::overlay(handle.ui_view.as_ptr() as *mut AnyObject);
    SEEN.with(|s| s.set(true));
    let dark = crate::theme::dark();
    PANEL.with(|p| unsafe {
        let material = if UIAccessibilityIsReduceTransparencyEnabled().as_bool() { Material::default() } else { material };
        let mut slot = p.borrow_mut();
        let Some(glass_class) = AnyClass::get(c"UIGlassEffect") else { return; };
        let glass = || {
            let effect: *mut AnyObject = msg_send![glass_class, effectWithStyle: 0isize];
            if let Some(tint) = material.tint {
                let _: () = msg_send![effect, setTintColor: super::glass::ui_color(tint)];
            }
            effect
        };
        if slot.is_none() {
            let Some(effect_view) = AnyClass::get(c"UIVisualEffectView") else { return; };
            let view: *mut AnyObject = msg_send![effect_view, alloc];
            let view: *mut AnyObject = msg_send![view, initWithEffect: glass()];
            if let (Some(corners), Some(corner)) = (AnyClass::get(c"UICornerConfiguration"), AnyClass::get(c"UICornerRadius")) {
                let fixed: *mut AnyObject = msg_send![corner, fixedRadius: 30f64];
                let config: *mut AnyObject = msg_send![corners, configurationWithUniformRadius: fixed];
                let _: () = msg_send![view, setCornerConfiguration: config];
            }
            let _: () = msg_send![view, setHidden: true];
            let _: () = msg_send![parent, addSubview: view];
            let pull = pull_recognizer(view);
            *slot = Some(Panel { view, pull, scroll: None, frame: Bounds::default(), nodes: vec![], dark: Some(dark), material, shown: false });
            let _: () = msg_send![view, setOverrideUserInterfaceStyle: if dark { 2isize } else { 1isize }];
        }
        let Some(panel) = slot.as_mut() else { return; };
        if panel.material.tint != material.tint {
            let _: () = msg_send![panel.view, setEffect: glass()];
        }
        if panel.dark != Some(dark) {
            // 深浅色跟应用的设置，不跟系统。
            let _: () = msg_send![panel.view, setOverrideUserInterfaceStyle: if dark { 2isize } else { 1isize }];
            panel.dark = Some(dark);
        }
        let rect = CGRect::new(CGPoint::new(f64::from(frame.origin.x), f64::from(frame.origin.y)), CGSize::new(f64::from(frame.size.width), f64::from(frame.size.height)));
        let fresh = !panel.shown;
        if fresh {
            panel.shown = true;
            FONT_PICK.with(|f| f.borrow_mut().take());
            CHOICE_PICK.with(|f| f.borrow_mut().take());
            SEG_PICK.with(|f| f.borrow_mut().take());
            // 上次是往下拖走的：位移还留着。
            let _: () = msg_send![panel.view, setTransform: IDENTITY];
            // 从下面一点升上来，同时淡入。
            let _: () = msg_send![panel.view, setFrame: CGRect::new(CGPoint::new(rect.origin.x, rect.origin.y + 24.), rect.size)];
            let _: () = msg_send![panel.view, setAlpha: 0f64];
            let _: () = msg_send![panel.view, setHidden: false];
            let _: () = msg_send![parent, bringSubviewToFront: panel.view];
        }
        if fresh || panel.frame != frame {
            let (view, to) = (panel.view as usize, rect);
            let body = block2::RcBlock::new(move || { let _: () = msg_send![view as *mut AnyObject, setFrame: to]; let _: () = msg_send![view as *mut AnyObject, setAlpha: 1f64]; });
            // 换页、换分隔符类型时卡片跟着内容变高变矮。照 Craft（分隔符页录屏）：弹簧，快起慢停、几乎不过冲，
            // 中途再点也从当前位置接着走（1 = BeginFromCurrentState，4 = AllowUserInteraction）。
            let none = std::ptr::null_mut::<AnyObject>();
            let _: () = msg_send![AnyClass::get(c"UIView").unwrap(), animateWithDuration: if fresh { 0.32f64 } else { 0.5f64 }, delay: 0f64, usingSpringWithDamping: if fresh { 1f64 } else { 0.86f64 }, initialSpringVelocity: 0f64, options: 5usize, animations: &*body, completion: none];
        }
        if fresh || panel.frame.size != frame.size || panel.nodes != nodes || panel.material.wash != material.wash {
            let content: *mut AnyObject = msg_send![panel.view, contentView];
            let _: () = msg_send![content, setClipsToBounds: true];
            // 同一页：标题栏的字一样就算（选了分隔符样式后多出「重置」，还是这一页）。
            let same_page = !fresh && match (panel.nodes.first(), nodes.first()) {
                (Some(Node::Header { title: a, .. }), Some(Node::Header { title: b, .. })) => a == b,
                (a, b) => a == b,
            };
            let offset = panel.scroll.filter(|_| same_page).map_or(CGPoint::new(0., 0.), |scroll| { let o: CGPoint = msg_send![scroll, contentOffset]; CGPoint::new(o.x, o.y + SCROLL_TOP.with(|t| t.get())) });
            SCROLL_TOP.with(|t| t.set(0.));
            // 页和页之间内容淡着换；同一页里多出、少了一行（比如分隔符从「线」换成「和纸胶带」多出「图案」）也淡着换，
            // 不然新的一行先蹦出来、卡片才慢慢长高。拖滑杆这类只改数值、高度不变的不淡，免得一路闪。
            let resized = panel.frame.size != frame.size;
            // 换了分段（分隔符样式）的选中：胶囊自己滑过去，整块不淡，不然旧胶囊的残影留在原处；多出、少了的「图案」一行跟着卡片长高露出来。
            let slid = same_page && panel.nodes.iter().zip(&nodes).any(|(a, b)| matches!((a, b), (Node::Segments { picked: p, symbols, titles, .. }, Node::Segments { picked: q, .. }) if p != q && !symbols.is_empty() && !titles.is_empty()));
            if !fresh && (!same_page || resized) && !slid {
                let fade: *mut AnyObject = msg_send![AnyClass::get(c"CATransition").unwrap(), animation];
                let _: () = msg_send![fade, setDuration: if same_page { 0.22f64 } else { 0.3f64 }];
                let layer: *mut AnyObject = msg_send![content, layer];
                let _: () = msg_send![layer, addAnimation: fade, forKey: ns_string("page")];
            }
            let kids: *mut AnyObject = msg_send![content, subviews];
            let kids: *mut AnyObject = msg_send![kids, copy];
            let n: usize = msg_send![kids, count];
            for i in 0..n { let kid: *mut AnyObject = msg_send![kids, objectAtIndex: i]; let _: () = msg_send![kid, removeFromSuperview]; }
            let _: () = msg_send![kids, release];
            if let Some(wash) = material.wash {
                fill(content, self::rect(0., 0., rect.size.width, rect.size.height), &Fill::Color(wash), 30.);
            }
            panel.scroll = build(content, &nodes, f64::from(frame.size.width), offset);
            // 里面能竖着滚的那块：先等下拉收起认不认（在顶上往下拖是收起，别的照常滚）。
            if let Some(scroll) = panel.scroll {
                let pan: *mut AnyObject = msg_send![scroll, panGestureRecognizer];
                let _: () = msg_send![pan, requireGestureRecognizerToFail: panel.pull];
            }
            panel.nodes = nodes;
        }
        panel.material = material;
        panel.frame = frame;
    });
}

// ---- 下拉收起（照 Craft）：按住面板往下拖，面板跟着走；拖过 80pt 或往下甩松手，就和点标题栏的 ✕ 一样收起这一层
// （子页回上一页），没有标题栏的第一页整块滑出底边、面板收起；不够就弹回。和编辑器底部面板的 `pull_sheet` 同一个约定。

const IDENTITY: CGAffineTransform = CGAffineTransform { a: 1., b: 0., c: 0., d: 1., tx: 0., ty: 0. };
const PULL_DISMISS: f64 = 80.;

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "QuietMdPanelPull"]
    struct PanelPull;

    impl PanelPull {
        #[unsafe(method(pan:))]
        fn pan(&self, g: *mut AnyObject) { unsafe { pulled(g) } }

        /// 只认往下拖；在里面那块滚动区上，它没在顶上时让它滚。
        #[unsafe(method(gestureRecognizerShouldBegin:))]
        fn should_begin(&self, g: *mut AnyObject) -> objc2::runtime::Bool { objc2::runtime::Bool::new(unsafe { pull_begins(g) }) }
    }
);

unsafe fn pull_begins(g: *mut AnyObject) -> bool {
    unsafe {
        let view: *mut AnyObject = msg_send![g, view];
        let v: CGPoint = msg_send![g, velocityInView: view];
        if v.y <= 0. || v.y < v.x.abs() { return false; }
        let scroll = PANEL.with(|p| p.borrow().as_ref().and_then(|p| p.scroll));
        if let Some(scroll) = scroll {
            let at: CGPoint = msg_send![g, locationInView: scroll];
            let inside: bool = msg_send![scroll, pointInside: at, withEvent: std::ptr::null_mut::<AnyObject>()];
            let offset: CGPoint = msg_send![scroll, contentOffset];
            if inside && offset.y + SCROLL_TOP.with(|t| t.get()) > 0.5 { return false; }
        }
        true
    }
}

unsafe fn pull_recognizer(view: *mut AnyObject) -> *mut AnyObject {
    thread_local! { static TARGET: Cell<*mut AnyObject> = const { Cell::new(std::ptr::null_mut()) }; }
    unsafe {
        let mut target = TARGET.with(|t| t.get());
        if target.is_null() {
            // 手势对目标和代理都不持有：这一个一直留着。
            target = msg_send![PanelPull::class(), new];
            TARGET.with(|t| t.set(target));
        }
        let g: *mut AnyObject = msg_send![AnyClass::get(c"UIPanGestureRecognizer").unwrap(), alloc];
        let g: *mut AnyObject = msg_send![g, initWithTarget: target, action: sel!(pan:)];
        let _: () = msg_send![g, setDelegate: target];
        let _: () = msg_send![view, addGestureRecognizer: g];
        let _: () = msg_send![g, release];
        g
    }
}

unsafe fn pulled(g: *mut AnyObject) {
    unsafe {
        let view: *mut AnyObject = msg_send![g, view];
        let host: *mut AnyObject = msg_send![view, superview];
        let t: CGPoint = msg_send![g, translationInView: host];
        // 往上拖只给一点阻力，不让面板离开底边。
        let dy = if t.y >= 0. { t.y } else { -(-t.y).powf(0.7) };
        let state: isize = msg_send![g, state];
        let shift = |ty: f64| CGAffineTransform { ty, ..IDENTITY };
        match state {
            1 | 2 => { let _: () = msg_send![view, setTransform: shift(dy)]; }
            3 | 4 => {
                let v: CGPoint = msg_send![g, velocityInView: host];
                let go = state == 3 && (dy > PULL_DISMISS || (v.y > 900. && dy > 12.));
                // 有标题栏的子页：✕ 是回上一层，面板留着；第一页没有标题栏，整个收起。
                let back = PANEL.with(|p| p.borrow().as_ref().and_then(|p| p.nodes.iter().find_map(|n| if let Node::Header { close, .. } = n { Some(close.clone()) } else { None })));
                let (view_ptr, size) = (view as usize, { let b: CGRect = msg_send![view, bounds]; b.size.height });
                let to = if go && back.is_none() { shift(size + 120.) } else { IDENTITY };
                let body = block2::RcBlock::new(move || { let _: () = msg_send![view_ptr as *mut AnyObject, setTransform: to]; });
                let _: () = msg_send![AnyClass::get(c"UIView").unwrap(), animateWithDuration: if go { 0.25f64 } else { 0.35f64 }, delay: 0f64, usingSpringWithDamping: if go { 1f64 } else { 0.8f64 }, initialSpringVelocity: 0f64, options: 0usize, animations: &*body, completion: std::ptr::null_mut::<AnyObject>()];
                if go {
                    super::glass::haptic();
                    send(back.unwrap_or_else(|| if super::format::is_open() { "fmt-done".into() } else { "close".into() }));
                }
            }
            _ => {}
        }
    }
}

// ---- 构件

#[repr(C)]
#[derive(Clone, Copy)]
struct CGAffineTransform { a: f64, b: f64, c: f64, d: f64, tx: f64, ty: f64 }
unsafe impl objc2::Encode for CGAffineTransform {
    const ENCODING: Encoding = Encoding::Struct("CGAffineTransform", &[f64::ENCODING, f64::ENCODING, f64::ENCODING, f64::ENCODING, f64::ENCODING, f64::ENCODING]);
}

#[repr(C)]
struct CGColor { _private: [u8; 0] }
unsafe impl RefEncode for CGColor { const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("CGColor", &[])); }

#[repr(C)]
struct CGPath { _private: [u8; 0] }
unsafe impl RefEncode for CGPath { const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("CGPath", &[])); }

unsafe fn ui(name: &std::ffi::CStr) -> *mut AnyObject { unsafe { msg_send![AnyClass::get(c"UIColor").unwrap(), performSelector: objc2::runtime::Sel::register(name)] } }
unsafe fn cg(color: *mut AnyObject) -> *const CGColor { unsafe { msg_send![color, CGColor] } }
unsafe fn rect(x: f64, y: f64, w: f64, h: f64) -> CGRect { CGRect::new(CGPoint::new(x, y), CGSize::new(w, h)) }
unsafe fn plain(parent: *mut AnyObject, frame: CGRect) -> *mut AnyObject {
    unsafe {
        let v: *mut AnyObject = msg_send![AnyClass::get(c"UIView").unwrap(), new];
        let _: () = msg_send![v, setFrame: frame];
        let _: () = msg_send![v, setUserInteractionEnabled: false];
        let _: () = msg_send![parent, addSubview: v];
        let _: () = msg_send![v, release];
        v
    }
}
unsafe fn round(view: *mut AnyObject, radius: f64) {
    unsafe {
        let layer: *mut AnyObject = msg_send![view, layer];
        let _: () = msg_send![layer, setCornerRadius: radius];
        let _: () = msg_send![layer, setMasksToBounds: true];
    }
}
/// 系统字体；`design` 是 serif / mono / rounded 时换成那一种设计。
unsafe fn font(size: f64, weight: f64, design: &str) -> *mut AnyObject {
    unsafe {
        let base: *mut AnyObject = msg_send![AnyClass::get(c"UIFont").unwrap(), systemFontOfSize: size, weight: weight];
        let design = match design { "serif" => "NSCTFontUIFontDesignSerif", "mono" => "NSCTFontUIFontDesignMonospaced", "rounded" => "NSCTFontUIFontDesignRounded", _ => return base };
        let descriptor: *mut AnyObject = msg_send![base, fontDescriptor];
        let descriptor: *mut AnyObject = msg_send![descriptor, fontDescriptorWithDesign: ns_string(design)];
        if descriptor.is_null() { return base; }
        msg_send![AnyClass::get(c"UIFont").unwrap(), fontWithDescriptor: descriptor, size: size]
    }
}
unsafe fn label(parent: *mut AnyObject, frame: CGRect, text: &str, font: *mut AnyObject, color: *mut AnyObject, align: isize) -> *mut AnyObject {
    unsafe {
        let l: *mut AnyObject = msg_send![AnyClass::get(c"UILabel").unwrap(), new];
        let _: () = msg_send![l, setFrame: frame];
        let _: () = msg_send![l, setText: ns_string(text)];
        let _: () = msg_send![l, setFont: font];
        let _: () = msg_send![l, setTextColor: color];
        let _: () = msg_send![l, setTextAlignment: align];
        let _: () = msg_send![l, setAdjustsFontSizeToFitWidth: true];
        let _: () = msg_send![l, setMinimumScaleFactor: 0.6f64];
        let _: () = msg_send![parent, addSubview: l];
        let _: () = msg_send![l, release];
        l
    }
}
unsafe fn symbol(name: &str, size: f64, weight: isize) -> *mut AnyObject {
    unsafe {
        let config: *mut AnyObject = msg_send![AnyClass::get(c"UIImageSymbolConfiguration").unwrap(), configurationWithPointSize: size, weight: weight];
        msg_send![AnyClass::get(c"UIImage").unwrap(), systemImageNamed: ns_string(name), withConfiguration: config]
    }
}
unsafe fn icon(parent: *mut AnyObject, frame: CGRect, name: &str, size: f64, color: *mut AnyObject) -> *mut AnyObject {
    unsafe {
        let v: *mut AnyObject = msg_send![AnyClass::get(c"UIImageView").unwrap(), new];
        let _: () = msg_send![v, setFrame: frame];
        let _: () = msg_send![v, setImage: symbol(name, size, 5)];
        let _: () = msg_send![v, setTintColor: color];
        // 4 = UIViewContentModeCenter
        let _: () = msg_send![v, setContentMode: 4isize];
        let _: () = msg_send![parent, addSubview: v];
        let _: () = msg_send![v, release];
        v
    }
}
/// 透明的按钮盖在 `frame` 上，点了交出 `act`。
unsafe fn tap(parent: *mut AnyObject, frame: CGRect, act: String) -> *mut AnyObject {
    unsafe {
        let b: *mut AnyObject = msg_send![AnyClass::get(c"UIButton").unwrap(), buttonWithType: 0isize];
        let _: () = msg_send![b, setFrame: frame];
        let handler = block2::RcBlock::new(move |_action: *mut AnyObject| if !MENU_UP.with(|m| m.replace(false)) { send(act.clone()) });
        let action: *mut AnyObject = msg_send![AnyClass::get(c"UIAction").unwrap(), actionWithTitle: ns_string(""), image: std::ptr::null_mut::<AnyObject>(), identifier: std::ptr::null_mut::<AnyObject>(), handler: &*handler];
        // 64 = UIControlEventTouchUpInside
        let _: () = msg_send![b, addAction: action, forControlEvents: 64usize];
        let _: () = msg_send![parent, addSubview: b];
        b
    }
}

thread_local! { static MENU_UP: Cell<bool> = const { Cell::new(false) }; }
/// 长按弹出了菜单，松手那一下不再算点卡片（不然长按「设置为默认样式」的同时也把这一套套上了）。
/// 按下时清掉；菜单弹出（UIControlEventMenuActionTriggered）时记一笔；`tap` 的松手看到这一笔就不交出动作。
/// 给了 `card` 时顺带做按压动效：按下微微缩，菜单弹出时震一下、卡片弹起再落回，松手或取消回原样。
unsafe fn menu_swallows_tap(button: *mut AnyObject, card: Option<*mut AnyObject>) {
    unsafe {
        let card = card.map(|c| c as usize);
        let scale = move |k: f64, spring: bool| if let Some(c) = card {
            let body = block2::RcBlock::new(move || { let _: () = msg_send![c as *mut AnyObject, setTransform: CGAffineTransform { a: k, b: 0., c: 0., d: k, tx: 0., ty: 0. }]; });
            let none = std::ptr::null_mut::<AnyObject>();
            // 4 = UIViewAnimationOptionAllowUserInteraction
            let _: () = msg_send![AnyClass::get(c"UIView").unwrap(), animateWithDuration: if spring { 0.45f64 } else { 0.16f64 }, delay: 0f64, usingSpringWithDamping: if spring { 0.55f64 } else { 1f64 }, initialSpringVelocity: 0f64, options: 4usize, animations: &*body, completion: none];
        };
        let down = block2::RcBlock::new(move |_: *mut AnyObject| { MENU_UP.with(|m| m.set(false)); scale(0.96, false); });
        let shown = block2::RcBlock::new(move |_: *mut AnyObject| {
            MENU_UP.with(|m| m.set(true));
            super::glass::haptic();
            if let Some(c) = card {
                let c = c as *mut AnyObject;
                let _: () = msg_send![c, setTransform: CGAffineTransform { a: 1.05, b: 0., c: 0., d: 1.05, tx: 0., ty: 0. }];
            }
            scale(1., true);
        });
        let up = block2::RcBlock::new(move |_: *mut AnyObject| scale(1., true));
        // 1 = TouchDown；1<<14 = MenuActionTriggered；64|128|256 = TouchUpInside|TouchUpOutside|TouchCancel
        for (handler, event) in [(&down, 1usize), (&shown, 1usize << 14), (&up, 64 | 128 | 256)] {
            let action: *mut AnyObject = msg_send![AnyClass::get(c"UIAction").unwrap(), actionWithHandler: &**handler];
            let _: () = msg_send![button, addAction: action, forControlEvents: event];
        }
    }
}
unsafe fn plain_button(parent: *mut AnyObject, frame: CGRect) -> *mut AnyObject {
    unsafe {
        let b: *mut AnyObject = msg_send![AnyClass::get(c"UIButton").unwrap(), buttonWithType: 0isize];
        let _: () = msg_send![b, setFrame: frame];
        let _: () = msg_send![parent, addSubview: b];
        b
    }
}
unsafe fn ui_menu(items: &[MenuItem]) -> *mut AnyObject {
    unsafe {
        let list: *mut AnyObject = msg_send![AnyClass::get(c"NSMutableArray").unwrap(), array];
        for item in items {
            let act = item.act.clone();
            let handler = block2::RcBlock::new(move |_action: *mut AnyObject| send(act.clone()));
            let image: *mut AnyObject = msg_send![AnyClass::get(c"UIImage").unwrap(), systemImageNamed: ns_string(item.symbol)];
            let action: *mut AnyObject = msg_send![AnyClass::get(c"UIAction").unwrap(), actionWithTitle: ns_string(&item.title), image: image, identifier: std::ptr::null_mut::<AnyObject>(), handler: &*handler];
            // 2 = UIMenuElementAttributesDestructive
            if item.danger { let _: () = msg_send![action, setAttributes: 2usize]; }
            let _: () = msg_send![list, addObject: action];
        }
        msg_send![AnyClass::get(c"UIMenu").unwrap(), menuWithChildren: list]
    }
}
unsafe fn named_tap(parent: *mut AnyObject, frame: CGRect, title: &str, act: String) -> *mut AnyObject {
    unsafe {
        let button = tap(parent, frame, act.clone());
        let _: () = msg_send![button, setAccessibilityIdentifier: ns_string(&act)];
        let _: () = msg_send![button, setAccessibilityLabel: ns_string(title)];
        button
    }
}
unsafe fn image_view(parent: *mut AnyObject, frame: CGRect, image: *mut AnyObject, mode: isize) -> *mut AnyObject {
    unsafe {
        let v: *mut AnyObject = msg_send![AnyClass::get(c"UIImageView").unwrap(), new];
        let _: () = msg_send![v, setFrame: frame];
        let _: () = msg_send![v, setImage: image];
        let _: () = msg_send![v, setContentMode: mode];
        let _: () = msg_send![parent, addSubview: v];
        let _: () = msg_send![v, release];
        v
    }
}

/// 把一块填上，按 `radius` 剪圆角。
unsafe fn fill(parent: *mut AnyObject, frame: CGRect, f: &Fill, radius: f64) -> *mut AnyObject {
    unsafe {
        match f {
            Fill::Color(c) => {
                let v = plain(parent, frame);
                let _: () = msg_send![v, setBackgroundColor: super::glass::ui_color(*c)];
                round(v, radius);
                v
            }
            Fill::Gradient(a, b, dir) => {
                let (from, to) = match dir { 1 => ((0., 0.5), (1., 0.5)), 2 => ((0., 0.), (1., 1.)), 3 => ((0., 1.), (1., 0.)), _ => ((0.5, 0.), (0.5, 1.)) };
                gradient(parent, frame, &[*a, *b], from, to, radius)
            }
            Fill::Image(path) => {
                let image: *mut AnyObject = msg_send![AnyClass::get(c"UIImage").unwrap(), imageWithContentsOfFile: ns_string(path)];
                // 2 = UIViewContentModeScaleAspectFill
                let v = image_view(parent, frame, image, 2);
                round(v, radius);
                v
            }
            Fill::Tape(path) => {
                let v = plain(parent, frame);
                round(v, radius);
                let class = AnyClass::get(c"UIImage").unwrap();
                let mut image: *mut AnyObject = msg_send![class, imageWithContentsOfFile: ns_string(path)];
                if !image.is_null() {
                    let mut size: CGSize = msg_send![image, size];
                    if frame.size.width * size.height > frame.size.height * size.width
                        && let Some(wrapped) = super::washi::repeat_texture(std::path::Path::new(path))
                    {
                        let continuous: *mut AnyObject = msg_send![class, imageWithContentsOfFile: ns_string(&wrapped.to_string_lossy())];
                        if !continuous.is_null() { image = continuous; size = msg_send![image, size]; }
                    }
                    let spans = super::washi::tile_spans(frame.size.width, frame.size.height, size.width, size.height);
                    for (x, width) in spans { image_view(v, rect(x, 0., width, frame.size.height), image, 0); }
                }
                v
            }
        }
    }
}

unsafe fn gradient(parent: *mut AnyObject, frame: CGRect, colors: &[Hsla], from: (f64, f64), to: (f64, f64), radius: f64) -> *mut AnyObject {
    unsafe {
        let view = plain(parent, frame);
        let layer: *mut AnyObject = msg_send![AnyClass::get(c"CAGradientLayer").unwrap(), layer];
        let stops: *mut AnyObject = msg_send![AnyClass::get(c"NSMutableArray").unwrap(), array];
        for color in colors { let _: () = msg_send![stops, addObject: cg(super::glass::ui_color(*color)) as *mut AnyObject]; }
        let _: () = msg_send![layer, setColors: stops];
        let _: () = msg_send![layer, setStartPoint: CGPoint::new(from.0, from.1)];
        let _: () = msg_send![layer, setEndPoint: CGPoint::new(to.0, to.1)];
        let _: () = msg_send![layer, setFrame: rect(0., 0., frame.size.width, frame.size.height)];
        let host: *mut AnyObject = msg_send![view, layer];
        let _: () = msg_send![host, addSublayer: layer];
        round(view, radius);
        view
    }
}

unsafe fn torn_path(size: CGSize) -> *const CGPath {
    unsafe {
        let (w, h) = (size.width, size.height);
        let path: *mut AnyObject = msg_send![AnyClass::get(c"UIBezierPath").unwrap(), bezierPath];
        let left = super::washi::torn_edge(false, h);
        let right = super::washi::torn_edge(true, h);
        let _: () = msg_send![path, moveToPoint: CGPoint::new(left[0].0, 0.)];
        for (dx, y) in right { let _: () = msg_send![path, addLineToPoint: CGPoint::new(w - dx, y)]; }
        for (x, y) in left.into_iter().rev() { let _: () = msg_send![path, addLineToPoint: CGPoint::new(x, y)]; }
        let _: () = msg_send![path, closePath];
        msg_send![path, CGPath]
    }
}

/// 胶带样张的撕口，纸边直接透出玻璃的底色。
unsafe fn torn_edges(view: *mut AnyObject, size: CGSize) {
    unsafe { mask_path(view, size, torn_path(size)); }
}

unsafe fn mask_path(view: *mut AnyObject, size: CGSize, path: *const CGPath) {
    unsafe {
        let mask: *mut AnyObject = msg_send![AnyClass::get(c"CAShapeLayer").unwrap(), layer];
        let _: () = msg_send![mask, setFrame: CGRect::new(CGPoint::new(0., 0.), size)];
        let _: () = msg_send![mask, setPath: path];
        let layer: *mut AnyObject = msg_send![view, layer];
        let _: () = msg_send![layer, setMask: mask];
    }
}

/// 图案页的选中线沿着胶带本身，在撕边外留一条玻璃底色。
unsafe fn tape_outline(parent: *mut AnyObject, frame: CGRect, color: *mut AnyObject) {
    unsafe {
        let gap = 2.5;
        let frame = rect(frame.origin.x - gap, frame.origin.y - gap, frame.size.width + 2. * gap, frame.size.height + 2. * gap);
        let view = plain(parent, frame);
        let outline: *mut AnyObject = msg_send![AnyClass::get(c"CAShapeLayer").unwrap(), layer];
        let _: () = msg_send![outline, setFrame: CGRect::new(CGPoint::new(0., 0.), frame.size)];
        let _: () = msg_send![outline, setPath: torn_path(frame.size)];
        let _: () = msg_send![outline, setFillColor: std::ptr::null::<CGColor>()];
        let _: () = msg_send![outline, setStrokeColor: cg(color)];
        let _: () = msg_send![outline, setLineJoin: ns_string("round")];
        let _: () = msg_send![outline, setLineWidth: 1.5f64];
        let layer: *mut AnyObject = msg_send![view, layer];
        let _: () = msg_send![layer, addSublayer: outline];
    }
}

/// 选中的描边：比那一块大一圈的空心框。
unsafe fn ring(parent: *mut AnyObject, frame: CGRect, radius: f64, color: *mut AnyObject, grow: f64, width: f64) {
    unsafe {
        let v = plain(parent, rect(frame.origin.x - grow, frame.origin.y - grow, frame.size.width + grow * 2., frame.size.height + grow * 2.));
        let layer: *mut AnyObject = msg_send![v, layer];
        let _: () = msg_send![layer, setCornerRadius: radius + grow];
        let _: () = msg_send![layer, setBorderWidth: width];
        let _: () = msg_send![layer, setBorderColor: cg(color)];
    }
}

#[repr(C)] #[derive(Clone, Copy)] struct Insets { top: f64, left: f64, bottom: f64, right: f64 }
unsafe impl objc2::Encode for Insets { const ENCODING: Encoding = Encoding::Struct("UIEdgeInsets", &[const { Encoding::Double }; 4]); }
thread_local! { /// 滚动区顶上让出的高度（图库页标题栏浮在内容上时）；`contentOffset` 加上它才是「滚了多少」。
    static SCROLL_TOP: Cell<f64> = const { Cell::new(0.) }; }

/// 磨砂的一条：系统的模糊材质，内容从它底下滚过时透出来糊开（照 Craft 样式图库的标题栏和底栏，IMG_9349）。
unsafe fn frosted(parent: *mut AnyObject, frame: CGRect, corners: usize) -> *mut AnyObject {
    unsafe {
        // 8 = UIBlurEffectStyleSystemMaterial：上下两条一样厚，标题栏不再透出面板后面的文档，底栏透的是卡片的颜色。
        let blur: *mut AnyObject = msg_send![AnyClass::get(c"UIBlurEffect").unwrap(), effectWithStyle: 8isize];
        let view: *mut AnyObject = msg_send![AnyClass::get(c"UIVisualEffectView").unwrap(), alloc];
        let view: *mut AnyObject = msg_send![view, initWithEffect: blur];
        let _: () = msg_send![view, setFrame: frame];
        // 贴着面板的那两个角跟面板一样圆（30pt、连续曲率），不然磨砂条把圆角切成直角。
        let layer: *mut AnyObject = msg_send![view, layer];
        let _: () = msg_send![layer, setCornerRadius: 30f64];
        let _: () = msg_send![layer, setMaskedCorners: corners];
        let _: () = msg_send![layer, setCornerCurve: ns_string("continuous")];
        let _: () = msg_send![view, setClipsToBounds: true];
        let _: () = msg_send![parent, addSubview: view];
        let _: () = msg_send![view, release];
        msg_send![view, contentView]
    }
}

/// 图库页（标题栏、滚动的卡片、底栏）：卡片滚到标题栏和底栏底下，两条都是磨砂，透出下面糊开的颜色。
unsafe fn build_floating(content: *mut AnyObject, nodes: &[Node], w: f64, offset: CGPoint) -> Option<*mut AnyObject> {
    unsafe {
        let [header @ Node::Header { .. }, Node::Scroll { nodes: inner, .. }, footer @ Node::StyleFooter { .. }] = nodes else { return None; };
        let (top, mid, bottom) = (node_h(header, w), node_h(&nodes[1], w), node_h(footer, w) + 14.);
        let view: *mut AnyObject = msg_send![AnyClass::get(c"UIScrollView").unwrap(), new];
        let full = f64::from(height(inner, w as f32));
        let _: () = msg_send![view, setFrame: rect(0., 0., w, top + mid + bottom)];
        let _: () = msg_send![view, setContentInsetAdjustmentBehavior: 2isize];
        // 卡片滚到最底下也不从面板的圆角里漏出来。
        let layer: *mut AnyObject = msg_send![view, layer];
        let _: () = msg_send![layer, setCornerRadius: 30f64];
        let _: () = msg_send![layer, setCornerCurve: ns_string("continuous")];
        let _: () = msg_send![view, setClipsToBounds: true];
        let insets = Insets { top, left: 0., bottom, right: 0. };
        let _: () = msg_send![view, setContentInset: insets];
        let _: () = msg_send![view, setVerticalScrollIndicatorInsets: insets];
        let _: () = msg_send![view, setContentSize: CGSize::new(w, full)];
        let _ = build(view, inner, w, CGPoint::new(0., 0.));
        let _: () = msg_send![content, addSubview: view];
        let _: () = msg_send![view, setContentOffset: CGPoint::new(0., offset.y.clamp(0., (full - mid).max(0.)) - top)];
        let _: () = msg_send![view, release];
        let bar = frosted(content, rect(0., 0., w, top), 3);
        build(bar, std::slice::from_ref(header), w, offset);
        let bar = frosted(content, rect(0., top + mid, w, bottom), 12);
        build(bar, std::slice::from_ref(footer), w, offset);
        let line = plain(content, rect(0., top + mid, w, 1. / 3.));
        let _: () = msg_send![line, setBackgroundColor: ui(c"separatorColor")];
        SCROLL_TOP.with(|t| t.set(top));
        Some(view)
    }
}

unsafe fn build(content: *mut AnyObject, nodes: &[Node], w: f64, offset: CGPoint) -> Option<*mut AnyObject> {
    unsafe {
        if matches!(nodes, [Node::Header { .. }, Node::Scroll { .. }, Node::StyleFooter { .. }]) { return build_floating(content, nodes, w, offset); }
        let pad = f64::from(PAD);
        let inner = w - pad * 2.;
        let (primary, secondary) = (ui(c"labelColor"), ui(c"secondaryLabelColor"));
        let well = ui(c"quaternarySystemFillColor");
        let mut scroll = None;
        let mut y = if matches!(nodes.first(), Some(Node::Grab)) { 6. } else { 0. };
        for node in nodes {
            let h = node_h(node, w);
            match node {
                Node::Grab => {
                    let bar = plain(content, rect((w - 36.) / 2., y, 36., 4.));
                    let _: () = msg_send![bar, setBackgroundColor: ui(c"tertiaryLabelColor")];
                    round(bar, 2.);
                    y += 10.;
                    continue;
                }
                Node::Header { title, warn, reset, close } => {
                    label(content, rect(pad + 4., y, inner - 110., 63.), title, font(17., 0.3, ""), primary, 0);
                    let mut x = w - pad - 4. - 26.;
                    // 关闭：一颗小圆钮。
                    let dot = plain(content, rect(x, y + 19., 26., 26.));
                    let _: () = msg_send![dot, setBackgroundColor: well];
                    round(dot, 13.);
                    icon(content, rect(x, y + 19., 26., 26.), "xmark", 11., secondary);
                    let close_button = tap(content, rect(x - 9., y + 10., 44., 44.), close.clone());
                    if close == "page:pattern" {
                        let _: () = msg_send![close_button, setAccessibilityIdentifier: ns_string("quiet.washi.back")];
                        let _: () = msg_send![close_button, setAccessibilityLabel: ns_string(crate::settings::tr("返回图案", "Back to patterns"))];
                    } else if close == "page:main" {
                        let _: () = msg_send![close_button, setAccessibilityIdentifier: ns_string("quiet.style.back")];
                        let _: () = msg_send![close_button, setAccessibilityLabel: ns_string(crate::settings::tr("返回样式", "Back to style"))];
                    } else if close == "page:background" {
                        let _: () = msg_send![close_button, setAccessibilityIdentifier: ns_string("quiet.background.back")];
                        let _: () = msg_send![close_button, setAccessibilityLabel: ns_string(crate::settings::tr("返回背景", "Back to background"))];
                    }
                    if let Some(reset) = reset {
                        x -= 40.;
                        icon(content, rect(x, y + 19., 26., 26.), "arrow.counterclockwise", 15., secondary);
                        tap(content, rect(x - 7., y + 10., 40., 44.), reset.clone());
                    }
                    if *warn { x -= 36.; icon(content, rect(x, y + 19., 26., 26.), "exclamationmark.triangle.fill", 15., ui(c"systemYellowColor")); }
                    let line = plain(content, rect(pad + 4., y + 63., inner - 8., 1. / 3.));
                    let _: () = msg_send![line, setBackgroundColor: ui(c"separatorColor")];
                }
                Node::Segments { symbols, titles, picked, act } => {
                    let n = symbols.len().max(titles.len()).max(1);
                    let cell = inner / n as f64;
                    if !symbols.is_empty() && !titles.is_empty() {
                        // 照 Craft（分隔符样式截图）：没选的是一颗颗只有图标的圆钮，选中的拉长成胶囊占满余下的宽，图标后面跟着名字，
                        // 主色字、浅主色底。换选中时同字体那一排：各颗滑到新位置，旧胶囊缩回圆钮、名字淡出，新的拉长、名字淡入。
                        use crate::theme::Theme;
                        let accent = super::glass::ui_color(Theme::primary());
                        let (dot, gap, top) = (40., 10., y + 10.);
                        let count = titles.len().min(symbols.len());
                        let wide = (inner - (dot + gap) * (count as f64 - 1.)).max(dot);
                        let layout = |p: usize| -> Vec<(f64, f64)> {
                            let mut x = pad;
                            (0..count).map(|i| { let w = if i == p { wide } else { dot }; let at = (x, w); x += w + gap; at }).collect()
                        };
                        let before = SEG_PICK.with(|f| f.replace(Some((act.clone(), *picked))))
                            .filter(|(a, p)| a == act && p != picked).map(|(_, p)| p);
                        let (from, to) = (layout(before.unwrap_or(*picked)), layout(*picked));
                        let tint = |sel: bool| super::glass::ui_color(if sel { Theme::primary().opacity(0.13) } else { Theme::fill_hover() });
                        let mut moves: Vec<(usize, CGRect, f64, Option<*mut AnyObject>, Option<*mut AnyObject>)> = vec![];
                        for i in 0..count {
                            let (on, was) = (i == *picked, before.map_or(i == *picked, |b| b == i));
                            // 图标和名字放在圆钮里、跟着它裁：胶囊缩回圆钮时名字不露到外面。
                            let chip = plain(content, rect(0., 0., 0., 0.));
                            round(chip, dot / 2.);
                            let _: () = msg_send![chip, setClipsToBounds: true];
                            let mark = icon(chip, rect(0., 0., 18., dot), symbols[i], 15., if was { accent } else { secondary });
                            let name = label(chip, rect(0., 0., 0., dot), &titles[i], font(16., 0.23, ""), accent, 0);
                            let _: () = msg_send![name, setIsAccessibilityElement: false];
                            let nw: f64 = { let s: CGSize = msg_send![name, sizeThatFits: CGSize::new(1000., dot)]; s.width.ceil() };
                            // 一颗在某种样子下：底，图标，名字，名字透明度。名字一直贴在图标后面，圆钮时淡掉。
                            let state = |open: bool, (x, w): (f64, f64)| {
                                let group = if open { 18. + 6. + nw } else { 18. };
                                let left = (w - group) / 2.;
                                (rect(x, top, w, dot), rect(left, 0., 18., dot), rect(left + 24., 0., nw, dot), if open { 1. } else { 0. })
                            };
                            let (start, end) = (state(was, from[i]), state(on, to[i]));
                            let _: () = msg_send![chip, setFrame: start.0];
                            let _: () = msg_send![chip, setBackgroundColor: tint(was)];
                            let _: () = msg_send![mark, setFrame: start.1];
                            let _: () = msg_send![name, setFrame: start.2];
                            let _: () = msg_send![name, setAlpha: start.3];
                            moves.push((chip as usize, end.0, 1., Some(tint(on)), None));
                            moves.push((mark as usize, end.1, 1., None, Some(if on { accent } else { secondary })));
                            moves.push((name as usize, end.2, end.3, None, None));
                            let button = named_tap(content, end.0, &titles[i], format!("{act}{i}"));
                            let _: () = msg_send![button, setSelected: on];
                        }
                        if before.is_some() {
                            let body = block2::RcBlock::new(move || for (view, frame, alpha, color, ink) in &moves {
                                let view = *view as *mut AnyObject;
                                let _: () = msg_send![view, setFrame: *frame];
                                let _: () = msg_send![view, setAlpha: *alpha];
                                if let Some(c) = color { let _: () = msg_send![view, setBackgroundColor: *c]; }
                                if let Some(c) = ink { let _: () = msg_send![view, setTintColor: *c]; }
                            });
                            let _: () = msg_send![AnyClass::get(c"UIView").unwrap(), animateWithDuration: 0.42f64, delay: 0f64, usingSpringWithDamping: 0.82f64, initialSpringVelocity: 0f64, options: 0usize, animations: &*body, completion: std::ptr::null_mut::<AnyObject>()];
                        }
                        y += h;
                        continue;
                    }
                    let track = plain(content, rect(pad, y + 13., inner, 36.));
                    let _: () = msg_send![track, setBackgroundColor: well];
                    round(track, 18.);
                    let pattern = act == "pat-tab:";
                    for i in 0..n {
                        let on = i == *picked;
                        let frame = rect(pad + cell * i as f64, y + 13., cell, 36.);
                        if on {
                            let inset = if pattern { 2. } else { 0. };
                            let pill = plain(content, rect(frame.origin.x + inset, frame.origin.y + inset, cell - 2. * inset, 36. - 2. * inset));
                            let surface = if pattern { ui(c"systemBackgroundColor") } else { primary };
                            let _: () = msg_send![pill, setBackgroundColor: surface];
                            round(pill, 18.);
                        }
                        let ink = if on && pattern { primary }
                            else if on { if crate::theme::dark() { ui(c"blackColor") } else { ui(c"whiteColor") } } else { secondary };
                        match (symbols.get(i), titles.get(i)) {
                            (Some(name), Some(title)) if on => {
                                // 图标在前、文字在后，两样合起来居中。字宽按字数估：中文一个 15，别的 8.5。
                                let tw: f64 = title.chars().map(|c| if c.is_ascii() { 8.5 } else { 15. }).sum();
                                let x = frame.origin.x + (cell - tw - 22.) / 2.;
                                icon(content, rect(x, y + 13., 18., 36.), name, 14., ink);
                                label(content, rect(x + 22., y + 13., tw + 4., 36.), title, font(15., 0.23, ""), ink, 0);
                            }
                            (Some(name), _) => { icon(content, frame, name, 15., ink); }
                            (None, Some(title)) => { label(content, frame, title, font(15., 0.23, ""), ink, 1); }
                            _ => {}
                        }
                        tap(content, frame, format!("{act}{i}"));
                    }
                }
                Node::BackgroundModes { titles, picked } => {
                    // 照 Craft（IMG_9365）：一条 36pt 高的胶囊槽，只有图标；选中的是一颗白色小胶囊、带一点影子，图标深色。
                    use crate::theme::Theme;
                    let width = (inner - 6.) / 4.;
                    let base = rect(pad, y + 10., inner, 36.);
                    fill(content, base, &Fill::Color(Theme::fill_hover()), 18.);
                    for (i, symbol) in ["rectangle.slash", "square.fill", "square.bottomhalf.filled", "photo"].into_iter().enumerate() {
                        let frame = rect(pad + 3. + width * i as f64, y + 13., width, 30.);
                        let on = i == *picked;
                        if on {
                            let pill = plain(content, frame);
                            let _: () = msg_send![pill, setBackgroundColor: if crate::theme::dark() { ui(c"systemGray3Color") } else { ui(c"whiteColor") }];
                            round(pill, 15.);
                            let layer: *mut AnyObject = msg_send![pill, layer];
                            let _: () = msg_send![layer, setShadowColor: cg(super::glass::ui_color(black()))];
                            let _: () = msg_send![layer, setShadowOpacity: 0.12f32];
                            let _: () = msg_send![layer, setShadowRadius: 3f64];
                            let _: () = msg_send![layer, setShadowOffset: CGSize::new(0., 1.)];
                            let _: () = msg_send![pill, setClipsToBounds: false];
                        }
                        icon(content, frame, symbol, 15., if on { primary } else { secondary });
                        let button = named_tap(content, frame, &titles[i], format!("bg-tab:{i}"));
                        let _: () = msg_send![button, setSelected: on];
                    }
                }
                Node::Fonts { items, picked, act } => {
                    use crate::theme::Theme;
                    let accent = super::glass::ui_color(Theme::primary());
                    let n = items.len().max(1) as f64;
                    let top = y + 14.;
                    // 每一项在选中 `p` 时的位置：（x，宽）。没有选中的（几块字体不一样）时平分，每颗都是一样宽的胶囊。
                    let layout = |p: Option<usize>| -> Vec<(f64, f64)> {
                        let wide = if p.is_some() { (inner - (FONT_DOT + FONT_GAP) * (n - 1.)).max(FONT_DOT) } else { (inner - FONT_GAP * (n - 1.)) / n };
                        let mut x = pad;
                        (0..items.len()).map(|i| { let w = if p.is_none() || p == Some(i) { wide } else { FONT_DOT }; let at = (x, w); x += w + FONT_GAP; at }).collect()
                    };
                    // 照 Craft（录屏 2026-11-20 19:44）：换选中时各颗滑到新位置，旧胶囊缩回圆钮、名字淡出，新的拉长、名字淡入，弹簧约 0.3s。
                    // 面板每次整张重排，这里记着上一次选的是哪颗，新画的视图从那时的样子动过去。
                    let before = FONT_PICK.with(|f| f.replace(Some((act.clone(), *picked))))
                        .filter(|(a, p)| a == act && p != picked).map(|(_, p)| p);
                    let (from, to) = (layout(before.flatten()), layout(*picked));
                    let mut moves: Vec<(usize, CGRect, f64, Option<*mut AnyObject>)> = vec![];
                    for (i, (sample, name, design)) in items.iter().enumerate() {
                        let face = font(17., 0., design);
                        let on = *picked == Some(i);
                        let ink = if on { accent } else { secondary };
                        let shown = |p: Option<usize>| p.is_none() || p == Some(i);
                        let chip = plain(content, rect(0., 0., 0., 0.));
                        round(chip, FONT_DOT / 2.);
                        let tint = |sel: bool| super::glass::ui_color(if sel { Theme::primary().opacity(0.13) } else { Theme::fill_hover() });
                        let sample_view = label(content, rect(0., 0., 0., FONT_DOT), sample, face, ink, 1);
                        let name_view = label(content, rect(0., 0., 0., FONT_DOT), name, face, ink, 0);
                        let measure = |v: *mut AnyObject| -> f64 { let s: CGSize = msg_send![v, sizeThatFits: CGSize::new(1000., FONT_DOT)]; s.width.ceil() };
                        let (sw, nw) = (measure(sample_view), measure(name_view));
                        // 一颗在某种选中下的样子：底，字样，名字，名字透明度。名字一直贴在字样后面，圆钮时淡掉。
                        let state = |p: Option<usize>, (x, w): (f64, f64)| {
                            let open = shown(p);
                            let group = if open { sw + 5. + nw } else { sw };
                            let left = x + ((w - group) / 2.).max(8.);
                            (rect(x, top, w, FONT_DOT), rect(left, top, sw, FONT_DOT), rect(left + sw + 5., top, nw, FONT_DOT), if open { 1. } else { 0. })
                        };
                        let end = state(*picked, to[i]);
                        let start = if before.is_some() { state(before.flatten(), from[i]) } else { end };
                        let was_on = before.flatten() == Some(i) || (before.is_none() && on);
                        let _: () = msg_send![chip, setFrame: start.0];
                        let _: () = msg_send![chip, setBackgroundColor: tint(was_on)];
                        let _: () = msg_send![sample_view, setFrame: start.1];
                        let _: () = msg_send![name_view, setFrame: start.2];
                        let _: () = msg_send![name_view, setAlpha: start.3];
                        moves.push((chip as usize, end.0, 1., Some(tint(on))));
                        moves.push((sample_view as usize, end.1, 1., None));
                        moves.push((name_view as usize, end.2, end.3, None));
                        let button = named_tap(content, end.0, name, format!("{act}{i}"));
                        let _: () = msg_send![button, setSelected: on];
                    }
                    if before.is_some() {
                        let body = block2::RcBlock::new(move || for (view, frame, alpha, color) in &moves {
                            let view = *view as *mut AnyObject;
                            let _: () = msg_send![view, setFrame: *frame];
                            let _: () = msg_send![view, setAlpha: *alpha];
                            if let Some(c) = color { let _: () = msg_send![view, setBackgroundColor: *c]; }
                        });
                        let _: () = msg_send![AnyClass::get(c"UIView").unwrap(), animateWithDuration: 0.42f64, delay: 0f64, usingSpringWithDamping: 0.82f64, initialSpringVelocity: 0f64, options: 0usize, animations: &*body, completion: std::ptr::null_mut::<AnyObject>()];
                    }
                }
                Node::Swatches { fills, picked, act, custom } => {
                    let background = act.starts_with("bg-");
                    let side = SWATCH.min((inner / 6. - 4.).max(1.));
                    let step = (inner - side) / 5.;
                    let radius = if background { 12. } else { side / 2. };
                    let at = |i: usize| rect(pad + step * (i % 6) as f64, y + (i / 6) as f64 * PITCH + 8., side, side);
                    for (i, f) in fills.iter().enumerate() {
                        let frame = at(i);
                        let on = *picked == Some(i);
                        // 选中的缩一圈，外面套一个同色的环。
                        let inset = if on { if background { 3. } else { 5. } } else { 0. };
                        let dot = rect(frame.origin.x + inset, frame.origin.y + inset, side - 2. * inset, side - 2. * inset);
                        fill(content, dot, f, radius - inset);
                        if on {
                            let tint = if background { super::glass::ui_color(crate::theme::Theme::primary()) }
                                else { match f { Fill::Color(c) | Fill::Gradient(c, ..) => super::glass::ui_color(*c), Fill::Image(_) | Fill::Tape(_) => primary } };
                            ring(content, frame, radius, tint, 0., if background { 1.5 } else { 2.5 });
                        } else { ring(content, frame, radius, ui(c"separatorColor"), 0., 0.5); }
                        let key = format!("{act}{i}");
                        if background {
                            let label = match f {
                                Fill::Color(c) => super::style::to_hex(*c),
                                Fill::Gradient(a, b, _) => format!("{} → {}", super::style::to_hex(*a), super::style::to_hex(*b)),
                                Fill::Image(_) | Fill::Tape(_) => String::new(),
                            };
                            let button = named_tap(content, frame, &label, key);
                            let _: () = msg_send![button, setSelected: on];
                        } else { tap(content, frame, key); }
                    }
                    if let Some((act, on, color)) = custom {
                        let frame = at(fills.len());
                        // 彩虹圈：一圈锥形渐变，中间是当前的颜色和一个向下的箭头。
                        let wheel = plain(content, frame);
                        let layer: *mut AnyObject = msg_send![AnyClass::get(c"CAGradientLayer").unwrap(), layer];
                        let _: () = msg_send![layer, setType: ns_string("conic")];
                        let colors: *mut AnyObject = msg_send![AnyClass::get(c"NSMutableArray").unwrap(), array];
                        for hue in [0., 0.17, 0.33, 0.5, 0.67, 0.83, 1.] {
                            let c: *mut AnyObject = msg_send![AnyClass::get(c"UIColor").unwrap(), colorWithHue: hue as f64, saturation: 0.85f64, brightness: 0.95f64, alpha: 1f64];
                            let _: () = msg_send![colors, addObject: cg(c) as *mut AnyObject];
                        }
                        let _: () = msg_send![layer, setColors: colors];
                        let _: () = msg_send![layer, setStartPoint: CGPoint::new(0.5, 0.5)];
                        let _: () = msg_send![layer, setEndPoint: CGPoint::new(0.5, 0.)];
                        let _: () = msg_send![layer, setFrame: rect(0., 0., side, side)];
                        let host: *mut AnyObject = msg_send![wheel, layer];
                        let _: () = msg_send![host, addSublayer: layer];
                        round(wheel, radius);
                        let inset = 4.;
                        let core = rect(frame.origin.x + inset, frame.origin.y + inset, side - inset * 2., side - inset * 2.);
                        fill(content, core, &Fill::Color(if *on { *color } else { crate::theme::Theme::bg_body() }), radius - inset);
                        let ink = if *on && color.l > 0.6 { ui(c"blackColor") } else { primary };
                        icon(content, core, "chevron.down", 12., ink);
                        let button = named_tap(content, frame, crate::settings::tr("自选颜色", "Custom color"), act.clone());
                        let _: () = msg_send![button, setSelected: *on];
                    }
                }
                Node::Caption(text) => { label(content, rect(pad, y + 8., inner, 18.), text, font(13., 0., ""), secondary, 0); }
                Node::Row { label: text, trailing } => {
                    label(content, rect(pad, y, inner * 0.5, 50.), text, font(17., 0., ""), primary, 0);
                    match trailing {
                        Trailing::Chip(f, act) => {
                            let frame = rect(w - pad - 54., y + 4., 54., 42.);
                            fill(content, frame, f, 9.);
                            named_tap(content, frame, text, act.clone());
                        }
                        Trailing::Switch(on, act) => {
                            let s: *mut AnyObject = msg_send![AnyClass::get(c"UISwitch").unwrap(), new];
                            let size: CGRect = msg_send![s, bounds];
                            let _: () = msg_send![s, setFrame: rect(w - pad - size.size.width, y + (50. - size.size.height) / 2., size.size.width, size.size.height)];
                            let _: () = msg_send![s, setOn: *on];
                            // 开关只管显示：它自己的手势在这层浮层里点不动（拖得动），上面盖一颗透明的按钮来接。
                            let _: () = msg_send![s, setUserInteractionEnabled: false];
                            let _: () = msg_send![content, addSubview: s];
                            let frame: CGRect = msg_send![s, frame];
                            let _: () = msg_send![s, release];
                            let button = named_tap(content, frame, text, act.clone());
                            let _: () = msg_send![button, setAccessibilityValue: ns_string(if *on { "1" } else { "0" })];
                        }
                        Trailing::Menu(current, options) => {
                            let config: *mut AnyObject = msg_send![AnyClass::get(c"UIButtonConfiguration").unwrap(), grayButtonConfiguration];
                            let _: () = msg_send![config, setTitle: ns_string(current)];
                            let _: () = msg_send![config, setImage: symbol("chevron.up.chevron.down", 12., 5)];
                            // 8 = NSDirectionalRectEdgeTrailing
                            let _: () = msg_send![config, setImagePlacement: 8usize];
                            let _: () = msg_send![config, setImagePadding: 10f64];
                            let _: () = msg_send![config, setBaseForegroundColor: primary];
                            let b: *mut AnyObject = msg_send![AnyClass::get(c"UIButton").unwrap(), buttonWithConfiguration: config, primaryAction: std::ptr::null_mut::<AnyObject>()];
                            let _: () = msg_send![b, setFrame: rect(w - pad - 160., y + 4., 160., 42.)];
                            let items: *mut AnyObject = msg_send![AnyClass::get(c"NSMutableArray").unwrap(), array];
                            for (title, act) in options {
                                let act = act.clone();
                                let handler = block2::RcBlock::new(move |_action: *mut AnyObject| send(act.clone()));
                                let item: *mut AnyObject = msg_send![AnyClass::get(c"UIAction").unwrap(), actionWithTitle: ns_string(title), image: std::ptr::null_mut::<AnyObject>(), identifier: std::ptr::null_mut::<AnyObject>(), handler: &*handler];
                                // 1 = UIMenuElementStateOn
                                if title == current { let _: () = msg_send![item, setState: 1isize]; }
                                let _: () = msg_send![items, addObject: item];
                            }
                            let menu: *mut AnyObject = msg_send![AnyClass::get(c"UIMenu").unwrap(), menuWithTitle: ns_string(""), image: std::ptr::null_mut::<AnyObject>(), identifier: std::ptr::null_mut::<AnyObject>(), options: 0usize, children: items];
                            let _: () = msg_send![b, setMenu: menu];
                            let _: () = msg_send![b, setShowsMenuAsPrimaryAction: true];
                            let _: () = msg_send![b, setAccessibilityLabel: ns_string(text)];
                            let _: () = msg_send![b, setAccessibilityValue: ns_string(current)];
                            let _: () = msg_send![content, addSubview: b];
                        }
                    }
                }
                Node::Choice { labels, picked, act } => {
                    let accent = crate::theme::Theme::primary();
                    let tint = super::glass::ui_color(accent.opacity(if crate::theme::dark() { 0.18 } else { 0.10 }));
                    // 选中的那颗宽（六成），另一颗窄；换选中时两颗滑到新宽度、底色跟着换，弹簧同字体那排。
                    let layout = |p: usize| -> [(f64, f64); 2] {
                        let wide = (inner - 8.) * 0.6;
                        let narrow = inner - 8. - wide;
                        if p == 0 { [(pad, wide), (pad + wide + 8., narrow)] } else { [(pad, narrow), (pad + narrow + 8., wide)] }
                    };
                    let before = CHOICE_PICK.with(|f| f.replace(Some((act.clone(), *picked))))
                        .filter(|(a, p)| a == act && p != picked).map(|(_, p)| p);
                    let (from, to) = (layout(before.unwrap_or(*picked)), layout(*picked));
                    let mut moves: Vec<(usize, CGRect, Option<f64>)> = vec![];
                    for (i, text) in labels.iter().take(2).enumerate() {
                        let selected = i == *picked;
                        let was = before.map_or(selected, |b| b == i);
                        let (start, end) = (rect(from[i].0, y, from[i].1, 44.), rect(to[i].0, y, to[i].1, 44.));
                        let pill = plain(content, start);
                        let _: () = msg_send![pill, setBackgroundColor: well];
                        round(pill, 22.);
                        // 选中的底色叠在灰底上，换选中时淡入淡出。
                        let glow = plain(pill, rect(0., 0., start.size.width, 44.));
                        let _: () = msg_send![glow, setBackgroundColor: tint];
                        let _: () = msg_send![glow, setAutoresizingMask: 18usize];
                        let _: () = msg_send![glow, setAlpha: if was { 1f64 } else { 0f64 }];
                        let ink = if selected { super::glass::ui_color(accent) } else { secondary };
                        let caption = label(content, start, text, font(16., if selected { 0.23 } else { 0. }, ""), ink, 1);
                        let _: () = msg_send![caption, setIsAccessibilityElement: false];
                        moves.push((pill as usize, end, None));
                        moves.push((glow as usize, rect(0., 0., end.size.width, 44.), Some(if selected { 1. } else { 0. })));
                        moves.push((caption as usize, end, None));
                        let button = named_tap(content, end, text, format!("{act}{i}"));
                        let _: () = msg_send![button, setSelected: selected];
                    }
                    if before.is_some() {
                        let body = block2::RcBlock::new(move || for (view, frame, alpha) in &moves {
                            let view = *view as *mut AnyObject;
                            let _: () = msg_send![view, setFrame: *frame];
                            if let Some(a) = alpha { let _: () = msg_send![view, setAlpha: *a]; }
                        });
                        let _: () = msg_send![AnyClass::get(c"UIView").unwrap(), animateWithDuration: 0.42f64, delay: 0f64, usingSpringWithDamping: 0.82f64, initialSpringVelocity: 0f64, options: 0usize, animations: &*body, completion: std::ptr::null_mut::<AnyObject>()];
                    }
                }
                Node::Tiles(rows) => {
                    let unit = (inner - GAP * 2.) / 3.;
                    let mut ty = y;
                    for row in rows {
                        let mut x = pad;
                        for tile in row {
                            let tw = unit * f64::from(tile.span) + GAP * f64::from(tile.span - 1);
                            let frame = rect(x, ty, tw, TILE);
                            let base = plain(content, frame);
                            let _: () = msg_send![base, setBackgroundColor: well];
                            round(base, 16.);
                            let corner = rect(x + 12., ty + 8., tw - 24., 22.);
                            match &tile.art {
                                Art::Ink { ink, paper } => style_cards::ink_tile(base, tw, &tile.title, *paper, *ink),
                                Art::Paper { paper, ink } => style_cards::paper_tile(base, tw, &tile.title, *paper, *ink),
                                Art::Stack { current, others } => {
                                    let title = label(base, rect(12., 8., tw - 24., 22.), &tile.title, font(14., 0.23, ""), primary, 0);
                                    let _: () = msg_send![title, setIsAccessibilityElement: false];
                                    style_cards::stack(base, tw, current, others);
                                }
                                Art::Rules(f) => {
                                    label(base, rect(12., 8., tw - 24., 22.), &tile.title, font(14., 0.23, ""), secondary, 0);
                                    match f {
                                        // 照 Craft：一截撕边的胶带，左边留一点、右边出格子。
                                        Some(f) => { let tape = rect(16., TILE - 52., tw, 26.); torn_edges(fill(base, tape, f, 0.), tape.size); }
                                        None => for (dy, h) in [(46., 1.), (32., 2.5), (17., 5.)] {
                                            let line = plain(base, rect(0., TILE - dy, tw, h));
                                            let _: () = msg_send![line, setBackgroundColor: secondary];
                                        },
                                    }
                                }
                                Art::Font(name, design) => {
                                    label(content, rect(x + 8., ty + 30., tw - 16., 44.), name, font(34., 0.3, design), primary, 1);
                                    label(content, rect(x + 8., ty + 74., tw - 16., 20.), &tile.title, font(14., 0., ""), secondary, 1);
                                }
                                Art::Cover(path) => {
                                    // 照 Craft：上面一块封面（没有时是一块浅灰底，中间一个图标），名字在下面。
                                    let shot = rect(x + 6., ty + 6., tw - 12., 70.);
                                    match path {
                                        Some(p) => { fill(content, shot, &Fill::Image(p.clone()), 10.); }
                                        None => { fill(content, shot, &Fill::Color(crate::theme::Theme::fill_hover_strong()), 10.); icon(content, shot, "photo", 24., secondary); }
                                    }
                                    label(content, rect(x + 6., ty + 84., tw - 12., 22.), &tile.title, font(14., 0.23, ""), secondary, 1);
                                }
                                Art::Fill(f) => {
                                    if let Some(f) = f { fill(content, frame, f, 16.); }
                                    // 铺了图或深色时字要看得见：字后面不另垫，颜色照亮度挑。
                                    let light = match f { Some(Fill::Color(c)) | Some(Fill::Gradient(c, ..)) => c.l > 0.6, Some(Fill::Image(_) | Fill::Tape(_)) => false, None => !crate::theme::dark() };
                                    let ink = if f.is_none() { secondary } else if light { ui(c"darkGrayColor") } else { ui(c"whiteColor") };
                                    label(content, corner, &tile.title, font(14., 0.23, ""), ink, 0);
                                }
                                Art::Symbol(name) => {
                                    label(content, corner, &tile.title, font(14., 0.23, ""), secondary, 0);
                                    icon(content, rect(x + tw - 64., ty + TILE - 60., 52., 48.), name, 30., secondary);
                                }
                            }
                            let button = tap(content, frame, tile.act.clone());
                            if tile.act == "page:background" || matches!(tile.art, Art::Ink { .. } | Art::Paper { .. } | Art::Stack { .. }) {
                                let _: () = msg_send![button, setAccessibilityIdentifier: ns_string(&tile.act)];
                                let _: () = msg_send![button, setAccessibilityLabel: ns_string(&tile.title)];
                                if let Art::Stack { current, .. } = &tile.art { let _: () = msg_send![button, setAccessibilityValue: ns_string(&current.name)]; }
                            }
                            x += tw + GAP;
                        }
                        ty += TILE + GAP;
                    }
                }
                Node::Thumbs { fills, picked, act } => {
                    let (tw, th) = ((inner - 10.) / 3., thumb_h(w));
                    for (i, f) in fills.iter().enumerate() {
                        let frame = rect(pad + (tw + 5.) * (i % 3) as f64, y + (th + 5.) * (i / 3) as f64, tw, th);
                        fill(content, frame, f, 10.);
                        if *picked == Some(i) { ring(content, frame, 10., super::glass::ui_color(crate::theme::Theme::primary()), 0., 2.); }
                        tap(content, frame, format!("{act}{i}"));
                    }
                }
                Node::Photos { items, library } => {
                    let (cols, cw, ch, pitch) = photo_grid(w, *library);
                    let accent = super::glass::ui_color(crate::theme::Theme::primary());
                    for (i, photo) in items.iter().enumerate() {
                        let frame = rect(pad + (cw + 8.) * (i % cols) as f64, y + pitch * (i / cols) as f64, cw, ch);
                        fill(content, frame, &Fill::Image(photo.path.clone()), 11.);
                        ring(content, frame, 11., if photo.picked { accent } else { ui(c"separatorColor") }, 0., if photo.picked { 1.7 } else { 0.5 });
                        let button = named_tap(content, rect(frame.origin.x, frame.origin.y, cw, pitch - 3.), &photo.title, photo.act.clone());
                        let _: () = msg_send![button, setSelected: photo.picked];
                        if let Some(credit) = &photo.credit {
                            let caption = label(content, rect(frame.origin.x + 2., frame.origin.y + ch + 2., cw - 4., 22.), credit, font(11., 0., ""), secondary, 0);
                            let _: () = msg_send![caption, setIsAccessibilityElement: false];
                            let _: () = msg_send![button, setAccessibilityHint: ns_string(credit)];
                        }
                    }
                }
                Node::SearchPhotos { prompt, query } => {
                    let frame = rect(pad, y + 6., inner, 43.);
                    fill(content, frame, &Fill::Color(crate::theme::Theme::fill_hover_strong()), 12.);
                    icon(content, rect(pad + 8., y + 6., 22., 43.), "magnifyingglass", 15., secondary);
                    let text = label(content, rect(pad + 36., y + 6., inner - 48., 43.), if query.is_empty() { prompt } else { query }, font(15., 0., ""), secondary, 0);
                    let _: () = msg_send![text, setIsAccessibilityElement: false];
                    let button = named_tap(content, frame, prompt, "bg-search".into());
                    let _: () = msg_send![button, setAccessibilityValue: ns_string(query)];
                }
                Node::StyleFooter { name, note, action, menu } => {
                    use crate::theme::Theme;
                    let button_w = action.as_ref().map_or(if menu.is_empty() { 0. } else { 44. }, |(_, act)| if *act == "gallery-delete" { 104. } else { 78. });
                    let lw = (inner - button_w - 14.).max(20.);
                    label(content, rect(pad + 4., y + 13., lw, 24.), name, font(17., 0.33, ""), primary, 0);
                    label(content, rect(pad + 4., y + 36., lw, 18.), note, font(13., 0., ""), secondary, 0);
                    if let Some((title, act)) = action {
                        let delete = *act == "gallery-delete";
                        let frame = rect(w - pad - button_w, y + 12., button_w, 44.);
                        fill(content, frame, &Fill::Color(if delete { Theme::danger_cell() } else { Theme::text() }), 22.);
                        let ink = super::glass::ui_color(if delete { Theme::danger() } else { Theme::bg_body() });
                        let text = label(content, frame, title, font(16., 0.31, ""), ink, 1);
                        let _: () = msg_send![text, setIsAccessibilityElement: false];
                        let button = tap(content, frame, (*act).into());
                        let _: () = msg_send![button, setAccessibilityLabel: ns_string(title)];
                        let _: () = msg_send![button, setAccessibilityIdentifier: ns_string(act)];
                    } else if !menu.is_empty() {
                        // 照 Craft（IMG_9350）：右边一个「…」，点开就是系统菜单。
                        let frame = rect(w - pad - 44., y + 12., 44., 44.);
                        icon(content, frame, "ellipsis", 19., primary);
                        let button = plain_button(content, frame);
                        let _: () = msg_send![button, setMenu: ui_menu(menu)];
                        let _: () = msg_send![button, setShowsMenuAsPrimaryAction: true];
                        let _: () = msg_send![button, setAccessibilityLabel: ns_string(crate::settings::tr("更多", "More"))];
                        let _: () = msg_send![button, setAccessibilityIdentifier: ns_string("style-more")];
                    }
                }
                Node::Cards { items, picked, act } => {
                    let (cols, tw, th) = style_grid(w);
                    let accent = super::glass::ui_color(crate::theme::Theme::primary());
                    for (i, card) in items.iter().enumerate() {
                        let frame = rect(pad + (tw + 12.) * (i % cols) as f64, y + (th + 33.) * (i / cols) as f64, tw, th);
                        let view = style_cards::preview(content, frame, card, style_cards::Preview::Gallery);
                        let on = *picked == Some(i);
                        // 照 Craft：选中的只是外面一圈主色描边，名字也换主色；没选的名字是浅一档的灰。
                        if on { ring(content, frame, 12., accent, 1.5, 2.); }
                        let text = label(content, rect(frame.origin.x + 4., frame.origin.y + th + 6., tw - 8., 21.), &card.name, font(15., 0., ""), if on { accent } else { secondary }, 0);
                        let _: () = msg_send![text, setIsAccessibilityElement: false];
                        let button = tap(content, rect(frame.origin.x, frame.origin.y, tw, th + 28.), format!("{act}{i}"));
                        let _: () = msg_send![button, setAccessibilityLabel: ns_string(&card.name)];
                        // 长按出系统菜单（UIButton 有菜单、又不把它当主动作时，长按弹出），点一下照旧是选用。
                        if !card.menu.is_empty() { let _: () = msg_send![button, setMenu: ui_menu(&card.menu)]; menu_swallows_tap(button, Some(view)); }
                        let _: () = msg_send![button, setSelected: on];
                    }
                }
                Node::Strips { items, cols } => {
                    let cols = (*cols).max(1);
                    let (cw, rh) = ((inner - 12. * (cols - 1) as f64) / cols as f64, strip_h(cols));
                    let (th, preview_h) = strip_preview(cols);
                    let accent = super::glass::ui_color(crate::theme::Theme::primary());
                    let mat = super::glass::ui_color(crate::theme::Theme::fill_hover_strong());
                    let caption_color = super::glass::ui_color(crate::theme::Theme::text_caption());
                    for (i, strip) in items.iter().enumerate() {
                        let cell = rect(pad + (cw + 12.) * (i % cols) as f64, y + rh * (i / cols) as f64, cw, rh - 2.);
                        let preview = rect(cell.origin.x, cell.origin.y, cw, preview_h);
                        if cols > 1 {
                            let base = plain(content, preview);
                            let _: () = msg_send![base, setBackgroundColor: mat];
                            round(base, 10.);
                        }
                        let inset = if cols == 1 { 5. } else { 8. };
                        let swatch = rect(cell.origin.x + inset, cell.origin.y + (preview_h - th) / 2., cw - 2. * inset, th);
                        torn_edges(fill(content, swatch, &strip.fill, 0.), swatch.size);
                        if cols > 1 {
                            let (color, width) = if strip.picked { (accent, 1.) } else { (ui(c"separatorColor"), 0.5) };
                            ring(content, preview, 10., color, 0., width);
                        } else if strip.picked { tape_outline(content, swatch, accent); }
                        let caption_y = cell.origin.y + preview_h + if cols == 1 { 4. } else { 6. };
                        let caption = label(content, rect(cell.origin.x + 2., caption_y, cw - 4., 18.), &strip.title, font(13., 0.23, ""), if strip.picked { accent } else { caption_color }, if cols == 1 { 1 } else { 0 });
                        if let Some(act) = &strip.act {
                            let _: () = msg_send![caption, setIsAccessibilityElement: false];
                            let button = tap(content, cell, act.clone());
                            let _: () = msg_send![button, setAccessibilityLabel: ns_string(&strip.title)];
                            let _: () = msg_send![button, setSelected: strip.picked];
                        }
                    }
                }
                Node::Scroll { nodes, .. } | Node::FixedScroll { nodes, .. } => {
                    let view: *mut AnyObject = msg_send![AnyClass::get(c"UIScrollView").unwrap(), new];
                    let full = f64::from(height(nodes, w as f32));
                    let _: () = msg_send![view, setFrame: rect(0., y, w, h)];
                    // 2 = UIScrollViewContentInsetAdjustmentNever：面板已经在安全区里。
                    let _: () = msg_send![view, setContentInsetAdjustmentBehavior: 2isize];
                    let _: () = msg_send![view, setContentSize: CGSize::new(w, full)];
                    let _: () = msg_send![view, setClipsToBounds: true];
                    let _ = build(view, nodes, w, CGPoint::new(0., 0.));
                    let _: () = msg_send![content, addSubview: view];
                    let _: () = msg_send![view, setContentOffset: CGPoint::new(0., offset.y.clamp(0., (full - h).max(0.)))];
                    let _: () = msg_send![view, release];
                    scroll = Some(view);
                }
                Node::Actions(items) | Node::Wide(items) => {
                    let wide = matches!(node, Node::Wide(_));
                    let (n, gap, th) = if wide { (items.len().max(1), 8., h - 5.) } else { (3, 5., thumb_h(w)) };
                    let tw = (inner - gap * (n - 1) as f64) / n as f64;
                    let surface = if wide { super::glass::ui_color(crate::theme::Theme::fill_hover()) } else { well };
                    let ink = if wide { super::glass::ui_color(crate::theme::Theme::text_caption()) } else { secondary };
                    for (i, (name, title, act)) in items.iter().enumerate() {
                        let frame = rect(pad + (tw + gap) * i as f64, y, tw, th);
                        let base = plain(content, frame);
                        let _: () = msg_send![base, setBackgroundColor: surface];
                        round(base, 10.);
                        if wide { ring(content, frame, 10., ui(c"separatorColor"), 0., 0.5); }
                        if wide && *name == "photo.on.rectangle" {
                            let preview = rect(frame.origin.x + (tw - 94.) / 2., y + 3., 94., 29.);
                            let tape = plain(content, preview);
                            let _: () = msg_send![tape, setBackgroundColor: surface];
                            torn_edges(tape, preview.size);
                            for (x, dy) in [(8., 1.), (38., 1.), (68., 1.), (23., 17.), (53., 17.)] {
                                let photo = icon(tape, rect(x, dy, 17., 14.), name, 9., ink);
                                let _: () = msg_send![photo, setAlpha: 0.40f64];
                            }
                            let t = CGAffineTransform { a: 0.9986, b: 0.0523, c: -0.0523, d: 0.9986, tx: 0., ty: 0. };
                            let _: () = msg_send![tape, setTransform: t];
                        } else {
                            let icon_h = if wide { 24. } else { 26. };
                            icon(content, rect(frame.origin.x, y + th / 2. - icon_h + if wide { 1. } else { 0. }, tw, icon_h), name, 18., ink);
                        }
                        let caption = label(content, rect(frame.origin.x + 4., y + th / 2. + 4., tw - 8., 18.), title, font(13., 0.23, ""), ink, 1);
                        let _: () = msg_send![caption, setIsAccessibilityElement: false];
                        named_tap(content, frame, title, act.clone());
                    }
                }
                Node::Dots { fills, picked, act } => {
                    let d = 34.;
                    let step = (inner - d) / 7.;
                    for (i, f) in fills.iter().enumerate() {
                        let frame = rect(pad + step * (i % 8) as f64, y + 6. + (i / 8) as f64 * 48., d, d);
                        let on = *picked == Some(i);
                        let dot = if on { rect(frame.origin.x + 4., frame.origin.y + 4., d - 8., d - 8.) } else { frame };
                        match f {
                            Some(c) => { fill(content, dot, &Fill::Color(*c), dot.size.width / 2.); ring(content, frame, d / 2., if on { super::glass::ui_color(*c) } else { ui(c"separatorColor") }, 0., if on { 2. } else { 1. / 3. }); }
                            None => {
                                ring(content, dot, dot.size.width / 2., ui(c"separatorColor"), 0., 1.);
                                let slash = plain(content, rect(dot.origin.x + dot.size.width / 2. - 0.75, dot.origin.y + 3., 1.5, dot.size.height - 6.));
                                let _: () = msg_send![slash, setBackgroundColor: ui(c"systemRedColor")];
                                let t = CGAffineTransform { a: 0.7071, b: 0.7071, c: -0.7071, d: 0.7071, tx: 0., ty: 0. };
                                let _: () = msg_send![slash, setTransform: t];
                                if on { ring(content, frame, d / 2., primary, 0., 2.); }
                            }
                        }
                        tap(content, frame, format!("{act}{i}"));
                    }
                }
                Node::Cells { cells, h, rule } => {
                    let total: f64 = cells.iter().map(|c| c.span).sum::<f64>().max(1.);
                    let gap = 8.;
                    let unit = (inner - gap * (cells.len().max(1) - 1) as f64) / total;
                    let ch = (h - 10.).min(52.);
                    let mut x = pad;
                    for cell in cells {
                        let cw = unit * cell.span;
                        let frame = rect(x, y + (h - ch) / 2., cw, ch);
                        x += cw + gap;
                        let accent = super::glass::ui_color(crate::theme::Theme::primary());
                        if cell.tile {
                            let base = plain(content, frame);
                            let _: () = msg_send![base, setBackgroundColor: ui(c"quaternarySystemFillColor")];
                            round(base, 12.);
                        }
                        // 选中：字和图标的一格是浅的主色底加一圈描边（Craft 的「正文」），只有图标的只铺浅底。
                        if cell.on {
                            let base = plain(content, frame);
                            let tint: *mut AnyObject = msg_send![accent, colorWithAlphaComponent: 0.14f64];
                            let _: () = msg_send![base, setBackgroundColor: tint];
                            round(base, 12.);
                            if matches!(cell.look, Look::Text(..)) { let edge: *mut AnyObject = msg_send![accent, colorWithAlphaComponent: 0.55f64]; ring(content, frame, 12., edge, 0., 1.5); }
                        }
                        let ink = if !cell.enabled { ui(c"tertiaryLabelColor") } else { primary };
                        // 字的宽度估一下（中文一个字一个字号，西文约一半），图标和竖线跟着字居中。
                        let width = |text: &str, size: f64| text.chars().map(|c| if c.is_ascii() { 0.56 } else { 1. }).sum::<f64>() * size;
                        let mid = frame.origin.x + cw / 2.;
                        match &cell.look {
                            Look::Text(text, sym, size, weight) => match sym {
                                Some(name) => {
                                    let tw = width(text, *size).min(cw - 34.);
                                    let left = mid - (tw + 26.) / 2.;
                                    icon(content, rect(left, frame.origin.y, 20., ch), name, 15., if cell.enabled { secondary } else { ink });
                                    label(content, rect(left + 26., frame.origin.y, tw + 4., ch), text, font(*size, *weight, ""), ink, 0);
                                }
                                None => { label(content, rect(frame.origin.x + 4., frame.origin.y, cw - 8., ch), text, font(*size, *weight, ""), ink, 1); }
                            },
                            Look::Symbol(name) => { icon(content, frame, name, 18., ink); }
                            Look::Bar(text) => {
                                let tw = width(text, 16.);
                                let left = mid - (tw + 12.) / 2.;
                                let bar = plain(content, rect(left, frame.origin.y + ch / 2. - 11., 2.5, 22.));
                                let _: () = msg_send![bar, setBackgroundColor: ink];
                                round(bar, 1.25);
                                label(content, rect(left + 12., frame.origin.y, tw + 4., ch), text, font(16., 0., ""), ink, 0);
                            }
                            Look::Wheel(color) => {
                                let side = 34.;
                                wheel(content, rect(mid - side / 2., frame.origin.y + (ch - side) / 2., side, side), *color);
                            }
                            Look::Font(sample, name, design) => {
                                label(content, rect(frame.origin.x + 4., frame.origin.y + 6., cw - 8., ch * 0.55), sample, font(22., 0.3, design), ink, 1);
                                label(content, rect(frame.origin.x + 4., frame.origin.y + ch * 0.58, cw - 8., ch * 0.32), name, font(12., 0., ""), secondary, 1);
                            }
                        }
                        if cell.enabled { tap(content, frame, cell.act.clone()); }
                    }
                    if *rule {
                        let line = plain(content, rect(pad + 4., y + h - 0.5, inner - 8., 1. / 3.));
                        let _: () = msg_send![line, setBackgroundColor: ui(c"separatorColor")];
                    }
                }
                Node::Empty(name, text) => {
                    icon(content, rect(pad, y + 22., inner, 30.), name, 24., ui(c"tertiaryLabelColor"));
                    label(content, rect(pad, y + 58., inner, 20.), text, font(15., 0., ""), secondary, 1);
                }
            }
            y += h;
        }
        scroll
    }
}

/// 彩虹圈（锥形渐变的一圈），中间一颗当前的颜色。
unsafe fn wheel(content: *mut AnyObject, frame: CGRect, color: Option<Hsla>) {
    unsafe {
        let ring_view = plain(content, frame);
        let layer: *mut AnyObject = msg_send![AnyClass::get(c"CAGradientLayer").unwrap(), layer];
        let _: () = msg_send![layer, setType: ns_string("conic")];
        let colors: *mut AnyObject = msg_send![AnyClass::get(c"NSMutableArray").unwrap(), array];
        for hue in [0., 0.17, 0.33, 0.5, 0.67, 0.83, 1.] {
            let c: *mut AnyObject = msg_send![AnyClass::get(c"UIColor").unwrap(), colorWithHue: hue as f64, saturation: 0.8f64, brightness: 0.95f64, alpha: 1f64];
            let _: () = msg_send![colors, addObject: cg(c) as *mut AnyObject];
        }
        let _: () = msg_send![layer, setColors: colors];
        let _: () = msg_send![layer, setStartPoint: CGPoint::new(0.5, 0.5)];
        let _: () = msg_send![layer, setEndPoint: CGPoint::new(0.5, 0.)];
        let _: () = msg_send![layer, setFrame: rect(0., 0., frame.size.width, frame.size.height)];
        let host: *mut AnyObject = msg_send![ring_view, layer];
        let _: () = msg_send![host, addSublayer: layer];
        round(ring_view, frame.size.width / 2.);
        let inset = 4.;
        let core = rect(frame.origin.x + inset, frame.origin.y + inset, frame.size.width - inset * 2., frame.size.height - inset * 2.);
        let center = fill(content, core, &Fill::Color(color.unwrap_or_else(crate::theme::Theme::bg_body)), core.size.width / 2.);
        let _: *mut AnyObject = center;
    }
}

/// 控件被点了：回到 GPUI。格式面板的动作都以 `fmt-` 开头，其余是样式面板的。
fn send(act: String) {
    super::menu::dispatch(move |w, window, cx| if act.starts_with("fmt-") { super::format::action(&act, w, window, cx) } else { super::stylepanel::action(&act, w, window, cx) });
}

unsafe fn ns_string(s: &str) -> *mut AnyObject {
    let c = std::ffi::CString::new(s).unwrap_or_default();
    unsafe { msg_send![AnyClass::get(c"NSString").unwrap(), stringWithUTF8String: c.as_ptr()] }
}
