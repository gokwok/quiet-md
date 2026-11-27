//! 设置 › 助手设置 › 文本：OpenAI 兼容的文本模型。
//! 「服务商」可以有好几个，各有接口地址，API Key 按服务商分开存在钥匙串里；「主模型」（能力优先）和「快速模型」（速度优先）
//! 各指向某个服务商的某个模型，两个可以来自不同的服务商。模型列表向各家服务要（`assistant_llm.rs`）。
//! 版式照语音页：上面两张并排的模型卡片，下面是服务商列表（每家一颗柔光球）和推理强度，最后可以给两个模型各发一句 ping 看通不通。
use std::{cell::{Cell, RefCell}, collections::HashMap, time::Instant};

use gpui::{prelude::FluentBuilder, *};
use serde::{Deserialize, Serialize};

use crate::{app::Workspace, input::TextField, settings::{Page, Preferences, prefs, row_chevron, row_ink, tr}, theme::{self, Theme, craft}, ui};
use super::{Field, Secret, general, llm::{self, Event, Model}, menu, roles::{header, orb}};

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Provider {
    /// 本机生成的 id，也是钥匙串里 Key 的账户名。
    pub id: String,
    pub name: String,
    /// 接口地址，不带末尾的 /。
    pub base_url: String,
}

/// 一个模型位置：哪个服务商的哪个模型，推理强度（空是不指定）。
#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Slot {
    pub provider: String,
    pub model: String,
    /// 选的时候服务给的显示名；服务连不上时卡片照样写得出名字。
    pub name: String,
    pub effort: String,
}

/// 两个位置的名称和没选模型时的一句话：主模型、快速模型。
/// (中文, English)
type Words = (&'static str, &'static str);
const SLOTS: [(Words, Words); 2] = [(("主模型", "Main"), ("能力优先", "Most capable")), (("快速模型", "Fast"), ("速度优先", "Fastest"))];
const SLOT_IDS: [&str; 2] = ["text-slot-main", "text-slot-fast"];

fn slot(p: &Preferences, i: usize) -> &Slot { if i == 0 { &p.text_main } else { &p.text_fast } }
fn slot_mut(p: &mut Preferences, i: usize) -> &mut Slot { if i == 0 { &mut p.text_main } else { &mut p.text_fast } }

/// (菜单项, 名称, 接口地址)：常用的几家，选了就填好名称和地址；别的服务选「自定义」自己填。
const PRESETS: &[(&str, (&str, &str), &str)] = &[
    ("text-add-openai", ("OpenAI", "OpenAI"), "https://api.openai.com/v1"),
    ("text-add-deepseek", ("DeepSeek", "DeepSeek"), "https://api.deepseek.com"),
    ("text-add-openrouter", ("OpenRouter", "OpenRouter"), "https://openrouter.ai/api/v1"),
    ("text-add-siliconflow", ("硅基流动", "SiliconFlow"), "https://api.siliconflow.cn/v1"),
];
const ADD_CUSTOM: &str = "text-add-custom";
const DELETE: &str = "text-provider-delete";
const EFFORT_AUTO: [&str; 2] = ["text-effort-0-auto", "text-effort-1-auto"];
const EFFORTS: [[&str; 8]; 2] = [
    ["text-effort-0-0", "text-effort-0-1", "text-effort-0-2", "text-effort-0-3", "text-effort-0-4", "text-effort-0-5", "text-effort-0-6", "text-effort-0-7"],
    ["text-effort-1-0", "text-effort-1-1", "text-effort-1-2", "text-effort-1-3", "text-effort-1-4", "text-effort-1-5", "text-effort-1-6", "text-effort-1-7"],
];
const SERVICE: &str = "com.s1dashu.quietmd.text-model";

/// `Loading` 带着这一次请求的编号：同一家又取了一次时，先回来的旧结果不算。
enum List { Loading(u64), Ready(Vec<Model>), Failed(String) }
/// 测试：发一句 ping，收到第一个字就算通了，记下用了多久。
enum Test { Idle, Running(Instant), Done(f32), Failed(String) }

thread_local! {
    /// 各服务商的模型列表：服务商 id → (取的时候的接口地址, 列表)。
    static LISTS: RefCell<HashMap<String, (String, List)>> = RefCell::new(HashMap::new());
    static REQUEST: Cell<u64> = const { Cell::new(0) };
    /// 各服务商的 Key 存没存着（`None` 是钥匙串查不了）。
    static KEYS: RefCell<HashMap<String, Option<bool>>> = RefCell::new(HashMap::new());
    static SEARCH: RefCell<Option<Entity<TextField>>> = const { RefCell::new(None) };
    /// 正在给哪个位置选模型。
    static PICKING: Cell<usize> = const { Cell::new(0) };
    /// 正在看的服务商。
    static EDITING: RefCell<String> = const { RefCell::new(String::new()) };
    static TESTS: RefCell<[Test; 2]> = const { RefCell::new([Test::Idle, Test::Idle]) };
    static TEST_RUN: Cell<u64> = const { Cell::new(0) };
}

fn secret(id: &str) -> Secret<'_> { Secret { service: SERVICE, account: id } }

fn key_status(id: &str) -> Option<bool> {
    KEYS.with(|k| *k.borrow_mut().entry(id.to_owned()).or_insert_with(|| super::key_saved(secret(id))))
}

