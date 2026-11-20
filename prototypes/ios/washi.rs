//! iOS 内置和纸胶带。名称和顺序来自录屏里的图库，画面是为 Quiet.md 重绘的纸纹。
//! 同一张纸纹用在图库、图案页和正文，等比缩放到胶带高度后向右续接。

use std::fs;

#[derive(Clone, Copy)]
pub(super) struct Tape {
    pub id: &'static str,
    pub name: &'static str,
    pub tint: u32,
    art: &'static [u8],
}

impl Tape {
    pub fn path(&self) -> Option<String> {
        let dir = std::env::temp_dir().join("quiet-md-washi-v3");
        let path = dir.join(format!("{}.jpg", self.id));
        if !path.is_file() {
            fs::create_dir_all(dir).ok()?;
            let temp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
            if fs::write(&temp, self.art).and_then(|()| fs::rename(&temp, &path)).is_err() {
                let _ = fs::remove_file(temp);
                return None;
            }
        }
        Some(path.to_string_lossy().into_owned())
    }
}

/// 从左端开始铺纸纹，最后一段由胶带边界裁掉。图库和正文共享起点与比例。
pub(super) fn tile_spans(width: f64, height: f64, source_width: f64, source_height: f64) -> impl Iterator<Item = (f64, f64)> {
    let tile = (height * source_width / source_height).max(1.);
    let count = if source_width > 0. && source_height > 0. { (width / tile).ceil() as usize } else { 0 };
    (0..count).map(move |i| (i as f64 * tile, tile))
}

/// 两边各有三道厚薄不一的撕口，坐标是从边缘向里和沿高度的位置。
pub(super) fn torn_edge(right: bool, h: f64) -> [(f64, f64); 7] {
    let depth = (h * 0.16).min(8.);
    let edge = if right { [(0.62, 0.), (0., 0.15), (0.71, 0.31), (0.19, 0.5), (0.85, 0.69), (0., 0.85), (0.55, 1.)] }
        else { [(0.9, 0.), (0., 0.17), (0.69, 0.34), (0.22, 0.52), (0.8, 0.68), (0., 0.86), (0.56, 1.)] };
    edge.map(|(x, y)| (depth * x, h * y))
}

macro_rules! tape {
    ($id:literal, $name:literal, $tint:literal) => {
        Tape { id: $id, name: $name, tint: $tint, art: include_bytes!(concat!("washi/", $id, ".jpg")) }
    };
}

const CHILDHOOD: Tape = tape!("childhood", "Childhood", 0xfbcf3a);
const GROOVY_WATER: Tape = tape!("groovy-water", "Groovy Water", 0x2e74ca);
const PEACHY_PEACH: Tape = tape!("peachy-peach", "Peachy Peach", 0xfa8367);

pub(super) const SUGGESTED: [Tape; 3] = [PEACHY_PEACH, GROOVY_WATER, CHILDHOOD];

