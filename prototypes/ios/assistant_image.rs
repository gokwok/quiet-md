//! 设置 › MarginAI › 图像：NovelAI 生图。服务商只有 NovelAI（接口地址可以换成同协议的别的服务），API Key 只在设备钥匙串里保存、按地址分开；
//! 模型可选 V5 和 V4.5（一排横滑的小卡片），尺寸三种，步数、CFG、采样器固定，种子每次随机。
//! 页面最上面是顶到屏幕边的「画布」：上一次试画的图铺满（没画过是自带的一张），高矮跟着画幅变；
//! 浮在图下沿的一排原生玻璃是画幅、试画的对象和「试画」（套着现在的画风、质量词，也可带入人物）。
//! 下面一张圆角的面板压着图的下沿，托着其余设置。静物试画顺手给这个画风存一张样张，画风那排用（`assistant_image_prompts.rs`）。
//! 连上以后「服务商连接」排到最后；没有 Key 时在最前。
use std::{cell::{Cell, RefCell}, collections::{HashMap, HashSet}, path::PathBuf, sync::Arc, time::{Duration, Instant}};

use futures::channel::oneshot;
use gpui::{prelude::FluentBuilder, *};
use serde_json::{json, Value};

use crate::{app::Workspace, settings::{prefs, row_chevron, row_ink, tr}, theme::{self, Theme, craft}, ui};
use crate::ios::{coveredit, general, glass, imagine, menu};
use super::{Card, Field, Secret};

pub const DEFAULT_MODEL: &str = "nai-diffusion-5-full";
const SERVICE: &str = "com.s1dashu.quietmd.novelai";
/// 账户接口也搬到了这个域名，`api.novelai.net` 只回 400。
const OFFICIAL: &str = "https://image.novelai.net";
const PATH: &str = "/ai/generate-image";
/// 四个都用同一种请求体实测出过图（2026-11-21）。说明照官方文档（docs.novelai.net/en/image/models）的说法：Full 数据更全，Curated 更干净。
const MODELS: &[Card] = &[
    ("image-model-v5-full", DEFAULT_MODEL, "NAI V5", "V5", ("数据最全，题材最广", "The broadest dataset"), ("完整", "Full")),
    ("image-model-v5-curated", "nai-diffusion-5-curated", "NAI V5 Curated", "V5", ("数据更干净，更稳妥", "A cleaner, safer dataset"), ("精选", "Curated")),
    ("image-model-v4-5-full", "nai-diffusion-4-5-full", "NAI V4.5", "V4.5", ("上一代，数据更全", "Previous, broader dataset"), ("完整", "Full")),
    ("image-model-v4-5-curated", "nai-diffusion-4-5-curated", "NAI V4.5 Curated", "V4.5", ("上一代，数据更干净", "Previous, cleaner dataset"), ("精选", "Curated")),
];
/// (元素 id, 键, 名称, 比例文字, 宽, 高)。偏好里空的键当 2:3。
#[allow(clippy::type_complexity)]
const SIZES: &[(&str, &str, (&str, &str), &str, u32, u32)] = &[
    ("image-size-portrait", "portrait", ("竖版", "Portrait"), "2:3", 832, 1216),
    ("image-size-landscape", "landscape", ("横版", "Landscape"), "3:2", 1216, 832),
    ("image-size-square", "square", ("方形", "Square"), "1:1", 1024, 1024),
];
const STEPS: u32 = 28;
const SCALE: f32 = 3.5;
const SAMPLER: &str = "k_euler_ancestral";
const TEST_PROMPT: &str = "no humans, scenery, a quiet wooden desk by the window, morning light, books, potted plant";
const TEST_PERSON_PROMPT: &str = "upper body, standing by a sunlit window, soft morning light, gentle smile, looking at viewer";
const TEST_SUBJECT_SCENE: &str = "image-test-subject-scene";
const TEST_PERSON_ITEM: &str = "image-test-person-";
/// 质量词（正向、负向）在设置里改，默认的在 `fdl_ai::imagine::QUALITY`；和画风是两回事。

#[derive(Clone)]
enum Test {
    Idle,
    Running((u32, u32)),
    Done { secs: f32, size: (u32, u32), meta: String, image: Arc<Image>, path: Option<PathBuf> },
    Failed(String),
}

thread_local! {
    static KEY_SAVED: Cell<Option<bool>> = const { Cell::new(None) };
    static TEST: RefCell<Test> = const { RefCell::new(Test::Idle) };
    static TEST_RUN: Cell<u64> = const { Cell::new(0) };
    static TEST_CALL: RefCell<Option<tokio::task::AbortHandle>> = const { RefCell::new(None) };
    /// 测试试画选中的人物 id；`None` 用固定静物场景。
    static TEST_PERSON: RefCell<Option<String>> = const { RefCell::new(None) };
    /// 试画菜单里各人物的菜单项 id（要 `&'static str`，同一个人只记一份）。
    static PERSON_ITEMS: RefCell<HashSet<&'static str>> = RefCell::new(HashSet::new());
    /// 测试图在窗口里的位置，点按放大全屏查看用。
    static TEST_FRAME: Cell<Option<Bounds<Pixels>>> = const { Cell::new(None) };
    /// 画布换画幅时从多高变过来：(第几次, 原来的高, 什么时候点的)。过了动画的时间就不算数（离开再回来不重放）。
    static MORPH: Cell<(usize, f32, Option<Instant>)> = const { Cell::new((0, 0., None)) };
    /// 各画风的样张：标签的指纹 → 文件。
    static THUMBS: RefCell<HashMap<String, PathBuf>> = RefCell::new(HashMap::new());
}