fn refresh_key(id: &str) { KEYS.with(|k| { k.borrow_mut().insert(id.to_owned(), super::key_saved(secret(id))); }); }

/// 写台本这类小活用的模型：快速模型，没选就用主模型。(接口地址, Key, 模型, 推理强度)。
pub(crate) fn quick() -> Option<(String, String, String, Option<String>)> {
    let p = prefs();
    [&p.text_fast, &p.text_main].into_iter().find_map(|s| {
        let provider = p.text_providers.iter().find(|pr| pr.id == s.provider && !s.model.is_empty())?;
        let key = super::read_secret(secret(&provider.id)).unwrap_or_default();
        Some((provider.base_url.clone(), key, s.model.clone(), Some(s.effort.clone()).filter(|e| !e.is_empty())))
    })
}

/// 助手写正文用的模型：主模型，没选就用快速模型。
pub(crate) fn main() -> Option<(String, String, String, Option<String>)> {
    let p = prefs();
    [&p.text_main, &p.text_fast].into_iter().find_map(|s| {
        let provider = p.text_providers.iter().find(|pr| pr.id == s.provider && !s.model.is_empty())?;
        let key = super::read_secret(secret(&provider.id)).unwrap_or_default();
        Some((provider.base_url.clone(), key, s.model.clone(), Some(s.effort.clone()).filter(|e| !e.is_empty())))
    })
}

fn find(id: &str) -> Option<Provider> { prefs().text_providers.into_iter().find(|p| p.id == id) }

/// 进文本的几页：重新看一眼钥匙串；哪家的模型列表还没有、地址变了或上次失败了，就去取。
pub fn opened(page: Page, cx: &mut Context<Workspace>) {
    if page == Page::AssistantText { KEYS.with(|k| k.borrow_mut().clear()); }
    if page == Page::AssistantModels {
        SEARCH.with(|search| {
            let mut search = search.borrow_mut();
            match search.as_ref() {
                Some(field) => field.update(cx, |f, cx| f.set_text("", cx)),
                None => {
                    let field = cx.new(|cx| TextField::new(tr("搜索模型", "Search models"), cx));
                    cx.observe(&field, |_, _, cx| cx.notify()).detach();
                    *search = Some(field);
                }
            }
        });
    }
    for p in prefs().text_providers.iter().filter(|p| !p.base_url.is_empty()) {
        let stale = LISTS.with(|l| l.borrow().get(&p.id).is_none_or(|(url, list)| *url != p.base_url || matches!(list, List::Failed(_))));
        if stale { fetch(p, cx); }
    }
}

fn fetch(p: &Provider, cx: &mut Context<Workspace>) {
    if p.base_url.is_empty() { LISTS.with(|l| l.borrow_mut().remove(&p.id)); return cx.notify(); }
    let run = REQUEST.with(|r| { r.set(r.get() + 1); r.get() });
    let (id, base) = (p.id.clone(), p.base_url.clone());
    LISTS.with(|l| l.borrow_mut().insert(id.clone(), (base.clone(), List::Loading(run))));
    let key = super::read_secret(secret(&id)).unwrap_or_default();
    cx.notify();
    cx.spawn(async move |this, cx| {
        let result = llm::models(base.clone(), key).await;
        let current = LISTS.with(|l| matches!(l.borrow().get(&id), Some((_, List::Loading(r))) if *r == run));
        if !current { return; }
        LISTS.with(|l| l.borrow_mut().insert(id, (base, match result { Ok(models) => List::Ready(models), Err(e) => List::Failed(e) })));
        let _ = this.update(cx, |_, cx| cx.notify());
    }).detach();
}

fn model_of(s: &Slot) -> Option<Model> {
    LISTS.with(|l| match l.borrow().get(&s.provider) { Some((_, List::Ready(models))) => models.iter().find(|m| m.id == s.model).cloned(), _ => None })
}

/// 位置上的模型叫什么：列表里的显示名，其次选的时候记下的，最后是 id（去掉「厂商/」前缀）。
fn slot_name(s: &Slot) -> String {
    model_of(s).map(|m| m.label().to_owned()).or_else(|| Some(s.name.clone()).filter(|n| !n.is_empty())).unwrap_or_else(|| s.model.rsplit('/').next().unwrap_or(&s.model).to_owned())
}

/// 地址只给人看主机和路径。
fn short_url(url: &str) -> &str { url.split_once("://").map_or(url, |(_, rest)| rest) }

pub fn page(cx: &mut Context<Workspace>) -> AnyElement {
    let p = prefs();
    let cards = div().mx(px(10.)).flex().gap(px(10.)).children((0..2).map(|i| slot_card(i, &p, cx)));
    let mut rows: Vec<AnyElement> = p.text_providers.iter().map(|pr| provider_row(pr, cx)).collect();
    rows.push(add_row("text-add", tr("添加服务商", "Add Provider")));
    let mut body = div().w_full().pt(px(18.)).pb(px(24.)).flex().flex_col().gap(px(32.))
        .child(div().w_full().flex().flex_col().child(header(tr("模型", "Models"))).child(cards))
        .child(general::titled(Some(tr("服务商", "Providers")), rows));
    let efforts: Vec<AnyElement> = (0..2).filter_map(|i| effort_row(i, &p)).collect();
    if !efforts.is_empty() { body = body.child(general::titled(Some(tr("推理强度", "Reasoning")), efforts)); }
    if (0..2).any(|i| { let s = slot(&p, i); !s.model.is_empty() && p.text_providers.iter().any(|pr| pr.id == s.provider) }) {
        body = body.child(general::titled(Some(tr("测试", "Test")), test_rows(&p, cx)));
    }
    body.into_any_element()
}

