//! 文档样式：一篇文档自己的文字颜色、字体、纸的颜色、纸后面的背景和分隔符的样子（照 Craft 的「样式」）。
//!
//! 存在文档库索引里（`Meta::style`，`store::DocStyle`），不动文档文件。画的时候分两路：主题表达得了的
//! （文字、纸、字体）每帧盖到所选主题上（`themes::overlay`），正文照常读主题；主题表达不了的（背景、和纸胶带）这里另画。
//! 没设样式的文档这两路都不走，和原来一模一样。面板在 `stylepanel.rs`。
use std::cell::RefCell;

use gpui::{prelude::FluentBuilder as _, *};

use crate::app::{View, Workspace};
use crate::model::Rule;
use crate::store::{Backdrop, DividerStyle, DocStyle};

thread_local! { static CUR: RefCell<Option<DocStyle>> = const { RefCell::new(None) }; }

/// 纸两侧露出背景的宽度（Craft 实测 40px@3x）。
pub const SIDE: f32 = 13.;
/// 有背景时，纸是一张卡片：顶上两个圆角，停在顶栏下面这么远（Craft 截图量的：顶栏按钮底到纸顶 27pt 左右，圆角约 28pt）。往上滚时卡片跟着走，到顶栏底下被裁掉。
pub const GAP: f32 = 24.;
pub const RADIUS: f32 = 28.;
/// 有背景时封面嵌在纸里，四周留这么多（Craft 约 20pt），圆角 10。
pub const COVER_INSET: f32 = 20.;

/// `#rrggbb` → 颜色。
pub fn hex(s: &str) -> Option<Hsla> {
    let v = u32::from_str_radix(s.strip_prefix('#')?, 16).ok().filter(|_| s.len() == 7)?;
    Some(rgb(v).into())
}
pub fn to_hex(c: Hsla) -> String {
    let c = Rgba::from(c);
    let b = |v: f32| (v.clamp(0., 1.) * 255.).round() as u32;
    format!("#{:02X}{:02X}{:02X}", b(c.r), b(c.g), b(c.b))
}

pub(super) fn contrast(a: Hsla, b: Hsla) -> f32 {
    let luminance = |color: Hsla| {
        let c = Rgba::from(color);
        let linear = |v: f32| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
        0.2126 * linear(c.r) + 0.7152 * linear(c.g) + 0.0722 * linear(c.b)
    };
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

/// 字体的名字 → 系统里的字体族。
pub fn family(name: &str) -> Option<&'static str> {
    let f = match name { "serif" => Some(".AppleSystemUIFontSerif"), "mono" => Some(crate::themes::SYSTEM_MONO), "rounded" => Some(".AppleSystemUIFontRounded"), _ => None };
    // 在这里（画之前）就开始下中文字体，编辑器这一帧画完能看到正在下载、下好后重画。
    if let Some(f) = f { cjk::fallback(f); }
    f
}

/// 系统的衬线、圆体只有西文字形，中文会退回苹方，看着像没换字体。iOS 没有自带的中文宋体、圆体，
/// 用系统按需下载的「宋体-简」「圆体-简」（第一次用到时由系统下载，之后一直在）：下好之前照旧是苹方，
/// 下好了排在后备字体最前面，再重画一次。等宽没有中文的，中文本来就等宽，仍用苹方。
pub mod cjk {
    use std::cell::Cell;
    use std::sync::atomic::{AtomicU8, AtomicU32, Ordering::Relaxed};

    use gpui::Context;
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject, Bool};

    /// 西文字体族 → 中文后备字体族和要下载的字重。
    const FACES: [(&str, &str, [&str; 2]); 2] = [
        (".AppleSystemUIFontSerif", "Songti SC", ["STSongti-SC-Regular", "STSongti-SC-Bold"]),
        (".AppleSystemUIFontRounded", "Yuanti SC", ["STYuanti-SC-Regular", "STYuanti-SC-Bold"]),
    ];
    const IDLE: u8 = 0;
    const LOADING: u8 = 1;
    const READY: u8 = 2;
    const FAILED: u8 = 3;
    static STATE: [AtomicU8; 2] = [AtomicU8::new(IDLE), AtomicU8::new(IDLE)];
    /// 每下好一种加一：编辑器的排版缓存带上它，下好后重新排。
    static GEN: AtomicU32 = AtomicU32::new(0);
    thread_local! { static WATCHING: Cell<bool> = const { Cell::new(false) }; }

    #[link(name = "CoreText", kind = "framework")]
    unsafe extern "C" {
        fn CTFontDescriptorCreateWithNameAndSize(name: *const AnyObject, size: f64) -> *mut AnyObject;
        fn CTFontDescriptorMatchFontDescriptorsWithProgressHandler(descriptors: *const AnyObject, mandatory: *const AnyObject, handler: &block2::Block<dyn Fn(u32, *const AnyObject) -> Bool>) -> bool;
    }

    /// `family` 的中文后备字体（还没下好时是 None，并开始下载）。
    pub fn fallback(family: &str) -> Option<&'static str> {
        let i = FACES.iter().position(|f| f.0 == family)?;
        match STATE[i].load(Relaxed) {
            READY => Some(FACES[i].1),
            IDLE => { fetch(i); None }
            _ => None,
        }
    }

    pub fn generation() -> u32 { GEN.load(Relaxed) }

    fn loading() -> bool { STATE.iter().any(|s| s.load(Relaxed) == LOADING) }

    /// 有字体正在下载时，下好后让这个视图重画一次。
    pub fn watch<T: 'static>(cx: &mut Context<T>) {
        if !loading() || WATCHING.with(|w| w.replace(true)) { return; }
        cx.spawn(async move |this, cx| {
            while loading() { cx.background_executor().timer(std::time::Duration::from_millis(300)).await; }
            WATCHING.with(|w| w.set(false));
            let _ = this.update(cx, |_, cx| cx.notify());
        }).detach();
    }

    fn fetch(i: usize) {
        if STATE[i].compare_exchange(IDLE, LOADING, Relaxed, Relaxed).is_err() { return; }
        // CTFontDescriptorMatchingState：DidFinish = 1，DidMatch = 7。已经在设备上的字体也走这一趟，很快就回来。
        let matched = std::sync::Arc::new(AtomicU8::new(0));
        let handler: block2::RcBlock<dyn Fn(u32, *const AnyObject) -> Bool> = block2::RcBlock::new(move |state: u32, _: *const AnyObject| -> Bool {
            if state == 7 { matched.store(1, Relaxed); }
            if state == 1 {
                STATE[i].store(if matched.load(Relaxed) == 1 { READY } else { FAILED }, Relaxed);
                GEN.fetch_add(1, Relaxed);
            }
            Bool::YES
        });
        unsafe {
            let list: *mut AnyObject = msg_send![AnyClass::get(c"NSMutableArray").unwrap(), array];
            for name in FACES[i].2 {
                let desc = CTFontDescriptorCreateWithNameAndSize(crate::ios::glass::ns_string(name), 0.);
                if desc.is_null() { continue; }
                let _: () = msg_send![list, addObject: desc];
                let _: () = msg_send![desc, release];
            }
            if !CTFontDescriptorMatchFontDescriptorsWithProgressHandler(list, std::ptr::null(), &handler) { STATE[i].store(FAILED, Relaxed); }
        }
    }
}

/// 打开着的那篇文档的样式（在文档页时）。
pub fn of(w: &Workspace) -> Option<DocStyle> {
    if w.view != View::Doc { return None; }
    let id = w.current.as_ref()?;
    w.entries.iter().find(|e| &e.id == id)?.meta.style.clone()
}