fn model_of(id: &str) -> &'static Card { MODELS.iter().find(|m| m.1 == id).unwrap_or(&MODELS[0]) }
/// 模型名对应的显示名（「NAI V5」）；不认识的原样。
pub(crate) fn model_name(id: &str) -> &str { MODELS.iter().find(|m| m.1 == id).map_or(id, |m| m.2) }
fn size_of(key: &str) -> &'static (&'static str, &'static str, (&'static str, &'static str), &'static str, u32, u32) { SIZES.iter().find(|s| s.1 == key).unwrap_or(&SIZES[0]) }

/// 一次生图要的东西。`characters` 是各人物的分角色提示词和排除词。种子不给就随机。
pub(crate) struct Request { pub prompt: String, pub negative: String, pub characters: Vec<(String, String)>, pub seed: Option<u32> }

/// 比例的键（`SIZES`）对应的宽高；不认识的当 2:3。
pub(crate) fn size(key: &str) -> (u32, u32) { let s = size_of(key); (s.4, s.5) }
/// 偏好里的比例键（空的当 2:3）。
pub(crate) fn size_key() -> &'static str { size_of(&prefs().image_size).1 }

/// 现在设置里的模型和尺寸。
pub(crate) fn current() -> (&'static str, (u32, u32)) {
    let p = prefs();
    let s = size_of(&p.image_size);
    (model_of(&p.image_model).1, (s.4, s.5))
}

/// 请求体。V4.5 和 V5 都要 V4 的 `v4_prompt` / `v4_negative_prompt` 结构（缺了服务端回 500）；`params_version` V5 是 4，V4.5 是 3。
/// 质量词和负面预设都由调用方自己拼，预设只是标记：V4.5 照老写法（`qualityToggle` 关、`ucPreset` 4），
/// V5 照网页端的 `params_version` 4，按名字写（`qualityPresetId`、`ucPresetId` 都是 `none`，V5 的负面预设只有三档，序号 4 对不上）。
/// 人物各占一条分角色提示词（不给位置，`use_coords` 关，由模型自己摆），负面那边同一位置是这个人的排除词（没有就空着）。
fn body(model: &str, (width, height): (u32, u32), r: &Request, seed: u32) -> Value {
    let v5 = model.starts_with("nai-diffusion-5");
    let center = json!([{ "x": 0.5, "y": 0.5 }]);
    let chars: Vec<Value> = r.characters.iter().map(|(c, uc)| json!({ "prompt": c, "uc": uc, "center": { "x": 0.5, "y": 0.5 }, "enabled": true })).collect();
    let captions: Vec<Value> = r.characters.iter().map(|(c, _)| json!({ "char_caption": c, "centers": center })).collect();
    let negatives: Vec<Value> = r.characters.iter().map(|(_, uc)| json!({ "char_caption": uc, "centers": center })).collect();
    let mut body = json!({ "input": r.prompt, "model": model, "action": "generate", "parameters": {
        "params_version": if v5 { 4 } else { 3 }, "width": width, "height": height, "scale": SCALE, "sampler": SAMPLER, "steps": STEPS,
        "n_samples": 1, "seed": seed, "noise_schedule": "karras", "cfg_rescale": 0,
        "legacy": false, "legacy_v3_extend": false, "characterPrompts": chars, "negative_prompt": r.negative,
        "v4_prompt": { "caption": { "base_caption": r.prompt, "char_captions": captions }, "use_coords": false, "use_order": true },
        "v4_negative_prompt": { "caption": { "base_caption": r.negative, "char_captions": negatives }, "legacy_uc": false } } });
    let presets = if v5 { json!({ "qualityPresetId": "none", "ucPresetId": "none" }) } else { json!({ "qualityToggle": false, "ucPreset": 4 }) };
    if let (Some(p), Value::Object(extra)) = (body["parameters"].as_object_mut(), presets) { p.extend(extra); }
    body
}