/// 一个模型位置的卡片（照语音模型卡片）：左上角是位置，中间模型名，底下服务商的光球和名字。没选的时候中间写「选择模型」。
fn slot_card(i: usize, p: &Preferences, cx: &mut Context<Workspace>) -> AnyElement {
    let s = slot(p, i);
    let provider = p.text_providers.iter().find(|pr| pr.id == s.provider).filter(|_| !s.model.is_empty());
    let (title, line) = SLOTS[i];
    div().id(SLOT_IDS[i]).flex_1().min_w_0().h(px(172.)).p(px(14.)).rounded(px(20.)).bg(general::card_bg()).flex().flex_col()
        .active(|d| d.bg(Theme::fill_pressed()))
        .child(div().h(px(20.)).text_size(px(13.)).font_weight(FontWeight::MEDIUM).text_color(Theme::text_caption()).child(tr(title.0, title.1)))
        .child(div().mt(px(12.)).flex_1().min_w_0().map(|d| match provider {
            None => d.flex().flex_col().gap(px(4.))
                .child(div().text_size(px(16.)).line_height(px(21.)).font_weight(FontWeight::SEMIBOLD).text_color(Theme::text()).child(tr("选择模型", "Choose Model")))
                .child(div().text_size(px(14.)).line_height(px(19.)).text_color(Theme::text_caption()).child(tr(line.0, line.1))),
            Some(_) => d.text_size(px(17.)).line_height(px(22.)).font_weight(FontWeight::SEMIBOLD).text_color(Theme::text()).line_clamp(3)
                .child(slot_name(s)),
        }))
        .child(match provider {
            Some(pr) => div().flex().items_center().gap(px(7.)).child(orb(&pr.id, 18.))
                .child(div().min_w_0().truncate().text_size(px(13.)).text_color(Theme::text_caption()).child(pr.name.clone())).into_any_element(),
            None => div().size(px(26.)).rounded_full().bg(Theme::bg_body()).flex().items_center().justify_center().child(ui::icon("plus", 14., Theme::text_caption())).into_any_element(),
        })
        .on_click(cx.listener(move |w, _, window, cx| {
            window.blur(cx);
            super::super::glass::haptic();
            if prefs().text_providers.is_empty() { return w.show_toast_kind(ui::ToastKind::Error, tr("请先添加服务商", "Add a provider first"), cx); }
            PICKING.with(|p| p.set(i));
            w.open_settings(Page::AssistantModels, cx);
        }))
        .into_any_element()
}

/// 推理强度一行（模型说了支持哪几档才有）：左边是位置，右边灰字写现在的档位，点开是系统菜单。
fn effort_row(i: usize, p: &Preferences) -> Option<AnyElement> {
    let s = slot(p, i);
    let m = model_of(s).filter(|m| !m.efforts.is_empty() && p.text_providers.iter().any(|pr| pr.id == s.provider))?;
    let current = m.efforts.iter().find(|e| **e == s.effort);
    let mut items = vec![menu::Item::new(EFFORT_AUTO[i], tr("默认", "Default"), "", 0).checked(current.is_none())];
    items.extend(m.efforts.iter().zip(EFFORTS[i]).map(|(e, id)| menu::Item::new(id, effort_label(e), "", 1).checked(Some(e) == current)));
    let (title, _) = SLOTS[i];
    Some(div().relative().w_full().h(px(54.)).px(px(18.)).flex().items_center().gap(px(12.))
        .child(div().flex_1().min_w_0().text_size(px(17.)).text_color(row_ink()).child(tr(title.0, title.1)))
        .child(div().flex_none().text_size(px(17.)).text_color(Theme::text_caption()).child(current.map_or_else(|| tr("默认", "Default").into(), |e| effort_label(e))))
        .child(ui::icon("chevron-up-down", 16., row_chevron()))
        .child(menu::button(["text-effort-menu-0", "text-effort-menu-1"][i], items))
        .into_any_element())
}

/// 服务商的状态：右边的灰字。
fn status(p: &Provider) -> &'static str {
    if p.base_url.is_empty() { return tr("未设置地址", "No Address"); }
    let state = LISTS.with(|l| match l.borrow().get(&p.id).map(|(_, list)| list) { Some(List::Ready(_)) => Some(true), Some(List::Failed(_)) => Some(false), _ => None });
    match state {
        Some(true) => tr("已连接", "Connected"),
        Some(false) if key_status(&p.id) == Some(false) => tr("添加 Key", "Add Key"),
        Some(false) => tr("无法连接", "Unavailable"),
        None => tr("连接中", "Connecting"),
    }
}

