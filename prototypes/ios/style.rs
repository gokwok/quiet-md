//! 文档样式：一篇文档自己的文字颜色、字体、纸的颜色、纸后面的背景和分隔符的样子（照 Craft 的「样式」）。
//!
//! 存在文档库索引里（`Meta::style`，`store::DocStyle`），不动文档文件。画的时候分两路：主题表达得了的
//! （文字、纸、字体）每帧盖到所选主题上（`themes::overlay`），正文照常读主题；主题表达不了的（背景、和纸胶带）这里另画。
//! 没设样式的文档这两路都不走，和原来一模一样。面板在 `stylepanel.rs`。
use std::cell::RefCell;

use gpui::{prelude::FluentBuilder as _, *};

use crate::app::{View, Workspace};
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

/// 字体的名字 → 系统里的字体族。
pub fn family(name: &str) -> Option<&'static str> {
    match name { "serif" => Some(".AppleSystemUIFontSerif"), "mono" => Some(crate::themes::SYSTEM_MONO), "rounded" => Some(".AppleSystemUIFontRounded"), _ => None }
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
pub fn size_capsule(w: &Workspace, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    if !sizing() || w.view != View::Doc { set_sizing(false); return None; }
    let id = w.current.clone()?;
    let now = match w.entries.iter().find(|e| e.id == id)?.meta.text_size { 0 => 3, n => n.min(8) };
    let theme = crate::theme::Theme::bg_overlay;
    let mut row = div().id("text-size").occlude().h(px(60.)).px(px(14.)).rounded_full().bg(crate::theme::raised(theme())).shadow(crate::theme::shadow_dialog()).flex().items_center();
    for step in 1..=8u8 {
        let d = 7. + 2.2 * (step - 1) as f32;
        let id = id.clone();
        row = row.child(div().id(("text-size-step", step as usize)).w(px(34.)).h(px(60.)).flex().items_center().justify_center()
            .child(div().size(px(d)).rounded_full().bg(if step == now { crate::theme::Theme::primary() } else { crate::theme::Theme::text_disabled() }))
            .on_click(cx.listener(move |w, _, _, cx| {
                // 第三档就是原样，不记。
                let step = if step == 3 { 0 } else { step };
                if let Err(e) = w.store.update_meta(&id, |m| m.text_size = step) { return w.show_toast_kind(crate::ui::ToastKind::Error, &e.to_string(), cx); }
                if let Some(e) = w.entries.iter_mut().find(|e| e.id == id) { e.meta.text_size = step; }
                cx.notify();
            })));
    }
    Some(deferred(div().id("text-size-mask").absolute().inset_0().occlude().bg(black().opacity(0.12)).flex().items_center().justify_center()
        .on_click(cx.listener(|_, _, _, cx| { set_sizing(false); cx.notify(); })).child(row)).with_priority(3).into_any_element())
}

/// 这一帧的文档样式。
pub fn current() -> Option<DocStyle> { CUR.with(|c| c.borrow().clone()) }
/// 有背景：纸两侧内缩，上沿让到顶栏下面，背景从四周露出来。
pub fn banded() -> bool { CUR.with(|c| c.borrow().as_ref().is_some_and(|s| s.background.is_some())) }

/// 有背景时顶栏上的字（标题、文件夹）用什么颜色：背景上沿深就用白的，浅就用深的。没有背景时照主题。
pub fn chrome_ink() -> Option<(Hsla, Hsla)> {
    let style = current()?;
    let top = match style.background.as_ref()? {
        Backdrop::Color { color } => hex(color)?,
        Backdrop::Gradient { from, to, dir } => if *dir == 0 || *dir == 2 { hex(from)? } else { let (a, b) = (Rgba::from(hex(from)?), Rgba::from(hex(to)?)); Rgba { r: (a.r + b.r) / 2., g: (a.g + b.g) / 2., b: (a.b + b.b) / 2., a: 1. }.into() },
        // 图片不去读像素：内置的照配方的底色，自己挑的照片当深色。
        Backdrop::Image { src, .. } => src.strip_prefix("builtin:").and_then(recipe).map(|r| rgb(r.0).into()).unwrap_or(black()),
    };
    let c = Rgba::from(top);
    let mut l = 0.299 * c.r + 0.587 * c.g + 0.114 * c.b;
    if style.faded { let p = Rgba::from(crate::editor_theme::Theme::bg_body()); l = l * 0.4 + (0.299 * p.r + 0.587 * p.g + 0.114 * p.b) * 0.6; }
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
    Some(fill.when(style.faded, |d| d.child(div().absolute().inset_0().bg(crate::editor_theme::Theme::bg_body().opacity(0.6)))).into_any_element())
}