/// 生成一张，返回 PNG。`url` 是 `endpoint()`。在 `llm::runtime()` 上跑。429 不自动重试（NovelAI 要求生图由人发起）。
pub(crate) async fn generate(url: &str, key: &str, model: &str, size: (u32, u32), r: &Request) -> Result<Vec<u8>, String> {
    let failed = |_| tr("无法连接图像服务", "Could not reach the image service").to_owned();
    // Cloudflare 拦 Python 默认的 User-Agent（1010）；写明是谁。
    let http = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(120)).user_agent(concat!("Margin/", env!("CARGO_PKG_VERSION"))).build().map_err(|e| e.to_string())?;
    let seed = r.seed.unwrap_or_else(|| uuid::Uuid::new_v4().as_u128() as u32);
    let response = http.post(url).bearer_auth(key).header("content-type", "application/json").body(body(model, size, r, seed).to_string()).send().await.map_err(failed)?;
    let status = response.status();
    let bytes = response.bytes().await.map_err(failed)?;
    if !status.is_success() {
        // `{"statusCode": …, "message": …}`；Cloudflare 拦下的是纯文本。
        let said = serde_json::from_slice::<Value>(&bytes).ok().and_then(|v| v.get("message").and_then(Value::as_str).map(str::to_owned));
        return Err(match status.as_u16() {
            401 => tr("API Key 无效", "The API Key is invalid").into(),
            402 => tr("需要有效的订阅或足够的 Anlas", "An active subscription or enough Anlas is needed").into(),
            429 => tr("NovelAI 正忙，稍后再试", "NovelAI is busy. Try again shortly").into(),
            code => said.unwrap_or_else(|| format!("HTTP {code}")),
        });
    }
    // 别的服务有的直接回 PNG，不包 zip。
    if bytes.starts_with(b"\x89PNG") { return Ok(bytes.to_vec()); }
    unzip(&bytes).ok_or_else(|| tr("服务没有返回图片", "The service returned no image").to_owned())
}

/// 返回的是只装一张 PNG 的 zip。本地头里的长度是 0（数据描述符在数据后面），所以从数据起点解到 deflate 流结束。
fn unzip(zip: &[u8]) -> Option<Vec<u8>> {
    let u16_at = |i: usize| zip.get(i..i + 2).map(|b| u16::from_le_bytes([b[0], b[1]]) as usize);
    if zip.get(..4)? != b"PK\x03\x04" { return None; }
    let data = zip.get(30 + u16_at(26)? + u16_at(28)?..)?;
    let png = match u16_at(8)? {
        8 => miniz_oxide::inflate::decompress_to_vec(data).ok()?,
        0 => data.get(..u32::from_le_bytes(zip.get(18..22)?.try_into().ok()?) as usize)?.to_vec(),
        _ => return None,
    };
    png.starts_with(b"\x89PNG").then_some(png)
}

// ---- 接口地址和 Key

/// 设置里填的地址（去掉末尾的斜杠）；空是官方的。
fn custom() -> Option<String> { Some(prefs().image_url.trim().trim_end_matches('/').to_owned()).filter(|u| !u.is_empty()) }
/// 生图请求发往哪里：填的是服务根地址就接上 `/ai/generate-image`，已经带着这一段的原样用。
pub(crate) fn endpoint() -> String {
    let base = custom().unwrap_or_else(|| OFFICIAL.into());
    if base.ends_with(PATH) { base } else { format!("{base}{PATH}") }
}
/// 地址的主机名，界面上显示用。
fn host(url: &str) -> String { reqwest::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_owned)).unwrap_or_else(|| url.to_owned()) }
/// Key 按地址分开存：官方的是原来那一项，别的地址各一项（换回官方不用重新填）。
fn account() -> String { custom().map_or_else(|| "api-key".into(), |u| format!("api-key@{u}")) }
fn with_secret<R>(f: impl FnOnce(Secret) -> R) -> R { let account = account(); f(Secret { service: SERVICE, account: &account }) }

// ---- 设置页

/// 钥匙串里（现在这个地址的）Key；没有就是还没添加。
pub(crate) fn key() -> Option<String> { with_secret(super::read_secret) }

pub fn opened() {
    KEY_SAVED.with(|k| k.set(with_secret(super::key_saved)));
    scan_thumbs();
    if TEST.with(|t| matches!(*t.borrow(), Test::Idle)) { rest(); }
}

// ---- 上一张试画（临时目录，系统清掉了就是没有）和画风样张

fn last_png() -> PathBuf { std::env::temp_dir().join("margin-image-test.png") }
fn last_meta() -> PathBuf { last_png().with_extension("json") }
/// 样张和头像照片放一处（只在这台设备上，系统不会清）：一张样张是一次生成换来的。
fn thumbs_dir() -> PathBuf { crate::store::Store::home_dir().join("assistant-style-thumbs") }

/// 没在试画时画布上放什么：上一张还在就是它，不在就是白纸。
fn rest() {
    let last = (|| {
        let v: Value = serde_json::from_slice(&std::fs::read(last_meta()).ok()?).ok()?;
        let n = |k: &str| v.get(k).and_then(Value::as_u64).map(|n| n as u32);
        let (secs, size, meta) = (v.get("secs")?.as_f64()? as f32, (n("width")?, n("height")?), v.get("meta")?.as_str()?.to_owned());
        let png = std::fs::read(last_png()).ok()?;
        Some(Test::Done { secs, size, meta, image: Arc::new(Image::from_bytes(ImageFormat::Png, png)), path: Some(last_png()) })
    })();
    TEST.with(|t| *t.borrow_mut() = last.unwrap_or(Test::Idle));
}
/// 换了地址或 Key：上一张不算数了。
fn forget() {
    let _ = std::fs::remove_file(last_png());
    let _ = std::fs::remove_file(last_meta());
    TEST.with(|t| *t.borrow_mut() = Test::Idle);
}