fn provider_row(p: &Provider, cx: &mut Context<Workspace>) -> AnyElement {
    let id = p.id.clone();
    div().id(SharedString::from(format!("text-provider-{}", p.id))).w_full().h(px(64.)).px(px(16.)).flex().items_center().gap(px(12.))
        .active(|d| d.bg(Theme::fill_pressed()))
        .child(orb(&p.id, 34.))
        .child(div().flex_1().min_w_0().flex().flex_col().gap(px(1.))
            .child(div().truncate().text_size(px(17.)).line_height(px(22.)).text_color(row_ink()).child(p.name.clone()))
            .child(div().truncate().text_size(px(13.)).line_height(px(18.)).text_color(Theme::text_caption()).child(short_url(&p.base_url).to_owned())))
        .child(div().flex_none().text_size(px(15.)).text_color(Theme::text_caption()).child(status(p)))
        .child(ui::icon("chevron-right", 14., row_chevron()))
        .on_click(cx.listener(move |w, _, _, cx| { super::super::glass::haptic(); open_provider(w, id.clone(), cx); }))
        .into_any_element()
}

/// 「添加服务商」：点开是系统菜单，常用的几家和「自定义…」。
fn add_row(key: &'static str, label: &'static str) -> AnyElement {
    let mut items: Vec<menu::Item> = PRESETS.iter().map(|&(id, name, _)| menu::Item::new(id, tr(name.0, name.1), "", 0)).collect();
    items.push(menu::Item::new(ADD_CUSTOM, tr("自定义…", "Custom…"), "", 1));
    div().relative().w_full().h(px(58.)).px(px(16.)).flex().items_center().gap(px(12.))
        .child(div().size(px(34.)).flex_none().rounded_full().bg(Theme::bg_body()).flex().items_center().justify_center().child(ui::icon("plus", 16., Theme::primary())))
        .child(div().text_size(px(17.)).text_color(Theme::primary()).child(label))
        .child(menu::button(key, items))
        .into_any_element()
}

fn open_provider(w: &mut Workspace, id: String, cx: &mut Context<Workspace>) {
    EDITING.with(|e| *e.borrow_mut() = id);
    w.open_settings(Page::AssistantProvider, cx);
}

fn add_provider(w: &mut Workspace, name: String, base_url: String, cx: &mut Context<Workspace>) {
    let provider = Provider { id: uuid::Uuid::new_v4().to_string(), name, base_url };
    w.set_pref(|p| p.text_providers.push(provider.clone()), cx);
    refresh_key(&provider.id);
    fetch(&provider, cx);
    open_provider(w, provider.id, cx);
}

/// 推理强度的说法；认不得的照原样写。
fn effort_label(effort: &str) -> SharedString {
    match effort {
        "none" => tr("关闭", "Off"), "minimal" => tr("最少", "Minimal"), "low" => tr("低", "Low"), "medium" => tr("中", "Medium"),
        "high" => tr("高", "High"), "xhigh" => tr("很高", "Very High"), "max" => tr("最高", "Max"), "ultra" => tr("极限", "Ultra"),
        other => return other.to_owned().into(),
    }.into()
}

fn seconds(s: f32) -> String { if prefs().english { format!("{s:.1} s") } else { format!("{s:.1} 秒") } }

/// 测试：两个模型同时发一句 ping，回了第一个字就算通了。每个模型一行，右边灰字写用了多久；不通就用红字写服务的说法。
fn test_rows(p: &Preferences, cx: &mut Context<Workspace>) -> Vec<AnyElement> {
    let running = TESTS.with(|t| t.borrow().iter().any(|t| matches!(t, Test::Running(_))));
    let label = if running { tr("停止", "Stop") } else { tr("测试连接", "Test Connection") };
    let action = div().id("text-test").w_full().h(px(52.)).px(px(18.)).flex().items_center().active(|d| d.bg(Theme::fill_pressed()))
        .text_size(px(17.)).text_color(Theme::primary()).child(label)
        .on_click(cx.listener(move |_, _, _, cx| { super::super::glass::haptic(); if running { stop(); cx.notify(); } else { run_tests(cx); } }));
    let mut rows = vec![action.into_any_element()];
    TESTS.with(|t| for (i, test) in t.borrow().iter().enumerate() {
        let (meta, error): (AnyElement, Option<String>) = match test {
            Test::Idle => continue,
            Test::Running(_) => (super::preview::bars(["text-test-bars-0", "text-test-bars-1"][i], 12., Theme::text_caption()), None),
            Test::Done(s) => (div().text_size(px(15.)).text_color(Theme::text_caption()).child(crate::localized_format!("可用 · {}", "OK · {}", seconds(*s))).into_any_element(), None),
            Test::Failed(e) => (div().text_size(px(15.)).text_color(Theme::danger()).child(tr("不可用", "Failed")).into_any_element(), Some(e.clone())),
        };
        let (title, _) = SLOTS[i];
        rows.push(div().w_full().min_h(px(54.)).px(px(18.)).py(px(12.)).flex().flex_col().justify_center().gap(px(4.))
            .child(div().flex().items_center().gap(px(10.))
                .child(div().flex_none().text_size(px(17.)).text_color(row_ink()).child(tr(title.0, title.1)))
                .child(div().flex_1().min_w_0().truncate().text_size(px(15.)).text_color(Theme::text_caption()).child(slot_name(slot(p, i))))
                .child(div().flex_none().child(meta)))
            .children(error.map(|e| div().text_size(px(14.)).line_height(px(19.)).text_color(Theme::danger()).child(e)))
            .into_any_element());
    });
    rows
}