/// 每帧开头调用：这一帧画的是哪篇文档，就盖上它的样式；别的页回到主题。按这一帧的状态推，不在开关文档时切，
/// 跟手返回一开始就换了页，主页不能带着文档的样式画。
pub fn begin(w: &Workspace) {
    // 助手也要知道这一帧画的是哪篇文档（它的预览和底部玻璃只在发起它的那篇里出）。
    super::assist::frame(if w.view == View::Doc { w.current.clone() } else { None });
    let style = of(w);
    let scale = if w.view == View::Doc { w.current.as_ref().and_then(|id| w.entries.iter().find(|e| &e.id == id)).map_or(0, |e| size_percent(e.meta.text_size)) } else { 0 };
    FACTOR.with(|f| f.set(if scale == 0 { 1. } else { scale as f32 / 100. }));
    let mut over = crate::themes::Over { scale, ..Default::default() };
    if let Some(s) = style.as_ref() {
        if let Some(c) = s.text.as_deref().and_then(hex) { over.tokens.push(("text", c)); }
        if let Some(c) = s.paper.as_deref().and_then(hex) { over.tokens.push(("bg_body", c)); }
        over.font = s.font.as_deref().and_then(family);
    }
    crate::themes::overlay(Some(over));
    // 纸换了颜色：这一页的深浅色跟着纸走，不跟应用。
    let paper = style.as_ref().and_then(|s| s.paper.as_deref()).and_then(hex).map(|c| { let c = Rgba::from(c); 0.299 * c.r + 0.587 * c.g + 0.114 * c.b < 0.5 });
    if crate::theme::set_doc_dark(paper) { crate::themes::touch(); }
    CUR.with(|c| *c.borrow_mut() = style);
}

/// 文字大小的八档（`Meta::text_size` 1–8，0 和 3 都是原样）：字号和行高乘上的百分数。
pub const SIZES: [u16; 8] = [86, 93, 100, 108, 117, 127, 138, 150];
thread_local! { static FACTOR: std::cell::Cell<f32> = const { std::cell::Cell::new(1.) }; }
/// 这一帧的文字放大了多少倍（没调过是 1）。
pub fn size_factor() -> f32 { FACTOR.with(|f| f.get()) }
pub fn size_percent(step: u8) -> u16 { match step { 0 | 3 => 0, n => SIZES[(n as usize).min(8) - 1] } }

thread_local! { static SIZING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
/// 文字大小那颗胶囊开着没有。
pub fn sizing() -> bool { SIZING.with(|s| s.get()) }
pub fn set_sizing(on: bool) { SIZING.with(|s| s.set(on)); }
/// 文字大小：屏幕中间浮出的一颗胶囊，八个由小到大的圆点，当前那档是主色；背后略压暗，点别处收起。
/// 点一颗或按住在胶囊上左右滑都能换档，每换一档轻轻震一下（照系统的选择触感）。
pub fn size_capsule(w: &Workspace, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    if !sizing() || w.view != View::Doc { set_sizing(false); return None; }
    let id = w.current.clone()?;
    let now = match w.entries.iter().find(|e| e.id == id)?.meta.text_size { 0 => 3, n => n.min(8) };
    let theme = crate::theme::Theme::bg_overlay;
    // 胶囊居中：左右各 14 的边，八格各 34 宽。按下的横坐标换算成第几档。
    const CELL: f32 = 34.;
    const EDGE: f32 = 14.;
    let step_at = |x: Pixels, window: &Window| -> u8 {
        let left = (f32::from(window.viewport_size().width) - (EDGE * 2. + CELL * 8.)) / 2. + EDGE;
        (((f32::from(x) - left) / CELL).floor().clamp(0., 7.) as u8) + 1
    };
    let set = move |w: &mut Workspace, id: &str, step: u8, cx: &mut Context<Workspace>| {
        let now = match w.entries.iter().find(|e| e.id == id).map(|e| e.meta.text_size) { Some(0) | None => 3, Some(n) => n.min(8) };
        if step == now { return; }
        // 第三档就是原样，不记。
        let stored = if step == 3 { 0 } else { step };
        if let Err(e) = w.store.update_meta(id, |m| m.text_size = stored) { return w.show_toast_kind(crate::ui::ToastKind::Error, &e.to_string(), cx); }
        if let Some(e) = w.entries.iter_mut().find(|e| e.id == id) { e.meta.text_size = stored; }
        super::glass::selection_haptic();
        cx.notify();
    };
    let mut row = div().id("text-size").occlude().h(px(60.)).px(px(EDGE)).rounded_full().bg(crate::theme::raised(theme())).shadow(crate::theme::shadow_dialog()).flex().items_center();
    for step in 1..=8u8 {
        let d = 7. + 2.2 * (step - 1) as f32;
        row = row.child(div().w(px(CELL)).h(px(60.)).flex().items_center().justify_center()
            .child(div().size(px(d)).rounded_full().bg(if step == now { crate::theme::Theme::primary() } else { crate::theme::Theme::text_disabled() })));
    }
    let (down, moved) = (id.clone(), id);
    let row = row
        .on_mouse_down(MouseButton::Left, cx.listener(move |w, ev: &MouseDownEvent, window, cx| { cx.stop_propagation(); set(w, &down, step_at(ev.position.x, window), cx); }))
        // 手指在胶囊上平移，GPUI 给的是滚动事件：位置一直是按下的那点，手指走了多少在 delta 里，自己累加出手指现在的横坐标。
        .on_scroll(cx.listener(move |w, ev: &ScrollEvent, window, cx| {
            cx.stop_propagation();
            thread_local! { static FINGER: std::cell::Cell<f32> = const { std::cell::Cell::new(0.) }; }
            if ev.source != ScrollSource::Touch { return; }
            let dx = match ev.delta { ScrollDelta::Pixels(d) => f32::from(d.x), ScrollDelta::Lines(d) => d.x * 20. };
            let x = FINGER.with(|f| { let x = if ev.touch_phase == TouchPhase::Started { f32::from(ev.position.x) + dx } else { f.get() + dx }; f.set(x); x });
            if matches!(ev.touch_phase, TouchPhase::Started | TouchPhase::Moved) { set(w, &moved, step_at(px(x), window), cx); }
        }))
        .on_click(|_, _, cx| cx.stop_propagation());
    Some(deferred(div().id("text-size-mask").absolute().inset_0().occlude().bg(black().opacity(0.12)).flex().items_center().justify_center()
        .on_click(cx.listener(|_, _, _, cx| { set_sizing(false); cx.notify(); })).child(row)).with_priority(3).into_any_element())
}

/// 这一帧的文档样式。
pub fn current() -> Option<DocStyle> { CUR.with(|c| c.borrow().clone()) }
/// 有背景：纸两侧内缩，上沿让到顶栏下面，背景从四周露出来。
pub fn banded() -> bool { CUR.with(|c| c.borrow().as_ref().is_some_and(|s| s.background.is_some())) }

// 淡化保留照片的层次和渐变的色差。正文、顶栏和缩略页用同样的纸色比例。
pub(super) const FADED_PAPER_OPACITY: f32 = 0.36;

/// 有背景时顶栏上的字（标题、文件夹）用什么颜色：背景上沿深就用白的，浅就用深的。没有背景时照主题。
pub fn chrome_ink() -> Option<(Hsla, Hsla)> {
    let style = current()?;
    let top = match style.background.as_ref()? {
        Backdrop::Color { color } => hex(color)?,
        Backdrop::Gradient { from, to, dir } => if *dir == 0 || *dir == 2 { hex(from)? } else { let (a, b) = (Rgba::from(hex(from)?), Rgba::from(hex(to)?)); Rgba { r: (a.r + b.r) / 2., g: (a.g + b.g) / 2., b: (a.b + b.b) / 2., a: 1. }.into() },
        Backdrop::Image { src, .. } => {
            let built_in = src.strip_prefix("builtin:").and_then(|name| photo_background(name).map(|p| p.top).or_else(|| recipe(name).map(|r| r.0)));
            built_in.map(|value| rgb(value).into()).or_else(|| sampled_top(&image_path(src)?)).unwrap_or(black())
        }
    };
    let c = Rgba::from(top);
    let mut l = 0.299 * c.r + 0.587 * c.g + 0.114 * c.b;
    if style.faded {
        let p = Rgba::from(crate::editor_theme::Theme::bg_body());
        l = l * (1. - FADED_PAPER_OPACITY) + (0.299 * p.r + 0.587 * p.g + 0.114 * p.b) * FADED_PAPER_OPACITY;
    }
    Some(if l < 0.55 { (white(), white().opacity(0.7)) } else { (rgb(0x202225).into(), Hsla::from(rgb(0x202225)).opacity(0.6)) })
}

/// 渐变的方向 → 角度。
fn angle(dir: u8) -> f32 { match dir { 1 => 90., 2 => 135., 3 => 45., _ => 180. } }

/// 纸后面的背景：铺满文档页。淡化时上面盖一层纸色。
pub fn backdrop() -> Option<AnyElement> { backdrop_of(&current()?) }
/// 主页缩略图的纸色和字色：跟文档的样式；只换了纸没换字时，字按纸的深浅取。没设的照主题。
pub fn card_look(style: Option<&DocStyle>) -> (Option<Hsla>, Option<Hsla>) {
    let Some(s) = style else { return (None, None) };
    let paper = s.paper.as_deref().and_then(hex);
    let ink = s.text.as_deref().and_then(hex).or_else(|| paper.and_then(|p| {
        let (p, t) = (Rgba::from(p), Rgba::from(crate::editor_theme::Theme::text()));
        let l = |c: Rgba| 0.299 * c.r + 0.587 * c.g + 0.114 * c.b;
        // 主题的字在这张纸上看不清才换。
        ((l(p) < 0.5) == (l(t) < 0.5)).then(|| if l(p) < 0.5 { rgb(0xebebeb).into() } else { rgb(0x1f2329).into() })
    }));
    (paper, ink)
}

/// 某个样式的背景（主页的缩略图外面那一圈也用它）。
pub fn backdrop_of(style: &DocStyle) -> Option<AnyElement> {
    let fill = div().absolute().inset_0();
    let fill = match style.background.as_ref()? {
        Backdrop::Color { color } => fill.bg(hex(color)?),
        Backdrop::Gradient { from, to, dir } => fill.bg(linear_gradient(angle(*dir), linear_color_stop(hex(from)?, 0.), linear_color_stop(hex(to)?, 1.))),
        Backdrop::Image { src, blur } => {
            let path = image_path(src)?;
            let path = if *blur { blurred(&path).unwrap_or(path) } else { path };
            fill.child(img(std::path::PathBuf::from(path)).size_full().object_fit(ObjectFit::Cover))
        }
    };
    let paper = style.paper.as_deref().and_then(hex).unwrap_or_else(crate::editor_theme::Theme::bg_body);
    Some(fill.when(style.faded, |d| d.child(div().absolute().inset_0().bg(paper.opacity(FADED_PAPER_OPACITY)))).into_any_element())
}

// ---- 背景图片

/// 内置的背景：程序画的几张柔光色块（自己配的，不是照搬的图片），第一次用到时画出来放在缓存目录里。
pub const BUILTIN: [&str; 9] = ["dusk", "peach", "ocean", "lagoon", "mist", "lilac", "linen", "moss", "ink"];

pub(super) struct PhotoBackdrop { pub id: &'static str, pub zh: &'static str, pub en: &'static str, pub(super) top: u32, pub(super) image: &'static [u8] }
macro_rules! photo {
    ($id:literal, $zh:literal, $en:literal, $top:literal) => {
        PhotoBackdrop { id: $id, zh: $zh, en: $en, top: $top, image: include_bytes!(concat!("backdrops/", $id, ".jpg")) }
    };
    // Claude 的样式的背景：`claude/draw.py` 画的，名字前面加 claude-。
    (claude $id:literal, $zh:literal, $en:literal, $top:literal) => {
        PhotoBackdrop { id: concat!("claude-", $id), zh: $zh, en: $en, top: $top, image: include_bytes!(concat!("claude/backdrops/", $id, ".jpg")) }
    };
}

// Margin 原创照片。颜色是各图顶部七分之一的取样，用来选择文档顶栏的字色。
pub(super) const PHOTO_BACKGROUNDS: [PhotoBackdrop; 6] = [
    photo!("reed-shore", "芦岸晨光", "Reed shore", 0xe2c7b4),
    photo!("tea-shadow", "茶影", "Tea shadow", 0x7b6b56),
    photo!("basalt-coast", "玄武海岸", "Basalt coast", 0xb9c5d7),
    photo!("paper-study", "书桌素纸", "Paper study", 0x81684d),
    photo!("fern-rain", "雨后蕨叶", "Ferns after rain", 0x697544),
    photo!("violet-ridge", "暮紫山脊", "Violet ridge", 0xc3bdcf),
];

// `stylepanel::claude_presets` 用的十二张（不在背景图库里单独列出）。颜色同上，是顶部的取样，构成那张手调成浅色（顶上是米色底上的色块）。
pub(super) const CLAUDE_BACKGROUNDS: [PhotoBackdrop; 12] = [
    photo!(claude "riso", "孔版", "Riso Press", 0xd18b64),
    photo!(claude "contour", "等高线", "Contour", 0x1e3c1c),
    photo!(claude "cyanotype", "蓝晒", "Cyanotype", 0x5077a7),
    photo!(claude "suminagashi", "墨流", "Suminagashi", 0x564845),
    photo!(claude "terrazzo", "水磨石", "Terrazzo", 0xc9a5a7),
    photo!(claude "sunburst", "日冕", "Sunburst", 0xef8787),
    photo!(claude "tideline", "青海波", "Tideline", 0x2d2f57),
    photo!(claude "phosphor", "荧屏", "Phosphor", 0x102510),
    photo!(claude "primary", "构成", "Primary", 0xd8c8a8),
    photo!(claude "ticket", "票根", "Ticket Stub", 0x036e45),
    photo!(claude "nocturne", "夜曲", "Nocturne", 0x2a0f0d),
    photo!(claude "moire", "莫尔", "Interference", 0x481943),
];

pub(super) fn photo_background(name: &str) -> Option<&'static PhotoBackdrop> {
    PHOTO_BACKGROUNDS.iter().chain(super::astra::STYLES.iter().map(|s| &s.background)).chain(CLAUDE_BACKGROUNDS.iter()).find(|p| p.id == name)
}