/// 画风的指纹：标识加标签。改了标签就对不上，旧样张等于没有。
fn fingerprint(id: &str, positive: &str, negative: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (id, positive.trim(), negative.trim()).hash(&mut h);
    format!("{:016x}", h.finish())
}
/// 文件名是 `<指纹>-<毫秒>.png`（每次换名字，图片缓存才认得是新的）；同一个指纹只留最新的一张。
fn scan_thumbs() {
    let mut found: HashMap<String, PathBuf> = HashMap::new();
    let mut files: Vec<PathBuf> = std::fs::read_dir(thumbs_dir()).map(|d| d.flatten().map(|e| e.path()).collect()).unwrap_or_default();
    files.sort();
    for path in files {
        let Some(key) = path.file_stem().and_then(|s| s.to_str()).and_then(|s| s.split('-').next()).map(str::to_owned) else { continue };
        if let Some(old) = found.insert(key, path) { let _ = std::fs::remove_file(old); }
    }
    THUMBS.with(|t| *t.borrow_mut() = found);
}
/// 这个画风的样张（用它画过静物才有）。
pub(super) fn style_thumb(id: &str, positive: &str, negative: &str) -> Option<PathBuf> {
    THUMBS.with(|t| t.borrow().get(&fingerprint(id, positive, negative)).cloned())
}
/// 样张：居中裁成 3:2，缩到卡片三倍大小。
fn thumbnail(png: &[u8]) -> Option<Vec<u8>> {
    let picture = ::image::load_from_memory_with_format(png, ::image::ImageFormat::Png).ok()?;
    let (w, h) = (picture.width(), picture.height());
    let (cw, ch) = if w * 2 > h * 3 { (h * 3 / 2, h) } else { (w, w * 2 / 3) };
    let small = picture.crop_imm((w - cw) / 2, (h - ch) / 2, cw, ch).resize_exact(450, 300, ::image::imageops::FilterType::Triangle);
    let mut out = Vec::new();
    small.write_to(&mut std::io::Cursor::new(&mut out), ::image::ImageFormat::Png).ok()?;
    Some(out)
}

const MORPH_TIME: Duration = Duration::from_millis(360);
/// 画布的高：竖的高、横的矮（图铺满，裁着看）。
fn stage((w, h): (u32, u32)) -> f32 { if w > h { 340. } else if w == h { 410. } else { 500. } }

/// 没画过时画布上的图：每种画幅一张，自带的（`canvas/*.webp`，编进二进制，只这一页用）。
fn sample(key: &str) -> Arc<Image> {
    thread_local! { static IMAGES: RefCell<[Option<Arc<Image>>; 3]> = const { RefCell::new([None, None, None]) }; }
    let (i, bytes): (usize, &[u8]) = match key {
        "landscape" => (1, include_bytes!("canvas/landscape.webp")),
        "square" => (2, include_bytes!("canvas/square.webp")),
        _ => (0, include_bytes!("canvas/portrait.webp")),
    };
    IMAGES.with(|images| images.borrow_mut()[i].get_or_insert_with(|| Arc::new(Image::from_bytes(ImageFormat::Webp, bytes.to_vec()))).clone())
}