/// 停下正在进行的测试（离开文本页时也调用）。
pub fn stop() {
    TEST_RUN.with(|r| r.set(r.get() + 1));
    TESTS.with(|t| for test in t.borrow_mut().iter_mut() { if let Test::Running(_) = test { *test = Test::Idle; } });
}

fn run_tests(cx: &mut Context<Workspace>) {
    let p = prefs();
    let run = TEST_RUN.with(|r| { r.set(r.get() + 1); r.get() });
    for i in 0..2 {
        let s = slot(&p, i).clone();
        let Some(provider) = p.text_providers.iter().find(|pr| pr.id == s.provider && !s.model.is_empty()) else { TESTS.with(|t| t.borrow_mut()[i] = Test::Idle); continue };
        let key = super::read_secret(secret(&provider.id)).unwrap_or_default();
        let effort = model_of(&s).filter(|m| m.efforts.contains(&s.effort)).map(|_| s.effort.clone());
        let mut events = llm::chat(provider.base_url.clone(), key, s.model.clone(), effort, vec![("user", "ping".into())]);
        let started = Instant::now();
        TESTS.with(|t| t.borrow_mut()[i] = Test::Running(started));
        cx.spawn(async move |this, cx| {
            use futures::StreamExt;
            // 收到第一个字就停（`events` 丢掉，请求跟着断开）；推理模型先想一会儿，接着等。
            let result = loop {
                match events.next().await {
                    Some(Event::Thinking | Event::Calls(_)) => continue,
                    Some(Event::Text(_)) => break Test::Done(started.elapsed().as_secs_f32()),
                    Some(Event::Failed(e)) => break Test::Failed(e),
                    Some(Event::Done) => break Test::Failed(tr("模型没有返回内容", "The model returned no text").into()),
                    None => break Test::Failed(tr("连接中断了", "The connection was interrupted").into()),
                }
            };
            if TEST_RUN.with(Cell::get) != run { return; }
            TESTS.with(|t| t.borrow_mut()[i] = result);
            let _ = this.update(cx, |_, cx| cx.notify());
        }).detach();
    }
    cx.notify();
}

// ---- 服务商（第二级）

/// 服务商：上面是光球、名称和连接状态（连不上时写服务自己的说法），下面名称、接口地址、API Key 三行，最后是删除。
pub fn provider(cx: &mut Context<Workspace>) -> AnyElement {
    let Some(p) = EDITING.with(|e| find(&e.borrow())) else { return div().into_any_element() };
    let (state, failed): (SharedString, bool) = if p.base_url.is_empty() { (tr("还没有接口地址", "No address yet").into(), false) } else {
        LISTS.with(|l| match l.borrow().get(&p.id).map(|(_, list)| list) {
            Some(List::Ready(models)) => (crate::localized_format!("已连接 · {} 个模型", "Connected · {} models", models.len()).into(), false),
            Some(List::Failed(e)) => (e.clone().into(), true),
            _ => (tr("连接中…", "Connecting…").into(), false),
        })
    };
    let key = match key_status(&p.id) { Some(true) => tr("已保存", "Saved"), Some(false) => tr("添加", "Add"), None => tr("钥匙串暂不可用", "Keychain unavailable") };
    let line = |id: &'static str, title: &'static str, value: SharedString| div().id(id).w_full().h(px(54.)).px(px(18.)).flex().items_center().gap(px(12.))
        .active(|d| d.bg(Theme::fill_pressed()))
        .child(div().flex_none().text_size(px(17.)).text_color(row_ink()).child(title))
        .child(div().flex_1().min_w_0().flex().justify_end().child(div().truncate().text_size(px(17.)).text_color(Theme::text_caption()).child(value)))
        .child(ui::icon("chevron-right", 14., row_chevron()));
    let rows = vec![
        line("text-provider-name", tr("名称", "Name"), p.name.clone().into()).on_click(cx.listener(|w, _, _, cx| edit_name(w, cx))).into_any_element(),
        line("text-provider-url", tr("接口地址", "Base URL"), if p.base_url.is_empty() { tr("未设置", "Not Set").into() } else { short_url(&p.base_url).to_owned().into() })
            .on_click(cx.listener(|w, _, _, cx| { let id = EDITING.with(|e| e.borrow().clone()); edit_url(w, Some(id), cx) })).into_any_element(),
        line("text-provider-key", "API Key", key.into()).on_click(cx.listener(|w, _, _, cx| edit_key(w, cx))).into_any_element(),
    ];
    // 删除：点一下弹出只有一项（红色）的系统菜单，再点一下才删，免得误触。
    let delete = div().relative().w_full().h(px(52.)).px(px(18.)).flex().items_center().text_size(px(17.)).text_color(Theme::danger())
        .child(tr("删除服务商", "Delete Provider")).child(menu::button("text-provider-delete-menu", vec![menu::Item::new(DELETE, tr("删除服务商", "Delete Provider"), "trash", 0).danger()]));
    div().w_full().pt(px(12.)).pb(px(24.)).flex().flex_col().gap(px(28.))
        .child(div().w_full().px(px(28.)).flex().flex_col().items_center()
            .child(div().rounded_full().shadow(theme::shadow_card()).child(orb(&p.id, 88.)))
            .child(div().mt(px(16.)).text_size(px(26.)).line_height(px(32.)).font_weight(FontWeight::BOLD).text_color(Theme::text()).text_center().child(p.name.clone()))
            .child(div().mt(px(6.)).text_size(px(14.)).line_height(px(20.)).text_center().line_clamp(3).text_color(if failed { Theme::danger() } else { Theme::text_caption() }).child(state)))
        .child(general::card(rows))
        .when(failed, |d| d.child(general::card(vec![general::action("text-provider-retry", tr("重新连接", "Reconnect"), false, |_, cx| {
            if let Some(p) = EDITING.with(|e| find(&e.borrow())) { fetch(&p, cx); }
        }, cx)])))
        .child(general::card(vec![delete.into_any_element()]))
        .into_any_element()
}

