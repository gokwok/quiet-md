//! iOS 内置和纸胶带。名称和顺序来自录屏里的图库，画面是为 Quiet.md 重绘的纸纹。
//! 同一张小图用在列表样张和正文；在临时目录里落一份，供 UIKit 和 GPUI 按文件读取。

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
        let dir = std::env::temp_dir().join("quiet-md-washi-v1");
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

macro_rules! tape {
    ($id:literal, $name:literal, $tint:literal) => {
        Tape { id: $id, name: $name, tint: $tint, art: include_bytes!(concat!("washi/", $id, ".jpg")) }
    };
}

const CHILDHOOD: Tape = tape!("childhood", "Childhood", 0xfbdc9b);
const GROOVY_WATER: Tape = tape!("groovy-water", "Groovy Water", 0x4567b3);
const PEACHY_PEACH: Tape = tape!("peachy-peach", "Peachy Peach", 0x888a6b);

pub(super) const SUGGESTED: [Tape; 3] = [PEACHY_PEACH, GROOVY_WATER, CHILDHOOD];

// 色值从各张纸纹取样，给键盘工具条上的胶带图标着色。
pub(super) const GALLERY: &[Tape] = &[
    CHILDHOOD,
    GROOVY_WATER,
    PEACHY_PEACH,
    tape!("mint-pixels", "Mint Pixels", 0xbdd2ad),
    tape!("blueberry-basket", "Blueberry Basket", 0x466496),
    tape!("colorama", "Colorama", 0xd97c59),
    tape!("swoopy-orange", "Swoopy Orange", 0xf79a2b),
    tape!("beige-paper", "Beige Paper", 0xf1ddbe),
    tape!("daisy-season", "Daisy Season", 0xbfa460),
    tape!("golden-blooms", "Golden Blooms", 0xe9a726),
    tape!("frosted-petals", "Frosted Petals", 0xb1c2cf),
    tape!("lemon-fields", "Lemon Fields", 0xfdda66),
    tape!("bubble-cells", "Bubble Cells", 0x4a5057),
    tape!("pyjama-pink", "Pyjama Pink", 0xf3bbb5),
    tape!("lavender-dream", "Lavender Dream", 0xe9d8e8),
    tape!("caramel-picnic", "Caramel Picnic", 0xe2a352),
    tape!("olive-cloth", "Olive Cloth", 0x9f9b66),
    tape!("cranberry-wool", "Cranberry Wool", 0xa8494f),
    tape!("terrazzo", "Terrazzo", 0xd7a074),
    tape!("mt-fuji", "Mt. Fuji", 0xa0826f),
    tape!("spring-whisper", "Spring Whisper", 0xb0907e),
    tape!("handle-with-care", "Handle with Care", 0xbe5f33),
    tape!("work-in-progress", "Work in Progress", 0xf0c543),
    tape!("taro-paradise", "Taro Paradise", 0x8e6e91),
    tape!("flora-fauna", "Flora Fauna", 0xbe562e),
    tape!("folk-art-blue", "Folk Art Blue", 0x567a9d),
    tape!("outline-flowers", "Outline Flowers", 0x786f64),
    tape!("imprint-flower", "Imprint Flower", 0xa7b0b4),
    tape!("flat-pastels", "Flat Pastels", 0xeba33d),
    tape!("sunny-seville", "Sunny Seville", 0xf3bd59),
    tape!("red-orchid", "Red Orchid", 0xc35b31),
    tape!("ocean-mosaic", "Ocean Mosaic", 0x577595),
    tape!("golden-palace", "Golden Palace", 0xc7994c),
    tape!("two-tone-leaf", "Two Tone Leaf", 0x7a766e),
    tape!("summer-napkin", "Summer Napkin", 0xedbb45),
    tape!("flower-feast", "Flower Feast", 0xf2a92b),
    tape!("azulejo", "Azulejo", 0x45629e),
    tape!("mediterranean-bloom", "Mediterranean Bloom", 0x456595),
    tape!("old-town-tile", "Old Town Tile", 0x586588),
    tape!("radiants", "Radiants", 0xc0d2e2),
    tape!("kids-room", "Kid's Room", 0xa4bca0),
    tape!("cuddly-bears", "Cuddly Bears", 0xd89d5a),
    tape!("stick-it", "Stick It", 0xdfd65a),
    tape!("eye-spy", "Eye Spy", 0xe06b99),
    tape!("old-money", "Old Money", 0x908c62),
    tape!("cosmic-signs", "Cosmic Signs", 0xa88927),
    tape!("bibbidi-bobbidi-boo", "Bibbidi Bobbidi Boo", 0xbfc159),
    tape!("magical-forest", "Magical Forest", 0x949863),
    tape!("andy-banana", "Andy Banana", 0xc0cd41),
    tape!("sheeptation", "Sheeptation", 0x898e90),
    tape!("doggy-power", "Doggy Power", 0x71726b),
    tape!("cowabanga", "Cowabanga", 0x6c7f90),
    tape!("frogalicious", "Frogalicious", 0xa5a46a),
    tape!("purr-statement", "Purr Statement", 0xbc793d),
    tape!("pattern-cats", "Pattern Cats", 0xa48f54),
    tape!("foliage-season", "Foliage Season", 0xe68743),
    tape!("zoo-wash", "Zoo Wash", 0xc87943),
    tape!("to-pug-or-not-to-pug", "To Pug, or Not to Pug", 0xa1856a),
    tape!("unamused-dogs", "Unamused Dogs", 0xb5674a),
    tape!("skate-dog", "Skate Dog", 0x907857),
    tape!("kitty-meal", "Kitty Meal", 0xad7e5e),
    tape!("puffs-and-cream", "Puffs and Cream", 0xcaa26d),
    tape!("cotton-meow", "Cotton Meow", 0xe0a17a),
    tape!("dino-parade", "Dino Parade", 0xfad877),
    tape!("fruity-season", "Fruity Season", 0xf7ae59),
    tape!("edible-plushi", "Edible Plushi", 0xde9d73),
    tape!("sushi-sketch", "Sushi Sketch", 0x7c7468),
    tape!("donut-or-teddy", "Donut or Teddy", 0xc4875b),
    tape!("donut-hearts", "Donut Hearts", 0xd4a163),
    tape!("secret-garden", "Secret Garden", 0xbaad81),
    tape!("nomadic-bloom", "Nomadic Bloom", 0xc4baa7),
    tape!("savoire-faire", "Savoire-Faire", 0x88909b),
    tape!("le-mans", "Le Mans", 0x7ca7d1),
    tape!("finish-line", "Finish Line", 0x76736c),
    tape!("tartan-beige", "Tartan Beige", 0x7a6f63),
    tape!("handmade", "Handmade", 0xdda73a),
    tape!("spooky", "Spooky", 0x787269),
    tape!("early-bird", "Early Bird", 0x9a8e54),
    tape!("emotional-support", "Emotional Support", 0x9c6f7b),
    tape!("improvise", "Improvise", 0x8c79b9),
    tape!("input-needed", "Input Needed", 0xad639e),
    tape!("just-weird", "Just Weird", 0xbdd287),
    tape!("let-it-go", "Let It Go", 0xa8c2d3),
    tape!("cap", "Cap", 0x435b89),
    tape!("no-cap", "No Cap", 0x333335),
    tape!("time-box", "Time Box", 0xa39274),
    tape!("tl-dr", "TL;DR", 0x6755a9),
    tape!("very-nice", "Very Nice", 0xadc28a),
    tape!("want", "Want", 0x80a6ca),
    tape!("the-notebook", "The Notebook", 0xe4dfcb),
    tape!("play-mode", "Play Mode", 0x96735c),
    tape!("lab-notes", "Lab Notes", 0xb39c84),
    tape!("atomic-grid", "Atomic Grid", 0xbbcad3),
    tape!("christmas-glow", "Christmas Glow", 0xa62223),
    tape!("winter-charm", "Winter Charm", 0xd9dbe6),
    tape!("new-years-resolutions", "New Year's Resolutions", 0x795a66),
];

pub(super) fn get(id: &str) -> Option<&'static Tape> { GALLERY.iter().find(|tape| tape.id == id) }
