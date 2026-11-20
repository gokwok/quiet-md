//! 原生的玻璃面板和拼它用的几样构件：样式面板（`stylepanel.rs`）的每一页都是一列 `Node`。
//!
//! 和 `card.rs` 同一条路：一张 `UIGlassEffect` 的卡片放在 `shot::overlay` 里，位置跟 GPUI 的占位（`slot`）；里面全是系统控件，
//! GPUI 画不到玻璃上面。内容用数据描述，变了就整张重排（面板很小，几十个视图）；控件点了把一个字符串交回 `stylepanel::action`，
//! 选了哪个颜色、哪一格都编在这个字符串里。尺寸照 Craft 实测（录屏 1320px 宽 = 440pt）。
use std::cell::{Cell, RefCell};

use gpui::*;
use objc2::encode::{Encoding, RefEncode};
use objc2::msg_send;
use objc2::runtime::{AnyClass, AnyObject};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

#[path = "style_cards.rs"]
mod style_cards;

/// 一块的填充：纯色、两色渐变（方向同 `store::Backdrop::Gradient`）、图片文件。
#[derive(Clone, PartialEq, Debug)]
pub enum Fill { Color(Hsla), Gradient(Hsla, Hsla, u8), Image(String) }

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
pub struct Card { pub fill: Option<Fill>, pub paper: Hsla, pub ink: Hsla, pub name: String, pub design: &'static str, pub faded: bool, pub divider: MiniDivider }

#[derive(Clone, PartialEq, Debug)]
pub enum MiniDivider { Line, Tape(String), Doodle(String) }

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
pub struct Strip { pub fill: Fill, pub title: String, pub picked: bool, pub act: String }

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
    StyleFooter { title: String, card: Card, action: Option<(String, &'static str)> },
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

fn rows(n: usize, per: usize) -> f64 { n.div_ceil(per) as f64 }
fn node_h(node: &Node, w: f64) -> f64 {
    match node {
        Node::Grab => 4.,
        Node::Header { .. } => 64.,
        Node::Segments { .. } => 13. + 36. + 10.,
        Node::BackgroundModes { .. } => 68.,
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
        Node::Actions(_) | Node::Wide(_) => thumb_h(w) + 5.,
        Node::Strips { items, cols, .. } => rows(items.len(), (*cols).max(1)) * strip_h(*cols),
        Node::Scroll { nodes, max_height } => f64::from(height(nodes, w as f32)).min(*max_height),
        Node::FixedScroll { height, .. } => *height,
        Node::Empty(..) => 110.,
        Node::Cells { h, .. } => *h,
        Node::Dots { fills, .. } => rows(fills.len(), 8) * 48. + 6.,
    }
}
/// 胶带一行多高：上下留白、胶带、名字。一列的胶带粗一些。
fn strip_h(cols: usize) -> f64 { 6. + if cols <= 1 { 34. } else { 30. } + 4. + 16. + 8. }
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
    (cols, card, (card * 0.96).min(250.))
}
/// 这一列内容要多高（宽 `w` 时），占位按它留。
pub fn height(nodes: &[Node], w: f32) -> f32 { (nodes.iter().map(|n| node_h(n, f64::from(w))).sum::<f64>() + if matches!(nodes.first(), Some(Node::Grab)) { 20. } else { 14. }) as f32 }

#[derive(Clone, Copy, Default, PartialEq)]
pub struct Material { pub tint: Option<Hsla>, pub wash: Option<Hsla> }

struct Panel { view: *mut AnyObject, scroll: Option<*mut AnyObject>, frame: Bounds<Pixels>, nodes: Vec<Node>, dark: Option<bool>, material: Material, shown: bool }
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
            *slot = Some(Panel { view, scroll: None, frame: Bounds::default(), nodes: vec![], dark: Some(dark), material, shown: false });
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
            // 从下面一点升上来，同时淡入。
            let _: () = msg_send![panel.view, setFrame: CGRect::new(CGPoint::new(rect.origin.x, rect.origin.y + 24.), rect.size)];
            let _: () = msg_send![panel.view, setAlpha: 0f64];
            let _: () = msg_send![panel.view, setHidden: false];
            let _: () = msg_send![parent, bringSubviewToFront: panel.view];
        }
        if fresh || panel.frame != frame {
            let (view, to) = (panel.view as usize, rect);
            let body = block2::RcBlock::new(move || { let _: () = msg_send![view as *mut AnyObject, setFrame: to]; let _: () = msg_send![view as *mut AnyObject, setAlpha: 1f64]; });
            // 换页时卡片跟着内容变高变矮（4 << 16 = CurveEaseOut 的近似：系统默认的缓动）。
            let _: () = msg_send![AnyClass::get(c"UIView").unwrap(), animateWithDuration: 0.28f64, animations: &*body];
        }
        if fresh || panel.frame.size != frame.size || panel.nodes != nodes || panel.material.wash != material.wash {
            let content: *mut AnyObject = msg_send![panel.view, contentView];
            let _: () = msg_send![content, setClipsToBounds: true];
            let same_page = !fresh && panel.nodes.first() == nodes.first();
            let offset = panel.scroll.filter(|_| same_page).map_or(CGPoint::new(0., 0.), |scroll| msg_send![scroll, contentOffset]);
            // 页和页之间内容淡着换。
            if !fresh && panel.nodes.first() != nodes.first() {
                let fade: *mut AnyObject = msg_send![AnyClass::get(c"CATransition").unwrap(), animation];
                let _: () = msg_send![fade, setDuration: 0.2f64];
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
            panel.nodes = nodes;
        }
        panel.material = material;
        panel.frame = frame;
    });
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
        let handler = block2::RcBlock::new(move |_action: *mut AnyObject| send(act.clone()));
        let action: *mut AnyObject = msg_send![AnyClass::get(c"UIAction").unwrap(), actionWithTitle: ns_string(""), image: std::ptr::null_mut::<AnyObject>(), identifier: std::ptr::null_mut::<AnyObject>(), handler: &*handler];
        // 64 = UIControlEventTouchUpInside
        let _: () = msg_send![b, addAction: action, forControlEvents: 64usize];
        let _: () = msg_send![parent, addSubview: b];
        b
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
/// 把一块填上：颜色、渐变或图片，按 `radius` 剪圆角。
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
                let v: *mut AnyObject = msg_send![AnyClass::get(c"UIImageView").unwrap(), new];
                let _: () = msg_send![v, setFrame: frame];
                let image: *mut AnyObject = msg_send![AnyClass::get(c"UIImage").unwrap(), imageWithContentsOfFile: ns_string(path)];
                let _: () = msg_send![v, setImage: image];
                // 2 = UIViewContentModeScaleAspectFill
                let _: () = msg_send![v, setContentMode: 2isize];
                let _: () = msg_send![parent, addSubview: v];
                let _: () = msg_send![v, release];
                round(v, radius);
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

