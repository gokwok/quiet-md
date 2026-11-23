//! 设置 › 助手设置 › 图像：NovelAI 生图。服务商只有 NovelAI（接口地址可以换成同协议的别的服务），API Key 只在设备钥匙串里保存、按地址分开；
//! 模型可选 V5 和 V4.5，尺寸三种（可视化画幅选择），步数、CFG、采样器固定，种子每次随机。
//! 下面是画风和人物（`assistant_image_prompts.rs`），「生成」里画幅与质量词，最后「测试生成」（套着现在的画风、质量词，也可带入人物试画，结果可点击全屏查看）。
use std::{cell::{Cell, RefCell}, collections::HashSet, path::PathBuf, sync::Arc, time::{Duration, Instant}};

use futures::channel::oneshot;
use gpui::{prelude::FluentBuilder, *};
use serde_json::{json, Value};

use crate::{app::Workspace, settings::{prefs, row_chevron, row_ink, tr}, theme::{self, Theme}, ui};
use crate::ios::{coveredit, general, glass, imagine, menu};
use super::{Card, Field, Secret};

pub const DEFAULT_MODEL: &str = "nai-diffusion-5-full";
const SERVICE: &str = "com.s1dashu.quietmd.novelai";
/// 账户接口也搬到了这个域名，`api.novelai.net` 只回 400。
const OFFICIAL: &str = "https://image.novelai.net";
const PATH: &str = "/ai/generate-image";
/// 四个都用同一种请求体实测出过图（2026-11-21）。
const MODELS: &[Card] = &[
    ("image-model-v5-full", DEFAULT_MODEL, "NAI V5", "V5", ("最新一代，题材最广", "Latest, widest range"), ("完整", "Full")),
    ("image-model-v5-curated", "nai-diffusion-5-curated", "NAI V5 Curated", "V5", ("用精选数据训练", "Trained on curated data"), ("精选", "Curated")),
    ("image-model-v4-5-full", "nai-diffusion-4-5-full", "NAI V4.5", "V4.5", ("上一代完整模型", "Previous generation"), ("完整", "Full")),
    ("image-model-v4-5-curated", "nai-diffusion-4-5-curated", "NAI V4.5 Curated", "V4.5", ("上一代精选模型", "Previous generation, curated"), ("精选", "Curated")),
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

pub fn opened() { KEY_SAVED.with(|k| k.set(with_secret(super::key_saved))); }

/// 可视化画幅选择（三枚按真实长宽比缩放的微缩画框，点一下即切换）。
fn size_picker(current_key: &str, cx: &mut Context<Workspace>) -> AnyElement {
    let items = SIZES.iter().map(|&(id, key, label, ratio, w, h)| {
        let on = key == current_key;
        let max_side = w.max(h) as f32;
        let (fw, fh) = (w as f32 / max_side * 40., h as f32 / max_side * 40.);
        let frame_bg = if on { Theme::block_hover() } else { Theme::bg_body() };
        let frame_border = if on { Theme::primary() } else { Theme::line_border() };
        div().id(id).flex_1().min_w_0().py(px(12.)).px(px(6.)).rounded(px(14.))
            .bg(if on { Theme::block_hover() } else { gpui::transparent_black() })
            .border_2().border_color(if on { Theme::primary() } else { gpui::transparent_black() })
            .flex().flex_col().items_center().gap(px(8.))
            .active(|d| d.bg(Theme::fill_pressed()))
            .child(div().h(px(44.)).flex().items_center().justify_center()
                .child(div().w(px(fw)).h(px(fh)).rounded(px(5.)).bg(frame_bg).border_1().border_color(frame_border)
                    .when(on, |d| d.shadow(theme::shadow_button()))
                    .flex().items_center().justify_center()
                    .child(div().text_size(px(10.)).font_weight(FontWeight::SEMIBOLD).text_color(if on { Theme::primary() } else { Theme::text_placeholder() }).child(ratio))))
            .child(div().flex().flex_col().items_center().gap(px(1.))
                .child(div().text_size(px(14.)).line_height(px(18.)).font_weight(if on { FontWeight::SEMIBOLD } else { FontWeight::MEDIUM })
                    .text_color(if on { Theme::primary() } else { row_ink() }).child(tr(label.0, label.1)))
                .child(div().text_size(px(11.)).line_height(px(15.)).text_color(Theme::text_caption()).child(format!("{w}×{h}"))))
            .on_click(cx.listener(move |w, _, _, cx| if !on { glass::haptic(); w.set_pref(|p| p.image_size = key.into(), cx); }))
    });
    div().w_full().px(px(12.)).py(px(12.)).flex().items_center().gap(px(6.)).children(items).into_any_element()
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
    let p = prefs();
    let size = size_of(&p.image_size);
    let mut gen_rows = vec![size_picker(size.1, cx)];
    gen_rows.extend(super::prompts::quality_rows(cx));
    div().w_full().pt(px(18.)).pb(px(28.)).flex().flex_col().gap(px(32.))
        .child(general::titled(Some(tr("服务商连接", "Provider Connection")), vec![provider, address]))
        .child(super::model_cards("image-models", tr("图像模型", "Image Model"), MODELS, model_of(&p.image_model).1, |p, id| p.image_model = id.into(), cx))
        .child(super::prompts::styles(cx))
        .child(super::prompts::people(cx))
        .child(general::titled(Some(tr("生成", "Generation")), gen_rows))
        .child(general::titled(None, test_rows(saved == Some(true), cx)))
        .into_any_element()
}

pub fn menu_action(action: &str, cx: &mut Context<Workspace>) -> bool {
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
            TEST.with(|t| *t.borrow_mut() = Test::Idle);
            opened();
            cx.notify();
        });
    });
    if !shown { w.show_toast_kind(ui::ToastKind::Error, tr("无法打开接口地址设置", "Could not open address settings"), cx); }
    cx.notify();
}

