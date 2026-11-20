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
pub enum Page { Main, Text, Font, Paper, Background, PhotoLibrary, Divider, Pattern, Gallery, Washi }

#[path = "background_gallery.rs"]
mod background_gallery;

thread_local! { static PAGE: Cell<Option<Page>> = const { Cell::new(None) }; }
// 背景页停在哪一段（没有背景时也能停在别的段上挑）。
thread_local! { static TAB: Cell<Option<usize>> = const { Cell::new(None) }; }
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

fn washi_strips(tapes: &[washi::Tape], pattern: &str, cols: usize) -> Node {
    let selected = pattern.strip_prefix("image:");
    let items = tapes.iter().filter_map(|t| Some(Strip {
        fill: Fill::Image(t.path()?),
        title: t.name.into(),
        picked: selected == Some(t.id),
        act: format!("pat-washi:{}", t.id),
    })).collect();
    Node::Strips { items, cols }
}

fn classic_washi_strips(pattern: &str) -> Node {
    let classic = super::style::GALLERY.iter().map(|t| (t.0, t.1, t.2))
        .chain(super::style::TAPES.into_iter().zip(super::style::TAPE_NAMES).map(|(id, (zh, en))| (id, zh, en)));
    let items = classic.filter_map(|(id, zh, en)| {
        let key = format!("image:{id}");
        Some(Strip { fill: Fill::Image(super::style::tape_image(&key, "", true)?), title: tr(zh, en).into(),
            picked: pattern == key, act: format!("pat-washi:{id}") })
    }).collect();
    Node::Strips { items, cols: 2 }
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
const DIRS: [(&str, &str); 4] = [("从上到下", "Top to bottom"), ("从左到右", "Left to right"), ("左上到右下", "Diagonal down"), ("左下到右上", "Diagonal up")];

// 胶带的颜色：录屏里那一页是压暗的，照着估的（接近 Tailwind 的 500 一档）。
const TAPE: [u32; 11] = [0xc050d8, 0x6a8fc8, 0x1d5fd0, 0x00a6c0, 0x2bb5a0, 0x00a86b, 0xc8b010, 0xd0a020, 0xd97706, 0xd64545, 0xd6459a];

/// 内置的样式（自己配的几套，不是 Craft 的那些）：名字，显示的名字，样式。第一套是什么都不改。
/// 每一套的字、纸、背景、分隔符是一起配的：纸取背景里最浅的那个调子再提亮，字取最深的那个调子。
fn presets() -> Vec<(&'static str, &'static str, DocStyle)> {
    let s = |id: &str, text: u32, paper: u32, font: &str, image: &str, divider: Option<DividerStyle>| DocStyle {
        text: Some(h(text)), paper: Some(h(paper)), font: (!font.is_empty()).then(|| font.to_string()),
        background: Some(Backdrop::Image { src: format!("builtin:{image}"), blur: false }), divider, preset: Some(id.to_string()), ..Default::default()
    };
    let tape = |pattern: &str, color: u32| Some(DividerStyle::Tape { pattern: pattern.into(), color: Some(h(color)) });
    vec![
        ("original", tr("原样", "Original"), DocStyle::default()),
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

/// 现在这套样式叫什么（主面板「样式」那一格里写的）。
fn style_name(style: &DocStyle) -> String {
    if style.is_plain() { return tr("原样", "Original").into(); }
    match style.preset.as_deref() {
        Some(p) => p.strip_prefix("saved:").map(str::to_string).or_else(|| presets().into_iter().find(|(id, ..)| *id == p).map(|(_, n, _)| n.to_string())).unwrap_or_else(|| tr("自定义样式", "Custom style").into()),
        None => tr("自定义样式", "Custom style").into(),
    }
}

/// 自己存下来的样式：名字和样式，放在文档库旁的 styles.json 里。
fn saved_path() -> std::path::PathBuf { crate::store::Store::root_dir().join("styles.json") }
/// 图库里摆出来的那几套：最近存的六套。
fn shown() -> Vec<(String, DocStyle)> { let mut all = saved(); all.reverse(); all.truncate(6); all }
fn saved() -> Vec<(String, DocStyle)> { std::fs::read(saved_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default() }
fn save_all(all: &[(String, DocStyle)]) -> std::io::Result<()> { std::fs::write(saved_path(), serde_json::to_vec_pretty(all).map_err(std::io::Error::other)?) }

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
        Some(DividerStyle::Tape { pattern, color }) => super::style::tape_image(pattern, color.as_deref().unwrap_or(super::style::TAPE), true).map(MiniDivider::Tape),
        Some(DividerStyle::Doodle) => super::style::doodle_image(&to_hex(ink)).map(MiniDivider::Doodle),
        None => None,
    }.unwrap_or(MiniDivider::Line);
    let design = FONTS.iter().find(|f| Some(f.0) == style.font.as_deref()).map_or("", |f| f.0);
    Card { fill: style.background.as_ref().and_then(backdrop_fill), paper, ink, design, name, faded: style.faded, divider }
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
            let cover = w.editor.as_ref().and_then(|e| e.read(cx).core.doc.cover.as_ref().map(|c| c.src.clone()));
            let current = style_card(style_name(style), style);
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
                    Tile { span: 1, title: tr("封面", "Cover").into(), art: Art::Cover(cover), act: "cover".into() },
                    Tile { span: 1, title: tr("背景", "Background").into(), art: Art::Fill(current.fill.clone()), act: "page:background".into() },
                    Tile { span: 1, title: tr("纸", "Paper").into(), art: Art::Paper { paper: current.paper, ink: current.ink }, act: "page:paper".into() },
                ],
                vec![
                    Tile { span: 2, title: tr("样式", "Style").into(), art: Art::Stack { current, others }, act: "page:gallery".into() },
                    Tile { span: 1, title: tr("分隔符", "Divider").into(), art: match &style.divider {
                        None => Art::Rules(None),
                        Some(DividerStyle::Tape { pattern, color }) => Art::Rules(super::style::tape_image(pattern, color.as_deref().unwrap_or(super::style::TAPE), true).map(Fill::Image)),
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
            Node::Segments { symbols: vec![], titles: FONTS.iter().map(|f| tr(f.1, f.2).to_string()).collect(), picked: FONTS.iter().position(|f| Some(f.0) == style.font.as_deref()).unwrap_or(0), act: "font:".into() },
        ],
        Page::Divider => {
            let kind = match &style.divider { None => 0, Some(DividerStyle::Tape { .. }) => 1, Some(DividerStyle::Doodle) => 2 };
            let mut v = vec![
                Node::Header { title: tr("分隔符样式", "Divider style").into(), warn: false, reset: style.divider.is_some().then(|| "div-kind:0".to_string()), close: back },
                // 照 Craft：选中那段写字，别的只画图标。
                Node::Segments { symbols: vec!["line.3.horizontal", "bandage", "signature"], titles: vec![tr("线", "Line").into(), tr("和纸胶带", "Washi tape").into(), tr("涂鸦", "Doodle").into()], picked: kind, act: "div-kind:".into() },
            ];
            if let Some(DividerStyle::Tape { pattern, color }) = &style.divider
                && let Some(tile) = super::style::tape_image(pattern, color.as_deref().unwrap_or(super::style::TAPE), true)
            {
                v.push(Node::Row { label: tr("图案", "Pattern").into(), trailing: Trailing::Chip(Fill::Image(tile), "page:pattern".into()) });
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
                v.push(Node::Thumbs { fills: super::style::PATTERNS.iter().filter_map(|p| super::style::tape_image(p, &color, true).map(Fill::Image)).collect(), picked: super::style::PATTERNS.iter().position(|p| *p == pattern), act: "pat:".into() });
                let picked = TAPE.iter().position(|v| same(&color, *v));
                v.push(Node::Swatches { fills: TAPE.iter().map(|v| Fill::Color(c(*v))).collect(), picked, act: "pat-color:".into(), custom: Some(("custom:tape".into(), picked.is_none(), hex(&color).unwrap_or(c(TAPE[0])))) });
            } else {
                v.push(Node::Caption(tr("建议", "Suggested").into()));
                v.push(washi_strips(&washi::SUGGESTED, &pattern, 1));
                v.push(Node::Wide(vec![("square.and.arrow.up", tr("上传", "Upload").into(), "pat-upload".into()), ("photo.on.rectangle.angled", tr("和纸图库", "Washi gallery").into(), "page:washi".into())]));
            }
            v
        }
        Page::Washi => {
            let pattern = match &style.divider { Some(DividerStyle::Tape { pattern, .. }) => pattern.as_str(), _ => "" };
            let max_height = f64::from(super::layout().2.height) * 0.62;
            vec![
                Node::Header { title: tr("和纸胶带", "Washi tape").into(), warn: false, reset: None, close: "page:pattern".into() },
                Node::Scroll { nodes: vec![
                    washi_strips(washi::GALLERY, pattern, 2),
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
                content.push(Node::Cards { items: mine.iter().map(|(n, s)| style_card(n.clone(), s)).collect(), picked: mine.iter().position(|(n, _)| style.preset.as_deref().and_then(|p| p.strip_prefix("saved:")) == Some(n.as_str())), act: "gallery-mine:".into() });
            }
            let all = presets();
            content.push(Node::Caption(tr("内置样式", "Built-in styles").into()));
            content.push(Node::Cards { items: all.iter().map(|(_, n, s)| style_card((*n).into(), s)).collect(), picked: all.iter().position(|(id, _, s)| if s.is_plain() { style.is_plain() } else { style.preset.as_deref() == Some(*id) }), act: "gallery:".into() });
            let action = if style.preset.as_deref().is_some_and(|p| p.starts_with("saved:")) { Some((tr("删除这个样式", "Delete style").into(), "gallery-delete")) }
                else { (!style.is_plain()).then(|| (tr("保存", "Save").into(), "gallery-save")) };
            let mut v = vec![
                Node::Header { title: tr("样式图库", "Style gallery").into(), warn: false, reset: None, close: back },
                Node::StyleFooter { title: tr("当前样式", "Current style").into(), card: style_card(style_name(style), style), action },
            ];
            let (_, safe, viewport) = super::layout();
            let width = f32::from(viewport.width - px(26.));
            let available = f32::from(viewport.height - safe.top - safe.bottom.max(px(34.))) - 8.;
            let max_height = available.min(f32::from(viewport.height) * 0.76) - super::panel::height(&v, width);
            v.insert(1, Node::Scroll { nodes: content, max_height: f64::from(max_height.max(1.)) });
            v
        }
        Page::PhotoLibrary => background_gallery::nodes(),
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
                        ("globe", tr("在线图库", "Online library").into(), "bg-library".into()),
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
        Page::Main | Page::Gallery | Page::Background | Page::PhotoLibrary => {
            let paper = crate::editor_theme::Theme::bg_body();
            let dark = crate::theme::dark();
            // 玻璃保留纸后面的颜色，纸色薄层压住穿过面板的高饱和图案。
            Material { tint: Some(paper.opacity(if dark { 0.30 } else { 0.12 })), wash: Some(paper.opacity(if dark { 0.25 } else { 0.07 })) }
        }
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
    let background_changed = style.background != before.background || style.faded != before.faded;
    // 从图库里套来的样式，手动改过一处就不算那一套了。
    if style != before && style.preset == before.preset { style.preset = None; }
    let style = (!style.is_plain()).then_some(style);
    if let Err(e) = w.store.set_style(&id, style.clone()) {
        w.show_toast_kind(crate::ui::ToastKind::Error, &e.to_string(), cx);
        return false;
    }
    w.entries[ix].meta.style = style;
    if background_changed && matches!(PAGE.with(|p| p.get()), Some(Page::Background | Page::PhotoLibrary)) {
        super::glass::selection_haptic();
    }
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

/// 面板里点了什么。动作是「名字:参数」。
pub fn action(act: &str, w: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let (name, arg) = act.split_once(':').unwrap_or((act, ""));
    let ix: usize = arg.parse().unwrap_or(0);
    match name {
        "page" => {
            let previous = PAGE.with(|p| p.replace(Some(match arg { "text" => Page::Text, "font" => Page::Font, "paper" => Page::Paper, "background" => Page::Background, "divider" => Page::Divider, "pattern" => Page::Pattern, "gallery" => Page::Gallery, "washi" => Page::Washi, _ => Page::Main })));
            TAB.with(|t| t.set(match (previous, arg) {
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
        // 封面是文档自己的内容：收起面板，打开编辑器里挑封面的那张。
        "cover" => { close(); if let Some(editor) = w.editor.clone() { editor.update(cx, |e, cx| e.add_cover(cx)); } cx.notify(); }
        "div-kind" => edit(w, cx, |s| s.divider = match (ix, &s.divider) {
            (1, Some(DividerStyle::Tape { .. })) => s.divider.clone(),
            (1, _) => Some(DividerStyle::Tape { pattern: super::style::PATTERNS[0].into(), color: None }),
            (2, _) => Some(DividerStyle::Doodle),
            _ => None,
        }),
        "pat-tab" => { TAB.with(|t| t.set(Some(ix))); cx.notify(); }
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
        "gallery-mine" => if let Some((n, mut style)) = shown().into_iter().nth(ix) { style.preset = Some(format!("saved:{n}")); edit(w, cx, |s| *s = style); },
        "gallery-save" => {
            let mut all = saved();
            let base = tr("自定义样式", "Custom style");
            let n = (1..).map(|i| if i == 1 { base.to_string() } else { format!("{base} {i}") }).find(|n| all.iter().all(|(m, _)| m != n)).unwrap_or_default();
            let Some(mut style) = super::style::of(w) else { return; };
            style.preset = None;
            all.push((n.clone(), style));
            if let Err(e) = save_all(&all) { return w.show_toast_kind(crate::ui::ToastKind::Error, &e.to_string(), cx); }
            edit(w, cx, |s| s.preset = Some(format!("saved:{n}")));
        }
        "gallery-delete" => {
            let Some(n) = super::style::of(w).and_then(|s| s.preset).and_then(|p| p.strip_prefix("saved:").map(str::to_string)) else { return; };
            let mut all = saved();
            all.retain(|(m, _)| *m != n);
            if let Err(e) = save_all(&all) { return w.show_toast_kind(crate::ui::ToastKind::Error, &e.to_string(), cx); }
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
