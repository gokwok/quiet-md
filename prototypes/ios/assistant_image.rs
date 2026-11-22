//! 设置 › 助手设置 › 图像：NovelAI 生图。服务商只有 NovelAI（接口地址可以换成同协议的别的服务），API Key 只在设备钥匙串里保存、按地址分开；模型可选 V5 和 V4.5，尺寸三种，
//! 步数、CFG、采样器固定，种子每次随机。下面是画风和人物（`assistant_image_prompts.rs`），「生成」里尺寸和质量词，最后一行「测试生成」（套着现在的画风和质量词）。
use std::{cell::{Cell, RefCell}, sync::Arc, time::{Duration, Instant}};

use futures::channel::oneshot;
use gpui::{prelude::FluentBuilder, *};
use serde_json::{json, Value};

use crate::{app::Workspace, settings::{prefs, row_chevron, row_ink, tr}, theme::Theme, ui};
use crate::ios::{general, glass, menu};
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
/// (菜单项, 键, 名称, 宽, 高)。偏好里空的键当 2:3。
#[allow(clippy::type_complexity)]
const SIZES: &[(&str, &str, (&str, &str), u32, u32)] = &[
    ("image-size-portrait", "portrait", ("竖版 2:3", "Portrait 2:3"), 832, 1216),
    ("image-size-landscape", "landscape", ("横版 3:2", "Landscape 3:2"), 1216, 832),
    ("image-size-square", "square", ("方形 1:1", "Square 1:1"), 1024, 1024),
];
const STEPS: u32 = 28;
const SCALE: f32 = 3.5;
const SAMPLER: &str = "k_euler_ancestral";
const TEST_PROMPT: &str = "no humans, scenery, a quiet wooden desk by the window, morning light, books, potted plant";
/// 质量词（正向、负向）在设置里改，默认的在 `fdl_ai::imagine::QUALITY`；和画风是两回事。

#[derive(Clone)]
enum Test { Idle, Running, Done(f32, (u32, u32), Arc<Image>), Failed(String) }

thread_local! {
    static KEY_SAVED: Cell<Option<bool>> = const { Cell::new(None) };
    static TEST: RefCell<Test> = const { RefCell::new(Test::Idle) };
    static TEST_RUN: Cell<u64> = const { Cell::new(0) };
    static TEST_CALL: RefCell<Option<tokio::task::AbortHandle>> = const { RefCell::new(None) };
}

fn model_of(id: &str) -> &'static Card { MODELS.iter().find(|m| m.1 == id).unwrap_or(&MODELS[0]) }
/// 模型名对应的显示名（「NAI V5」）；不认识的原样。
pub(crate) fn model_name(id: &str) -> &str { MODELS.iter().find(|m| m.1 == id).map_or(id, |m| m.2) }
fn size_of(key: &str) -> &'static (&'static str, &'static str, (&'static str, &'static str), u32, u32) { SIZES.iter().find(|s| s.1 == key).unwrap_or(&SIZES[0]) }

/// 一次生图要的东西。`characters` 是各人物的分角色提示词和排除词。种子不给就随机。
pub(crate) struct Request { pub prompt: String, pub negative: String, pub characters: Vec<(String, String)>, pub seed: Option<u32> }

/// 比例的键（`SIZES`）对应的宽高；不认识的当 2:3。
pub(crate) fn size(key: &str) -> (u32, u32) { let s = size_of(key); (s.3, s.4) }
/// 偏好里的比例键（空的当 2:3）。
pub(crate) fn size_key() -> &'static str { size_of(&prefs().image_size).1 }

/// 现在设置里的模型和尺寸。
pub(crate) fn current() -> (&'static str, (u32, u32)) {
    let p = prefs();
    let s = size_of(&p.image_size);
    (model_of(&p.image_model).1, (s.3, s.4))
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
    let items = SIZES.iter().map(|s| menu::Item::new(s.0, tr(s.2.0, s.2.1), "", 0).checked(s.1 == size.1).sub(format!("{} × {}", s.3, s.4))).collect();
    let size_row = general::choice("image-size-choice", tr("尺寸", "Size"), format!("{} · {} × {}", tr(size.2.0, size.2.1), size.3, size.4), items);
    div().w_full().pt(px(18.)).flex().flex_col().gap(px(32.))
        .child(general::titled(Some(tr("服务商连接", "Provider Connection")), vec![provider, address]))
        .child(super::model_cards("image-models", tr("图像模型", "Image Model"), MODELS, model_of(&p.image_model).1, |p, id| p.image_model = id.into(), cx))
        .child(super::prompts::styles(cx))
        .child(super::prompts::people(cx))
        .child(general::titled(Some(tr("生成", "Generation")), std::iter::once(size_row).chain(super::prompts::quality_rows(cx)).collect()))
        .child(general::titled(None, test_rows(saved == Some(true), cx)))
        .into_any_element()
}

