//! 文档的「样式」面板（照 Craft）：主面板七格，点进去是各自的一页。内容拼成 `panel::Node`，由原生的玻璃面板画；
//! 点了什么由 `action` 收，改的是这篇文档的 `DocStyle`（`style.rs` 负责画出来）。面板保持正文亮度，改动立即可见；点面板外收起。
use std::cell::{Cell, RefCell};

use gpui::*;
use objc2::{ClassType, define_class, msg_send};
use objc2::runtime::{AnyClass, AnyObject, NSObject};

use super::panel::{Art, Card, Fill, Material, Node, PhotoTile, Strip, Tile, Trailing};
use super::style::{hex, to_hex};
use super::washi;
use crate::app::{View, Workspace};
use crate::settings::tr;
use crate::store::{Backdrop, DividerStyle, DocStyle};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Page { Main, Text, Font, Paper, Background, PhotoLibrary, Divider, Pattern, Gallery, Washi, Cover }

#[path = "background_gallery.rs"]
mod background_gallery;
#[path = "unsplash.rs"]
mod unsplash;

thread_local! { static PAGE: Cell<Option<Page>> = const { Cell::new(None) }; }
// 背景页停在哪一段（没有背景时也能停在别的段上挑）。
thread_local! { static TAB: Cell<Option<usize>> = const { Cell::new(None) }; }
// 分隔符从和纸胶带换成别的之前用的那卷（图案、颜色），换回和纸胶带时接着用。
thread_local! { static LAST_TAPE: RefCell<Option<DividerStyle>> = const { RefCell::new(None) }; }
// 同一轮设置里，切回某种背景时继续用刚才选的值。
thread_local! {
    static BACKGROUNDS: RefCell<[Option<Backdrop>; 3]> = const { RefCell::new([None, None, None]) };
    static SUGGESTIONS: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
}

pub fn is_open() -> bool { PAGE.with(|p| p.get()).is_some() }
pub fn open() {
    PAGE.with(|p| p.set(Some(Page::Main)));
    TAB.with(|t| t.set(None));
    BACKGROUNDS.with(|drafts| *drafts.borrow_mut() = [None, None, None]);
    SUGGESTIONS.with(|photos| photos.borrow_mut().clear());
}
pub fn close() { PAGE.with(|p| p.set(None)); }

fn tape_preview(pattern: &str, color: &str) -> Option<Fill> {
    super::style::tape_image(pattern, color).map(Fill::Tape)
}

fn washi_strips<'a>(tapes: impl IntoIterator<Item = &'a washi::Tape>, pattern: &str, cols: usize) -> Node {
    let selected = pattern.strip_prefix("image:");
    let items = tapes.into_iter().filter_map(|t| Some(Strip {
        fill: Fill::Tape(t.path()?),
        title: t.name.into(),
        picked: selected == Some(t.id),
        act: Some(format!("pat-washi:{}", t.id)),
    })).collect();
    Node::Strips { items, cols }
}

fn classic_tapes() -> impl Iterator<Item = (&'static str, &'static str, &'static str)> {
    super::style::GALLERY.iter().map(|t| (t.0, t.1, t.2))
        .chain(super::style::TAPES.into_iter().zip(super::style::TAPE_NAMES).map(|(id, (zh, en))| (id, zh, en)))
}

fn classic_washi_strips(pattern: &str) -> Node {
    let items = classic_tapes().filter_map(|(id, zh, en)| {
        let key = format!("image:{id}");
        Some(Strip { fill: tape_preview(&key, "")?, title: tr(zh, en).into(),
            picked: pattern == key, act: Some(format!("pat-washi:{id}")) })
    }).collect();
    Node::Strips { items, cols: 2 }
}

fn current_washi_strip(pattern: &str) -> Option<Strip> {
    let id = pattern.strip_prefix("image:")?;
    if washi::SUGGESTED.iter().any(|tape| tape.id == id) { return None; }
    let (title, act) = if id.starts_with("file:") { (tr("上传的胶带", "Uploaded tape"), None) }
        else {
            let title = washi::get(id).map(|tape| tape.name)
                .or_else(|| classic_tapes().find(|tape| tape.0 == id).map(|(_, zh, en)| tr(zh, en)))?;
            (title, Some(format!("pat-washi:{id}")))
        };
    Some(Strip { fill: tape_preview(pattern, "")?, title: title.into(), picked: true, act })
}

// 色板：Craft 实测（录屏里量的，对上了 Tailwind v4 的色阶）。每组第一颗是「默认」，不在这里。
const TEXT: [u32; 10] = [0x202225, 0xf4f4f5, 0x721378, 0x1c398e, 0x104e64, 0x0b4f4a, 0x006045, 0xa65f00, 0x82181a, 0x861043];
const PAPER: [u32; 10] = [0xffffff, 0x222222, 0xfae8ff, 0xdbeafe, 0xcefafe, 0xcbfbf1, 0xd0fae5, 0xfef9c2, 0xffe2e2, 0xfce7f3];
const SOLID: [u32; 11] = [0xffd6a8, 0xffb900, 0x51a2ff, 0x193cb8, 0x00bba7, 0x004f3b, 0x007595, 0xff6900, 0xc10007, 0xffc9c9, 0xc4b4ff];
const GRADIENT: [(u32, u32); 12] = [
    (0x43c7ac, 0xdbf6ae), (0xd4f6c0, 0xf7bdbd), (0xe4e8e9, 0xa1adb6), (0xdfb49a, 0xb5cbcb), (0xaa9eec, 0xefc1d9), (0xe8cba0, 0xdbb481),
    (0xdbc2bc, 0x7f81a8), (0xcff2d3, 0xf2958e), (0xe1d0eb, 0xf7b4b5), (0x8a91a9, 0xccd3e0), (0xffecac, 0xffcf99), (0x68a8d7, 0xe1f2f8),
];
const FONTS: [(&str, &str, &str, &str); 4] = [("", "默认", "Default", "System"), ("serif", "衬线", "Serif", "Serif"), ("mono", "等宽", "Mono", "Mono"), ("rounded", "圆体", "Rounded", "Rounded")];
/// 字体圆钮里的字样（照 Craft）。
const SAMPLES: [&str; 4] = ["Aa", "Ss", "00", "Rr"];
const DIRS: [(&str, &str); 4] = [("从上到下", "Top to bottom"), ("从左到右", "Left to right"), ("左上到右下", "Diagonal down"), ("左下到右上", "Diagonal up")];

// 胶带色卡：11/19 录屏 58s 的中心像素，从 BT.2020 PQ 转成 sRGB 后取样。
const TAPE: [u32; 11] = [0xa700b7, 0x183bb6, 0x175cfb, 0x0491b6, 0x059588, 0x05ba7c, 0xefaf01, 0xfd9800, 0xfe6800, 0xe5010b, 0xe40376];

