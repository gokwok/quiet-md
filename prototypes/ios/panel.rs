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
    /// 「文本颜色」：深色的一格，字是当前的文字颜色。
    Ink(Hsla),
    /// 字体：大字是字体的名字，用它自己的字体写；下面一行小字。
    Font(String, &'static str),
    /// 封面：一张缩略图在上，名字在下；没有封面时是个图标。
    Cover(Option<String>),
    /// 整格铺满（背景、纸），名字在左上角。
    Fill(Option<Fill>),
    /// 名字在左上角，右下一个图标（样式、分隔符）。
    Symbol(&'static str),
    /// 纸：整格是纸的颜色，右下角折起一个角。
    Paper(Hsla),
    /// 样式：右边一叠小卡片，最上面那张是现在的样式（背景，纸，字，名字）。
    Stack(Option<Fill>, Hsla, Hsla, String),
    /// 分隔符：下面三条由细到粗的线；胶带时是一条胶带。
    Rules(Option<Fill>),
}

/// 样式图库里的一张卡片。
#[derive(Clone, PartialEq, Debug)]
pub struct Card { pub fill: Fill, pub paper: Hsla, pub ink: Hsla, pub name: String }

#[derive(Clone, PartialEq, Debug)]
pub enum Node {
    /// 主面板顶上的小横条。
    Grab,
    /// 标题栏：标题，对比度不够的警告，重置（动作），关闭（动作）。下面一条线。
    Header { title: String, warn: bool, reset: Option<String>, close: String },
    /// 一条分段：图标（SF Symbols）或文字，当前那一段是深色的胶囊。点了交出 `act` + 序号。
    Segments { symbols: Vec<&'static str>, titles: Vec<String>, picked: usize, act: String },
    /// 色点，一排六个。点了交出 `act` + 序号；`custom` 是最后那颗彩虹圈（动作，是否选中，圈里的颜色）。
    Swatches { fills: Vec<Fill>, picked: Option<usize>, act: String, custom: Option<(String, bool, Hsla)> },
    Caption(String),
    Row { label: String, trailing: Trailing },
    /// 并排的两颗胶囊，选中的那颗字是主色。
    Choice { labels: Vec<String>, picked: usize, act: String },
    Tiles(Vec<Vec<Tile>>),
    /// 缩略图，一排三张，选中的描边。点了交出 `act` + 序号。
    Thumbs { fills: Vec<Fill>, picked: Option<usize>, act: String },
    /// 样式的卡片，一排三张：背景上压一张小纸，纸上写名字；下面再写一遍。选中的描边。
    Cards { items: Vec<Card>, picked: Option<usize>, act: String },
    /// 一排带图标的方格按钮（图标，标题，动作）。
    Actions(Vec<(&'static str, String, String)>),
    /// 空着时的一句话。
    Empty(&'static str, String),
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
        Node::Swatches { fills, custom, .. } => rows(fills.len() + custom.is_some() as usize, 6) * PITCH + 4.,
        Node::Caption(_) => 30.,
        Node::Row { .. } => 50.,
        Node::Choice { .. } => 44. + 6.,
        Node::Tiles(rows) => rows.len() as f64 * (TILE + GAP) - GAP + 8.,
        Node::Thumbs { fills, .. } => rows(fills.len(), 3) * (thumb_h(w) + 5.),
        Node::Cards { items, .. } => rows(items.len(), 3) * (thumb_h(w) + 26.),
        Node::Actions(_) => thumb_h(w) + 5.,
        Node::Empty(..) => 110.,
    }
}
fn thumb_h(w: f64) -> f64 { ((w - PAD as f64 * 2. - 10.) / 3. * 0.68).round() }
/// 这一列内容要多高（宽 `w` 时），占位按它留。
pub fn height(nodes: &[Node], w: f32) -> f32 { (nodes.iter().map(|n| node_h(n, f64::from(w))).sum::<f64>() + if matches!(nodes.first(), Some(Node::Grab)) { 20. } else { 14. }) as f32 }

struct Panel { view: *mut AnyObject, frame: Bounds<Pixels>, nodes: Vec<Node>, dark: Option<bool>, shown: bool }
thread_local! { static PANEL: RefCell<Option<Panel>> = const { RefCell::new(None) }; }
thread_local! { static SEEN: Cell<bool> = const { Cell::new(false) }; }

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
pub fn slot(nodes: Vec<Node>) -> impl IntoElement {
    canvas(move |bounds, window, _| place(bounds, nodes.clone(), window), |_, _, _, _| {}).absolute().inset_0()
}

fn place(frame: Bounds<Pixels>, nodes: Vec<Node>, window: &Window) {
    let Ok(handle) = HasWindowHandle::window_handle(window) else { return; };
    let RawWindowHandle::UiKit(handle) = handle.as_raw() else { return; };
    let parent = super::shot::overlay(handle.ui_view.as_ptr() as *mut AnyObject);
    SEEN.with(|s| s.set(true));
    let dark = crate::theme::dark();
    PANEL.with(|p| unsafe {
        let mut slot = p.borrow_mut();
        if slot.is_none() {
            let (Some(effect_view), Some(glass)) = (AnyClass::get(c"UIVisualEffectView"), AnyClass::get(c"UIGlassEffect")) else { return; };
            let view: *mut AnyObject = msg_send![effect_view, alloc];
            let effect: *mut AnyObject = msg_send![glass, effectWithStyle: 0isize];
            let view: *mut AnyObject = msg_send![view, initWithEffect: effect];
            if let (Some(corners), Some(corner)) = (AnyClass::get(c"UICornerConfiguration"), AnyClass::get(c"UICornerRadius")) {
                let fixed: *mut AnyObject = msg_send![corner, fixedRadius: 30f64];
                let config: *mut AnyObject = msg_send![corners, configurationWithUniformRadius: fixed];
                let _: () = msg_send![view, setCornerConfiguration: config];
            }
            let _: () = msg_send![view, setHidden: true];
            let _: () = msg_send![parent, addSubview: view];
            *slot = Some(Panel { view, frame: Bounds::default(), nodes: vec![], dark: None, shown: false });
        }
        let Some(panel) = slot.as_mut() else { return; };
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
        if fresh || panel.frame.size.width != frame.size.width || panel.nodes != nodes {
            let content: *mut AnyObject = msg_send![panel.view, contentView];
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
            build(content, &nodes, f64::from(frame.size.width));
            panel.nodes = nodes;
        }
        panel.frame = frame;
    });
}

// ---- 构件

#[repr(C)]
struct CGColor { _private: [u8; 0] }
unsafe impl RefEncode for CGColor { const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("CGColor", &[])); }

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
                let v = plain(parent, frame);
                let layer: *mut AnyObject = msg_send![AnyClass::get(c"CAGradientLayer").unwrap(), layer];
                let colors: *mut AnyObject = msg_send![AnyClass::get(c"NSMutableArray").unwrap(), array];
                for c in [a, b] { let _: () = msg_send![colors, addObject: cg(super::glass::ui_color(*c)) as *mut AnyObject]; }
                let _: () = msg_send![layer, setColors: colors];
                let (from, to) = match dir { 1 => ((0., 0.5), (1., 0.5)), 2 => ((0., 0.), (1., 1.)), 3 => ((0., 1.), (1., 0.)), _ => ((0.5, 0.), (0.5, 1.)) };
                let _: () = msg_send![layer, setStartPoint: CGPoint::new(from.0, from.1)];
                let _: () = msg_send![layer, setEndPoint: CGPoint::new(to.0, to.1)];
                let _: () = msg_send![layer, setFrame: rect(0., 0., frame.size.width, frame.size.height)];
                let host: *mut AnyObject = msg_send![v, layer];
                let _: () = msg_send![host, addSublayer: layer];
                round(v, radius);
                v
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

unsafe fn build(content: *mut AnyObject, nodes: &[Node], w: f64) {
    unsafe {
        let pad = f64::from(PAD);
        let inner = w - pad * 2.;
        let (primary, secondary) = (ui(c"labelColor"), ui(c"secondaryLabelColor"));
        let well = ui(c"tertiarySystemFillColor");
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
                    tap(content, rect(x - 9., y + 10., 44., 44.), close.clone());
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
                    for i in 0..n {
                        let on = i == *picked;
                        let frame = rect(pad + cell * i as f64, y + 13., cell, 36.);
                        if on {
                            let pill = plain(content, frame);
                            let _: () = msg_send![pill, setBackgroundColor: ui(c"labelColor")];
                            round(pill, 18.);
                        }
                        let ink = if on { if crate::theme::dark() { ui(c"blackColor") } else { ui(c"whiteColor") } } else { secondary };
                        if let Some(name) = symbols.get(i) { icon(content, frame, name, 15., ink); }
                        else if let Some(title) = titles.get(i) { label(content, frame, title, font(15., 0.23, ""), ink, 1); }
                        tap(content, frame, format!("{act}{i}"));
                    }
                }
                Node::Swatches { fills, picked, act, custom } => {
                    let step = (inner - SWATCH) / 5.;
                    let at = |i: usize| rect(pad + step * (i % 6) as f64, y + (i / 6) as f64 * PITCH + 8., SWATCH, SWATCH);
                    for (i, f) in fills.iter().enumerate() {
                        let frame = at(i);
                        let on = *picked == Some(i);
                        // 选中的缩一圈，外面套一个同色的环。
                        let dot = if on { rect(frame.origin.x + 5., frame.origin.y + 5., SWATCH - 10., SWATCH - 10.) } else { frame };
                        fill(content, dot, f, dot.size.width / 2.);
                        if on { let tint = match f { Fill::Color(c) | Fill::Gradient(c, ..) => super::glass::ui_color(*c), Fill::Image(_) => primary }; ring(content, frame, SWATCH / 2., tint, 0., 2.5); }
                        else { ring(content, frame, SWATCH / 2., ui(c"separatorColor"), 0., 1. / 3.); }
                        tap(content, frame, format!("{act}{i}"));
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
                        let _: () = msg_send![layer, setFrame: rect(0., 0., SWATCH, SWATCH)];
                        let host: *mut AnyObject = msg_send![wheel, layer];
                        let _: () = msg_send![host, addSublayer: layer];
                        round(wheel, SWATCH / 2.);
                        let inset = 4.;
                        let core = rect(frame.origin.x + inset, frame.origin.y + inset, SWATCH - inset * 2., SWATCH - inset * 2.);
                        fill(content, core, &Fill::Color(if *on { *color } else { crate::theme::Theme::bg_body() }), core.size.width / 2.);
                        let ink = if *on && color.l > 0.6 { ui(c"blackColor") } else { primary };
                        icon(content, core, "chevron.down", 12., ink);
                        tap(content, frame, act.clone());
                    }
                }
                Node::Caption(text) => { label(content, rect(pad, y + 8., inner, 18.), text, font(13., 0., ""), secondary, 0); }
                Node::Row { label: text, trailing } => {
                    label(content, rect(pad, y, inner * 0.5, 50.), text, font(17., 0., ""), primary, 0);
                    match trailing {
                        Trailing::Chip(f, act) => {
                            let frame = rect(w - pad - 54., y + 4., 54., 42.);
                            fill(content, frame, f, 9.);
                            tap(content, frame, act.clone());
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
                            tap(content, frame, act.clone());
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
                            let _: () = msg_send![content, addSubview: b];
                        }
                    }
                }
                Node::Choice { labels, picked, act } => {
                    // 实测：前一颗占六成。
                    let widths = [(inner - 8.) * 0.6, (inner - 8.) * 0.4];
                    let mut x = pad;
                    for (i, text) in labels.iter().take(2).enumerate() {
                        let frame = rect(x, y, widths[i], 44.);
                        let pill = plain(content, frame);
                        let _: () = msg_send![pill, setBackgroundColor: well];
                        round(pill, 22.);
                        let ink = if i == *picked { super::glass::ui_color(crate::theme::Theme::primary()) } else { secondary };
                        label(content, frame, text, font(17., if i == *picked { 0.23 } else { 0. }, ""), ink, 1);
                        tap(content, frame, format!("{act}{i}"));
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
                                Art::Ink(c) => {
                                    // 笔尖从格子底下探出来一截（裁在格子里）。
                                    let _: () = msg_send![base, setBackgroundColor: ui(c"systemBackgroundColor")];
                                    label(base, rect(6., 22., tw - 12., 30.), &tile.title, font(20., 0.3, ""), super::glass::ui_color(*c), 1);
                                    icon(base, rect(0., TILE - 58., tw, 80.), "pencil.tip", 62., super::glass::ui_color(*c));
                                }
                                Art::Paper(c) => {
                                    let _: () = msg_send![base, setBackgroundColor: super::glass::ui_color(*c)];
                                    let dark = c.l < 0.5;
                                    label(base, rect(12., 8., tw - 24., 22.), &tile.title, font(14., 0.23, ""), if dark { ui(c"whiteColor") } else { ui(c"darkGrayColor") }, 0);
                                    // 折起来的角：一块比纸亮一点的圆角方片压在右下，带一点影子。
                                    let fold = plain(base, rect(tw - 44., TILE - 44., 64., 64.));
                                    let _: () = msg_send![fold, setBackgroundColor: super::glass::ui_color(Hsla { l: if dark { c.l + 0.09 } else { (c.l - 0.05).max(0.) }, ..*c })];
                                    let layer: *mut AnyObject = msg_send![fold, layer];
                                    let _: () = msg_send![layer, setCornerRadius: 20f64];
                                    let _: () = msg_send![layer, setShadowOpacity: 0.28f32];
                                    let _: () = msg_send![layer, setShadowRadius: 7f64];
                                    let _: () = msg_send![layer, setShadowOffset: CGSize::new(-2., -2.)];
                                }
                                Art::Stack(f, paper, ink, name) => {
                                    label(base, rect(12., 8., tw - 24., 22.), &tile.title, font(14., 0.23, ""), secondary, 0);
                                    // 后面两张只露出右边一条，越往后越矮。
                                    let (cw, ch) = (78., 112.);
                                    let (cx0, cy0) = (tw - cw - 62., 30.);
                                    for (dx, dy, alpha) in [(34., 16., 0.35), (17., 8., 0.6)] {
                                        let back = plain(base, rect(cx0 + dx, cy0 + dy, cw, ch));
                                        let color: *mut AnyObject = msg_send![ui(c"whiteColor"), colorWithAlphaComponent: alpha];
                                        let _: () = msg_send![back, setBackgroundColor: color];
                                        round(back, 9.);
                                    }
                                    let front = plain(base, rect(cx0, cy0, cw, ch));
                                    let _: () = msg_send![front, setBackgroundColor: super::glass::ui_color(*paper)];
                                    round(front, 9.);
                                    let layer: *mut AnyObject = msg_send![front, layer];
                                    let _: () = msg_send![layer, setBorderWidth: 0.5f64];
                                    let _: () = msg_send![layer, setBorderColor: cg(ui(c"separatorColor"))];
                                    let text = label(front, rect(8., 7., cw - 16., 30.), name, font(10., 0.4, ""), super::glass::ui_color(*ink), 0);
                                    let _: () = msg_send![text, setNumberOfLines: 2isize];
                                    if let Some(f) = f { fill(front, rect(0., 44., cw, 26.), f, 0.); }
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
                            tap(content, frame, tile.act.clone());
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
                Node::Cards { items, picked, act } => {
                    let (tw, th) = ((inner - 10.) / 3., thumb_h(w));
                    for (i, card) in items.iter().enumerate() {
                        let frame = rect(pad + (tw + 5.) * (i % 3) as f64, y + (th + 26.) * (i / 3) as f64, tw, th);
                        fill(content, frame, &card.fill, 10.);
                        let sheet = plain(content, rect(frame.origin.x + tw * 0.3, frame.origin.y + th * 0.2, tw * 0.4, th * 0.8));
                        let _: () = msg_send![sheet, setBackgroundColor: super::glass::ui_color(card.paper)];
                        round(sheet, 5.);
                        label(content, rect(frame.origin.x + tw * 0.3 + 4., frame.origin.y + th * 0.2 + 5., tw * 0.4 - 8., 10.), &card.name, font(7., 0.4, ""), super::glass::ui_color(card.ink), 0);
                        let on = *picked == Some(i);
                        if on { ring(content, frame, 10., super::glass::ui_color(crate::theme::Theme::primary()), 0., 2.); }
                        label(content, rect(frame.origin.x, frame.origin.y + th + 3., tw, 16.), &card.name, font(12., 0.23, ""), if on { super::glass::ui_color(crate::theme::Theme::primary()) } else { secondary }, 0);
                        tap(content, frame, format!("{act}{i}"));
                    }
                }
                Node::Actions(items) => {
                    let (tw, th) = ((inner - 10.) / 3., thumb_h(w));
                    for (i, (name, title, act)) in items.iter().enumerate() {
                        let frame = rect(pad + (tw + 5.) * i as f64, y, tw, th);
                        let base = plain(content, frame);
                        let _: () = msg_send![base, setBackgroundColor: well];
                        round(base, 10.);
                        icon(content, rect(frame.origin.x, y + th / 2. - 26., tw, 26.), name, 18., secondary);
                        label(content, rect(frame.origin.x + 4., y + th / 2. + 4., tw - 8., 18.), title, font(13., 0.23, ""), secondary, 1);
                        tap(content, frame, act.clone());
                    }
                }
                Node::Empty(name, text) => {
                    icon(content, rect(pad, y + 22., inner, 30.), name, 24., ui(c"tertiaryLabelColor"));
                    label(content, rect(pad, y + 58., inner, 20.), text, font(15., 0., ""), secondary, 1);
                }
            }
            y += h;
        }
    }
}

/// 控件被点了：回到 GPUI，交给 `stylepanel::action`。
fn send(act: String) { super::menu::dispatch(move |w, window, cx| super::stylepanel::action(&act, w, window, cx)); }

unsafe fn ns_string(s: &str) -> *mut AnyObject {
    let c = std::ffi::CString::new(s).unwrap_or_default();
    unsafe { msg_send![AnyClass::get(c"NSString").unwrap(), stringWithUTF8String: c.as_ptr()] }
}