fn edit_name(w: &mut Workspace, cx: &mut Context<Workspace>) {
    super::super::glass::haptic();
    let Some(p) = EDITING.with(|e| find(&e.borrow())) else { return };
    let id = p.id.clone();
    if !super::prompt(tr("服务商名称", "Provider Name"), None, tr("名称", "Name"), &p.name, Field::Text, None, move |value| {
        let Some(name) = value.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty()) else { return };
        let id = id.clone();
        menu::dispatch(move |w, _, cx| w.set_pref(|p| if let Some(pr) = p.text_providers.iter_mut().find(|pr| pr.id == id) { pr.name = name; }, cx));
    }) { w.show_toast_kind(ui::ToastKind::Error, tr("无法打开名称设置", "Could not open name settings"), cx); }
}

/// 改某个服务商的地址；`id` 为空是新建一个自定义服务商（名称先用主机名）。
fn edit_url(w: &mut Workspace, id: Option<String>, cx: &mut Context<Workspace>) {
    super::super::glass::haptic();
    let current = id.as_deref().and_then(find).map(|p| p.base_url).unwrap_or_default();
    let body = tr("OpenAI 兼容接口的地址，通常以 /v1 结尾。", "The address of an OpenAI-compatible API, usually ending in /v1.");
    if !super::prompt(tr("接口地址", "Base URL"), Some(body), "https://api.example.com/v1", &current, Field::Url, None, move |value| {
        let Some(url) = value else { return };
        let url = url.trim().trim_end_matches('/').to_owned();
        let host = url.split_once("://").map(|(_, rest)| rest.split(['/', ':']).next().unwrap_or(rest).to_owned()).filter(|h| !h.is_empty());
        let ok = (url.starts_with("https://") || url.starts_with("http://")) && host.is_some() && !url.contains(char::is_whitespace);
        let id = id.clone();
        menu::dispatch(move |w, _, cx| {
            if !ok { return w.show_toast_kind(ui::ToastKind::Error, tr("请输入以 http:// 或 https:// 开头的地址", "Enter an address starting with http:// or https://"), cx); }
            match id {
                None => add_provider(w, host.unwrap_or_default(), url, cx),
                Some(id) => {
                    w.set_pref(|p| if let Some(pr) = p.text_providers.iter_mut().find(|pr| pr.id == id) { pr.base_url = url.clone(); }, cx);
                    if let Some(p) = find(&id) { fetch(&p, cx); }
                }
            }
        });
    }) { w.show_toast_kind(ui::ToastKind::Error, tr("无法打开接口地址设置", "Could not open Base URL settings"), cx); }
}

fn edit_key(w: &mut Workspace, cx: &mut Context<Workspace>) {
    super::super::glass::haptic();
    let id = EDITING.with(|e| e.borrow().clone());
    let body = tr("API Key 仅保存在此设备的钥匙串中。", "The API Key is stored in this device’s Keychain.");
    let remove = (key_status(&id) == Some(true)).then(|| tr("移除 API Key", "Remove API Key"));
    if !super::prompt("API Key", Some(body), "API Key", "", Field::Secret, remove, move |value| {
        let status = match &value {
            Some(key) if !super::valid_key(key) => return menu::dispatch(|w, _, cx| w.show_toast_kind(ui::ToastKind::Error, tr("请输入有效的 API Key", "Enter a valid API Key"), cx)),
            Some(key) => super::save_secret(secret(&id), key),
            None => super::delete_secret(secret(&id)),
        };
        let saved = value.is_some();
        let id = id.clone();
        menu::dispatch(move |w, _, cx| {
            if status == 0 || !saved && status == super::ITEM_NOT_FOUND { refresh_key(&id); if let Some(p) = find(&id) { fetch(&p, cx); } return; }
            log::warn!("Text model Keychain update returned {status}");
            w.show_toast_kind(ui::ToastKind::Error, if saved { tr("无法保存 API Key", "Could not save API Key") } else { tr("无法移除 API Key", "Could not remove API Key") }, cx);
        });
    }) { w.show_toast_kind(ui::ToastKind::Error, tr("无法打开 API Key 设置", "Could not open API key settings"), cx); }
}

/// 删除服务商：Key 从钥匙串里删掉，用着它的模型位置清空。
fn delete_provider(w: &mut Workspace, cx: &mut Context<Workspace>) {
    let id = EDITING.with(|e| e.borrow().clone());
    stop();
    let status = super::delete_secret(secret(&id));
    if !matches!(status, 0 | super::ITEM_NOT_FOUND) { log::warn!("Text model Keychain delete returned {status}"); }
    w.set_pref(|p| {
        p.text_providers.retain(|pr| pr.id != id);
        for i in 0..2 { let s = slot_mut(p, i); if s.provider == id { *s = Slot::default(); } }
    }, cx);
    LISTS.with(|l| l.borrow_mut().remove(&id));
    KEYS.with(|k| k.borrow_mut().remove(&id));
    TESTS.with(|t| *t.borrow_mut() = [Test::Idle, Test::Idle]);
    super::super::settings_back(w, cx);
}