/// 一张内置背景：底色，几团光（颜色，圆心 x、y，半径，浓度；位置和半径按画幅的比例），颗粒有多重。
type Recipe = (u32, &'static [(u32, f32, f32, f32, f32)], i32);
fn recipe(name: &str) -> Option<Recipe> {
    Some(match name {
        "dusk" => (0x1a1638, &[(0x4a2f9a, 0.85, 0.05, 0.75, 1.), (0xd8433f, 0.2, 0.55, 0.6, 0.95), (0xff9a52, 0.05, 0.8, 0.45, 0.9), (0x9a2f78, 0.65, 0.4, 0.5, 0.8), (0x120f2a, 1., 1., 0.7, 1.)], 9),
        "peach" => (0xffd9c4, &[(0xffb3a1, 0.1, 0.15, 0.7, 0.95), (0xffe6a8, 0.9, 0.3, 0.65, 0.9), (0xf7a8c4, 0.75, 0.9, 0.7, 0.85), (0xfff3e6, 0.4, 0.55, 0.4, 0.7)], 5),
        "ocean" => (0x0f2f5c, &[(0x2a7fd6, 0.2, 0.1, 0.75, 0.95), (0x35c2c9, 0.9, 0.45, 0.6, 0.75), (0x0b1d44, 0.15, 0.95, 0.75, 1.), (0x7fb6ff, 0.6, 0.05, 0.4, 0.55)], 7),
        "lagoon" => (0xcdeee3, &[(0x8fdcc8, 0.15, 0.2, 0.7, 0.9), (0xa8d8f0, 0.9, 0.15, 0.6, 0.85), (0xf2f7c9, 0.8, 0.85, 0.6, 0.8), (0xe9fbf4, 0.35, 0.65, 0.45, 0.7)], 5),
        "mist" => (0xd3ccd8, &[(0xeeb0a8, 0.25, 0.45, 0.6, 0.85), (0xa5bfd2, 0.9, 0.25, 0.6, 0.9), (0xf5efe9, 0.6, 0.9, 0.55, 0.85), (0xb990a6, 0.05, 0.05, 0.45, 0.7)], 5),
        "lilac" => (0xd9cdf5, &[(0xb7a4f0, 0.15, 0.1, 0.7, 0.9), (0xf3c6e6, 0.9, 0.35, 0.6, 0.85), (0xc6dcff, 0.3, 0.9, 0.65, 0.85), (0xf4eefe, 0.6, 0.5, 0.35, 0.6)], 5),
        "linen" => (0xe7dfcf, &[(0xf6f1e6, 0.3, 0.25, 0.75, 0.9), (0xd3c6ab, 0.95, 0.95, 0.65, 0.8), (0xefe4cf, 0.8, 0.15, 0.4, 0.6)], 10),
        "moss" => (0x4f5a2a, &[(0x9aa24a, 0.35, 0.3, 0.65, 0.9), (0x2c3a1c, 0.95, 0.95, 0.7, 1.), (0xc2b85a, 0.05, 0.85, 0.4, 0.6), (0x6f8a4a, 0.9, 0.1, 0.45, 0.7)], 8),
        "ink" => (0x0e1016, &[(0x24324f, 0.2, 0.15, 0.7, 0.95), (0x3a2a55, 0.9, 0.55, 0.6, 0.7), (0x07080c, 0.5, 1., 0.7, 1.), (0x41608f, 0.75, 0.05, 0.3, 0.45)], 8),
        _ => return None,
    })
}

fn cache_dir() -> std::path::PathBuf { std::env::temp_dir().join("quiet-md-backdrops") }

/// 背景图片的文件：内置的（`builtin:<名字>`，没画过就现画）或文档库里的文件。
pub fn image_path(src: &str) -> Option<String> {
    // 自己挑的照片：文件在文档库的 backdrops 目录里，索引里只记文件名（iOS 上沙盒的路径每次安装都变）。
    if let Some(file) = src.strip_prefix("file:") { let path = photos_dir().join(file); return path.exists().then(|| path.to_string_lossy().to_string()); }
    let name = src.strip_prefix("builtin:")?;
    if let Some(photo) = photo_background(name) {
        let path = cache_dir().join(format!("{name}-1.jpg"));
        if !path.is_file() {
            std::fs::create_dir_all(cache_dir()).ok()?;
            let temp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
            if std::fs::write(&temp, photo.image).and_then(|()| std::fs::rename(&temp, &path)).is_err() {
                let _ = std::fs::remove_file(temp);
                return None;
            }
        }
        return Some(path.to_string_lossy().to_string());
    }
    let path = cache_dir().join(format!("{name}-3.png"));
    if !path.exists() {
        use resvg::tiny_skia::*;
        let (base, blobs, grain) = recipe(name)?;
        let (w, h) = (540u32, 1170u32);
        let mut pm = Pixmap::new(w, h)?;
        let rgb = |v: u32, a: f32| Color::from_rgba8((v >> 16) as u8, (v >> 8) as u8, v as u8, (a.clamp(0., 1.) * 255.) as u8);
        pm.fill(rgb(base, 1.));
        let all = Rect::from_xywh(0., 0., w as f32, h as f32)?;
        for (color, x, y, r, a) in blobs {
            let at = Point::from_xy(x * w as f32, y * h as f32);
            // 中间一段慢慢淡：光团的边是化开的，不是一圈一圈的。
            let stops = vec![GradientStop::new(0., rgb(*color, *a)), GradientStop::new(0.45, rgb(*color, a * 0.62)), GradientStop::new(0.75, rgb(*color, a * 0.22)), GradientStop::new(1., rgb(*color, 0.))];
            let shader = RadialGradient::new(at, at, r * h as f32, stops, SpreadMode::Pad, Transform::identity())?;
            pm.fill_rect(all, &Paint { shader, ..Default::default() }, Transform::identity(), None);
        }
        // 从左上斜下来一道很淡的光，再压一层细颗粒：色块有了纸一样的质感，渐变也不起条纹。
        let sweep = LinearGradient::new(Point::from_xy(0., 0.), Point::from_xy(w as f32, h as f32), vec![GradientStop::new(0., Color::from_rgba8(255, 255, 255, 26)), GradientStop::new(0.5, Color::from_rgba8(255, 255, 255, 0)), GradientStop::new(1., Color::from_rgba8(0, 0, 0, 22))], SpreadMode::Pad, Transform::identity())?;
        pm.fill_rect(all, &Paint { shader: sweep, ..Default::default() }, Transform::identity(), None);
        let mut seed = 0x2545f491u32;
        for px in pm.pixels_mut() {
            seed ^= seed << 13; seed ^= seed >> 17; seed ^= seed << 5;
            let d = (seed % (grain as u32 * 2 + 1)) as i32 - grain;
            let f = |v: u8| (v as i32 + d).clamp(0, 255) as u8;
            if let Some(c) = PremultipliedColorU8::from_rgba(f(px.red()), f(px.green()), f(px.blue()), 255) { *px = c; }
        }
        std::fs::create_dir_all(cache_dir()).ok()?;
        std::fs::write(&path, pm.encode_png().ok()?).ok()?;
    }
    Some(path.to_string_lossy().to_string())
}

/// UIKit 解码会处理系统相册里照片的方向和颜色空间。
fn scaled_photo(path: &str, short: f64, long: f64) -> Option<image::DynamicImage> {
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject};
    #[link(name = "UIKit", kind = "framework")]
    unsafe extern "C" { fn UIImagePNGRepresentation(image: *mut AnyObject) -> *mut AnyObject; }
    unsafe {
        let cpath = std::ffi::CString::new(path).ok()?;
        let ns: *mut AnyObject = msg_send![AnyClass::get(c"NSString")?, stringWithUTF8String: cpath.as_ptr()];
        let image: *mut AnyObject = msg_send![AnyClass::get(c"UIImage")?, imageWithContentsOfFile: ns];
        if image.is_null() { return None; }
        let size: objc2_core_foundation::CGSize = msg_send![image, size];
        if size.width < 1. || size.height < 1. { return None; }
        let k = (short / size.width.min(size.height)).min(long / size.width.max(size.height)).min(1.);
        let small = objc2_core_foundation::CGSize::new((size.width * k).round().max(1.), (size.height * k).round().max(1.));
        let format: *mut AnyObject = msg_send![AnyClass::get(c"UIGraphicsImageRendererFormat")?, preferredFormat];
        let _: () = msg_send![format, setScale: 1f64];
        let renderer: *mut AnyObject = msg_send![AnyClass::get(c"UIGraphicsImageRenderer")?, alloc];
        let renderer: *mut AnyObject = msg_send![renderer, initWithSize: small, format: format];
        let addr = image as usize;
        let draw = block2::RcBlock::new(move |_context: *mut AnyObject| { let _: () = msg_send![addr as *mut AnyObject, drawInRect: objc2_core_foundation::CGRect::new(objc2_core_foundation::CGPoint::new(0., 0.), small)]; });
        let shrunk: *mut AnyObject = msg_send![renderer, imageWithActions: &*draw];
        let _: () = msg_send![renderer, release];
        let data = UIImagePNGRepresentation(shrunk);
        if data.is_null() { return None; }
        let bytes: *const std::ffi::c_void = msg_send![data, bytes];
        let len: usize = msg_send![data, length];
        image::load_from_memory_with_format(std::slice::from_raw_parts(bytes as *const u8, len), image::ImageFormat::Png).ok()
    }
}

