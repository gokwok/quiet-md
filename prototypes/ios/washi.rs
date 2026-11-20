//! iOS 内置和纸胶带。名称和顺序来自录屏里的图库，画面是为 Quiet.md 重绘的纸纹。
//! 同一张纸纹用在图库、图案页和正文，等比缩放到胶带高度后向右续接。

use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

fn cache_dir() -> PathBuf { std::env::temp_dir().join("quiet-md-washi-v5") }

fn cache_file(path: &Path, write: impl FnOnce(&Path) -> Option<()>) -> Option<()> {
    if path.is_file() { return Some(()); }
    fs::create_dir_all(path.parent()?).ok()?;
    let temp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let result = write(&temp).and_then(|()| fs::rename(&temp, path).ok());
    if result.is_none() { let _ = fs::remove_file(temp); }
    result
}

#[derive(Clone, Copy)]
pub(super) struct Tape {
    pub id: &'static str,
    pub name: &'static str,
    pub tint: u32,
    art: &'static [u8],
}

impl Tape {
    pub fn path(&self) -> Option<String> {
        let path = cache_dir().join(format!("{}.jpg", self.id));
        cache_file(&path, |temp| fs::write(temp, self.art).ok())?;
        Some(path.to_string_lossy().into_owned())
    }
}

/// 长胶带用同一幅纸纹。只有超出原画时才准备一份可续接的缓存，供 UIKit 和 GPUI 共用。
pub(super) fn repeat_texture(path: &Path) -> Option<PathBuf> {
    let mut key = std::hash::DefaultHasher::new();
    path.hash(&mut key);
    let cached = cache_dir().join(format!("wrap-{:016x}.png", key.finish()));
    cache_file(&cached, |temp| {
        let mut reader = image::ImageReader::open(path).ok()?.with_guessed_format().ok()?;
        if !matches!(reader.format(), Some(image::ImageFormat::Jpeg | image::ImageFormat::Png)) { return None; }
        let mut limits = image::Limits::default();
        limits.max_alloc = Some(32 * 1024 * 1024);
        reader.limits(limits);
        use image::ImageDecoder;
        let mut decoder = reader.into_decoder().ok()?;
        let (w, h) = decoder.dimensions();
        if u64::from(w) * u64::from(h) > 8_000_000 { return None; }
        let orientation = decoder.orientation().ok()?;
        let mut art = image::DynamicImage::from_decoder(decoder).ok()?;
        art.apply_orientation(orientation);
        let mut art = art.into_rgba8();
        if let Some(width) = repeat_width(path).filter(|&width| width < art.width()) {
            // 在纹样相位对齐的地方续接，例如保住棋盘格的间距和完整的号码。
            let height = art.height();
            art = image::imageops::crop_imm(&art, 0, 0, width, height).to_image();
        } else {
            soften_join(&mut art);
        }
        art.save_with_format(temp, image::ImageFormat::Png).ok()
    })?;
    Some(cached)
}

/// 逐款核对后确认的内置纹样循环宽度，单位为原图像素。
fn repeat_width(path: &Path) -> Option<u32> {
    if path.parent()? != cache_dir() { return None; }
    Some(match path.file_stem()?.to_str()? {
        "mint-pixels" => 543,
        "blueberry-basket" => 678,
        "lavender-dream" => 767,
        "olive-cloth" => 619,
        "sunny-seville" => 739,
        "old-town-tile" => 750,
        "skate-dog" => 620,
        "edible-plushi" => 634,
        "le-mans" => 699,
        "finish-line" => 600,
        "want" => 720,
        _ => return None,
    })
}

/// 保留图案的起点，在末端极窄的一段把色彩收进下一轮的纹理，消除首尾间的色阶跳变。
fn soften_join(art: &mut image::RgbaImage) {
    let (width, height) = art.dimensions();
    let feather = (height / 6).clamp(2, 40).min(width / 4);
    if feather < 2 { return; }
    // 两端本身已能直接对接的纸纹不需要过渡。
    let already_continuous = (0..height).all(|y| {
        let (left, right) = (art.get_pixel(0, y), art.get_pixel(width - 1, y));
        left.0.iter().zip(right.0).all(|(a, b)| a.abs_diff(b) < 8)
    });
    if already_continuous { return; }
    for y in 0..height {
        for offset in 0..feather {
            let x = width - feather + offset;
            let t = offset as f32 / (feather - 1) as f32;
            let weight = t * t * (3. - 2. * t);
            let left = *art.get_pixel(feather - 1 - offset, y);
            let right = art.get_pixel_mut(x, y);
            let (a, b) = (f32::from(right[3]) * (1. - weight), f32::from(left[3]) * weight);
            let alpha = a + b;
            if alpha > 0. {
                for (value, next) in right.0[..3].iter_mut().zip(left.0) {
                    *value = ((f32::from(*value) * a + f32::from(next) * b) / alpha).round() as u8;
                }
            }
            right[3] = alpha.round() as u8;
        }
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
const GROOVY_WATER: Tape = Tape { id: "groovy-water", name: "Groovy Water", tint: 0x2e74ca, art: include_bytes!("washi/groovy-water-long.jpg") };
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
    Tape { id: "zoo-wash", name: "Zoo Wash", tint: 0xab7756, art: include_bytes!("washi/zoo-wash-long.jpg") },
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