/// 内置的样式（自己配的几套，不是 Craft 的那些）：名字，显示的名字，样式。第一套是什么都不改。
/// 每一套的字、纸、背景、分隔符是一起配的：纸取背景里最浅的那个调子再提亮，字取最深的那个调子。
fn presets() -> Vec<(&'static str, &'static str, DocStyle)> {
    let s = |id: &str, text: u32, paper: u32, font: &str, image: &str, divider: Option<DividerStyle>| DocStyle {
        text: Some(h(text)), paper: Some(h(paper)), font: (!font.is_empty()).then(|| font.to_string()),
        background: Some(Backdrop::Image { src: format!("builtin:{image}"), blur: false }), divider, preset: Some(id.to_string()), ..Default::default()
    };
    let tape = |pattern: &str, color: u32| Some(DividerStyle::Tape { pattern: pattern.into(), color: Some(h(color)) });
    // 照 Craft 的内置样式（IMG_9349、9356）：一张照片作背景，纸色、字色、字体和胶带都从照片里配出来；深色照片配深色的纸。
    let washi = |id: &str| Some(DividerStyle::Tape { pattern: format!("image:{id}"), color: None });
    vec![
        ("original", tr("原样", "Original"), DocStyle::default()),
        ("reed-dawn", tr("晨芦", "Reed Dawn"), s("reed-dawn", 0x5a3a22, 0xfbf4ea, "serif", "reed-shore", washi("caramel-picnic"))),
        ("tea-hour", tr("茶影", "Tea Hour"), s("tea-hour", 0x4a3b2c, 0xf3eadb, "serif", "tea-shadow", Some(DividerStyle::Doodle))),
        ("basalt", tr("海崖", "Basalt Coast"), s("basalt", 0x23384d, 0xf4f6f8, "", "basalt-coast", washi("ocean-mosaic"))),
        ("desk", tr("书桌", "Writing Desk"), s("desk", 0x3b2f25, 0xfaf6ee, "mono", "paper-study", washi("the-notebook"))),
        ("fern-night", tr("雨林", "Fern Night"), s("fern-night", 0xede6cf, 0x1f2e1c, "serif", "fern-rain", washi("olive-cloth"))),
        ("violet-ridge", tr("暮山", "Violet Ridge"), s("violet-ridge", 0x3e3456, 0xf7f4fa, "rounded", "violet-ridge", washi("lavender-dream"))),
        ("holiday", tr("冬灯", "Winter Lights"), s("holiday", 0xf4e3c8, 0x3a1416, "serif", "dusk", washi("christmas-glow"))),
        ("sepia", tr("旧纸", "Sepia"), s("sepia", 0x3b2f20, 0xf8f1e3, "serif", "linen", Some(DividerStyle::Doodle))),
        ("blossom", tr("桃花", "Blossom"), s("blossom", 0x6b1d3a, 0xfff6f4, "rounded", "peach", tape("dots", 0xf2a0b5))),
        ("ocean", tr("海", "Ocean"), s("ocean", 0x16305c, 0xf4f8ff, "", "ocean", tape("stripes", 0x2a7fd6))),
        ("mint", tr("薄荷", "Mint"), s("mint", 0x0b4f4a, 0xf4fcf9, "", "lagoon", tape("diamonds", 0x4fc3ad))),
        ("lilac", tr("丁香", "Lilac"), s("lilac", 0x3a2a6b, 0xfaf7ff, "serif", "lilac", tape("lattice", 0x9d8be8))),
        ("forest", tr("林间", "Forest"), s("forest", 0x22381f, 0xf5f7ee, "serif", "moss", tape("chevron", 0x7c8f3f))),
        ("midnight", tr("午夜", "Midnight"), s("midnight", 0xe9e9ee, 0x17181d, "", "ink", tape("plain", 0x41608f))),
        ("dusk", tr("暮色", "Dusk"), s("dusk", 0xf3eaf6, 0x221a2e, "serif", "dusk", Some(DividerStyle::Tape { pattern: "image:sunset".into(), color: None }))),
    ]
}

/// 署了名的一套样式：名字，显示的名字，样式，配的封面（名字和图；只在文档还没有封面时加上）。
pub(super) type Authored = (&'static str, &'static str, DocStyle, Option<(&'static str, &'static [u8])>);
/// 署了名的样式集：图库里每位作者一组（照 Craft「CRAFT 的样式」），排在内置样式后面：
/// 组名，底栏的署名，样式。
fn collections() -> Vec<(&'static str, &'static str, Vec<Authored>)> {
    vec![
        (tr("Astra 的样式", "Astra's styles"), tr("由 Astra 设计", "Designed by Astra"), super::astra::presets()),
        (tr("Claude 的样式", "Claude's styles"), tr("由 Claude 设计", "Designed by Claude"), claude_presets()),
    ]
}
/// 按名字找内置或署了名的一套，连同底栏的署名。
fn find_preset(id: &str) -> Option<(&'static str, DocStyle, &'static str)> {
    presets().into_iter().find(|(p, ..)| *p == id).map(|(_, n, s)| (n, s, tr("内置样式", "Built-in style")))
        .or_else(|| collections().into_iter().find_map(|(_, by, set)| set.into_iter().find(|(p, ..)| *p == id).map(|(_, n, s, _)| (n, s, by))))
}

/// Claude 设计的十二套（和 Fable 一起定的方案）。背景、胶带、封面都是 `claude/draw.py` 画的；每套用一种不同的手法
/// （孔版网点、等高线、蓝晒、墨流、水磨石、放射光、青海波、示波器、构成、票根、五线谱、莫尔纹），四套深色、四套彩色的纸。
/// 张扬的都在纸外面和分隔符上，字和纸的对比都在 8:1 以上，长文照样读得下去。
fn claude_presets() -> Vec<Authored> {
    let c = |id: &str, text: u32, paper: u32, font: &str, faded: bool, divider: Option<DividerStyle>| DocStyle {
        text: Some(h(text)), paper: Some(h(paper)), font: (!font.is_empty()).then(|| font.to_string()),
        background: Some(Backdrop::Image { src: format!("builtin:claude-{id}"), blur: false }), faded, divider, preset: Some(format!("claude-{id}")),
    };
    let washi = |id: &str| Some(DividerStyle::Tape { pattern: format!("image:claude-{id}"), color: None });
    macro_rules! cover { ($id:literal) => { Some(($id, include_bytes!(concat!("claude/covers/", $id, ".jpg")).as_slice())) }; }
    vec![
        // 孔版：双色孔版印的校样，粉、黄、蓝三版叠印，套得微微错位，网点和颗粒都留着。
        ("claude-riso", tr("孔版", "Riso Press"), c("riso", 0x1b2a8f, 0xfff3e0, "", false, washi("riso")), cover!("riso")),
        // 等高线：夜里的测绘图，墨绿底上荧光黄绿的等高线，山顶插一面橙旗；胶带是标尺。
        ("claude-contour", tr("等高线", "Contour"), c("contour", 0xf4ffd6, 0x0e3b2e, "mono", false, washi("contour")), cover!("contour")),
        // 蓝晒：普鲁士蓝的晒图纸上压过蕨叶，叶影是白的，曝光深浅不一；纸是标本签。
        ("claude-cyanotype", tr("蓝晒", "Cyanotype"), c("cyanotype", 0x173f75, 0xf4f0e6, "mono", false, washi("cyanotype")), cover!("cyanotype")),
        // 墨流：水面上漂的墨，黑、骨白，一道朱红；分隔线是一笔手画。
        ("claude-suminagashi", tr("墨流", "Suminagashi"), c("suminagashi", 0xf2ead8, 0x14161c, "serif", false, Some(DividerStyle::Doodle)), None),
        // 水磨石：粉色水泥里嵌着糖果色的石子，胶带换成薄荷底。
        ("claude-terrazzo", tr("水磨石", "Terrazzo"), c("terrazzo", 0x5a1f3d, 0xfff6ec, "rounded", false, washi("terrazzo")), None),
        // 日冕：马戏海报里的太阳，番茄红和粉的光芒，纸是一整张明黄。
        ("claude-sunburst", tr("日冕", "Sunburst"), c("sunburst", 0x2a0e00, 0xffd400, "rounded", false, washi("sunburst")), cover!("sunburst")),
        // 青海波：靛蓝的波纹一层压一层，海上一轮朱红的太阳。
        ("claude-tideline", tr("青海波", "Tideline"), c("tideline", 0x0b2a5b, 0xf3eedd, "serif", false, washi("tideline")), cover!("tideline")),
        // 荧屏：暗室里开着的示波器，绿色的李萨如图形和扫描线；整张淡化，字是荧光绿。
        ("claude-phosphor", tr("荧屏", "Phosphor"), c("phosphor", 0x7cff6b, 0x07140b, "mono", true, washi("phosphor")), cover!("phosphor")),
        // 构成：红、蓝、黄和黑的几何积木，米色纸。
        ("claude-primary", tr("构成", "Primary"), c("primary", 0x111111, 0xf2ebdd, "", false, washi("primary")), cover!("primary")),
        // 票根：台球桌的绿呢上一张泡泡糖粉的纸，胶带是一卷入场券。
        ("claude-ticket", tr("票根", "Ticket Stub"), c("ticket", 0x3b0a14, 0xff9ec4, "serif", false, washi("ticket")), None),
        // 夜曲：酒红的夜里飘着金色的五线谱；分隔线是一道连音线似的手画。
        ("claude-nocturne", tr("夜曲", "Nocturne"), c("nocturne", 0xffe9a8, 0x3a0d12, "serif", false, Some(DividerStyle::Doodle)), None),
        // 莫尔：两组同心圈在品红底上相撞，出了干涉纹；纸是电光青。
        ("claude-moire", tr("莫尔", "Interference"), c("moire", 0x001b3a, 0x3fe0ff, "", false, washi("moire")), None),
    ]
}

/// 现在这套样式叫什么（主面板「样式」那一格里写的）。
fn style_name(style: &DocStyle) -> String {
    if style.is_plain() { return tr("原样", "Original").into(); }
    match style.preset.as_deref() {
        Some(p) => p.strip_prefix("saved:").map(str::to_string).or_else(|| find_preset(p).map(|(n, ..)| n.to_string())).unwrap_or_else(|| tr("自定义样式", "Custom style").into()),
        None => tr("自定义样式", "Custom style").into(),
    }
}