fn sampled_top(path: &str) -> Option<Hsla> {
    thread_local! { static SAMPLE: RefCell<Option<(String, Option<Hsla>)>> = const { RefCell::new(None) }; }
    SAMPLE.with(|cache| {
        if let Some((_, color)) = cache.borrow().as_ref().filter(|(last, _)| last == path) { return *color; }
        let color = scaled_photo(path, 32., 96.).map(|photo| {
            let top = photo.crop_imm(0, 0, photo.width(), (photo.height() / 7).max(1));
            let [r, g, b] = top.resize_exact(1, 1, image::imageops::FilterType::Triangle).to_rgb8().get_pixel(0, 0).0;
            rgb(u32::from_be_bytes([0, r, g, b])).into()
        });
        *cache.borrow_mut() = Some((path.into(), color));
        color
    })
}

/// 高斯模糊的缓存保留原画幅。关闭模糊就直接显示原图。
pub(super) fn blurred(path: &str) -> Option<String> {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::hash::DefaultHasher::new();
    path.hash(&mut hasher);
    let out = cache_dir().join(format!("blur-2-{:016x}.png", hasher.finish()));
    if !out.is_file() {
        std::fs::create_dir_all(cache_dir()).ok()?;
        // 给滤镜留足采样点，同时控制全景长图的缓存大小。
        let source = scaled_photo(path, 320., 1200.)?;
        let radius = (source.width().min(source.height()) as f32 * 0.0175).max(0.5);
        let temp = out.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
        if source.blur(radius).save_with_format(&temp, image::ImageFormat::Png).is_err() || std::fs::rename(&temp, &out).is_err() {
            let _ = std::fs::remove_file(temp);
            return None;
        }
    }
    Some(out.to_string_lossy().to_string())
}