// 色值从各张纸纹取样，给键盘工具条上的胶带图标着色。
pub(super) const GALLERY: &[Tape] = &[
    CHILDHOOD,
    GROOVY_WATER,
    PEACHY_PEACH,
    tape!("mint-pixels", "Mint Pixels", 0xbdd2ad),
    tape!("blueberry-basket", "Blueberry Basket", 0x466496),
    tape!("colorama", "Colorama", 0xfec421),
    tape!("swoopy-orange", "Swoopy Orange", 0xeb6a26),
    tape!("beige-paper", "Beige Paper", 0xf0cc98),
    tape!("daisy-season", "Daisy Season", 0xcabd36),
    tape!("golden-blooms", "Golden Blooms", 0xfbba06),
    tape!("frosted-petals", "Frosted Petals", 0xb7dbd0),
    tape!("lemon-fields", "Lemon Fields", 0xfee13f),
    tape!("bubble-cells", "Bubble Cells", 0x4a5b6a),
    tape!("pyjama-pink", "Pyjama Pink", 0xfcbfa3),
    tape!("lavender-dream", "Lavender Dream", 0x6465a2),
    tape!("caramel-picnic", "Caramel Picnic", 0xe2a352),
    tape!("olive-cloth", "Olive Cloth", 0x9f9b66),
    tape!("cranberry-wool", "Cranberry Wool", 0xa8494f),
    tape!("terrazzo", "Terrazzo", 0xd7a074),
    tape!("mt-fuji", "Mt. Fuji", 0xb5705c),
    tape!("spring-whisper", "Spring Whisper", 0xb0907e),
    tape!("handle-with-care", "Handle with Care", 0xed6322),
    tape!("work-in-progress", "Work in Progress", 0xfae01a),
    tape!("taro-paradise", "Taro Paradise", 0xb39bb3),
    tape!("flora-fauna", "Flora Fauna", 0xa46554),
    tape!("folk-art-blue", "Folk Art Blue", 0x7babd2),
    tape!("outline-flowers", "Outline Flowers", 0x908777),
    tape!("imprint-flower", "Imprint Flower", 0xd1a248),
    tape!("flat-pastels", "Flat Pastels", 0xe68667),
    tape!("sunny-seville", "Sunny Seville", 0xe6a854),
    tape!("red-orchid", "Red Orchid", 0xc03f2c),
    tape!("ocean-mosaic", "Ocean Mosaic", 0x656d86),
    tape!("golden-palace", "Golden Palace", 0xb97727),
    tape!("two-tone-leaf", "Two Tone Leaf", 0x9d937f),
    tape!("summer-napkin", "Summer Napkin", 0xedbb45),
    tape!("flower-feast", "Flower Feast", 0xf5b42e),
    tape!("azulejo", "Azulejo", 0x6d728b),
    tape!("mediterranean-bloom", "Mediterranean Bloom", 0x5d6985),
    tape!("old-town-tile", "Old Town Tile", 0x5f7698),
    tape!("radiants", "Radiants", 0xf572d2),
    tape!("kids-room", "Kid's Room", 0xa4bca0),
    tape!("cuddly-bears", "Cuddly Bears", 0xb77c61),
    tape!("stick-it", "Stick It", 0xeef439),
    tape!("eye-spy", "Eye Spy", 0x9a507c),
    tape!("old-money", "Old Money", 0x908c62),
    tape!("cosmic-signs", "Cosmic Signs", 0xa88927),
    tape!("bibbidi-bobbidi-boo", "Bibbidi Bobbidi Boo", 0xbfc159),
    tape!("magical-forest", "Magical Forest", 0xdcb25b),
    tape!("andy-banana", "Andy Banana", 0xfc258d),
    tape!("sheeptation", "Sheeptation", 0xa9c4d6),
    tape!("doggy-power", "Doggy Power", 0x878981),
    tape!("cowabanga", "Cowabanga", 0x1ac3cb),
    tape!("frogalicious", "Frogalicious", 0x9bb544),
    tape!("purr-statement", "Purr Statement", 0xbc793d),
    tape!("pattern-cats", "Pattern Cats", 0xa28e47),
    tape!("foliage-season", "Foliage Season", 0xb54c38),
    Tape { id: "zoo-wash", name: "Zoo Wash", tint: 0xab7756, art: include_bytes!("washi/zoo-wash-repeat.jpg") },
    tape!("to-pug-or-not-to-pug", "To Pug, or Not to Pug", 0xb9ada2),
    tape!("unamused-dogs", "Unamused Dogs", 0xc5fc18),
    tape!("skate-dog", "Skate Dog", 0x9f6e59),
    tape!("kitty-meal", "Kitty Meal", 0xad7e5e),
    tape!("puffs-and-cream", "Puffs and Cream", 0xcaa26d),
    tape!("cotton-meow", "Cotton Meow", 0xe0a17a),
    tape!("dino-parade", "Dino Parade", 0xceb965),
    tape!("fruity-season", "Fruity Season", 0xf7ae59),
    tape!("edible-plushi", "Edible Plushi", 0xf2731c),
    tape!("sushi-sketch", "Sushi Sketch", 0x7c7468),
    tape!("donut-or-teddy", "Donut or Teddy", 0xc4875b),
    tape!("donut-hearts", "Donut Hearts", 0xfdd321),
    tape!("secret-garden", "Secret Garden", 0xb76e63),
    tape!("nomadic-bloom", "Nomadic Bloom", 0xc4baa7),
    tape!("savoire-faire", "Savoire-Faire", 0x88909b),
    tape!("le-mans", "Le Mans", 0x7ca7d1),
    tape!("finish-line", "Finish Line", 0x76736c),
    tape!("tartan-beige", "Tartan Beige", 0x7a6f63),
    tape!("handmade", "Handmade", 0xe45383),
    tape!("spooky", "Spooky", 0x141817),
    tape!("early-bird", "Early Bird", 0xa7b621),
    tape!("emotional-support", "Emotional Support", 0xffc4b6),
    tape!("improvise", "Improvise", 0x953eef),
    tape!("input-needed", "Input Needed", 0xe825d7),
    tape!("just-weird", "Just Weird", 0x81e23a),
    tape!("let-it-go", "Let It Go", 0x91c9ec),
    tape!("cap", "Cap", 0x244384),
    tape!("no-cap", "No Cap", 0x16191e),
    tape!("time-box", "Time Box", 0x8f684e),
    tape!("tl-dr", "TL;DR", 0x6e4de4),
    tape!("very-nice", "Very Nice", 0x8bc853),
    tape!("want", "Want", 0x258fcd),
    tape!("the-notebook", "The Notebook", 0xeccfad),
    tape!("play-mode", "Play Mode", 0x96735c),
    tape!("lab-notes", "Lab Notes", 0xb39c84),
    tape!("atomic-grid", "Atomic Grid", 0xbbcad3),
    tape!("christmas-glow", "Christmas Glow", 0xb23c26),
    tape!("winter-charm", "Winter Charm", 0xd1a282),
    tape!("new-years-resolutions", "New Year's Resolutions", 0x2f2d2a),
];

pub(super) fn get(id: &str) -> Option<&'static Tape> { GALLERY.iter().find(|tape| tape.id == id) }
