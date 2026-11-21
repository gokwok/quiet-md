//! Astra 的十二套文档样式。背景、胶带和封面共用带作者前缀的名字，整套选用和单独挑胶带都从这张表取。
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

// top 是背景上沿的实测平均色（供顶栏选字色），tint 取自胶带的印花。
macro_rules! style {
    ($id:literal, $zh:literal, $en:literal, $ink:literal, $paper:literal, $font:literal, $top:literal, $tint:literal) => {
        Style {
            background: PhotoBackdrop { id: concat!("astra-", $id), zh: $zh, en: $en, top: $top, image: include_bytes!(concat!("astra/backdrops/", $id, ".jpg")) },
            tape: Tape { id: concat!("astra-", $id), name: $en, tint: $tint, art: include_bytes!(concat!("astra/washi/", $id, ".jpg")) },
            cover: include_bytes!(concat!("astra/covers/", $id, ".jpg")), ink: $ink, paper: $paper, font: $font,
        }
    };
}

pub(super) const STYLES: &[Style] = &[
    style!("skywhale", "鲸背天文台", "Whale Observatory", "#F4DFC0", "#13263B", "serif", 0x12212f, 0xb99152),
    style!("citrus", "柑橘邮局", "Citrus Post", "#253F31", "#FFF1D7", "rounded", 0xa96148, 0xe84522),
    style!("glacier", "冰川折页", "Glacier Fold", "#193450", "#E2F3EF", "", 0xa0bbc6, 0x3e6cad),
    style!("matinee", "胭脂放映室", "Velvet Matinee", "#F9E1CF", "#441C2B", "serif", 0x320c08, 0xc95055),
    style!("lunar", "月面修理铺", "Lunar Workshop", "#332817", "#FFE39A", "mono", 0xf9b303, 0xeeb749),
    style!("rain", "霓虹雨巷", "Neon Downpour", "#DFE2FF", "#25223B", "mono", 0x6032a3, 0x76e2da),
    style!("peacock", "孔雀花窗", "Peacock Glass", "#193E37", "#E0EFE0", "serif", 0x53792c, 0x328476),
    style!("dune", "沙海驿站", "Dune Express", "#502B26", "#FBE1BC", "", 0xcf6835, 0xa85542),
    style!("brocade", "云锦来信", "Crane Brocade", "#FFE4C8", "#51263D", "rounded", 0x872d33, 0xec9167),
    style!("gravity", "桃色引力", "Soft Gravity", "#442F50", "#F5E4F2", "rounded", 0xd1a7a5, 0x957a95),
    style!("medusae", "夜航水母", "Midnight Medusae", "#D4F1DF", "#112F31", "serif", 0x142118, 0xe9ab98),
    style!("stairs", "无尽阶梯", "Endless Stairs", "#2E302C", "#F2EDDF", "", 0x94836f, 0xd84433),
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