pub fn menu_action(action: &str, w: &mut Workspace, cx: &mut Context<Workspace>) -> bool {
    if let Some(&(_, name, url)) = PRESETS.iter().find(|pr| pr.0 == action) { add_provider(w, tr(name.0, name.1).into(), url.into(), cx); return true; }
    if action == ADD_CUSTOM { edit_url(w, None, cx); return true; }
    if action == DELETE { delete_provider(w, cx); return true; }
    for i in 0..2 {
        let pick = if action == EFFORT_AUTO[i] { Some(None) } else { EFFORTS[i].iter().position(|e| *e == action).map(Some) };
        let Some(pick) = pick else { continue };
        let efforts = model_of(slot(&prefs(), i)).map(|m| m.efforts).unwrap_or_default();
        let effort = pick.and_then(|n| efforts.get(n).cloned()).unwrap_or_default();
        w.set_pref(|p| slot_mut(p, i).effort = effort, cx);
        return true;
    }
    false
}

// ---- 选模型（第二级）

/// 上下文长度：百万以上写 M（正好是整数个 MiB 时按 1024 算，1048576 是 1M），以下写 K（能被 1024 整除的按 1024 算，131072 是 128K）。
fn context(n: u64) -> String {
    let short = if n >= 999_500 {
        let m = if n.is_multiple_of(1 << 20) { (1u64 << 20) as f64 } else { 1e6 };
        let v = (n as f64 / m * 10.).round() / 10.;
        if v.fract() == 0. { format!("{v:.0}M") } else { format!("{v:.1}M") }
    } else { let k = if n.is_multiple_of(1024) { 1024 } else { 1000 }; format!("{}K", (n + k / 2) / k) };
    crate::localized_format!("{} 上下文", "{} context", short)
}

/// 给一个位置选模型：所有服务商的模型按服务商分组，组名前是它的光球。模型多于八个时上面有搜索。
/// 哪家没连上，就在那一组里写原因，可以重新连接；每组最后都能手动填模型 ID。点一个就选它并回到文本页。
pub fn models(cx: &mut Context<Workspace>) -> AnyElement {
    let p = prefs();
    let current = slot(&p, PICKING.with(Cell::get)).clone();
    let search = SEARCH.with(|s| s.borrow().clone());
    let query = search.as_ref().map_or_else(String::new, |f| f.read(cx).text.trim().to_lowercase());
    let message = |text: SharedString| div().px(px(18.)).py(px(16.)).text_center().text_size(px(15.)).line_height(px(21.)).text_color(Theme::text_caption()).child(text).into_any_element();
    let total: usize = LISTS.with(|l| l.borrow().values().map(|(_, list)| if let List::Ready(m) = list { m.len() } else { 0 }).sum());
    let matches = |pr: &Provider| query.is_empty() || LISTS.with(|l| match l.borrow().get(&pr.id) {
        Some((_, List::Ready(models))) => models.iter().any(|m| m.id.to_lowercase().contains(&query) || m.name.as_ref().is_some_and(|n| n.to_lowercase().contains(&query))),
        _ => false,
    });
    let sections: Vec<AnyElement> = p.text_providers.iter().filter(|pr| matches(pr)).map(|pr| {
        let (mut rows, settled): (Vec<AnyElement>, bool) = LISTS.with(|l| match l.borrow().get(&pr.id).map(|(_, list)| list) {
            Some(List::Ready(models)) => {
                let shown: Vec<&Model> = models.iter().filter(|m| query.is_empty() || m.id.to_lowercase().contains(&query) || m.name.as_ref().is_some_and(|n| n.to_lowercase().contains(&query))).collect();
                if shown.is_empty() { (vec![message(if models.is_empty() { tr("没有列出模型", "No models listed") } else { tr("没有找到模型", "No models found") }.into())], true) }
                else { (shown.into_iter().map(|m| model_row(m, &pr.id, current.provider == pr.id && current.model == m.id, cx)).collect(), true) }
            }
            Some(List::Failed(e)) => (vec![message(e.clone().into()), general::action(retry_id(&pr.id), tr("重新连接", "Reconnect"), false, { let id = pr.id.clone(); move |_, cx| if let Some(p) = find(&id) { fetch(&p, cx); } }, cx)], true),
            _ => (vec![skeleton()], false),
        });
        if settled && query.is_empty() {
            let id = pr.id.clone();
            rows.push(div().id(SharedString::from(format!("text-manual-{}", pr.id))).w_full().h(px(52.)).px(px(18.)).flex().items_center()
                .active(|d| d.bg(Theme::fill_pressed())).text_size(px(17.)).text_color(Theme::primary()).child(tr("输入模型 ID", "Enter Model ID"))
                .on_click(cx.listener(move |w, _, _, cx| enter_model(w, id.clone(), cx))).into_any_element());
        }
        div().w_full().flex().flex_col()
            .child(div().px(px(26.)).pb(px(10.)).flex().items_center().gap(px(8.)).child(orb(&pr.id, 20.))
                .child(div().min_w_0().truncate().text_size(px(18.)).line_height(px(24.)).font_weight(FontWeight::BOLD).text_color(Theme::text()).child(pr.name.clone())))
            .child(general::card(rows))
            .into_any_element()
    }).collect();
    div().w_full().pt(px(18.)).pb(px(24.)).flex().flex_col().gap(px(28.))
        .when(total > 8, |d| d.children(search.map(|f| general::card(vec![div().px(px(18.)).h(px(50.)).flex().items_center().gap(px(10.))
            .child(ui::icon("search", 17., Theme::text_caption())).child(div().flex_1().min_w_0().child(f)).into_any_element()]))))
        .when(sections.is_empty(), |d| d.child(general::card(vec![message(tr("没有找到模型", "No models found").into())])))
        .children(sections)
        .into_any_element()
}