pub fn menu_action(action: &str, w: &mut Workspace, cx: &mut Context<Workspace>) -> bool {
    let Some(s) = SIZES.iter().find(|s| s.0 == action) else { return false };
    if prefs().image_size != s.1 { w.set_pref(|p| p.image_size = s.1.into(), cx); }
    true
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

/// 测试：用现在的模型和尺寸生成一张固定内容的图，写用了多久，图放在下面；不通就用红字写服务的说法。没有 Key 时灰着。
fn test_rows(saved: bool, cx: &mut Context<Workspace>) -> Vec<AnyElement> {
    let test = TEST.with(|t| t.borrow().clone());
    let running = matches!(test, Test::Running);
    let action = div().id("image-test").w_full().h(px(52.)).px(px(18.)).flex().items_center()
        .text_size(px(17.)).text_color(Theme::primary()).child(if running { tr("停止", "Stop") } else { tr("测试生成", "Test Generation") })
        .when(!saved && !running, |d| d.opacity(0.4))
        .when(saved || running, |d| d.active(|d| d.bg(Theme::fill_pressed()))
            .on_click(cx.listener(move |_, _, _, cx| { glass::haptic(); if running { stop(); } else { run_test(cx); } cx.notify(); })));
    let mut rows = vec![action.into_any_element()];
    let (model, _) = current();
    let name = model_of(model).2;
    let line = |meta: AnyElement| div().w_full().min_h(px(54.)).px(px(18.)).py(px(12.)).flex().items_center().gap(px(10.))
        .child(div().flex_1().min_w_0().truncate().text_size(px(17.)).text_color(row_ink()).child(name))
        .child(div().flex_none().child(meta));
    match test {
        Test::Idle => {}
        Test::Running => rows.push(line(super::preview::bars("image-test-bars", 12., Theme::text_caption())).into_any_element()),
        Test::Done(s, (w, h), image) => {
            // 放进 300 见方的框里。
            let k = 300. / w.max(h) as f32;
            rows.push(div().w_full().flex().flex_col()
                .child(line(div().text_size(px(15.)).text_color(Theme::text_caption()).child(crate::localized_format!("可用 · {}", "OK · {}", seconds(s))).into_any_element()))
                .child(div().w_full().px(px(18.)).pb(px(18.)).flex().justify_center()
                    .child(img(image).w(px(w as f32 * k)).h(px(h as f32 * k)).rounded(px(12.))))
                .into_any_element());
        }
        Test::Failed(e) => rows.push(div().w_full().flex().flex_col()
            .child(line(div().text_size(px(15.)).text_color(Theme::danger()).child(tr("不可用", "Failed")).into_any_element()))
            .child(div().px(px(18.)).pb(px(12.)).text_size(px(14.)).line_height(px(19.)).text_color(Theme::danger()).child(e))
            .into_any_element()),
    }
    rows
}

/// 停下正在进行的测试：请求跟着断开，回来的结果不要了。
fn stop() {
    TEST_RUN.with(|r| r.set(r.get() + 1));
    if let Some(call) = TEST_CALL.with(|c| c.borrow_mut().take()) { call.abort(); }
    TEST.with(|t| if matches!(*t.borrow(), Test::Running) { *t.borrow_mut() = Test::Idle; });
}

fn run_test(cx: &mut Context<Workspace>) {
    let Some(key) = key() else { return };
    stop();
    let run = TEST_RUN.with(Cell::get);
    let (model, size) = current();
    let lib = super::prompts::lib();
    let style = lib.current_style(crate::settings::prefs().english);
    let p = fdl_ai::imagine::assemble(style.as_ref(), &[], TEST_PROMPT, None, (lib.quality(), lib.quality_negative()));
    let request = Request { prompt: p.base, negative: p.negative, characters: vec![], seed: None };
    let started = Instant::now();
    TEST.with(|t| *t.borrow_mut() = Test::Running);
    let (tx, rx) = oneshot::channel();
    let url = endpoint();
    let call = super::llm::runtime().spawn(async move { let _ = tx.send(generate(&url, &key, model, size, &request).await); });
    TEST_CALL.with(|c| *c.borrow_mut() = Some(call.abort_handle()));
    cx.spawn(async move |this, cx| {
        let result = rx.await.unwrap_or_else(|_| Err(tr("请求中断了", "The request was interrupted").into()));
        if TEST_RUN.with(Cell::get) != run { return; }
        TEST_CALL.with(|c| c.borrow_mut().take());
        let test = match result {
            Ok(png) => Test::Done(started.elapsed().as_secs_f32(), size, Arc::new(Image::from_bytes(ImageFormat::Png, png))),
            Err(e) => Test::Failed(e),
        };
        TEST.with(|t| *t.borrow_mut() = test);
        let _ = this.update(cx, |_, cx| cx.notify());
    }).detach();
}