/// 画布：顶到屏幕上沿和左右两边。试画过的图铺满（点开看全图）；试画中是扫光的占位；没画过或失败是这种画幅自带的一张图（`sample`）。
/// 图的上沿盖一道很淡的底色，让页名看得清。换画幅时高矮变过去。
/// 下沿浮着一排玻璃：左边三格画幅，右边试画的对象（静物或一个写了外貌的人物，系统菜单）和「试画」。没有 Key 时没有右边两颗。
fn studio(saved: bool, cx: &mut Context<Workspace>) -> AnyElement {
    let top = crate::ios::chrome_top();
    let page = crate::ios::home_bg();
    let test = TEST.with(|t| t.borrow().clone());
    let running = matches!(test, Test::Running(_));
    let (_, now) = current();
    let key = size_key();
    let ph = stage(if let Test::Running(s) = test { s } else { now });
    let (epoch, from, at) = MORPH.with(Cell::get);
    let fh = if at.is_some_and(|t| t.elapsed() < MORPH_TIME) { from } else { ph };
    let glass = glass::supported();
    let ink = Theme::text();
    let hero = div().relative().mx(px(-8.)).mt(px(-top)).overflow_hidden();
    let hero = match &test {
        Test::Running(_) => hero.child(imagine::developing(460., ph, 0.)),
        Test::Done { image, path, .. } => {
            let path = path.clone();
            hero.child(img(image.clone()).absolute().inset_0().size_full().object_fit(ObjectFit::Cover))
                .child(canvas(|b, _, _| TEST_FRAME.with(|f| f.set(Some(b))), |_, _, _, _| {}).absolute().inset_0())
                .when(path.is_some(), |d| d.child(div().id("image-test-preview").absolute().top(px(top)).bottom_0().left_0().right_0().on_click(cx.listener(move |_, _, _, _| {
                    glass::haptic();
                    if let (Some(p), Some(frame)) = (path.as_ref(), TEST_FRAME.with(Cell::get)) { coveredit::view(p, frame, 0.); }
                }))))
        }
        Test::Idle | Test::Failed(_) => hero.child(img(sample(key)).absolute().inset_0().size_full().object_fit(ObjectFit::Cover)),
    };
    let hero = hero.when(!running, |d| d.child(div().absolute().top_0().left_0().right_0().h(px(top + 36.)).bg(linear_gradient(180., linear_color_stop(page.opacity(0.72), 0.), linear_color_stop(page.opacity(0.), 1.)))));
    // 下沿那一排。
    let picked = SIZES.iter().position(|s| s.1 == key).unwrap_or(0);
    let sizes = div().relative().flex_none().w(px(144.)).h(px(48.)).rounded_full().occlude()
        .map(|d| if glass {
            d.child(glass::page_tab_pane("image-size-glass", vec![("sf:rectangle.portrait", 19., ink), ("sf:rectangle", 19., ink), ("sf:square", 19., ink)],
                Some((picked, theme::ca(craft::N1000, 0.12))), SIZES.iter().map(|s| s.0).collect()))
        } else {
            d.bg(Theme::bg_body()).p(px(4.)).flex().children(SIZES.iter().enumerate().map(|(i, &(id, _, _, ratio, ..))| div().id(id).flex_1().h_full().rounded_full()
                .flex().items_center().justify_center().text_size(px(13.)).font_weight(FontWeight::MEDIUM)
                .when(i == picked, |d| d.bg(theme::ca(craft::N1000, 0.1))).text_color(if i == picked { ink } else { Theme::text_caption() }).child(ratio)
                .on_click(cx.listener(move |w, _, _, cx| { menu_action(id, w, cx); }))))
        });
    let people = testable_people();
    let chosen = selected_test_person(&people);
    let subject = (saved && !people.is_empty() && !running).then(|| {
        let mut items = vec![menu::Item::new(TEST_SUBJECT_SCENE, tr("静物场景", "Still Life"), "photo", 0).checked(chosen.is_none())];
        for c in &people {
            items.push(menu::Item::new(person_item(&c.id), c.name.clone(), "person", 1).checked(chosen.as_ref().is_some_and(|x| x.id == c.id)));
        }
        // 一颗只有字的胶囊：「静物」或人物的名字（长的截短）。点了出系统菜单（菜单按钮透明，盖在玻璃上）。
        let name: String = chosen.as_ref().map_or_else(|| tr("静物", "Scene").to_owned(), |c| if c.name.chars().count() > 5 { c.name.chars().take(4).chain(['…']).collect() } else { c.name.clone() });
        let text_w: f32 = name.chars().map(|c| if c.is_ascii() { 9. } else { 17. }).sum();
        div().relative().flex_none().w(px(20. + text_w + 20.)).h(px(48.)).rounded_full().occlude()
            .map(|d| if glass { d.child(glass::page_pane("image-test-subject-glass", None, None, Some((name.into(), ink)))) }
                else { d.bg(Theme::bg_body()).flex().items_center().justify_center().text_size(px(16.)).text_color(ink).child(name) })
            .child(menu::page_button("image-test-subject", items))
    });
    let go = (saved || running).then(|| {
        let label = if running { tr("停止", "Stop") } else { tr("试画", "Test Draw") };
        // 和旁边两颗一样的透明玻璃，只有字：不要图标，也不反色。胶囊按字的宽度定。
        let text_w: f32 = label.chars().map(|c| if c.is_ascii() { 9. } else { 17. }).sum();
        div().id("image-test").relative().flex_none().w(px(22. + text_w + 22.)).h(px(48.)).rounded_full().occlude()
            .map(|d| if glass { d.child(glass::page_pane_heavy("image-test-glass", None, None, Some((label.into(), ink)))) }
                else { d.bg(Theme::bg_body()).flex().items_center().justify_center().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).text_color(ink).child(label) })
            .on_click(cx.listener(move |_, _, _, cx| { glass::haptic(); glass::pulse("image-test-glass"); if running { stop(); rest(); } else { run_test(cx); } cx.notify(); }))
    });
    hero.child(div().absolute().left(px(16.)).right(px(16.)).bottom(px(44.)).h(px(48.)).flex().items_center().gap(px(10.))
            .child(sizes).child(div().flex_1()).children(subject).children(go))
        .with_animation(("image-canvas", epoch), Animation::new(MORPH_TIME).with_easing(crate::ios::assist::ease_out_quint()), move |d, t| d.h(px(fh + (ph - fh) * t)))
        .into_any_element()
}