/// 自己存下来的样式：名字和样式，放在文档库旁的 styles.json 里。
fn saved_path() -> std::path::PathBuf { crate::store::Store::root_dir().join("styles.json") }
/// 图库里摆出来的那几套：最近存的六套。
fn shown() -> Vec<(String, DocStyle)> { let mut all = saved(); all.reverse(); all.truncate(6); all }
fn saved() -> Vec<(String, DocStyle)> { std::fs::read(saved_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default() }
fn save_all(all: &[(String, DocStyle)]) -> std::io::Result<()> { std::fs::write(saved_path(), serde_json::to_vec_pretty(all).map_err(std::io::Error::other)?) }
/// 默认样式：新建文档时套上（照 Craft「设置为默认样式」），放在文档库旁的 default-style.json；没有这个文件就是「原样」。
fn default_path() -> std::path::PathBuf { crate::store::Store::root_dir().join("default-style.json") }
fn default_style() -> Option<DocStyle> { std::fs::read(default_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()) }
/// 新建的文档套上默认样式（`Workspace::new_doc` 建好后、列表刷新前调用）。
pub fn apply_default(w: &mut Workspace, id: &str) {
    if let Some(style) = default_style().filter(|s| !s.is_plain()) { let _ = w.store.set_style(id, Some(style)); }
}
/// 某一套（`p:内置id` 或 `s:自己存的名字`）是不是默认样式。
fn is_default(key: &str) -> bool {
    match default_style() { None => key == "p:original", Some(d) => d.preset.as_deref() == Some(&key_preset(key)) }
}
fn key_preset(key: &str) -> String { match key.split_once(':') { Some(("s", n)) => format!("saved:{n}"), Some((_, id)) => id.to_string(), None => String::new() } }
/// 卡片长按和底栏「…」的菜单（照 Craft，IMG_9350/9351）：内置的只有「设置为默认样式」，自己存的还能重命名、删除。
fn style_menu(key: &str) -> Vec<super::panel::MenuItem> {
    use super::panel::MenuItem;
    let mut items = vec![];
    if let Some(name) = key.strip_prefix("s:") { items.push(MenuItem { title: tr("重命名", "Rename").into(), symbol: "character.cursor.ibeam", act: format!("style-rename:{name}"), danger: false }); }
    if is_default(key) { if key != "p:original" { items.push(MenuItem { title: tr("取消默认样式", "Remove as Default").into(), symbol: "doc.badge.ellipsis", act: "style-default:p:original".into(), danger: false }); } }
    else { items.push(MenuItem { title: tr("设置为默认样式", "Set as Default Style").into(), symbol: "doc.text.image", act: format!("style-default:{key}"), danger: false }); }
    if let Some(name) = key.strip_prefix("s:") { items.push(MenuItem { title: tr("删除", "Delete").into(), symbol: "trash", act: format!("style-delete:{name}"), danger: true }); }
    items
}

fn c(v: u32) -> Hsla { rgb(v).into() }
fn h(v: u32) -> String { format!("#{v:06X}") }
fn same(a: &str, v: u32) -> bool { a.eq_ignore_ascii_case(&h(v)) }

/// 文字和纸的对比度太低（WCAG 的比值不到 2.5）：标题栏给个警告，不替使用者改色。
fn low_contrast() -> bool {
    super::style::contrast(crate::editor_theme::Theme::text(), crate::editor_theme::Theme::bg_body()) < 2.5
}

fn backdrop_fill(b: &Backdrop) -> Option<Fill> {
    Some(match b {
        Backdrop::Color { color } => Fill::Color(hex(color)?),
        Backdrop::Gradient { from, to, dir } => Fill::Gradient(hex(from)?, hex(to)?, *dir),
        Backdrop::Image { src, blur } => {
            let path = super::style::image_path(src)?;
            Fill::Image(if *blur { super::style::blurred(&path).unwrap_or(path) } else { path })
        }
    })
}

fn style_card(name: String, style: &DocStyle) -> Card {
    use super::panel::MiniDivider;
    let paper = style.paper.as_deref().and_then(hex).unwrap_or_else(|| crate::themes::base_token("bg_body"));
    let ink = style.text.as_deref().and_then(hex).unwrap_or_else(|| crate::themes::base_token("text"));
    let divider = match &style.divider {
        Some(DividerStyle::Tape { pattern, color }) => tape_preview(pattern, color.as_deref().unwrap_or(super::style::TAPE)).map(MiniDivider::Tape),
        Some(DividerStyle::Doodle) => super::style::doodle_image(&to_hex(ink)).map(MiniDivider::Doodle),
        None => None,
    }.unwrap_or(MiniDivider::Line);
    let design = FONTS.iter().find(|f| Some(f.0) == style.font.as_deref()).map_or("", |f| f.0);
    Card { fill: style.background.as_ref().and_then(backdrop_fill), paper, ink, design, name, faded: style.faded, divider, cover: None, menu: vec![] }
}

fn background_tab(background: &Option<Backdrop>) -> usize {
    match background { None => 0, Some(Backdrop::Color { .. }) => 1, Some(Backdrop::Gradient { .. }) => 2, Some(Backdrop::Image { .. }) => 3 }
}

fn suggested_backgrounds(background: &Option<Backdrop>) -> Vec<PhotoTile> {
    let selected = if let Some(Backdrop::Image { src, .. }) = background { Some(src.as_str()) } else { None };
    let mut sources: Vec<String> = selected.into_iter().map(str::to_string).collect();
    let suggestions = SUGGESTIONS.with(|photos| {
        let photos = photos.borrow();
        if photos.is_empty() { super::style::PHOTO_BACKGROUNDS.iter().map(|photo| photo.id).collect() } else { photos.clone() }
    });
    for id in suggestions {
        let src = format!("builtin:{id}");
        if !sources.contains(&src) { sources.push(src); }
    }
    sources.into_iter().filter_map(|src| {
        let path = super::style::image_path(&src)?;
        let title = src.strip_prefix("builtin:").and_then(super::style::photo_background).map_or_else(
            || if src.starts_with("file:") { tr("当前照片", "Current photo").into() } else { src.trim_start_matches("builtin:").to_string() },
            |photo| tr(photo.zh, photo.en).into(),
        );
        Some(PhotoTile { path, title, credit: None, picked: selected == Some(src.as_str()), act: format!("bg-image:{src}") })
    }).take(6).collect()
}

fn shuffle_suggestions() {
    let mut photos: Vec<_> = super::style::PHOTO_BACKGROUNDS.iter().map(|p| p.id).chain(super::style::BUILTIN).collect();
    for (i, random) in uuid::Uuid::new_v4().as_bytes().iter().enumerate().take(photos.len()).skip(1) {
        photos.swap(i, usize::from(*random) % (i + 1));
    }
    SUGGESTIONS.with(|v| *v.borrow_mut() = photos);
}

fn nodes(page: Page, style: &DocStyle, w: &Workspace, cx: &App) -> Vec<Node> {
    let back = "page:main".to_string();
    match page {
        Page::Main => {
            let font = FONTS.iter().find(|f| Some(f.0) == style.font.as_deref()).unwrap_or(&FONTS[0]);
            let cover = w.editor.as_ref().and_then(|e| e.read(cx).core.doc.cover.as_ref().and_then(|c| crate::store::image_path(&c.src))).filter(|p| p.is_file()).map(|p| p.to_string_lossy().into_owned());
            // 「样式」那一格前面那张是这篇文档本身：带上它的封面（照 Craft）。
            let current = Card { cover: cover.clone(), ..style_card(style_name(style), style) };
            // 后面露出不同风格的页边，前面永远是当前文档。
            let others = presets().into_iter()
                .filter(|(id, _, _)| matches!(*id, "ocean" | "blossom" | "lilac") && Some(*id) != style.preset.as_deref())
                .take(2).map(|(_, name, preset)| style_card(name.into(), &preset)).collect();
            vec![Node::Grab, Node::Tiles(vec![
                vec![
                    Tile { span: 1, title: tr("文本颜色", "Text color").into(), art: Art::Ink { ink: current.ink, paper: current.paper }, act: "page:text".into() },
                    Tile { span: 2, title: tr("字体", "Font").into(), art: Art::Font(font.3.into(), font.0), act: "page:font".into() },
                ],
                vec![
                    Tile { span: 1, title: tr("封面", "Cover").into(), art: Art::Cover(cover), act: "page:cover".into() },
                    Tile { span: 1, title: tr("背景", "Background").into(), art: Art::Fill(current.fill.clone()), act: "page:background".into() },
                    Tile { span: 1, title: tr("纸", "Paper").into(), art: Art::Paper { paper: current.paper, ink: current.ink }, act: "page:paper".into() },
                ],
                vec![
                    Tile { span: 2, title: tr("样式", "Style").into(), art: Art::Stack { current, others }, act: "page:gallery".into() },
                    Tile { span: 1, title: tr("分隔符", "Divider").into(), art: match &style.divider {
                        None => Art::Rules(None),
                        Some(DividerStyle::Tape { pattern, color }) => Art::Rules(tape_preview(pattern, color.as_deref().unwrap_or(super::style::TAPE))),
                        Some(DividerStyle::Doodle) => Art::Symbol("scribble"),
                    }, act: "page:divider".into() },
                ],
            ])]
        }
        Page::Text | Page::Paper => {
            let text = page == Page::Text;
            let (set, palette, base): (&Option<String>, &[u32], Hsla) = if text { (&style.text, &TEXT, crate::themes::base_token("text")) } else { (&style.paper, &PAPER, crate::themes::base_token("bg_body")) };
            let mut fills = vec![Fill::Color(base)];
            fills.extend(palette.iter().map(|v| Fill::Color(c(*v))));
            let picked = match set { None => Some(0), Some(s) => palette.iter().position(|v| same(s, *v)).map(|i| i + 1) };
            let key = if text { "text" } else { "paper" };
            vec![
                Node::Header { title: if text { tr("文本颜色", "Text color") } else { tr("文档颜色", "Page color") }.into(), warn: low_contrast(), reset: set.is_some().then(|| format!("{key}:0")), close: back },
                Node::Swatches { fills, picked, act: format!("{key}:"), custom: Some((format!("custom:{key}"), picked.is_none(), set.as_deref().and_then(hex).unwrap_or(base))) },
            ]
        }
        Page::Font => vec![
            Node::Header { title: tr("字体", "Font").into(), warn: false, reset: style.font.is_some().then(|| "font:0".to_string()), close: back },
            // 照 Craft：字样圆钮，选中的拉长成「字样 名字」的胶囊；名字用英文（Craft 中文界面里也是）。
            Node::Fonts { items: FONTS.iter().zip(SAMPLES).map(|(f, s)| (s.to_string(), f.3.to_string(), f.0)).collect(), picked: Some(FONTS.iter().position(|f| Some(f.0) == style.font.as_deref()).unwrap_or(0)), act: "font:".into() },
        ],
        Page::Divider => {
            let kind = match &style.divider { None => 0, Some(DividerStyle::Tape { .. }) => 1, Some(DividerStyle::Doodle) => 2 };
            let mut v = vec![
                Node::Header { title: tr("分隔符样式", "Divider style").into(), warn: false, reset: style.divider.is_some().then(|| "div-kind:0".to_string()), close: back },
                // 照 Craft：选中那段写字，别的只画图标。
                Node::Segments { symbols: vec!["line.3.horizontal", "bandage", "signature"], titles: vec![tr("线", "Line").into(), tr("和纸胶带", "Washi tape").into(), tr("涂鸦", "Doodle").into()], picked: kind, act: "div-kind:".into() },
            ];
            if let Some(DividerStyle::Tape { pattern, color }) = &style.divider
                && let Some(tile) = tape_preview(pattern, color.as_deref().unwrap_or(super::style::TAPE))
            {
                v.push(Node::Row { label: tr("图案", "Pattern").into(), trailing: Trailing::Chip(tile, "page:pattern".into()) });
            }
            v
        }
        Page::Pattern => {
            let (pattern, color) = match &style.divider { Some(DividerStyle::Tape { pattern, color }) => (pattern.clone(), color.clone().unwrap_or(super::style::TAPE.into())), _ => (super::style::PATTERNS[0].into(), super::style::TAPE.into()) };
            let image = pattern.starts_with("image:");
            let tab = TAB.with(|t| t.get()).unwrap_or(image as usize);
            let mut v = vec![
                Node::Header { title: tr("图案", "Pattern").into(), warn: false, reset: None, close: "page:divider".into() },
                Node::Segments { symbols: vec!["circle.grid.3x3.fill", "photo"], titles: vec![], picked: tab, act: "pat-tab:".into() },
            ];
            if tab == 0 {
                v.push(Node::Thumbs { fills: super::style::PATTERNS.iter().filter_map(|p| tape_preview(p, &color)).collect(), picked: super::style::PATTERNS.iter().position(|p| *p == pattern), act: "pat:".into() });
                let picked = TAPE.iter().position(|v| same(&color, *v));
                v.push(Node::Swatches { fills: TAPE.iter().map(|v| Fill::Color(c(*v))).collect(), picked, act: "pat-color:".into(), custom: Some(("custom:tape".into(), picked.is_none(), hex(&color).unwrap_or(c(TAPE[0])))) });
            } else {
                let mut content = vec![];
                if let Some(strip) = current_washi_strip(&pattern) {
                    content.push(Node::Caption(tr("当前胶带", "Current tape").into()));
                    content.push(Node::Strips { items: vec![strip], cols: 1 });
                }
                content.push(Node::Caption(tr("建议", "Suggested").into()));
                content.push(washi_strips(&washi::SUGGESTED, &pattern, 1));
                content.push(Node::Wide(vec![("photo.on.rectangle", tr("上传", "Upload").into(), "pat-upload".into()), ("ticket", tr("和纸图库", "Washi gallery").into(), "page:washi".into())]));
                let (_, safe, viewport) = super::layout();
                let width = f32::from(viewport.width - px(26.));
                let max_height = f32::from(viewport.height - safe.top - safe.bottom.max(px(34.))) - super::panel::height(&v, width) - 12.;
                if super::panel::height(&content, width) > max_height { v.push(Node::Scroll { nodes: content, max_height: f64::from(max_height.max(1.)) }); }
                else { v.extend(content); }
            }
            v
        }
        Page::Washi => {
            let pattern = match &style.divider { Some(DividerStyle::Tape { pattern, .. }) => pattern.as_str(), _ => "" };
            let max_height = f64::from(super::layout().2.height) * 0.67;
            vec![
                Node::Header { title: tr("和纸胶带", "Washi tape").into(), warn: false, reset: None, close: "page:pattern".into() },
                Node::Scroll { nodes: vec![
                    washi_strips(washi::GALLERY, pattern, 2),
                    Node::Caption(tr("Astra 的胶带", "Astra's tapes").into()),
                    washi_strips(super::astra::STYLES.iter().map(|s| &s.tape), pattern, 2),
                    Node::Caption(tr("Claude 的胶带", "Claude's tapes").into()),
                    washi_strips(washi::CLAUDE, pattern, 2),
                    Node::Caption(tr("经典胶带", "Classic tapes").into()),
                    classic_washi_strips(pattern),
                ], max_height },
            ]
        }
        Page::Gallery => {
            let mine = shown();
            let mut content = vec![];
            if !mine.is_empty() {
                content.push(Node::Caption(tr("你的样式", "Your styles").into()));
                content.push(Node::Cards { items: mine.iter().map(|(n, s)| super::panel::Card { menu: style_menu(&format!("s:{n}")), ..style_card(n.clone(), s) }).collect(), picked: mine.iter().position(|(n, _)| style.preset.as_deref().and_then(|p| p.strip_prefix("saved:")) == Some(n.as_str())), act: "gallery-mine:".into() });
            }
            let all = presets();
            content.push(Node::Caption(tr("内置样式", "Built-in styles").into()));
            content.push(Node::Cards { items: all.iter().map(|(id, n, s)| super::panel::Card { menu: style_menu(&format!("p:{id}")), ..style_card((*n).into(), s) }).collect(), picked: all.iter().position(|(id, _, s)| if s.is_plain() { style.is_plain() } else { style.preset.as_deref() == Some(*id) }), act: "gallery:".into() });
            // 署了名的几组放在内置样式后面，卡片上带着配的封面。
            for (k, (title, _, set)) in collections().into_iter().enumerate() {
                content.push(Node::Caption(title.into()));
                content.push(Node::Cards { items: set.iter().map(|(id, n, s, cover)| super::panel::Card { menu: style_menu(&format!("p:{id}")), cover: cover.and_then(|(name, bytes)| authored_cover(name, bytes)), ..style_card((*n).into(), s) }).collect(), picked: set.iter().position(|(id, ..)| style.preset.as_deref() == Some(*id)), act: format!("gallery-by{k}:") });
            }
            // 现在用的是哪一套：内置的、自己存的就给「…」菜单，改过还没存的给「保存」。
            let key = if style.is_plain() { Some("p:original".to_string()) } else { style.preset.as_deref().map(|p| p.strip_prefix("saved:").map_or_else(|| format!("p:{p}"), |n| format!("s:{n}"))) };
            let menu = key.as_deref().map(style_menu).unwrap_or_default();
            let action = key.is_none().then(|| (tr("保存", "Save").into(), "gallery-save"));
            let default_note = key.as_deref().is_some_and(|k| k != "p:original" && is_default(k));
            let mut v = vec![
                Node::Header { title: tr("样式图库", "Style gallery").into(), warn: false, reset: None, close: back },
                Node::StyleFooter { name: style_name(style), note: if style.preset.as_deref().is_some_and(|p| p.starts_with("saved:")) { tr("由你创建", "Created by you") }
                    else if style.is_plain() { tr("内置样式", "Built-in style") }
                    else if let Some((_, _, by)) = style.preset.as_deref().and_then(find_preset) { by }
                    else { tr("还没有保存", "Not saved yet") }.to_string() + if default_note { tr(" · 默认样式", " · Default") } else { "" }, action, menu },
            ];
            let (_, safe, viewport) = super::layout();
            let width = f32::from(viewport.width - px(26.));
            let available = f32::from(viewport.height - safe.top - safe.bottom.max(px(34.))) - 8.;
            let max_height = available.min(f32::from(viewport.height) * 0.76) - super::panel::height(&v, width);
            v.insert(1, Node::Scroll { nodes: content, max_height: f64::from(max_height.max(1.)) });
            v
        }
        Page::PhotoLibrary => background_gallery::nodes(),
        Page::Cover => {
            let has = w.editor.as_ref().is_some_and(|e| e.read(cx).has_cover());
            let items = COVERS.iter().filter_map(|name| Some(PhotoTile { path: cover_thumb(name)?, title: (*name).into(), credit: None, picked: false, act: format!("cover-preset:{name}") })).collect();
            vec![
                Node::Header { title: tr("选择一张图片", "Choose an image").into(), warn: false, reset: has.then(|| "cover-reset".to_string()), close: back },
                Node::Actions(vec![("arrow.clockwise", tr("随机", "Random").into(), "cover-random".into()),
                    ("magnifyingglass", "Unsplash".into(), "cover-unsplash".into()),
                    ("photo.on.rectangle", tr("相册", "Photos").into(), "cover-photos".into())]),
                Node::Photos { items, library: false },
            ]
        }
        Page::Background => {
            let tab = TAB.with(|t| t.get()).unwrap_or_else(|| background_tab(&style.background));
            let mut v = vec![
                Node::Header { title: tr("背景", "Background").into(), warn: false, reset: style.background.is_some().then(|| "bg-tab:0".to_string()), close: back },
                Node::BackgroundModes { titles: [("无", "None"), ("纯色", "Solid"), ("渐变", "Gradient"), ("照片", "Photo")].map(|(zh, en)| tr(zh, en).into()), picked: tab },
            ];
            let effect = |v: &mut Vec<Node>| {
                v.push(Node::Caption(tr("效果", "Effect").into()));
                v.push(Node::Choice { labels: vec![tr("沉浸式", "Immersive").into(), tr("淡化", "Faded").into()], picked: style.faded as usize, act: "faded:".into() });
            };
            match tab {
                1 => {
                    let now = match &style.background { Some(Backdrop::Color { color }) => Some(color.as_str()), _ => None };
                    let picked = now.and_then(|s| SOLID.iter().position(|v| same(s, *v)));
                    v.push(Node::Swatches { fills: SOLID.iter().map(|v| Fill::Color(c(*v))).collect(), picked, act: "bg-color:".into(), custom: Some(("custom:bg".into(), now.is_some() && picked.is_none(), now.and_then(hex).unwrap_or(c(SOLID[0])))) });
                    effect(&mut v);
                }
                2 => {
                    let (from, to, dir) = match &style.background { Some(Backdrop::Gradient { from, to, dir }) => (hex(from).unwrap_or(c(GRADIENT[0].0)), hex(to).unwrap_or(c(GRADIENT[0].1)), *dir), _ => (c(GRADIENT[0].0), c(GRADIENT[0].1), 0) };
                    let picked = GRADIENT.iter().position(|g| to_hex(from) == h(g.0) && to_hex(to) == h(g.1));
                    v.push(Node::Swatches { fills: GRADIENT.iter().map(|g| Fill::Gradient(c(g.0), c(g.1), dir)).collect(), picked, act: "bg-gradient:".into(), custom: None });
                    v.push(Node::Caption(tr("渐变", "Gradient").into()));
                    v.push(Node::Row { label: tr("起始颜色", "Start color").into(), trailing: Trailing::Chip(Fill::Color(from), "custom:from".into()) });
                    v.push(Node::Row { label: tr("结束颜色", "End color").into(), trailing: Trailing::Chip(Fill::Color(to), "custom:to".into()) });
                    let name = |i: usize| tr(DIRS[i].0, DIRS[i].1).to_string();
                    v.push(Node::Row { label: tr("方向", "Direction").into(), trailing: Trailing::Menu(name(usize::from(dir).min(3)), (0..4).map(|i| (name(i), format!("bg-dir:{i}"))).collect()) });
                    effect(&mut v);
                }
                3 => {
                    let now = match &style.background { Some(Backdrop::Image { src, .. }) => Some(src.as_str()), _ => None };
                    let blur = matches!(&style.background, Some(Backdrop::Image { blur: true, .. }));
                    v.push(Node::Photos { items: suggested_backgrounds(&style.background), library: false });
                    v.push(Node::Actions(vec![("arrow.clockwise", tr("随机", "Random").into(), "bg-random".into()),
                        if unsplash::available() { ("magnifyingglass", "Unsplash".into(), "bg-library".into()) } else { ("globe", tr("在线图库", "Online library").into(), "bg-library".into()) },
                        ("photo.on.rectangle", tr("相册", "Photos").into(), "bg-photos".into())]));
                    if let Some(credit) = now.and_then(background_gallery::credit) { v.push(Node::Caption(credit)); }
                    v.push(Node::Caption(tr("效果", "Effect").into()));
                    v.push(Node::Row { label: tr("模糊图像", "Blur image").into(), trailing: Trailing::Switch(blur, "bg-blur".into()) });
                    v.push(Node::Choice { labels: vec![tr("沉浸式", "Immersive").into(), tr("淡化", "Faded").into()], picked: style.faded as usize, act: "faded:".into() });
                }
                _ => v.push(Node::Empty("rectangle.on.rectangle.angled", tr("添加背景使其更美观", "Add a background to make it prettier").into())),
            }
            let (_, safe, viewport) = super::layout();
            let width = f32::from(viewport.width - px(26.));
            let available = f32::from(viewport.height - safe.top - safe.bottom.max(px(34.))) - 12.;
            if super::panel::height(&v, width) > available {
                let content = v.split_off(2);
                let max = available - super::panel::height(&v, width);
                v.push(Node::Scroll { nodes: content, max_height: f64::from(max.max(1.)) });
            }
            v
        }
    }
}

/// 面板的占位（文档页、面板开着时）：贴在底边上面，两侧各留 13pt（Craft 实测）。主面板点外面就收起。
pub fn element(w: &Workspace, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    if w.view != View::Doc || w.current.is_none() { close(); return None; }
    let page = PAGE.with(|p| p.get())?;
    let style = super::style::of(w).unwrap_or_default();
    let nodes = nodes(page, &style, w, cx);
    let (_, _, viewport) = super::layout();
    let width = viewport.width - px(26.);
    let height = px(super::panel::height(&nodes, f32::from(width)));
    let material = match page {
        // 图库：卡片铺满整页，底下的文档不该再透出来（不然顶上透的是文档、底栏透的是卡片，两头不一样）。
        Page::Gallery => {
            let paper = crate::editor_theme::Theme::bg_body();
            Material { tint: Some(paper.opacity(0.3)), wash: Some(paper.opacity(if crate::theme::dark() { 0.7 } else { 0.78 })) }
        }
        Page::Main | Page::Background | Page::PhotoLibrary | Page::Cover => {
            let paper = crate::editor_theme::Theme::bg_body();
            let dark = crate::theme::dark();
            // 玻璃保留纸后面的颜色；照 Craft 压一层厚一点的纸色，面板读起来是一块磨砂，不让背景图透得太花。
            Material { tint: Some(paper.opacity(if dark { 0.30 } else { 0.12 })), wash: Some(paper.opacity(if dark { 0.32 } else { 0.38 })) }
        }
        Page::Divider | Page::Pattern | Page::Washi => Material { tint: Some(crate::theme::Theme::bg_body().opacity(0.13)), wash: None },
        _ => Material::default(),
    };
    Some(div().id("style-panel").absolute().left(px(13.)).bottom(super::insets().bottom.max(px(34.)) + px(1.)).w(width).h(height).occlude()
        .child(super::panel::slot(nodes, material)).into_any_element())
}

/// 改这篇文档的样式：存进索引，改了手上的那份，让正文重画。
fn edit(w: &mut Workspace, cx: &mut Context<Workspace>, f: impl FnOnce(&mut DocStyle)) {
    update_style(w, cx, f);
}

fn update_style(w: &mut Workspace, cx: &mut Context<Workspace>, f: impl FnOnce(&mut DocStyle)) -> bool {
    let Some(id) = w.current.clone() else { return false; };
    let Some(ix) = w.entries.iter().position(|e| e.id == id) else { return false; };
    let mut style = w.entries[ix].meta.style.clone().unwrap_or_default();
    let before = style.clone();
    f(&mut style);
    let felt = (DocStyle { preset: before.preset.clone(), ..style.clone() }) != before;
    // 从图库里套来的样式，手动改过一处就不算那一套了。
    if style != before && style.preset == before.preset { style.preset = None; }
    let style = (!style.is_plain()).then_some(style);
    if let Err(e) = w.store.set_style(&id, style.clone()) {
        w.show_toast_kind(crate::ui::ToastKind::Error, &e.to_string(), cx);
        return false;
    }
    w.entries[ix].meta.style = style;
    // 文档样式的每一处改动都轻轻一震（取色盘拖动中的连续变化不进这里）。
    // 只是记下「来自哪一套」变了（删掉存的样式时）不算，删除自己有一下更重的。
    if felt { super::glass::selection_haptic(); }
    if let Some(editor) = w.editor.clone() { editor.update(cx, |_, cx| cx.notify()); }
    cx.notify();
    true
}

fn set_photo(src: String, w: &mut Workspace, cx: &mut Context<Workspace>) -> bool {
    update_style(w, cx, |s| {
        let blur = matches!(&s.background, Some(Backdrop::Image { blur: true, .. }));
        s.background = Some(Backdrop::Image { src, blur });
    })
}

/// 封面页下面那六张自带封面（每个系列挑几张）。
const COVERS: [&str; 6] = ["landscape-1", "aurora-2", "gradient-3", "geometry-1", "landscape-4", "aurora-5"];

/// 署了名的样式配的封面：缩略图要文件路径，第一次用时写到临时目录。
fn authored_cover(name: &str, bytes: &[u8]) -> Option<String> {
    let path = std::env::temp_dir().join("quiet-covers").join(format!("by-{name}.jpg"));
    if !path.is_file() { std::fs::create_dir_all(path.parent()?).ok()?; std::fs::write(&path, bytes).ok()?; }
    Some(path.to_string_lossy().into_owned())
}

/// 自带封面编在二进制里；缩略图要文件路径，第一次用时写到临时目录。
fn cover_thumb(name: &str) -> Option<String> {
    let path = std::env::temp_dir().join("quiet-covers").join(format!("{name}.jpg"));
    if !path.is_file() {
        let bytes = crate::editor::cover::preset_bytes(name)?;
        std::fs::create_dir_all(path.parent()?).ok()?;
        std::fs::write(&path, bytes).ok()?;
    }
    Some(path.to_string_lossy().into_owned())
}

/// Unsplash 选中的背景照片：连同出处存进照片目录，再设为背景。
fn unsplash_background(bytes: &[u8], author: &str, link: &str, w: &mut Workspace, cx: &mut Context<Workspace>) {
    let credit = background_gallery::Credit { title: String::new(), author: author.into(), license: "Unsplash".into(), license_url: "https://unsplash.com/license".into(), source: link.into() };
    match background_gallery::store_photo(bytes, &credit) {
        Ok(src) => { if !set_photo(src, w, cx) { cx.notify(); } }
        Err(_) => w.show_toast_kind(crate::ui::ToastKind::Error, tr("没能存下这张图片", "Couldn’t save the image"), cx),
    }
}

/// 面板里点了什么。动作是「名字:参数」。
pub fn action(act: &str, w: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let (name, arg) = act.split_once(':').unwrap_or((act, ""));
    let ix: usize = arg.parse().unwrap_or(0);
    match name {
        // 第一页往下拖走了（`panel.rs` 的下拉收起）。
        "close" => { close(); cx.notify(); }
        "page" => {
            // 进出各个子页（点格子进去、标题栏返回）都轻轻一震。
            super::glass::haptic();
            let previous = PAGE.with(|p| p.replace(Some(match arg { "text" => Page::Text, "font" => Page::Font, "paper" => Page::Paper, "background" => Page::Background, "divider" => Page::Divider, "pattern" => Page::Pattern, "gallery" => Page::Gallery, "washi" => Page::Washi, "cover" => Page::Cover, _ => Page::Main })));
            TAB.with(|t| t.set(match (previous, arg) {
                (Some(Page::Washi), "pattern") => Some(1),
                (Some(Page::PhotoLibrary), "background") => Some(3),
                _ => None,
            }));
            cx.notify();
        }
        "text" => edit(w, cx, |s| s.text = ix.checked_sub(1).and_then(|i| TEXT.get(i)).map(|v| h(*v))),
        "paper" => edit(w, cx, |s| s.paper = ix.checked_sub(1).and_then(|i| PAPER.get(i)).map(|v| h(*v))),
        "font" => edit(w, cx, |s| s.font = FONTS.get(ix).filter(|f| !f.0.is_empty()).map(|f| f.0.to_string())),
        "faded" => edit(w, cx, |s| s.faded = ix == 1),
        // 换到哪一段就用上那一类的背景（原来就是这一类的不动）；第一段是去掉背景。
        "bg-tab" if ix < 4 => {
            let committed = update_style(w, cx, |s| {
                let before = background_tab(&s.background);
                if before == ix { return; }
                s.background = BACKGROUNDS.with(|d| {
                    let mut drafts = d.borrow_mut();
                    if let Some(prev) = before.checked_sub(1) { drafts[prev] = s.background.clone(); }
                    ix.checked_sub(1).and_then(|i| drafts[i].clone())
                }).or_else(|| match ix {
                    1 => Some(Backdrop::Color { color: h(SOLID[0]) }),
                    2 => Some(Backdrop::Gradient { from: h(GRADIENT[0].0), to: h(GRADIENT[0].1), dir: 0 }),
                    3 => Some(Backdrop::Image { src: format!("builtin:{}", super::style::PHOTO_BACKGROUNDS[0].id), blur: false }),
                    _ => None,
                });
                if ix == 0 { s.faded = false; }
            });
            if committed { TAB.with(|t| t.set(Some(ix))); }
        }
        "bg-color" => edit(w, cx, |s| if let Some(v) = SOLID.get(ix) { s.background = Some(Backdrop::Color { color: h(*v) }); }),
        "bg-gradient" => edit(w, cx, |s| if let Some(g) = GRADIENT.get(ix) {
            let dir = match &s.background { Some(Backdrop::Gradient { dir, .. }) => *dir, _ => 0 };
            s.background = Some(Backdrop::Gradient { from: h(g.0), to: h(g.1), dir });
        }),
        "bg-dir" if ix < DIRS.len() => edit(w, cx, |s| if let Some(Backdrop::Gradient { dir, .. }) = &mut s.background { *dir = ix as u8; }),
        "bg-image" => if let Some(name) = arg.strip_prefix("builtin:")
            && (super::style::photo_background(name).is_some() || super::style::BUILTIN.contains(&name)) { set_photo(arg.into(), w, cx); },
        "bg-random" => { shuffle_suggestions(); super::glass::haptic(); cx.notify(); }
        "bg-library" if unsplash::available() => unsplash::open(unsplash::Target::Background),
        "bg-library" => {
            PAGE.with(|p| p.set(Some(Page::PhotoLibrary)));
            background_gallery::load("", w, window, cx);
        }
        "bg-search" => background_gallery::prompt(),
        "bg-retry" => background_gallery::retry(w, window, cx),
        "bg-use-photo" => if let Ok(id) = arg.parse() { background_gallery::apply(id, w, window, cx); },
        "bg-blur" => edit(w, cx, |s| if let Some(Backdrop::Image { blur, .. }) = &mut s.background { *blur = !*blur; }),
        "bg-photos" => {
            let document = w.current.clone();
            let root = crate::store::Store::root_dir();
            let rx = super::prompt_for_images(cx, PathPromptOptions { files: true, directories: false, multiple: false, prompt: None });
            cx.spawn_in(window, async move |this, cx| {
                let Ok(Ok(Some(paths))) = rx.await else { return; };
                let Some(from) = paths.into_iter().next() else { return; };
                let _ = this.update_in(cx, |w, _, cx| {
                    if w.current != document || crate::store::Store::root_dir() != root
                        || PAGE.with(|p| p.get()) != Some(Page::Background)
                        || !matches!(super::style::of(w).and_then(|s| s.background), Some(Backdrop::Image { .. })) { return; }
                    let ext = from.extension().and_then(|s| s.to_str()).unwrap_or("jpg").to_ascii_lowercase();
                    let file = format!("{}.{ext}", uuid::Uuid::new_v4());
                    let dir = super::style::photos_dir();
                    let target = dir.join(&file);
                    if std::fs::create_dir_all(&dir).and_then(|_| std::fs::copy(&from, &target)).is_err() {
                        let _ = std::fs::remove_file(target);
                        return w.show_toast_kind(crate::ui::ToastKind::Error, tr("没能存下这张图片", "Couldn’t save the image"), cx);
                    }
                    if !set_photo(format!("file:{file}"), w, cx) { let _ = std::fs::remove_file(target); }
                });
            }).detach();
        }
        // 封面页（照 Craft 录屏 00:17）：随机、Unsplash、相册，下面六张自带的封面。
        "cover-random" => { super::glass::haptic(); if let Some(editor) = w.editor.clone() { editor.update(cx, |e, cx| e.random_cover(cx)); } cx.notify(); }
        "cover-preset" => if COVERS.contains(&arg) && let Some(editor) = w.editor.clone() { super::glass::selection_haptic(); editor.update(cx, |e, cx| e.apply_preset(arg, cx)); cx.notify(); },
        "cover-reset" => { super::glass::delete_haptic(); if let Some(editor) = w.editor.clone() { editor.update(cx, |e, cx| { e.set_cover_image(None, cx); }); } cx.notify(); }
        "cover-unsplash" if unsplash::available() => unsplash::open(unsplash::Target::Cover),
        "cover-unsplash" => w.show_toast_kind(crate::ui::ToastKind::Error, tr("这个版本没有配置 Unsplash", "Unsplash isn’t set up in this build"), cx),
        "cover-photos" => {
            let document = w.current.clone();
            let rx = super::prompt_for_images(cx, PathPromptOptions { files: true, directories: false, multiple: false, prompt: None });
            cx.spawn_in(window, async move |this, cx| {
                let Ok(Ok(Some(paths))) = rx.await else { return; };
                let Some(from) = paths.into_iter().next() else { return; };
                let ext = from.extension().and_then(|s| s.to_str()).unwrap_or("jpg").to_ascii_lowercase();
                let Ok(bytes) = std::fs::read(&from) else { return; };
                let _ = this.update_in(cx, |w, _, cx| {
                    if w.current != document { return; }
                    if let Some(editor) = w.editor.clone() { super::glass::selection_haptic(); editor.update(cx, |e, cx| { e.set_cover_image(Some((&bytes, &ext)), cx); }); }
                    cx.notify();
                });
            }).detach();
        }
        // 从和纸胶带换走时记下当时的图案和颜色，换回来接着用，不回到默认图案。
        "div-kind" => edit(w, cx, |s| s.divider = match (ix, &s.divider) {
            (1, Some(DividerStyle::Tape { .. })) => s.divider.clone(),
            (_, tape @ Some(DividerStyle::Tape { .. })) if ix != 1 => { LAST_TAPE.with(|t| *t.borrow_mut() = tape.clone()); if ix == 2 { Some(DividerStyle::Doodle) } else { None } }
            (1, _) => LAST_TAPE.with(|t| t.borrow().clone()).or(Some(DividerStyle::Tape { pattern: super::style::PATTERNS[0].into(), color: None })),
            (2, _) => Some(DividerStyle::Doodle),
            _ => None,
        }),
        "pat-tab" => { if TAB.with(|t| t.replace(Some(ix))) != Some(ix) { super::glass::selection_haptic(); } cx.notify(); }
        "pat-washi" if washi::get(arg).is_some() || super::style::TAPES.contains(&arg) || super::style::GALLERY.iter().any(|g| g.0 == arg) => {
            let pattern = format!("image:{arg}");
            edit(w, cx, |s| { let color = match &s.divider { Some(DividerStyle::Tape { color, .. }) => color.clone(), _ => None }; s.divider = Some(DividerStyle::Tape { pattern, color }); });
        }
        // 上传一张图当胶带：和背景照片一样拷进文档库，胶带记 `image:file:<文件名>`。
        "pat-upload" => {
            let rx = super::prompt_for_images(cx, PathPromptOptions { files: true, directories: false, multiple: false, prompt: None });
            cx.spawn_in(window, async move |this, cx| {
                let Ok(Ok(Some(paths))) = rx.await else { return; };
                let Some(from) = paths.into_iter().next() else { return; };
                let _ = this.update_in(cx, |w, _, cx| {
                    let ext = from.extension().and_then(|s| s.to_str()).unwrap_or("jpg").to_ascii_lowercase();
                    let file = format!("{}.{ext}", uuid::Uuid::new_v4());
                    let dir = super::style::photos_dir();
                    if std::fs::create_dir_all(&dir).and_then(|_| std::fs::copy(&from, dir.join(&file))).is_err() { return w.show_toast_kind(crate::ui::ToastKind::Error, tr("没能存下这张图片", "Couldn’t save the image"), cx); }
                    edit(w, cx, |s| { let color = match &s.divider { Some(DividerStyle::Tape { color, .. }) => color.clone(), _ => None }; s.divider = Some(DividerStyle::Tape { pattern: format!("image:file:{file}"), color }); });
                });
            }).detach();
        }
        "pat" | "pat-color" => edit(w, cx, |s| {
            let (pattern, color) = match &s.divider { Some(DividerStyle::Tape { pattern, color }) => (pattern.clone(), color.clone()), _ => (super::style::PATTERNS[0].to_string(), None) };
            s.divider = Some(match name {
                "pat" => DividerStyle::Tape { pattern: super::style::PATTERNS.get(ix).unwrap_or(&"plain").to_string(), color },
                _ => DividerStyle::Tape { pattern: if pattern.starts_with("image:") { super::style::PATTERNS[0].to_string() } else { pattern }, color: TAPE.get(ix).map(|v| h(*v)) },
            });
        }),
        "gallery" => if let Some((_, _, preset)) = presets().into_iter().nth(ix) { edit(w, cx, |s| *s = preset); },
        // 署了名的：套上样式；配了封面、文档又还没有封面的，顺手加上（不换掉别人自己挑的封面）。
        n if n.starts_with("gallery-by") => if let Some((_, _, preset, cover)) = n["gallery-by".len()..].parse::<usize>().ok().and_then(|k| collections().into_iter().nth(k)).and_then(|(.., set)| set.into_iter().nth(ix)) {
            edit(w, cx, |s| *s = preset);
            if let Some((_, bytes)) = cover && let Some(editor) = w.editor.clone() && !editor.read(cx).has_cover() { editor.update(cx, |e, cx| { e.set_cover_image(Some((bytes, "jpg")), cx); }); }
        },
        "gallery-mine" => if let Some((n, mut style)) = shown().into_iter().nth(ix) { style.preset = Some(format!("saved:{n}")); edit(w, cx, |s| *s = style); },
        // 照 Craft（IMG_9347）：先弹「保存样式」让人起名；空着就用「自定义样式」，重名时在后面加数字。
        "gallery-save" => {
            let base = tr("自定义样式", "Custom style");
            if !super::assistant::prompt(tr("保存样式", "Save Style"), None, base, "", super::assistant::Field::Text, None, move |value| {
                let Some(typed) = value else { return; };
                super::menu::dispatch(move |w, _, cx| {
                    let mut all = saved();
                    let base = if typed.is_empty() { base.to_string() } else { typed.clone() };
                    let n = (1..).map(|i| if i == 1 { base.clone() } else { format!("{base} {i}") }).find(|n| all.iter().all(|(m, _)| m != n)).unwrap_or_default();
                    let Some(mut style) = super::style::of(w) else { return; };
                    style.preset = None;
                    all.push((n.clone(), style));
                    if let Err(e) = save_all(&all) { return w.show_toast_kind(crate::ui::ToastKind::Error, &e.to_string(), cx); }
                    edit(w, cx, |s| s.preset = Some(format!("saved:{n}")));
                });
            }) { w.show_toast_kind(crate::ui::ToastKind::Error, tr("没能打开保存对话框", "Couldn’t open the save dialog"), cx); }
        }
        "style-default" => {
            let key = arg;
            let style = if key == "p:original" { None } else if let Some(n) = key.strip_prefix("s:") {
                saved().into_iter().find(|(m, _)| m == n).map(|(_, mut s)| { s.preset = Some(format!("saved:{n}")); s })
            } else { key.strip_prefix("p:").and_then(find_preset).map(|(_, s, _)| s) };
            let done = match style { None => std::fs::remove_file(default_path()).or_else(|e| if e.kind() == std::io::ErrorKind::NotFound { Ok(()) } else { Err(e) }),
                Some(s) => serde_json::to_vec_pretty(&s).map_err(std::io::Error::other).and_then(|b| std::fs::write(default_path(), b)) };
            match done {
                Err(e) => w.show_toast_kind(crate::ui::ToastKind::Error, &e.to_string(), cx),
                Ok(()) => { w.show_toast(if key == "p:original" { tr("新文档不再套用样式", "New documents use no style") } else { tr("新文档会用这套样式", "New documents will use this style") }, cx); cx.notify(); }
            }
        }
        "style-rename" => {
            let old = arg.to_string();
            if !super::assistant::prompt(tr("重命名样式", "Rename Style"), None, arg, arg, super::assistant::Field::Text, None, move |value| {
                let Some(new) = value.filter(|v| !v.is_empty() && *v != old) else { return; };
                let old = old.clone();
                super::menu::dispatch(move |w, _, cx| {
                    let mut all = saved();
                    if all.iter().any(|(m, _)| *m == new) { return w.show_toast_kind(crate::ui::ToastKind::Error, tr("已经有同名的样式了", "A style with that name already exists"), cx); }
                    let Some(entry) = all.iter_mut().find(|(m, _)| *m == old) else { return; };
                    entry.0 = new.clone();
                    if let Err(e) = save_all(&all) { return w.show_toast_kind(crate::ui::ToastKind::Error, &e.to_string(), cx); }
                    if let Some(mut d) = default_style().filter(|d| d.preset.as_deref() == Some(&format!("saved:{old}"))) {
                        d.preset = Some(format!("saved:{new}"));
                        if let Ok(b) = serde_json::to_vec_pretty(&d) { let _ = std::fs::write(default_path(), b); }
                    }
                    if super::style::of(w).and_then(|s| s.preset).as_deref() == Some(&format!("saved:{old}")) { edit(w, cx, |s| s.preset = Some(format!("saved:{new}"))); }
                    cx.notify();
                });
            }) { w.show_toast_kind(crate::ui::ToastKind::Error, tr("没能打开重命名对话框", "Couldn’t open the rename dialog"), cx); }
        }
        "style-delete" => {
            let n = arg.to_string();
            let mut all = saved();
            all.retain(|(m, _)| *m != n);
            if let Err(e) = save_all(&all) { return w.show_toast_kind(crate::ui::ToastKind::Error, &e.to_string(), cx); }
            super::glass::delete_haptic();
            if default_style().is_some_and(|d| d.preset.as_deref() == Some(&format!("saved:{n}"))) { let _ = std::fs::remove_file(default_path()); }
            if super::style::of(w).and_then(|s| s.preset).as_deref() == Some(&format!("saved:{n}")) { edit(w, cx, |s| s.preset = None); } else { cx.notify(); }
        }
        "gallery-delete" => {
            let Some(n) = super::style::of(w).and_then(|s| s.preset).and_then(|p| p.strip_prefix("saved:").map(str::to_string)) else { return; };
            let mut all = saved();
            all.retain(|(m, _)| *m != n);
            if let Err(e) = save_all(&all) { return w.show_toast_kind(crate::ui::ToastKind::Error, &e.to_string(), cx); }
            super::glass::delete_haptic();
            edit(w, cx, |s| s.preset = None);
        }
        "custom" => pick_color(arg, w),
        "picked" => if let Some((target, color)) = arg.split_once('=') {
            let color = color.to_string();
            edit(w, cx, |s| match target {
                "text" => s.text = Some(color),
                "paper" => s.paper = Some(color),
                "bg" => s.background = Some(Backdrop::Color { color }),
                "tape" => { let pattern = match &s.divider { Some(DividerStyle::Tape { pattern, .. }) if !pattern.starts_with("image:") => pattern.clone(), _ => super::style::PATTERNS[0].to_string() }; s.divider = Some(DividerStyle::Tape { pattern, color: Some(color) }); }
                "from" | "to" => {
                    let (mut from, mut to, dir) = match &s.background { Some(Backdrop::Gradient { from, to, dir }) => (from.clone(), to.clone(), *dir), _ => (h(GRADIENT[0].0), h(GRADIENT[0].1), 0) };
                    if target == "from" { from = color } else { to = color }
                    s.background = Some(Backdrop::Gradient { from, to, dir });
                }
                _ => {}
            });
        },
        _ => {}
    }
}

// ---- 自选颜色：系统的取色器（网格 / 光谱 / 滑杆，十六进制，存下来的颜色都是它自己的）

thread_local! { static TARGET: RefCell<String> = const { RefCell::new(String::new()) }; }
thread_local! { static PICK_DELEGATE: Cell<*mut AnyObject> = const { Cell::new(std::ptr::null_mut()) }; }

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "QuietMdColorPickerDelegate"]
    struct ColorDelegate;

    impl ColorDelegate {
        #[unsafe(method(colorPickerViewController:didSelectColor:continuously:))]
        fn selected(&self, _picker: *mut AnyObject, color: *mut AnyObject, continuously: bool) {
            // 手指还在拖的那些不要：每一下都要写索引、派生一份主题。松手时系统会再报一次定下来的颜色。
            if continuously { return; }
            let (mut r, mut g, mut b, mut a) = (0f64, 0f64, 0f64, 0f64);
            let ok: bool = unsafe { msg_send![color, getRed: &mut r, green: &mut g, blue: &mut b, alpha: &mut a] };
            if !ok { return; }
            let byte = |v: f64| (v.clamp(0., 1.) * 255.).round() as u32;
            let target = TARGET.with(|t| t.borrow().clone());
            let act = format!("picked:{target}=#{:02X}{:02X}{:02X}", byte(r), byte(g), byte(b));
            super::menu::dispatch(move |w, window, cx| action(&act, w, window, cx));
        }
    }
);