/// 每个服务商的「重新连接」要一个固定的 id：按服务商 id 记下来，不重复分配。
fn retry_id(id: &str) -> &'static str {
    thread_local! { static IDS: RefCell<HashMap<String, &'static str>> = RefCell::new(HashMap::new()); }
    IDS.with(|ids| *ids.borrow_mut().entry(id.to_owned()).or_insert_with(|| Box::leak(format!("text-retry-{id}").into_boxed_str())))
}

/// 模型一行：名称，下面一行灰字是 id（有显示名时）、上下文长度、是否推理；选中的打勾。
fn model_row(m: &Model, provider: &str, on: bool, cx: &mut Context<Workspace>) -> AnyElement {
    let mut facts: Vec<String> = Vec::new();
    if m.name.is_some() { facts.push(m.id.clone()); }
    if let Some(n) = m.context { facts.push(context(n)); }
    if m.reasoning { facts.push(tr("推理", "Reasoning").into()); }
    let (id, provider) = (m.id.clone(), provider.to_owned());
    div().id(SharedString::from(format!("text-model-{provider}-{}", m.id))).w_full().min_h(px(54.)).px(px(18.)).py(px(11.)).flex().items_center().gap(px(12.))
        .active(|d| d.bg(Theme::fill_pressed()))
        .child(div().flex_1().min_w_0().flex().flex_col().gap(px(2.))
            .child(div().truncate().text_size(px(17.)).line_height(px(22.)).text_color(row_ink()).child(m.label().to_owned()))
            .when(!facts.is_empty(), |d| d.child(div().truncate().text_size(px(14.)).line_height(px(19.)).text_color(Theme::text_caption()).child(facts.join(" · ")))))
        .child(div().w(px(20.)).flex_none().flex().justify_end().when(on, |d| d.child(ui::icon("check", 16., Theme::text()))))
        .on_click(cx.listener(move |w, _, window, cx| { window.blur(cx); choose(w, provider.clone(), id.clone(), cx); }))
        .into_any_element()
}

/// 选定：换了模型就清掉这个位置的测试结果；新模型也支持原来的推理强度才留着。
fn choose(w: &mut Workspace, provider: String, model: String, cx: &mut Context<Workspace>) {
    super::super::glass::haptic();
    let i = PICKING.with(Cell::get);
    let s = slot(&prefs(), i).clone();
    if s.provider != provider || s.model != model {
        stop();
        TESTS.with(|t| t.borrow_mut()[i] = Test::Idle);
        let mut next = Slot { provider, model, ..Slot::default() };
        next.name = model_of(&next).and_then(|m| m.name).unwrap_or_default();
        if model_of(&next).is_some_and(|m| m.efforts.contains(&s.effort)) { next.effort = s.effort; }
        w.set_pref(|p| *slot_mut(p, i) = next, cx);
    }
    super::super::settings_back(w, cx);
}

/// 服务不列模型、或者列表里没有想用的：手动填 id。
fn enter_model(w: &mut Workspace, provider: String, cx: &mut Context<Workspace>) {
    super::super::glass::haptic();
    let s = slot(&prefs(), PICKING.with(Cell::get)).clone();
    let value = if s.provider == provider { s.model } else { String::new() };
    if !super::prompt(tr("模型 ID", "Model ID"), None, "gpt-5", &value, Field::Url, None, move |value| {
        let Some(id) = value.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty()) else { return };
        let provider = provider.clone();
        menu::dispatch(move |w, _, cx| choose(w, provider, id, cx));
    }) { w.show_toast_kind(ui::ToastKind::Error, tr("无法打开模型设置", "Could not open model settings"), cx); }
}

/// 载入时的几行骨架，和音色列表一样轻轻地明暗。
fn skeleton() -> AnyElement {
    let block = |w: f32, h: f32| div().w(px(w)).h(px(h)).rounded_full().bg(theme::ca(craft::N1000, 0.07));
    div().w_full().flex().flex_col().children((0..3).map(|i| div().w_full().h(px(64.)).px(px(18.)).flex().flex_col().justify_center().gap(px(9.))
        .child(block(110. + (i * 37 % 70) as f32, 12.)).child(block(170. + (i * 53 % 60) as f32, 9.))))
        .with_animation("text-skeleton", Animation::new(std::time::Duration::from_millis(1100)).repeat(), |d, t| d.opacity(0.55 + 0.45 * (0.5 - 0.5 * (t * std::f32::consts::TAU).cos())))
        .into_any_element()
}