/// 模型小方块的底图：每一代一张（`canvas/model-5.webp` 蓝天、`model-45.webp` 土路，从画布的自带图里裁的）。
fn plate(old: bool) -> Arc<Image> {
    thread_local! { static IMAGES: RefCell<[Option<Arc<Image>>; 2]> = const { RefCell::new([None, None]) }; }
    let bytes: &[u8] = if old { include_bytes!("canvas/model-45.webp") } else { include_bytes!("canvas/model-5.webp") };
    IMAGES.with(|images| images.borrow_mut()[old as usize].get_or_insert_with(|| Arc::new(Image::from_bytes(ImageFormat::Webp, bytes.to_vec()))).clone())
}

/// 模型：一排四个带底图的小方块，大数字是哪一代，下面小字是完整 / 精选；选中的外面描一圈。
/// 上面一行写选中那个的名字和一句说明（照官方文档的说法）。
fn models(cx: &mut Context<Workspace>) -> AnyElement {
    const R: f32 = 16.;
    let current = model_of(&prefs().image_model).1;
    let caption = MODELS.iter().find(|m| m.1 == current).map(|&(_, _, _, version, line, tag)| (format!("{version} {}", tr(tag.0, tag.1)), tr(line.0, line.1)));
    let tiles = MODELS.iter().map(|&(action, model, _, version, _, tag)| {
        let on = model == current;
        let old = version != "V5";
        div().id(action).flex_1().min_w_0().h(px(92.)).p(px(3.)).rounded(px(R + 5.)).border_2().border_color(if on { Theme::text() } else { transparent_black() })
            .child(div().relative().size_full().rounded(px(R)).overflow_hidden()
                .child(img(plate(old)).absolute().inset_0().size_full().rounded(px(R)).object_fit(ObjectFit::Cover))
                .child(div().absolute().inset_0().rounded(px(R)).bg(black().opacity(if old { 0.26 } else { 0.14 })))
                .child(div().absolute().left(px(11.)).bottom(px(9.)).flex().flex_col()
                    .child(div().text_size(px(27.)).line_height(px(30.)).font_weight(FontWeight::BOLD).text_color(white()).child(version.trim_start_matches('V')))
                    .child(div().text_size(px(12.)).line_height(px(16.)).font_weight(FontWeight::MEDIUM).text_color(white().opacity(0.92)).child(tr(tag.0, tag.1)))))
            .on_click(cx.listener(move |w, _, _, cx| if !on { glass::haptic(); w.set_pref(move |p| p.image_model = model.into(), cx); }))
    });
    div().w_full().flex().flex_col()
        .child(super::roles::header(tr("模型", "Model")))
        .children(caption.map(|(name, line)| div().px(px(27.)).pb(px(8.)).flex().items_baseline().gap(px(8.))
            .child(div().flex_none().text_size(px(15.)).line_height(px(20.)).font_weight(FontWeight::SEMIBOLD).text_color(Theme::text()).child(name))
            .child(div().min_w_0().truncate().text_size(px(14.)).line_height(px(20.)).text_color(Theme::text_caption()).child(line))))
        .child(div().id("image-models").w_full().px(px(5.)).flex().children(tiles))
        .into_any_element()
}

pub fn page(cx: &mut Context<Workspace>) -> AnyElement {
    let saved = KEY_SAVED.with(Cell::get);
    let status = match saved {
        Some(true) => tr("已连接", "Connected"),
        Some(false) => tr("添加 API Key", "Add API Key"),
        None => tr("钥匙串暂不可用", "Keychain unavailable"),
    };
    let provider = div().id("novelai-provider").w_full().h(px(54.)).px(px(18.)).flex().items_center().gap(px(10.))
        .active(|d| d.bg(Theme::fill_pressed()))
        .child(div().flex_1().min_w_0().text_size(px(17.)).text_color(row_ink()).child("NovelAI"))
        .child(div().flex_none().text_size(px(17.)).text_color(Theme::text_caption()).child(status))
        .child(ui::icon("chevron-right", 14., row_chevron()))
        .on_click(cx.listener(|w, _, _, cx| open_key(w, cx))).into_any_element();
    let address = super::writing::line("novelai-url", tr("接口地址", "Address"), custom().map_or_else(|| tr("官方", "Official").to_owned(), |u| host(&u)), true)
        .on_click(cx.listener(|w, _, _, cx| open_url(w, cx))).into_any_element();
    let connected = saved == Some(true);
    // 面板的抬头：大字是现在的画风。平时下面什么都不写；试画中、失败了才多一行。
    let style = super::prompts::lib().current_style(prefs().english).map_or_else(|| tr("默认", "Default").to_owned(), |s| s.name);
    let note = TEST.with(|t| match &*t.borrow() {
        Test::Running(_) => Some((tr("正在生成…", "Generating…").to_owned(), false)),
        Test::Failed(e) => Some((e.clone(), true)),
        _ => None,
    });
    let head = div().px(px(18.)).flex().flex_col().gap(px(3.))
        .child(div().text_size(px(24.)).line_height(px(30.)).font_weight(FontWeight::BOLD).text_color(Theme::text()).child(style))
        .children(note.map(|(text, bad)| div().text_size(px(14.)).line_height(px(19.)).text_color(if bad { Theme::danger() } else { Theme::text_caption() }).child(text))).into_any_element();
    let mut sections = vec![head, models(cx), super::prompts::styles(cx), super::prompts::people(cx),
        general::titled(Some(tr("生成", "Generation")), super::prompts::quality_rows(cx)).into_any_element()];
    // 还没连上时，先连。
    sections.insert(if connected { sections.len() } else { 1 }, general::titled(Some(tr("服务商连接", "Provider Connection")), vec![provider, address]).into_any_element());
    div().w_full().flex().flex_col()
        .child(studio(connected, cx))
        .child(div().relative().mx(px(-8.)).mt(px(-28.)).px(px(8.)).pt(px(22.)).rounded_t(px(28.)).bg(crate::ios::home_bg()).flex().flex_col().gap(px(30.)).children(sections))
        .into_any_element()
}