/// 弹出系统的取色器，半屏高，上面的正文还看得见；挑的颜色一路交回 `picked:目标=#rrggbb`。
fn pick_color(target: &str, w: &Workspace) {
    let style = super::style::of(w).unwrap_or_default();
    let now = match target {
        "text" => Some(crate::editor_theme::Theme::text()),
        "paper" => Some(crate::editor_theme::Theme::bg_body()),
        "bg" => match &style.background { Some(Backdrop::Color { color }) => hex(color), _ => None },
        "from" => match &style.background { Some(Backdrop::Gradient { from, .. }) => hex(from), _ => None },
        "to" => match &style.background { Some(Backdrop::Gradient { to, .. }) => hex(to), _ => None },
        _ => None,
    };
    TARGET.with(|t| *t.borrow_mut() = target.to_string());
    unsafe {
        let Some(class) = AnyClass::get(c"UIColorPickerViewController") else { return; };
        let controller = super::files::root_controller();
        if controller.is_null() { return; }
        let busy: *mut AnyObject = msg_send![controller, presentedViewController];
        if !busy.is_null() { return; }
        let picker: *mut AnyObject = msg_send![class, new];
        let _: () = msg_send![picker, setSupportsAlpha: false];
        if let Some(c) = now { let _: () = msg_send![picker, setSelectedColor: super::glass::ui_color(c)]; }
        let mut delegate = PICK_DELEGATE.with(|d| d.get());
        if delegate.is_null() {
            // 取色器对代理是弱引用：这一个一直留着。
            delegate = msg_send![ColorDelegate::class(), new];
            PICK_DELEGATE.with(|d| d.set(delegate));
        }
        let _: () = msg_send![picker, setDelegate: delegate];
        let _: () = msg_send![picker, setOverrideUserInterfaceStyle: if crate::theme::dark() { 2isize } else { 1isize }];
        let sheet: *mut AnyObject = msg_send![picker, sheetPresentationController];
        if !sheet.is_null() && let Some(detent) = AnyClass::get(c"UISheetPresentationControllerDetent") {
            let medium: *mut AnyObject = msg_send![detent, mediumDetent];
            let detents: *mut AnyObject = msg_send![AnyClass::get(c"NSMutableArray").unwrap(), arrayWithObject: medium];
            if matches!(target, "bg" | "from" | "to") {
                let large: *mut AnyObject = msg_send![detent, largeDetent];
                let _: () = msg_send![detents, addObject: large];
            }
            let _: () = msg_send![sheet, setDetents: detents];
            // 不压暗上面的正文，改色时看得清。
            let _: () = msg_send![sheet, setLargestUndimmedDetentIdentifier: ns_string("com.apple.UIKit.medium")];
            let _: () = msg_send![sheet, setPrefersGrabberVisible: true];
        }
        let _: () = msg_send![controller, presentViewController: picker, animated: true, completion: std::ptr::null_mut::<AnyObject>()];
        let _: () = msg_send![picker, release];
    }
}

unsafe fn ns_string(s: &str) -> *mut AnyObject {
    let c = std::ffi::CString::new(s).unwrap_or_default();
    unsafe { msg_send![AnyClass::get(c"NSString").unwrap(), stringWithUTF8String: c.as_ptr()] }
}