/// 胶带样张的撕口，纸边直接透出玻璃的底色。
unsafe fn torn_edges(view: *mut AnyObject, size: CGSize) {
    unsafe {
        let (w, h) = (size.width, size.height);
        let path: *mut AnyObject = msg_send![AnyClass::get(c"UIBezierPath").unwrap(), bezierPath];
        let _: () = msg_send![path, moveToPoint: CGPoint::new(3., 0.)];
        for (x, y) in [(w - 3., 0.), (w, h * 0.16), (w - 3.5, h * 0.35), (w - 1., h * 0.56), (w - 4., h * 0.72),
            (w, h * 0.89), (w - 2.5, h), (2., h), (0., h * 0.87), (3.5, h * 0.66), (1., h * 0.45), (4., h * 0.25), (0.5, h * 0.12)] {
            let _: () = msg_send![path, addLineToPoint: CGPoint::new(x, y)];
        }
        let _: () = msg_send![path, closePath];
        mask_path(view, size, msg_send![path, CGPath]);
    }
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

unsafe fn build(content: *mut AnyObject, nodes: &[Node], w: f64, offset: CGPoint) -> Option<*mut AnyObject> {
    unsafe {
        let pad = f64::from(PAD);
        let inner = w - pad * 2.;
        let (primary, secondary) = (ui(c"labelColor"), ui(c"secondaryLabelColor"));
        let well = ui(c"tertiarySystemFillColor");
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
                    let track = plain(content, rect(pad, y + 13., inner, 36.));
                    let _: () = msg_send![track, setBackgroundColor: well];
                    round(track, 18.);
                    let cell = inner / n as f64;
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
                    use crate::theme::Theme;
                    let width = (inner - 18.) / 4.;
                    let accent = super::glass::ui_color(Theme::primary());
                    let base = rect(pad, y + 10., inner, 47.);
                    fill(content, base, &Fill::Color(Theme::fill_hover()), 15.);
                    for (i, symbol) in ["rectangle.slash", "square.fill", "square.bottomhalf.filled", "photo"].into_iter().enumerate() {
                        let frame = rect(pad + 3. + (width + 4.) * i as f64, y + 13., width, 41.);
                        let on = i == *picked;
                        if on {
                            fill(content, frame, &Fill::Color(Theme::primary().opacity(0.12)), 12.);
                            ring(content, frame, 12., accent, 0., 0.65);
                        }
                        let ink = if on { accent } else { secondary };
                        icon(content, rect(frame.origin.x, y + 15., width, 20.), symbol, 14., ink);
                        let text = label(content, rect(frame.origin.x + 2., y + 34., width - 4., 17.), &titles[i], font(11., 0.24, ""), ink, 1);
                        let _: () = msg_send![text, setIsAccessibilityElement: false];
                        let button = named_tap(content, frame, &titles[i], format!("bg-tab:{i}"));
                        let _: () = msg_send![button, setSelected: on];
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
                                else { match f { Fill::Color(c) | Fill::Gradient(c, ..) => super::glass::ui_color(*c), Fill::Image(_) => primary } };
                            ring(content, frame, radius, tint, 0., if background { 1.5 } else { 2.5 });
                        } else { ring(content, frame, radius, ui(c"separatorColor"), 0., 0.5); }
                        let key = format!("{act}{i}");
                        if background {
                            let label = match f {
                                Fill::Color(c) => super::style::to_hex(*c),
                                Fill::Gradient(a, b, _) => format!("{} → {}", super::style::to_hex(*a), super::style::to_hex(*b)),
                                Fill::Image(_) => String::new(),
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
                    let widths = [(inner - 8.) * 0.6, (inner - 8.) * 0.4];
                    let accent = crate::theme::Theme::primary();
                    let tint = accent.opacity(if crate::theme::dark() { 0.18 } else { 0.10 });
                    let mut x = pad;
                    for (i, text) in labels.iter().take(2).enumerate() {
                        let selected = i == *picked;
                        let frame = rect(x, y, widths[i], 44.);
                        let pill = plain(content, frame);
                        let _: () = msg_send![pill, setBackgroundColor: well];
                        round(pill, 22.);
                        if selected { fill(content, frame, &Fill::Color(tint), 22.); }
                        let ink = if selected { super::glass::ui_color(accent) } else { secondary };
                        let caption = label(content, frame, text, font(17., if selected { 0.30 } else { 0. }, ""), ink, 1);
                        let _: () = msg_send![caption, setIsAccessibilityElement: false];
                        let button = named_tap(content, frame, text, format!("{act}{i}"));
                        let _: () = msg_send![button, setSelected: selected];
                        x += widths[i] + 8.;
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
                                        Some(f) => { fill(base, rect(0., TILE - 44., tw, 18.), f, 0.); }
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
                                    let shot = rect(x + 6., ty + 6., tw - 12., 68.);
                                    match path {
                                        Some(p) => { fill(content, shot, &Fill::Image(p.clone()), 8.); }
                                        None => { icon(content, shot, "photo", 26., secondary); }
                                    }
                                    label(content, rect(x + 6., ty + 82., tw - 12., 22.), &tile.title, font(14., 0.23, ""), secondary, 1);
                                }
                                Art::Fill(f) => {
                                    if let Some(f) = f { fill(content, frame, f, 16.); }
                                    // 铺了图或深色时字要看得见：字后面不另垫，颜色照亮度挑。
                                    let light = match f { Some(Fill::Color(c)) | Some(Fill::Gradient(c, ..)) => c.l > 0.6, Some(Fill::Image(_)) => false, None => !crate::theme::dark() };
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
                        if photo.picked {
                            let tick = rect(frame.origin.x + cw - 23., frame.origin.y + 4., 19., 19.);
                            let dot = plain(content, tick);
                            let _: () = msg_send![dot, setBackgroundColor: accent];
                            round(dot, 9.5);
                            icon(content, tick, "checkmark", 9., ui(c"whiteColor"));
                        }
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
                Node::StyleFooter { title, card, action } => {
                    use crate::theme::Theme;
                    fill(content, rect(0., y, w, h + 14.), &Fill::Color(Theme::fill_hover()), 0.);
                    let rule = plain(content, rect(pad, y, inner, 0.5));
                    let _: () = msg_send![rule, setBackgroundColor: ui(c"separatorColor")];
                    style_cards::preview(content, rect(pad, y + 11., 47., 47.), card, style_cards::Preview::Badge);
                    let button_w = action.as_ref().map_or(0., |(_, act)| if *act == "gallery-delete" { 110. } else { 76. });
                    let lw = (inner - 61. - button_w - 10.).max(20.);
                    label(content, rect(pad + 58., y + 11., lw, 18.), title, font(11., 0.23, ""), secondary, 0);
                    label(content, rect(pad + 58., y + 28., lw, 27.), &card.name, font(17., 0.33, ""), primary, 0);
                    if let Some((title, act)) = action {
                        let delete = *act == "gallery-delete";
                        let frame = rect(w - pad - button_w, y + 13., button_w, 44.);
                        fill(content, rect(frame.origin.x, frame.origin.y + 2., button_w, 40.),
                            &Fill::Color(if delete { Theme::danger_cell() } else { Theme::text() }), 20.);
                        let ink = super::glass::ui_color(if delete { Theme::danger() } else { Theme::bg_body() });
                        let text = label(content, frame, title, font(14., 0.31, ""), ink, 1);
                        let _: () = msg_send![text, setIsAccessibilityElement: false];
                        let button = tap(content, frame, (*act).into());
                        let _: () = msg_send![button, setAccessibilityLabel: ns_string(title)];
                        let _: () = msg_send![button, setAccessibilityIdentifier: ns_string(act)];
                    }
                }
                Node::Cards { items, picked, act } => {
                    let (cols, tw, th) = style_grid(w);
                    let accent = super::glass::ui_color(crate::theme::Theme::primary());
                    for (i, card) in items.iter().enumerate() {
                        let frame = rect(pad + (tw + 12.) * (i % cols) as f64, y + (th + 33.) * (i / cols) as f64, tw, th);
                        style_cards::preview(content, frame, card, style_cards::Preview::Gallery);
                        let on = *picked == Some(i);
                        if on {
                            ring(content, frame, 14., accent, 0., 1.5);
                            let check = rect(frame.origin.x + tw - 29., frame.origin.y + 7., 22., 22.);
                            let dot = plain(content, check);
                            let _: () = msg_send![dot, setBackgroundColor: accent];
                            round(dot, 11.);
                            icon(content, check, "checkmark", 10., ui(c"whiteColor"));
                        }
                        let text = label(content, rect(frame.origin.x + 2., frame.origin.y + th + 5., tw - 4., 21.), &card.name, font(14., 0.23, ""), if on { accent } else { primary }, 0);
                        let _: () = msg_send![text, setIsAccessibilityElement: false];
                        let button = tap(content, rect(frame.origin.x, frame.origin.y, tw, th + 28.), format!("{act}{i}"));
                        let _: () = msg_send![button, setAccessibilityLabel: ns_string(&card.name)];
                        let _: () = msg_send![button, setSelected: on];
                    }
                }
                Node::Strips { items, cols } => {
                    let cols = (*cols).max(1);
                    let (cw, rh) = ((inner - 8. * (cols - 1) as f64) / cols as f64, strip_h(cols));
                    let th = if cols == 1 { 34. } else { 30. };
                    for (i, strip) in items.iter().enumerate() {
                        let cell = rect(pad + (cw + 8.) * (i % cols) as f64, y + rh * (i / cols) as f64, cw, rh - 2.);
                        if strip.picked {
                            let base = plain(content, cell);
                            let _: () = msg_send![base, setBackgroundColor: well];
                            round(base, 12.);
                        }
                        let swatch = rect(cell.origin.x + 8., cell.origin.y + 6., cw - 16., th);
                        torn_edges(fill(content, swatch, &strip.fill, 0.), swatch.size);
                        label(content, rect(cell.origin.x + 4., cell.origin.y + 6. + th + 4., cw - 8., 16.), &strip.title, font(12., 0.23, ""), if strip.picked { super::glass::ui_color(crate::theme::Theme::primary()) } else { secondary }, if cols == 1 { 1 } else { 0 });
                        let button = tap(content, cell, strip.act.clone());
                        let _: () = msg_send![button, setAccessibilityLabel: ns_string(&strip.title)];
                        let _: () = msg_send![button, setSelected: strip.picked];
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
                    let n = if matches!(node, Node::Wide(_)) { items.len().max(1) } else { 3 };
                    let (tw, th) = ((inner - 5. * (n - 1) as f64) / n as f64, thumb_h(w));
                    for (i, (name, title, act)) in items.iter().enumerate() {
                        let frame = rect(pad + (tw + 5.) * i as f64, y, tw, th);
                        let base = plain(content, frame);
                        let _: () = msg_send![base, setBackgroundColor: well];
                        round(base, 10.);
                        icon(content, rect(frame.origin.x, y + th / 2. - 26., tw, 26.), name, 18., secondary);
                        let caption = label(content, rect(frame.origin.x + 4., y + th / 2. + 4., tw - 8., 18.), title, font(13., 0.23, ""), secondary, 1);
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