/// 自己挑的背景照片放的地方。
pub fn photos_dir() -> std::path::PathBuf { crate::store::Store::root_dir().join("backdrops") }

// ---- 分隔符：和纸胶带、涂鸦

/// 可换色的几何纸纹。标识也用在已存的文档样式里。
pub const PATTERNS: [&str; 6] = ["lattice", "dots", "diamonds", "chevron", "stripes", "plain"];
pub const TAPES: [&str; 3] = ["water", "sunset", "mint"];
/// 旧版的三条建议胶带，仍可在经典胶带里选择。
pub const TAPE_NAMES: [(&str, &str); 3] = [("水色", "Water"), ("晚霞", "Sunset"), ("薄荷", "Mint")];

/// 和纸图库的经典胶带：
/// 名字（存成 `image:<名字>`），中文名，英文名，画法，底色，花色。
#[derive(Clone, Copy, PartialEq)]
pub enum Motif { Gingham, Waves, Polka, Checker, Confetti, Plaid, Blossom, Zigzag, Stars, Bands, Kraft }
pub const GALLERY: [(&str, &str, &str, Motif, u32, u32); 12] = [
    ("gingham", "蓝白格", "Blue Gingham", Motif::Gingham, 0xf3f6fc, 0x5f84cc),
    ("waves", "青海波", "Indigo Waves", Motif::Waves, 0x2e4a7a, 0xdde7f5),
    ("polka", "奶油圆点", "Cream Polka", Motif::Polka, 0xf6edd9, 0xe08a6b),
    ("checker", "鼠尾草棋盘", "Sage Checker", Motif::Checker, 0xe3eadb, 0x98ad85),
    ("confetti", "彩纸屑", "Confetti", Motif::Confetti, 0xfffaf1, 0xe25d5d),
    ("plaid", "红格呢", "Red Plaid", Motif::Plaid, 0xc44b4b, 0x2a2626),
    ("blossom", "小碎花", "Little Blossoms", Motif::Blossom, 0xfbe4ea, 0xdc7a96),
    ("zigzag", "向阳折线", "Sunny Zigzag", Motif::Zigzag, 0xf5cf5c, 0xfffbe8),
    ("stars", "星夜", "Starry Night", Motif::Stars, 0x232f55, 0xf2d68a),
    ("matcha", "抹茶条纹", "Matcha Stripes", Motif::Bands, 0xd4e3b8, 0x86a866),
    ("kraft", "牛皮纸", "Kraft Paper", Motif::Kraft, 0xc6a379, 0x8d6a45),
    ("ink", "墨点", "Ink Dots", Motif::Polka, 0xfbfaf7, 0x2b2b2b),
];
/// 没挑颜色时胶带的颜色。
pub const TAPE: &str = "#A700B7";

pub fn divider_styled() -> bool { CUR.with(|c| c.borrow().as_ref().is_some_and(|s| s.divider.is_some())) }

/// 工具条上「——」和样式板用哪一套图标：线、和纸胶带（带胶带的颜色）还是涂鸦。
pub enum RuleGlyphs { Line, Tape(Hsla), Doodle }
pub fn rule_glyphs() -> RuleGlyphs {
    match current().and_then(|s| s.divider) {
        Some(DividerStyle::Tape { pattern, color }) => RuleGlyphs::Tape(match pattern.strip_prefix("image:") {
            // 图片胶带：取它中间那一色。
            Some("sunset") => rgb(0xe06666).into(),
            Some("mint") => rgb(0x6cc5b0).into(),
            Some("water") => rgb(0x5b78c2).into(),
            Some(n) if n.starts_with("file:") => crate::theme::Theme::icon(),
            Some(n) if let Some(tape) = super::washi::get(n) => rgb(tape.tint).into(),
            // 经典胶带：浅底的取花色，深底的取底色。
            Some(n) => GALLERY.iter().find(|g| g.0 == n).map(|g| { let b: Hsla = rgb(g.4).into(); if b.l > 0.75 { rgb(g.5).into() } else { b } }).unwrap_or_else(|| rgb(0x5b78c2).into()),
            None => color.as_deref().and_then(hex).unwrap_or_else(|| rgb(0xa700b7).into()),
        }),
        Some(DividerStyle::Doodle) => RuleGlyphs::Doodle,
        None => RuleGlyphs::Line,
    }
}