pub fn menu_action(action: &str, w: &mut Workspace, cx: &mut Context<Workspace>) -> bool {
    if let Some(m) = MODELS.iter().find(|m| m.0 == action) {
        if prefs().image_model != m.1 { w.set_pref(|p| p.image_model = m.1.into(), cx); }
        return true;
    }
    if let Some(s) = SIZES.iter().find(|s| s.0 == action) {
        if size_key() != s.1 {
            glass::haptic();
            MORPH.with(|m| m.set((m.get().0 + 1, stage(current().1), Some(Instant::now()))));
            w.set_pref(|p| p.image_size = s.1.into(), cx);
        }
        return true;
    }
    if action == TEST_SUBJECT_SCENE {
        TEST_PERSON.with(|p| *p.borrow_mut() = None);
        cx.notify();
        return true;
    }
    if let Some(id) = action.strip_prefix(TEST_PERSON_ITEM) {
        TEST_PERSON.with(|p| *p.borrow_mut() = Some(id.to_owned()));
        cx.notify();
        return true;
    }
    false
}

fn open_key(w: &mut Workspace, cx: &mut Context<Workspace>) {
    glass::haptic();
    opened();
    let body = tr("API Key 仅保存在此设备的钥匙串中。", "The API Key is stored in this device’s Keychain.");
    let remove = (KEY_SAVED.with(Cell::get) == Some(true)).then(|| tr("移除 API Key", "Remove API Key"));
    let shown = super::prompt("NovelAI API Key", Some(body), "API Key", "", Field::Secret, remove, |value| match value {
        Some(key) if !super::valid_key(&key) => menu::dispatch(|w, _, cx| w.show_toast_kind(ui::ToastKind::Error, tr("请输入有效的 API Key", "Enter a valid API Key"), cx)),
        Some(key) => key_result(with_secret(|s| super::save_secret(s, &key)), true),
        None => key_result(with_secret(super::delete_secret), false),
    });
    if !shown { w.show_toast_kind(ui::ToastKind::Error, tr("无法打开 API Key 设置", "Could not open API key settings"), cx); }
    cx.notify();
}

/// 接口地址：系统的对话框，填同 NovelAI 协议的服务地址；填了别的地址时多一个「恢复官方地址」。
/// 换了地址，Key 跟着换成那个地址的那一项，测试结果清掉。
fn open_url(w: &mut Workspace, cx: &mut Context<Workspace>) {
    glass::haptic();
    let body = tr("使用与 NovelAI 相同接口的服务。API Key 按地址分别保存。", "Use a service with the same API as NovelAI. API Keys are saved per address.");
    let reset = custom().is_some().then(|| tr("恢复官方地址", "Use Official Address"));
    let shown = super::prompt(tr("接口地址", "Address"), Some(body), OFFICIAL, &custom().unwrap_or_default(), Field::Url, reset, |value| {
        let url = value.unwrap_or_default().trim().trim_end_matches('/').to_owned();
        let valid = url.is_empty() || reqwest::Url::parse(&url).is_ok_and(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some());
        menu::dispatch(move |w, _, cx| {
            if !valid { return w.show_toast_kind(ui::ToastKind::Error, tr("请输入以 http:// 或 https:// 开头的地址", "Enter an address starting with http:// or https://"), cx); }
            // 填官方地址等于没填。
            let url = if url == OFFICIAL || url == format!("{OFFICIAL}{PATH}") { String::new() } else { url };
            if prefs().image_url != url { w.set_pref(move |p| p.image_url = url, cx); }
            stop();
            forget();
            opened();
            cx.notify();
        });
    });
    if !shown { w.show_toast_kind(ui::ToastKind::Error, tr("无法打开接口地址设置", "Could not open address settings"), cx); }
    cx.notify();
}