// ---- 背景图片

/// 内置的背景：程序画的几张柔光色块（自己配的，不是照搬的图片），第一次用到时画出来放在缓存目录里。
pub const BUILTIN: [&str; 9] = ["dusk", "peach", "ocean", "lagoon", "mist", "lilac", "linen", "moss", "ink"];

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

/// 模糊过的那一张：让系统缩成很小的一张存下来，画的时候再拉大，就是糊的。GPUI 自己没有模糊。
fn blurred(path: &str) -> Option<String> {
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject};
    use std::hash::{Hash, Hasher};
    #[link(name = "UIKit", kind = "framework")]
    unsafe extern "C" { fn UIImagePNGRepresentation(image: *mut AnyObject) -> *mut AnyObject; }
    let mut hasher = std::hash::DefaultHasher::new();
    path.hash(&mut hasher);
    let out = cache_dir().join(format!("blur-{:016x}.png", hasher.finish()));
    if !out.exists() {
        std::fs::create_dir_all(cache_dir()).ok()?;
        unsafe {
            let cpath = std::ffi::CString::new(path).ok()?;
            let ns: *mut AnyObject = msg_send![AnyClass::get(c"NSString")?, stringWithUTF8String: cpath.as_ptr()];
            let image: *mut AnyObject = msg_send![AnyClass::get(c"UIImage")?, imageWithContentsOfFile: ns];
            if image.is_null() { return None; }
            let size: objc2_core_foundation::CGSize = msg_send![image, size];
            if size.width < 1. || size.height < 1. { return None; }
            // 短边 24 个像素（scale 1 的画布）。
            let k = 24. / size.width.min(size.height);
            let small = objc2_core_foundation::CGSize::new((size.width * k).round(), (size.height * k).round());
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
            std::fs::write(&out, std::slice::from_raw_parts(bytes as *const u8, len)).ok()?;
        }
    }
    Some(out.to_string_lossy().to_string())
}

/// 自己挑的背景照片放的地方。
pub fn photos_dir() -> std::path::PathBuf { crate::store::Store::root_dir().join("backdrops") }

// ---- 分隔符：和纸胶带、涂鸦

/// 胶带的图案（自己画的几种几何纹）和渐变胶带（`image:<名字>`）。
pub const PATTERNS: [&str; 6] = ["lattice", "dots", "diamonds", "chevron", "stripes", "plain"];
pub const TAPES: [&str; 3] = ["water", "sunset", "mint"];
/// 没挑颜色时胶带的颜色。
pub const TAPE: &str = "#C050D8";

pub fn divider_styled() -> bool { CUR.with(|c| c.borrow().as_ref().is_some_and(|s| s.divider.is_some())) }

