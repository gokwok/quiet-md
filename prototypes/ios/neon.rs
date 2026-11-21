//! Neon 的十二套文档样式。背景、胶带和封面共用带作者前缀的名字，整套选用和单独挑胶带都从这张表取。
use crate::settings::tr;
use crate::store::{Backdrop, DividerStyle, DocStyle};
use super::{style::PhotoBackdrop, stylepanel::Authored, washi::Tape};

pub(super) struct Style {
    pub background: PhotoBackdrop,
    pub tape: Tape,
    cover: &'static [u8],
    ink: &'static str,
    paper: &'static str,
    font: &'static str,
}

// top 是背景上沿的实测平均色（供顶栏选字色），tint 取自胶带的主色。
macro_rules! style {
    ($id:literal, $zh:literal, $en:literal, $ink:literal, $paper:literal, $font:literal, $top:literal, $tint:literal) => {
        Style {
            background: PhotoBackdrop { id: concat!("neon-", $id), zh: $zh, en: $en, top: $top, image: include_bytes!(concat!("neon/backdrops/", $id, ".jpg")) },
            tape: Tape { id: concat!("neon-", $id), name: $en, tint: $tint, art: include_bytes!(concat!("neon/washi/", $id, ".jpg")) },
            cover: include_bytes!(concat!("neon/covers/", $id, ".jpg")), ink: $ink, paper: $paper, font: $font,
        }
    };
}

pub(super) const STYLES: &[Style] = &[
    style!("prism", "棱镜", "Prism", "#15171E", "#FAF9F6", "", 0x171822, 0x3ce8b5),
    style!("celadon", "影青", "Celadon", "#16332D", "#EEF5F2", "serif", 0xaecec3, 0x78a89b),
    style!("caustics", "焦散", "Caustics", "#0B2E46", "#F4FBFC", "rounded", 0x96def1, 0x29aee0),
    style!("obsidian", "金缮", "Kintsugi", "#F3EADB", "#141312", "serif", 0x201d17, 0xd9a84e),
    style!("dichroic", "虹彩", "Dichroic", "#EEF2FF", "#111326", "mono", 0x5a3f74, 0xff3ea5),
    style!("terracotta", "赤陶", "Terracotta", "#3F1C14", "#FDF4EC", "", 0x606690, 0xd46646),
    style!("aurora", "极光", "Aurora", "#E3FAF4", "#0C1922", "", 0x256c72, 0x24f0a8),
    style!("vermilion", "朱漆", "Lacquer", "#3B100E", "#FBF5EC", "serif", 0xad2e24, 0xb8291e),
    style!("cobalt", "群青", "Cobalt", "#F7F9FF", "#12277A", "mono", 0x0531aa, 0x002fa7),
    style!("matcha", "枯山水", "Karesansui", "#182918", "#DCE8D3", "serif", 0xd0dac7, 0x7a9371),
    style!("amber", "琥珀", "Amber", "#FCEBD0", "#23150B", "rounded", 0x89471c, 0xe08a1e),
    style!("silver", "银盐", "Silver Halide", "#141518", "#F2F3F5", "mono", 0x525459, 0x5a5d66),
];

pub(super) fn presets() -> Vec<Authored> {
    STYLES.iter().map(|s| {
        let id = s.background.id;
        let style = DocStyle {
            text: Some(s.ink.into()), paper: Some(s.paper.into()), font: (!s.font.is_empty()).then(|| s.font.into()),
            background: Some(Backdrop::Image { src: format!("builtin:{id}"), blur: false }),
            divider: Some(DividerStyle::Tape { pattern: format!("image:{id}"), color: None }),
            preset: Some(id.into()), ..Default::default()
        };
        (id, tr(s.background.zh, s.background.en), style, Some((id, s.cover)))
    }).collect()
}