fn key_result(status: i32, saved: bool) {
    let ok = status == 0 || !saved && status == super::ITEM_NOT_FOUND;
    if ok { KEY_SAVED.with(|k| k.set(Some(saved))); stop(); forget(); }
    else { log::warn!("NovelAI Keychain update returned {status}"); }
    menu::dispatch(move |w, _, cx| {
        if ok { cx.notify(); } else {
            let text = if saved { tr("无法保存 API Key", "Could not save API Key") } else { tr("无法移除 API Key", "Could not remove API Key") };
            w.show_toast_kind(ui::ToastKind::Error, text, cx);
        }
    });
}


/// 试画可选的人物（写了外貌的才能入画）。
fn testable_people() -> Vec<fdl_ai::imagine::Character> {
    super::prompts::lib().characters.into_iter().filter(|c| !c.look.trim().is_empty()).collect()
}

fn person_item(id: &str) -> &'static str {
    let key = format!("{TEST_PERSON_ITEM}{id}");
    PERSON_ITEMS.with(|t| {
        let mut t = t.borrow_mut();
        if let Some(x) = t.get(key.as_str()) { return *x; }
        let x: &'static str = Box::leak(key.into_boxed_str());
        t.insert(x);
        x
    })
}

/// 当前选中的试画人物（若已被删或清空外貌则返回 `None`，回退静物场景）。
fn selected_test_person(people: &[fdl_ai::imagine::Character]) -> Option<fdl_ai::imagine::Character> {
    let id = TEST_PERSON.with(|p| p.borrow().clone())?;
    people.iter().find(|c| c.id == id).cloned()
}

/// 停下正在进行的测试：请求跟着断开，回来的结果不要了。
fn stop() {
    TEST_RUN.with(|r| r.set(r.get() + 1));
    if let Some(call) = TEST_CALL.with(|c| c.borrow_mut().take()) { call.abort(); }
    TEST.with(|t| if matches!(*t.borrow(), Test::Running(_)) { *t.borrow_mut() = Test::Idle; });
}

fn run_test(cx: &mut Context<Workspace>) {
    let Some(key) = key() else { return };
    stop();
    let run = TEST_RUN.with(Cell::get);
    let (model, size) = current();
    let lib = super::prompts::lib();
    let style = lib.current_style(crate::settings::prefs().english);
    let people = testable_people();
    let person = selected_test_person(&people);
    let cast: Vec<&fdl_ai::imagine::Character> = person.iter().collect();
    let prompt_text = if person.is_some() { TEST_PERSON_PROMPT } else { TEST_PROMPT };
    let p = fdl_ai::imagine::assemble(style.as_ref(), &cast, prompt_text, None, (lib.quality(), lib.quality_negative()));
    let request = Request { prompt: p.base, negative: p.negative, characters: p.characters.into_iter().map(|c| (c.tags, c.negative)).collect(), seed: None };
    let mut parts = vec![model_of(model).2.to_owned()];
    if let Some(s) = &style { parts.push(s.name.clone()); }
    if let Some(c) = &person { parts.push(c.name.clone()); }
    let meta = parts.join(" · ");
    // 静物才给画风留样张：各画风画的是同一样东西，摆在一排才好比。
    let sample = person.is_none().then(|| fingerprint(style.as_ref().map_or("", |s| &s.id), style.as_ref().map_or("", |s| &s.positive), style.as_ref().map_or("", |s| &s.negative)));
    let started = Instant::now();
    TEST.with(|t| *t.borrow_mut() = Test::Running(size));
    let (tx, rx) = oneshot::channel();
    let url = endpoint();
    let wants = sample.is_some();
    let call = super::llm::runtime().spawn(async move {
        let result = generate(&url, &key, model, size, &request).await;
        let thumb = result.as_ref().ok().filter(|_| wants).and_then(|png| thumbnail(png));
        let _ = tx.send((result, thumb));
    });
    TEST_CALL.with(|c| *c.borrow_mut() = Some(call.abort_handle()));
    cx.spawn(async move |this, cx| {
        let (result, thumb) = rx.await.unwrap_or_else(|_| (Err(tr("请求中断了", "The request was interrupted").into()), None));
        if TEST_RUN.with(Cell::get) != run { return; }
        TEST_CALL.with(|c| c.borrow_mut().take());
        let test = match result {
            Ok(png) => {
                let secs = started.elapsed().as_secs_f32();
                let saved_path = std::fs::write(last_png(), &png).ok().map(|()| last_png());
                let _ = std::fs::write(last_meta(), json!({ "secs": secs, "width": size.0, "height": size.1, "meta": meta }).to_string());
                if let (Some(key), Some(thumb)) = (sample, thumb) {
                    let millis = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis());
                    let _ = std::fs::create_dir_all(thumbs_dir());
                    let _ = std::fs::write(thumbs_dir().join(format!("{key}-{millis:013}.png")), thumb);
                    scan_thumbs();
                }
                Test::Done { secs, size, meta, image: Arc::new(Image::from_bytes(ImageFormat::Png, png)), path: saved_path }
            }
            Err(e) => Test::Failed(e),
        };
        TEST.with(|t| *t.borrow_mut() = test);
        let _ = this.update(cx, |_, cx| cx.notify());
    }).detach();
}