/// 图库、图案页和正文都从同一张纸纹的左端开始铺。
pub fn tape_image(pattern: &str, color: &str) -> Option<String> {
    use resvg::tiny_skia::*;
    let image = pattern.strip_prefix("image:");
    if let Some(name) = image {
        // 上传的照片放在背景图片的目录，随文档库一起移动。
        if let Some(file) = name.strip_prefix("file:") { let p = photos_dir().join(file); return p.exists().then(|| p.to_string_lossy().to_string()); }
        if let Some(tape) = super::washi::get(name) { return tape.path(); }
    }
    let color = if image.is_some() { "" } else { color.trim_start_matches('#') };
    let path = cache_dir().join(format!("tape-{}-{}-8.png", pattern.replace(':', "_"), color));
    if path.exists() { return Some(path.to_string_lossy().to_string()); }
    // 小样上也要看得清纹样。把纸纹连续绘到整行细胶带也够用的长度。
    let (w, h, u) = (3600, 120, 2.7);
    let (wf, hf) = (w as f32, h as f32);
    let mut pm = Pixmap::new(w, h)?;
    let all = Rect::from_xywh(0., 0., wf, hf)?;
    let rgba = |v: u32, a: u8| Color::from_rgba8((v >> 16) as u8, (v >> 8) as u8, v as u8, a);
    let solid = |c: Color| Paint { shader: Shader::SolidColor(c), anti_alias: true, ..Default::default() };
    if let Some(&(_, _, _, motif, base, ink)) = image.and_then(|n| GALLERY.iter().find(|g| g.0 == n)) {
        pm.fill(rgba(base, 255));
        draw_motif(&mut pm, motif, base, ink, u)?;
    } else if let Some(name) = image {
        let (a, b, c) = match name { "sunset" => (0xf6b26b, 0xe06666, 0x8e7cc3), "mint" => (0xb6e3c8, 0x6cc5b0, 0xd9f2e4), _ => (0xa9b8dc, 0x5b78c2, 0xb9c4e2) };
        let shader = LinearGradient::new(Point::from_xy(0., 0.), Point::from_xy(120. * u, 0.), vec![GradientStop::new(0., rgba(a, 255)), GradientStop::new(0.5, rgba(b, 255)), GradientStop::new(1., rgba(c, 255))], SpreadMode::Reflect, Transform::identity())?;
        pm.fill_rect(all, &Paint { shader, ..Default::default() }, Transform::identity(), None);
    } else {
        let base = u32::from_str_radix(color, 16).ok()?;
        pm.fill(rgba(base, 255));
        let (light, dark) = (solid(Color::from_rgba8(255, 255, 255, 96)), solid(Color::from_rgba8(0, 0, 0, 60)));
        let stroke = |width: f32| Stroke { width: width * u, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Default::default() };
        let mut pb = PathBuilder::new();
        match pattern {
            "lattice" => {
                pm.fill_rect(all, &solid(rgba(0xffffff, 126)), Transform::identity(), None);
                let (dx, dy) = (26. * u, 14.8 * u);
                for col in -1..=(wf / dx).ceil() as i32 {
                    let x = col as f32 * dx;
                    pb.move_to(x, 0.); pb.line_to(x, hf);
                    let mut y = (col.rem_euclid(2) as f32 - 2.) * dy;
                    while y < hf + dy {
                        pb.move_to(x + dx, y - dy); pb.line_to(x, y); pb.line_to(x + dx, y + dy);
                        y += 2. * dy;
                    }
                }
                pm.stroke_path(&pb.finish()?, &solid(rgba(base, 255)), &stroke(0.9), Transform::identity(), None);
            }
            "dots" => {
                let step = 21. * u;
                let (mut y, mut row) = (6. * u, 0);
                while y < hf { let mut x = (0.25 + 0.5 * (row % 2) as f32) * step; while x < wf { pb.push_circle(x, y, 1.6 * u); x += step; } y += step; row += 1; }
                pm.fill_path(&pb.finish()?, &light, FillRule::Winding, Transform::identity(), None);
            }
            "diamonds" => {
                let (a, b) = (22. * u, 25. * u);
                let mut faces = [PathBuilder::new(), PathBuilder::new(), PathBuilder::new()];
                let (mut y, mut row) = (0., 0);
                while y < hf + b {
                    let mut x = (row % 2) as f32 * a;
                    while x < wf + a {
                        for (face, points) in faces.iter_mut().zip([
                            [(0., 0.), (-a, -b / 2.), (0., -b), (a, -b / 2.)],
                            [(0., 0.), (a, -b / 2.), (a, b / 2.), (0., b)],
                            [(0., 0.), (0., b), (-a, b / 2.), (-a, -b / 2.)],
                        ]) {
                            face.move_to(x + points[0].0, y + points[0].1);
                            for (dx, dy) in points.into_iter().skip(1) { face.line_to(x + dx, y + dy); }
                            face.close();
                        }
                        x += 2. * a;
                    }
                    y += 1.5 * b; row += 1;
                }
                for (face, tone) in faces.into_iter().zip([solid(rgba(0xffffff, 150)), light, dark]) {
                    pm.fill_path(&face.finish()?, &tone, FillRule::Winding, Transform::identity(), None);
                }
            }
            "chevron" => {
                pm.fill_rect(all, &solid(rgba(0xffffff, 126)), Transform::identity(), None);
                let a = 58. * u;
                let mut y = -a; while y < hf + a { let (mut x, mut up) = (0., true); pb.move_to(0., y + a); while x < wf { x += a; up = !up; pb.line_to(x, if up { y + a } else { y }); } y += 7.5 * u; }
                pm.stroke_path(&pb.finish()?, &solid(rgba(base, 255)), &stroke(2.6), Transform::identity(), None);
            }
            "stripes" => {
                pm.fill_rect(all, &solid(rgba(0xffffff, 126)), Transform::identity(), None);
                let mut x = -30. * u;
                while x < wf {
                    let mut y = -12. * u;
                    pb.move_to(x, y);
                    while y < hf {
                        pb.line_to(x, y + 15. * u); pb.line_to(x + 25. * u, y + 29. * u);
                        pb.line_to(x + 25. * u, y + 48. * u); pb.line_to(x, y + 62. * u);
                        y += 62. * u;
                    }
                    x += 8. * u;
                }
                pm.stroke_path(&pb.finish()?, &solid(rgba(base, 255)), &stroke(1.3), Transform::identity(), None);
            }
            _ => {}
        }
    }
    std::fs::create_dir_all(cache_dir()).ok()?;
    std::fs::write(&path, pm.encode_png().ok()?).ok()?;
    Some(path.to_string_lossy().to_string())
}