/// 一条胶带或胶带的一块样张（面板里用）。画好的放在缓存目录，按图案、颜色、大小记名字。
pub fn tape_image(pattern: &str, color: &str, tile: bool) -> Option<String> {
    use resvg::tiny_skia::*;
    let path = cache_dir().join(format!("tape-{}-{}-{}-3.png", pattern.replace(':', "_"), color.trim_start_matches('#'), tile as u8));
    if path.exists() { return Some(path.to_string_lossy().to_string()); }
    // 一个点 3 个像素。长条 1000×16pt，画的时候按高度铺、两头裁掉；样张 110×70pt。
    let u = 3f32;
    let (w, h) = if tile { (330u32, 210u32) } else { (3000u32, 48u32) };
    let (wf, hf) = (w as f32, h as f32);
    let mut pm = Pixmap::new(w, h)?;
    let all = Rect::from_xywh(0., 0., wf, hf)?;
    let rgba = |v: u32, a: u8| Color::from_rgba8((v >> 16) as u8, (v >> 8) as u8, v as u8, a);
    let solid = |c: Color| Paint { shader: Shader::SolidColor(c), anti_alias: true, ..Default::default() };
    if let Some(name) = pattern.strip_prefix("image:") {
        let (a, b, c) = match name { "sunset" => (0xf6b26b, 0xe06666, 0x8e7cc3), "mint" => (0xb6e3c8, 0x6cc5b0, 0xd9f2e4), _ => (0xa9b8dc, 0x5b78c2, 0xb9c4e2) };
        let shader = LinearGradient::new(Point::from_xy(0., 0.), Point::from_xy(120. * u, 0.), vec![GradientStop::new(0., rgba(a, 255)), GradientStop::new(0.5, rgba(b, 255)), GradientStop::new(1., rgba(c, 255))], SpreadMode::Reflect, Transform::identity())?;
        pm.fill_rect(all, &Paint { shader, ..Default::default() }, Transform::identity(), None);
    } else {
        let base = u32::from_str_radix(color.trim_start_matches('#'), 16).ok()?;
        pm.fill(rgba(base, 255));
        let (light, dark) = (solid(Color::from_rgba8(255, 255, 255, 96)), solid(Color::from_rgba8(0, 0, 0, 60)));
        let stroke = |width: f32| Stroke { width: width * u, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Default::default() };
        let mut pb = PathBuilder::new();
        match pattern {
            "lattice" => {
                let mut x = -hf; while x < wf + hf { pb.move_to(x, 0.); pb.line_to(x + hf * 0.6, hf); pb.move_to(x + hf * 0.6, 0.); pb.line_to(x, hf); x += 8. * u; }
                let mut y = 4. * u; while y < hf { pb.move_to(0., y); pb.line_to(wf, y); y += 8. * u; }
                pm.stroke_path(&pb.finish()?, &light, &stroke(0.8), Transform::identity(), None);
            }
            "dots" => {
                let (mut y, mut row) = (4. * u, 0); while y < hf { let mut x = if row % 2 == 0 { 4. * u } else { 8. * u }; while x < wf { pb.push_circle(x, y, 0.9 * u); x += 8. * u; } y += 8. * u; row += 1; }
                pm.fill_path(&pb.finish()?, &dark, FillRule::Winding, Transform::identity(), None);
            }
            "diamonds" => {
                let (a, b) = (6. * u, 4. * u);
                let mut y = 0.; while y < hf + b { let mut x = 0.; while x < wf + a { pb.move_to(x, y - b); pb.line_to(x + a, y); pb.line_to(x, y + b); pb.line_to(x - a, y); pb.close(); x += 2. * a; } y += 2. * b; }
                pm.fill_path(&pb.finish()?, &light, FillRule::EvenOdd, Transform::identity(), None);
            }
            "chevron" => {
                let mut y = -4. * u; while y < hf + 4. * u { let (mut x, mut up) = (0., false); pb.move_to(0., y); while x < wf { x += 4. * u; up = !up; pb.line_to(x, if up { y + 4. * u } else { y }); } y += 4. * u; }
                pm.stroke_path(&pb.finish()?, &light, &stroke(1.), Transform::identity(), None);
            }
            "stripes" => {
                let mut x = -hf; while x < wf { pb.move_to(x, hf); pb.line_to(x + hf, 0.); x += 6. * u; }
                pm.stroke_path(&pb.finish()?, &light, &stroke(2.), Transform::identity(), None);
            }
            _ => {}
        }
    }
    std::fs::create_dir_all(cache_dir()).ok()?;
    std::fs::write(&path, pm.encode_png().ok()?).ok()?;
    Some(path.to_string_lossy().to_string())
}

/// 涂鸦：一笔手画似的波浪线，用给的颜色。
fn doodle_image(color: &str) -> Option<String> {
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

/// 这篇文档的分隔符（样式里换了的话）。胶带两头的锯齿是拿纸色的小三角盖出来的。
pub fn divider() -> Option<AnyElement> {
    let style = current()?;
    let paper = crate::editor_theme::Theme::bg_body();
    Some(match style.divider.as_ref()? {
        DividerStyle::Tape { pattern, color } => {
            let path = tape_image(pattern, color.as_deref().unwrap_or(TAPE), false)?;
            div().h(px(28.)).w_full().flex().items_center().child(
                div().relative().h(px(16.)).w_full().overflow_hidden()
                    .child(img(std::path::PathBuf::from(path)).size_full().object_fit(ObjectFit::Cover))
                    .child(canvas(|_, _, _| {}, move |b, _, window, _| {
                        for i in 0..4 {
                            let y = b.top() + px(4. * i as f32);
                            for (x, dx) in [(b.left(), px(3.)), (b.right(), px(-3.))] {
                                let mut p = Path::new(point(x, y));
                                p.line_to(point(x + dx, y + px(2.)));
                                p.line_to(point(x, y + px(4.)));
                                window.paint_path(p, paper);
                            }
                        }
                    }).absolute().inset_0()),
            ).into_any_element()
        }
        DividerStyle::Doodle => {
            let path = doodle_image(&to_hex(crate::editor_theme::Theme::text()))?;
            div().h(px(24.)).w_full().flex().items_center().child(img(std::path::PathBuf::from(path)).w_full().h(px(14.)).object_fit(ObjectFit::Fill)).into_any_element()
        }
    })
}