fn key_result(status: i32, saved: bool) {
    let ok = status == 0 || !saved && status == super::ITEM_NOT_FOUND;
    if ok { KEY_SAVED.with(|k| k.set(Some(saved))); stop(); TEST.with(|t| *t.borrow_mut() = Test::Idle); }
    else { log::warn!("NovelAI Keychain update returned {status}"); }
    menu::dispatch(move |w, _, cx| {
        if ok { cx.notify(); } else {
            let text = if saved { tr("无法保存 API Key", "Could not save API Key") } else { tr("无法移除 API Key", "Could not remove API Key") };
            w.show_toast_kind(ui::ToastKind::Error, text, cx);
        }
    });
}

fn seconds(s: f32) -> String { if prefs().english { format!("{s:.1} s") } else { format!("{s:.1} 秒") } }

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

/// 测试：用现在的模型、尺寸、画风和质量词生成一张图（默认窗边静物，也可以选已建人物试画），
/// 生成中用按目标比例呈现的扫光呼吸占位，生成后点击图片可全屏放大查看。没有 Key 时灰着。
fn test_rows(saved: bool, cx: &mut Context<Workspace>) -> Vec<AnyElement> {
    let test = TEST.with(|t| t.borrow().clone());
    let running = matches!(test, Test::Running(_));
    let people = testable_people();
    let chosen = selected_test_person(&people);
    let subject_btn = (!people.is_empty() && !running).then(|| {
        let mut items = vec![menu::Item::new(TEST_SUBJECT_SCENE, tr("静物场景", "Still Life"), "photo", 0).checked(chosen.is_none())];
        for c in &people {
            items.push(menu::Item::new(person_item(&c.id), c.name.clone(), "person", 1).checked(chosen.as_ref().is_some_and(|x| x.id == c.id)));
        }
        let label = chosen.as_ref().map_or_else(|| tr("静物场景", "Still Life").to_owned(), |c| c.name.clone());
        div().relative().flex_none().h(px(30.)).px(px(10.)).rounded_full().bg(Theme::bg_body()).border_1().border_color(Theme::line_divider())
            .flex().items_center().gap(px(4.))
            .child(div().max_w(px(110.)).truncate().text_size(px(13.)).text_color(Theme::text_caption()).child(label))
            .child(ui::icon("chevron-up-down", 12., row_chevron()))
            .child(menu::button("image-test-subject", items))
    });
    let action = div().w_full().h(px(52.)).px(px(18.)).flex().items_center().gap(px(10.))
        .child(div().id("image-test").flex_1().h_full().flex().items_center()
            .text_size(px(17.)).font_weight(FontWeight::MEDIUM)
            .text_color(if running { Theme::danger() } else { Theme::primary() })
            .child(if running { tr("停止生成", "Stop") } else { tr("测试生成", "Test Generation") })
            .when(!saved && !running, |d| d.opacity(0.4))
            .when(saved || running, |d| d.active(|d| d.opacity(0.6))
                .on_click(cx.listener(move |_, _, _, cx| { glass::haptic(); if running { stop(); } else { run_test(cx); } cx.notify(); }))))
        .children(subject_btn);
    let mut rows = vec![action.into_any_element()];
    let (model, _) = current();
    let name = model_of(model).2;
    let line = |title: String, meta: AnyElement| div().w_full().min_h(px(48.)).px(px(18.)).py(px(10.)).flex().items_center().gap(px(10.))
        .child(div().flex_1().min_w_0().truncate().text_size(px(15.)).text_color(Theme::text_caption()).child(title))
        .child(div().flex_none().child(meta));
    match test {
        Test::Idle => {}
        Test::Running((w, h)) => {
            let k = 260. / w.max(h) as f32;
            let (pw, ph) = (w as f32 * k, h as f32 * k);
            rows.push(div().w_full().flex().flex_col()
                .child(line(name.to_owned(), div().text_size(px(14.)).text_color(Theme::text_caption()).child(tr("正在生成…", "Generating…")).into_any_element()))
                .child(div().w_full().px(px(18.)).pb(px(18.)).flex().justify_center()
                    .child(div().relative().w(px(pw)).h(px(ph)).rounded(px(14.)).overflow_hidden().child(imagine::developing(pw, ph, 14.))))
                .into_any_element());
        }
        Test::Done { secs, size: (w, h), meta, image, path } => {
            let k = 280. / w.max(h) as f32;
            let (pw, ph) = (w as f32 * k, h as f32 * k);
            let view_path = path.clone();
            rows.push(div().w_full().flex().flex_col()
                .child(line(meta, div().text_size(px(14.)).text_color(Theme::text_caption()).child(crate::localized_format!("可用 · {}", "OK · {}", seconds(secs))).into_any_element()))
                .child(div().w_full().px(px(18.)).pb(px(18.)).flex().justify_center()
                    .child(div().id("image-test-preview").relative().w(px(pw)).h(px(ph)).rounded(px(14.)).overflow_hidden().shadow(theme::shadow_card())
                        .child(img(image).size_full().rounded(px(14.)))
                        .child(canvas(|b, _, _| TEST_FRAME.with(|f| f.set(Some(b))), |_, _, _, _| {}).absolute().inset_0())
                        .when(view_path.is_some(), |d| d.active(|d| d.opacity(0.88)).on_click(cx.listener(move |_, _, _, _| {
                            glass::haptic();
                            if let (Some(p), Some(frame)) = (view_path.as_ref(), TEST_FRAME.with(Cell::get)) { coveredit::view(p, frame, 14.); }
                        })))))
                .into_any_element());
        }
        Test::Failed(e) => rows.push(div().w_full().flex().flex_col()
            .child(line(name.to_owned(), div().text_size(px(14.)).text_color(Theme::danger()).child(tr("不可用", "Failed")).into_any_element()))
            .child(div().px(px(18.)).pb(px(14.)).text_size(px(14.)).line_height(px(19.)).text_color(Theme::danger()).child(e))
            .into_any_element()),
    }
    rows
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
    // 上一张的临时文件不要了。
    TEST.with(|t| if let Test::Done { path: Some(p), .. } = &*t.borrow() { let _ = std::fs::remove_file(p); });
    let started = Instant::now();
    TEST.with(|t| *t.borrow_mut() = Test::Running(size));
    let (tx, rx) = oneshot::channel();
    let url = endpoint();
    let call = super::llm::runtime().spawn(async move { let _ = tx.send(generate(&url, &key, model, size, &request).await); });
    TEST_CALL.with(|c| *c.borrow_mut() = Some(call.abort_handle()));
    cx.spawn(async move |this, cx| {
        let result = rx.await.unwrap_or_else(|_| Err(tr("请求中断了", "The request was interrupted").into()));
        if TEST_RUN.with(Cell::get) != run { return; }
        TEST_CALL.with(|c| c.borrow_mut().take());
        let test = match result {
            Ok(png) => {
                let path = std::env::temp_dir().join(format!("margin-image-test-{run}.png"));
                let saved_path = std::fs::write(&path, &png).ok().map(|()| path);
                Test::Done { secs: started.elapsed().as_secs_f32(), size, meta, image: Arc::new(Image::from_bytes(ImageFormat::Png, png)), path: saved_path }
            }
            Err(e) => Test::Failed(e),
        };
        TEST.with(|t| *t.borrow_mut() = test);
        let _ = this.update(cx, |_, cx| cx.notify());
    }).detach();
}