/// 图库胶带的花纹，画在已经铺了底色的 `pm` 上；`u` 是一个点几个像素。花纹是一个个小格子重复，样张和长条都一样大小。
fn draw_motif(pm: &mut resvg::tiny_skia::Pixmap, motif: Motif, base: u32, ink: u32, u: f32) -> Option<()> {
    use resvg::tiny_skia::*;
    let (wf, hf) = (pm.width() as f32, pm.height() as f32);
    let color = |v: u32, a: u8| Color::from_rgba8((v >> 16) as u8, (v >> 8) as u8, v as u8, a);
    let paint = |v: u32, a: u8| Paint { shader: Shader::SolidColor(color(v, a)), anti_alias: true, ..Default::default() };
    let stroke = |width: f32| Stroke { width: width * u, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Default::default() };
    let id = Transform::identity();
    let mut seed = 0x51ed270bu32;
    let mut rnd = move || { seed ^= seed << 13; seed ^= seed >> 17; seed ^= seed << 5; (seed % 10000) as f32 / 10000. };
    let mut pb = PathBuilder::new();
    match motif {
        // 格子：横竖两组半透明的带子，交叉的地方自然深一些。
        Motif::Gingham | Motif::Plaid => {
            let (step, band) = if motif == Motif::Gingham { (8. * u, 4. * u) } else { (12. * u, 5. * u) };
            let mut x = 0.; while x < wf { if let Some(r) = Rect::from_xywh(x, 0., band, hf) { pb.push_rect(r); } x += step; }
            let mut y = 0.; while y < hf { if let Some(r) = Rect::from_xywh(0., y, wf, band) { pb.push_rect(r); } y += step; }
            pm.fill_path(&pb.finish()?, &paint(ink, if motif == Motif::Gingham { 90 } else { 110 }), FillRule::Winding, id, None);
            if motif == Motif::Plaid {
                let mut pb = PathBuilder::new();
                let mut x = 9. * u; while x < wf { pb.move_to(x, 0.); pb.line_to(x, hf); x += step; }
                let mut y = 9. * u; while y < hf { pb.move_to(0., y); pb.line_to(wf, y); y += step; }
                pm.stroke_path(&pb.finish()?, &paint(0xf2d06b, 200), &stroke(0.5), id, None);
            }
        }
        // 一层层的半圆，从上往下盖，每个半圆里三道圈。
        Motif::Waves => {
            let r = 6. * u;
            let (mut row, mut y) = (0, 0.);
            while y < hf + r {
                let mut x = if row % 2 == 0 { 0. } else { r };
                let mut rings = PathBuilder::new();
                let mut disc = PathBuilder::new();
                while x < wf + r { disc.push_circle(x, y, r); for k in [0.95, 0.68, 0.4] { rings.push_circle(x, y, r * k); } x += 2. * r; }
                pm.fill_path(&disc.finish()?, &paint(base, 255), FillRule::Winding, id, None);
                pm.stroke_path(&rings.finish()?, &paint(ink, 230), &stroke(0.55), id, None);
                y += r * 0.5; row += 1;
            }
        }
        Motif::Polka => {
            let (step, r) = if ink == 0x2b2b2b { (6. * u, 0.8 * u) } else { (9. * u, 2. * u) };
            let (mut row, mut y) = (0, step / 2.);
            while y < hf + step { let mut x = if row % 2 == 0 { step / 2. } else { step }; while x < wf { pb.push_circle(x, y, r); x += step; } y += step / 2. * 1.0; row += 1; }
            pm.fill_path(&pb.finish()?, &paint(ink, 235), FillRule::Winding, id, None);
        }
        Motif::Checker => {
            let a = 6. * u;
            let (mut j, mut y) = (0, 0.);
            while y < hf { let mut x = (j % 2) as f32 * a; while x < wf { if let Some(r) = Rect::from_xywh(x, y, a, a) { pb.push_rect(r); } x += 2. * a; } y += a; j += 1; }
            pm.fill_path(&pb.finish()?, &paint(ink, 200), FillRule::Winding, id, None);
        }
        // 彩纸屑：几种颜色的短道道，方向随便。
        Motif::Confetti => {
            let colors = [ink, 0x4f9be0, 0xf2b84b, 0x58b98a, 0xa47be0];
            let n = (wf * hf / (40. * u * u)) as usize;
            let mut bins: Vec<PathBuilder> = colors.iter().map(|_| PathBuilder::new()).collect();
            for _ in 0..n {
                let (x, y, t, k) = (rnd() * wf, rnd() * hf, rnd() * std::f32::consts::TAU, (rnd() * colors.len() as f32) as usize % colors.len());
                let l = 1.6 * u;
                bins[k].move_to(x - t.cos() * l, y - t.sin() * l); bins[k].line_to(x + t.cos() * l, y + t.sin() * l);
            }
            for (pb, c) in bins.into_iter().zip(colors) { if let Some(p) = pb.finish() { pm.stroke_path(&p, &paint(c, 230), &stroke(1.), id, None); } }
        }
        // 小碎花：五片圆瓣，中间一个黄点，错开着排。
        Motif::Blossom => {
            let step = 11. * u;
            let mut centers = PathBuilder::new();
            let (mut row, mut y) = (0, step / 2.);
            while y < hf + step {
                let mut x = if row % 2 == 0 { step / 2. } else { step };
                while x < wf + step {
                    let (jx, jy, rot) = ((rnd() - 0.5) * 2. * u, (rnd() - 0.5) * 2. * u, rnd() * 1.2);
                    for k in 0..5 { let t = rot + k as f32 * std::f32::consts::TAU / 5.; pb.push_circle(x + jx + t.cos() * 1.7 * u, y + jy + t.sin() * 1.7 * u, 1.3 * u); }
                    centers.push_circle(x + jx, y + jy, 0.8 * u);
                    x += step;
                }
                y += step / 2.; row += 1;
            }
            pm.fill_path(&pb.finish()?, &paint(ink, 235), FillRule::Winding, id, None);
            pm.fill_path(&centers.finish()?, &paint(0xf6d36b, 255), FillRule::Winding, id, None);
        }
        Motif::Zigzag => {
            let (a, step) = (4. * u, 7. * u);
            let mut y = 2. * u;
            while y < hf + a { let (mut x, mut up) = (0., true); pb.move_to(0., y); while x < wf { x += a; up = !up; pb.line_to(x, if up { y } else { y + a * 0.8 }); } y += step; }
            pm.stroke_path(&pb.finish()?, &paint(ink, 240), &stroke(1.3), id, None);
        }
        // 星夜：四个尖的小星星，夹着几粒小点。
        Motif::Stars => {
            let n = (wf * hf / (90. * u * u)) as usize;
            let mut dots = PathBuilder::new();
            for i in 0..n {
                let (x, y) = (rnd() * wf, rnd() * hf);
                if i % 3 == 0 {
                    let (r, k) = (2.4 * u, 0.7 * u);
                    pb.move_to(x, y - r); pb.line_to(x + k, y - k); pb.line_to(x + r, y); pb.line_to(x + k, y + k);
                    pb.line_to(x, y + r); pb.line_to(x - k, y + k); pb.line_to(x - r, y); pb.line_to(x - k, y - k); pb.close();
                } else { dots.push_circle(x, y, 0.45 * u); }
            }
            pm.fill_path(&pb.finish()?, &paint(ink, 255), FillRule::Winding, id, None);
            pm.fill_path(&dots.finish()?, &paint(ink, 170), FillRule::Winding, id, None);
        }
        // 竖条：一宽一窄。
        Motif::Bands => {
            let mut x = 0.;
            while x < wf { for (o, w) in [(0., 4. * u), (6.5 * u, 1.2 * u)] { if let Some(r) = Rect::from_xywh(x + o, 0., w, hf) { pb.push_rect(r); } } x += 10. * u; }
            pm.fill_path(&pb.finish()?, &paint(ink, 200), FillRule::Winding, id, None);
        }
        // 牛皮纸：一层颗粒，再撒几根纤维。
        Motif::Kraft => {
            let mut s2 = 0x2545f491u32;
            for px in pm.pixels_mut() {
                s2 ^= s2 << 13; s2 ^= s2 >> 17; s2 ^= s2 << 5;
                let d = (s2 % 23) as i16 - 11;
                let c = px.demultiply();
                let f = |v: u8| (v as i16 + d).clamp(0, 255) as u8;
                *px = ColorU8::from_rgba(f(c.red()), f(c.green()), f(c.blue()), 255).premultiply();
            }
            let n = (wf * hf / (60. * u * u)) as usize;
            for _ in 0..n { let (x, y, t) = (rnd() * wf, rnd() * hf, (rnd() - 0.5) * 0.8); let l = (2. + rnd() * 4.) * u; pb.move_to(x, y); pb.line_to(x + t.cos() * l, y + t.sin() * l); }
            pm.stroke_path(&pb.finish()?, &paint(ink, 120), &stroke(0.35), id, None);
        }
    }
    Some(())
}

/// 涂鸦：一笔手画似的波浪线，用给的颜色。
pub(super) fn doodle_image(color: &str) -> Option<String> {
    use resvg::tiny_skia::*;
    let path = cache_dir().join(format!("doodle-{}-1.png", color.trim_start_matches('#')));
    if path.exists() { return Some(path.to_string_lossy().to_string()); }
    let v = u32::from_str_radix(color.trim_start_matches('#'), 16).ok()?;
    let (w, h) = (1080u32, 42u32);
    let mut pm = Pixmap::new(w, h)?;
    let mid = h as f32 / 2.;
    let mut seed = 0x9e3779b9u32;
    let mut noise = || { seed ^= seed << 13; seed ^= seed >> 17; seed ^= seed << 5; (seed % 1000) as f32 / 1000. - 0.5 };
    let mut pb = PathBuilder::new();
    let (mut x, mut up) = (8f32, true);
    pb.move_to(x, mid);
    while x < w as f32 - 40. {
        let step = 26. + noise() * 12.;
        let y = mid + if up { -1. } else { 1. } * (9. + noise() * 6.);
        pb.quad_to(x + step / 2., y, x + step, mid + noise() * 4.);
        x += step; up = !up;
    }
    let paint = Paint { shader: Shader::SolidColor(Color::from_rgba8((v >> 16) as u8, (v >> 8) as u8, v as u8, 200)), anti_alias: true, ..Default::default() };
    pm.stroke_path(&pb.finish()?, &paint, &Stroke { width: 4.5, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Default::default() }, Transform::identity(), None);
    std::fs::create_dir_all(cache_dir()).ok()?;
    std::fs::write(&path, pm.encode_png().ok()?).ok()?;
    Some(path.to_string_lossy().to_string())
}

/// 涂鸦的几种（照 Craft，录屏 2026-11-19 20:32）：分隔线五种里除了分页，各对一种手画的线。
#[derive(Clone, Copy, PartialEq)]
enum Doodle { Wave, Curl, Loop, Chain, Scribble }

/// 一种涂鸦画成的图：宽 `w`、高 `h`（pt），画在中间一条线上；颜色是正文色。
fn doodle_variant(kind: Doodle, color: &str) -> Option<(String, f32, f32)> {
    use resvg::tiny_skia::*;
    let (w, h) = match kind { Doodle::Wave => (360., 14.), Doodle::Curl => (64., 14.), Doodle::Loop => (360., 22.), Doodle::Chain => (330., 22.), Doodle::Scribble => (360., 16.) };
    let path = cache_dir().join(format!("doodle-{}-{}-2.png", kind as u8, color.trim_start_matches('#')));
    if path.exists() { return Some((path.to_string_lossy().to_string(), w, h)); }
    if kind == Doodle::Wave { return doodle_image(color).map(|p| (p, w, h)); }
    let v = u32::from_str_radix(color.trim_start_matches('#'), 16).ok()?;
    let u = 3f32;
    let mut pm = Pixmap::new((w * u) as u32, (h * u) as u32)?;
    let mid = h / 2.;
    let mut seed = 0x2545f491u32;
    let mut noise = || { seed ^= seed << 13; seed ^= seed >> 17; seed ^= seed << 5; (seed % 1000) as f32 / 1000. - 0.5 };
    let mut pb = PathBuilder::new();
    let pt = |x: f32, y: f32| (x * u, y * u);
    // 一串向上打的圈：每圈往右走 `a`，x = a·t/2π + b·sin t、y = 基线 − c·(1 − cos t)/2；b 比 a/2π 大就打圈。
    let loops = |pb: &mut PathBuilder, x0: f32, n: usize, a: f32, b: f32, c: f32, y0: f32, jitter: f32, noise: &mut dyn FnMut() -> f32| -> f32 {
        let tau = std::f32::consts::TAU;
        let mut k = 1.;
        for i in 0..=n * 24 {
            if i % 24 == 0 { k = 1. + jitter * noise(); }
            let t = i as f32 / 24. * tau;
            let (x, y) = (x0 + a * t / tau + b * k * t.sin(), y0 - c * k * (1. - t.cos()) / 2.);
            pb.line_to(x * u, y * u);
        }
        x0 + a * n as f32
    };
    let stroke = |width: f32| Stroke { width: width * u, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Default::default() };
    let mut width = 1.4;
    match kind {
        Doodle::Curl => {
            let (sx, sy) = pt(2., mid + 2.); pb.move_to(sx, sy);
            let x = loops(&mut pb, 6., 4, 13., 4.5, 8., mid + 2., 0.15, &mut noise);
            let (ex, ey) = pt((x + 6.).min(w - 2.), mid + 2.); pb.line_to(ex, ey);
        }
        Doodle::Loop => {
            let base = mid + 5.;
            let (sx, sy) = pt(2., base); pb.move_to(sx, sy);
            let x0 = w / 2. - 10.;
            let (lx, ly) = pt(x0, base - 0.5); pb.line_to(lx, ly);
            let x = loops(&mut pb, x0, 1, 20., 7., 13., base, 0., &mut noise);
            let (ex, ey) = pt(x + 2., base); pb.line_to(ex, ey);
            let (fx, fy) = pt(w - 2., base + 0.5); pb.line_to(fx, fy);
        }
        Doodle::Chain => {
            let base = mid + 4.;
            let (sx, sy) = pt(2., base + 1.); pb.move_to(sx, sy);
            let (lx, ly) = pt(18., base); pb.quad_to(sx + 8. * u, sy - 3., lx, ly);
            let x = loops(&mut pb, 18., 13, 22., 6., 12., base, 0.3, &mut noise);
            let (ex, ey) = pt(w - 2., base + 1.); pb.quad_to((x + 8.) * u, (base - 2.) * u, ex, ey);
        }
        Doodle::Scribble => {
            // 来回两笔，像拿笔描粗的一道。
            width = 1.8;
            for (pass, dy) in [(0, -1.2f32), (1, 1.)] {
                let mut x = 3f32;
                let (sx, sy) = pt(x, mid + dy); pb.move_to(sx, sy);
                while x < w - 3. {
                    let step = 40. + noise() * 20.;
                    let nx = (x + step).min(w - 3.);
                    let (cx, cy) = pt((x + nx) / 2., mid + dy + noise() * 3. + if pass == 0 { -0.6 } else { 0.6 });
                    let (ex, ey) = pt(nx, mid + dy + noise() * 1.2);
                    pb.quad_to(cx, cy, ex, ey);
                    x = nx;
                }
            }
        }
        Doodle::Wave => {}
    }
    let paint = Paint { shader: Shader::SolidColor(Color::from_rgba8((v >> 16) as u8, (v >> 8) as u8, v as u8, 210)), anti_alias: true, ..Default::default() };
    pm.stroke_path(&pb.finish()?, &paint, &stroke(width), Transform::identity(), None);
    std::fs::create_dir_all(cache_dir()).ok()?;
    std::fs::write(&path, pm.encode_png().ok()?).ok()?;
    Some((path.to_string_lossy().to_string(), w, h))
}

/// 胶带的几种大小（宽 pt，None 是整行；高 pt）。照 Craft 量的（录屏 2026-11-19 20:32，1pt = 3px）。
fn tape_size(rule: Rule) -> (Option<f32>, u32) {
    match rule { Rule::Dots => (Some(68.), 28), Rule::Dotted => (Some(188.), 40), Rule::Line => (Some(262.), 50), Rule::Thick => (None, 56), _ => (None, 16) }
}

/// 一条胶带：图案按高度铺开，两端和图库使用同样的撕边轮廓。
fn tape(path: String, w: Option<f32>, h: f32, paper: Hsla) -> Div {
    let path = std::path::PathBuf::from(path);
    let source = Resource::from(path.clone());
    let art = canvas(move |bounds, window, cx| {
        let image = window.use_asset::<ImgResourceLoader>(&source, cx)?.ok()?;
        let size = image.size(0);
        if f32::from(bounds.size.width) * size.height.0 as f32 > f32::from(bounds.size.height) * size.width.0 as f32
            && let Some(wrapped) = super::washi::repeat_texture(&path)
        {
            return window.use_asset::<ImgResourceLoader>(&Resource::from(wrapped), cx)?.ok().or(Some(image));
        }
        Some(image)
    }, |bounds, loaded, window, _| {
        let Some(image) = loaded else { return; };
        let size = image.size(0);
        let spans = super::washi::tile_spans(f64::from(f32::from(bounds.size.width)), f64::from(f32::from(bounds.size.height)), f64::from(size.width.0), f64::from(size.height.0));
        for (x, width) in spans {
            let tile = Bounds::new(point(bounds.left() + px(x as f32), bounds.top()), gpui::size(px(width as f32), bounds.size.height));
            let _ = window.paint_image(bounds, tile, Corners::default(), image.clone(), 0, false);
        }
    }).absolute().inset_0();
    div().relative().h(px(h)).flex_none().overflow_hidden().map(|d| match w { Some(w) => d.w(px(w)).max_w_full(), None => d.w_full() })
        .child(art)
        .child(canvas(|_, _, _| {}, move |b, _, window, _| {
            for (right, x, direction) in [(false, b.left(), 1.), (true, b.right(), -1.)] {
                let mut p = Path::new(point(x, b.top()));
                for (dx, dy) in super::washi::torn_edge(right, h as f64) {
                    p.line_to(point(x + px(direction * dx as f32), b.top() + px(dy as f32)));
                }
                p.line_to(point(x, b.bottom()));
                window.paint_path(p, paper);
            }
        }).absolute().inset_0())
}

/// 这篇文档的分隔符（样式里换了的话）。分隔线的五种各对一种：胶带是不同的大小，涂鸦是不同的画法；
/// 分页单独画；`---` 用普通分隔线的尺寸。
pub fn divider(rule: Rule) -> Option<AnyElement> {
    let style = current()?;
    let paper = crate::editor_theme::Theme::bg_body();
    Some(match style.divider.as_ref()? {
        DividerStyle::Tape { pattern, color } => {
            let (w, h) = tape_size(rule);
            let path = tape_image(pattern, color.as_deref().unwrap_or(TAPE))?;
            div().py(px(if rule == Rule::Plain { 6. } else { 10. })).w_full().flex().items_center().justify_center().child(tape(path, w, h as f32, paper)).into_any_element()
        }
        DividerStyle::Doodle => {
            let kind = match rule { Rule::Dots => Doodle::Curl, Rule::Dotted => Doodle::Loop, Rule::Line => Doodle::Chain, Rule::Thick => Doodle::Scribble, _ => Doodle::Wave };
            let (path, w, h) = doodle_variant(kind, &to_hex(crate::editor_theme::Theme::text()))?;
            let image = img(std::path::PathBuf::from(path)).h(px(h)).object_fit(ObjectFit::Fill);
            let image = if w < 200. { image.w(px(w)) } else if kind == Doodle::Chain { image.w(relative(0.92)) } else { image.w_full() };
            div().py(px(if kind == Doodle::Wave { 5. } else { 8. })).w_full().flex().items_center().justify_center().child(image).into_any_element()
        }
    })
}
