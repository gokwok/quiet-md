//! 设置 › MarginAI › 助手（`fdl_ai::assistants`，存在全局库的 `assistants.json`，各空间共用）。
//! 第一级是助手的列表（头像、名字、设定的第一行），最后一行「添加助手」；内置的 MarginAI 排最前。
//! 点开一个：大头像（右下角换头像，借语音的「选择头像」页）、名称；设定和开场白（在系统文字页里改）；
//! 核心（酒馆 / Agent，酒馆再选预设）、默认权限、模型（跟随全局，或借「选择模型」页选一个）、形象（图像的人物）、声线（语音角色）；
//! 复制一份、删除（内置的删不了）。每改一处整份写回去。
//! 设置 › 高级设置 里的「MarginAI 资料位置」也在这里：全局库放 iCloud 还是本机（`fdl_ai::global::set_place`）。
use std::{cell::RefCell, collections::HashMap, rc::Rc};

use fdl_ai::assistants::{Assistant, Assistants, Core, Kind, Model, Permission};
use fdl_ai::global::{self, Place};
use fdl_ai::tavern::library::{self, Settings};
use gpui::{prelude::FluentBuilder, *};

use crate::{app::Workspace, settings::{Page, row_chevron, row_ink, tr}, theme::{self, Theme, craft}, ui};
use super::{Field, general, menu, sheet, text::Slot, writing::{add_row, delete_row, intern, line, text_block}};

const PLACE_LOCAL: &str = "marginai-place-local";
const PLACE_ICLOUD: &str = "marginai-place-icloud";
const CORE_TAVERN: &str = "marginai-core-tavern";
const CORE_AGENT: &str = "marginai-core-agent";
const READ: &str = "marginai-perm-read";
const WRITE: &str = "marginai-perm-write";
const MODEL_FOLLOW: &str = "marginai-model-follow";
const MODEL_PICK: &str = "marginai-model-pick";
const NO_CHARACTER: &str = "marginai-character-none";
const NO_VOICE: &str = "marginai-voice-none";
const DELETE: &str = "marginai-delete";
/// 菜单里一项一个助手、预设、人物或声线：`<前缀><标识>`。
const PRESET: &str = "marginai-preset:";
const CHARACTER: &str = "marginai-character:";
const VOICE: &str = "marginai-voice:";
/// 空间设置的「默认助手」：`<前缀><标识>`，空标识是内置的。
const SPACE: &str = "marginai-space:";

/// 空间设置里「默认助手」菜单的一项。
pub(crate) fn space_item(a: &Assistant, on: bool) -> menu::Item {
    menu::Item::new(key(SPACE, if a.is_builtin() { "" } else { &a.id }), a.name.clone(), "", 0).checked(on)
}

thread_local! {
    static ALL: RefCell<Option<Assistants>> = const { RefCell::new(None) };
    /// 正在看的助手。
    static EDITING: RefCell<String> = const { RefCell::new(String::new()) };
}

fn dir() -> std::path::PathBuf { super::super::assist::dir() }
fn haptic() { super::super::glass::haptic(); }
fn toast(w: &mut Workspace, text: &str, cx: &mut Context<Workspace>) { w.show_toast_kind(ui::ToastKind::Error, text, cx); }
pub(crate) fn all() -> Assistants { ALL.with(|a| a.borrow_mut().get_or_insert_with(|| Assistants::load(&dir())).clone()) }
fn change(f: impl FnOnce(&mut Assistants)) {
    ALL.with(|a| {
        let mut a = a.borrow_mut();
        let all = a.get_or_insert_with(|| Assistants::load(&dir()));
        f(all);
        if let Err(e) = all.save(&dir()) { log::warn!("Could not save assistants: {e}"); }
    });
}
fn editing() -> String { EDITING.with(|e| e.borrow().clone()) }
fn open_one() -> Option<Assistant> { all().get(&editing()).cloned() }
fn change_one(f: impl FnOnce(&mut Assistant)) { let id = editing(); change(|a| if let Some(x) = a.get_mut(&id) { f(x); }); }

/// 进助手的两页都重读一次（另一台设备可能改过）。
pub fn opened() { ALL.with(|a| *a.borrow_mut() = Some(Assistants::load(&dir()))); }

pub fn title() -> &'static str { open_one().map_or("", |a| intern(&a.name)) }

/// 菜单项的标识要 `&'static str`：按前缀和标识记一份，不重复分配。
pub(crate) fn key(prefix: &str, id: &str) -> &'static str {
    thread_local! { static KEYS: RefCell<HashMap<String, &'static str>> = RefCell::new(HashMap::new()); }
    let k = format!("{prefix}{id}");
    KEYS.with(|keys| *keys.borrow_mut().entry(k.clone()).or_insert_with(|| Box::leak(k.into_boxed_str())))
}

/// 助手头像：换过的是自带头像或照片；没换（或照片不在这台设备上）是名字的第一个字。
pub(crate) fn face(a: &Assistant, size: f32) -> AnyElement {
    super::roles::avatar_image(a.avatar.as_deref(), size).unwrap_or_else(|| {
        let first: String = a.name.trim().chars().next().map_or_else(String::new, |ch| ch.to_uppercase().collect());
        div().size(px(size)).flex_none().rounded_full().bg(general::card_bg()).border_1().border_color(theme::ca(craft::N1000, 0.06))
            .flex().items_center().justify_center()
            .child(div().text_size(px(size * 0.4)).line_height(px(size * 0.5)).font_weight(FontWeight::MEDIUM).text_color(Theme::text_caption()).child(first))
            .into_any_element()
    })
}

fn first_line(s: &str) -> Option<String> { s.lines().map(str::trim).find(|l| !l.is_empty()).map(str::to_owned) }
fn core_name(k: Kind) -> &'static str { match k { Kind::Tavern => tr("酒馆", "Tavern"), Kind::Agent => "Agent" } }

// ---- 存放位置（设置第一级）

/// 设置 › 高级设置 里的「MarginAI 资料位置」：全局库放 iCloud 还是本机，就地弹出系统菜单；这台设备没有 iCloud 时 iCloud 那项置灰。
/// 值下面一行灰字说清楚里面是什么、所有空间共用（和空间自己的存放位置不是一回事）。
pub fn place_row() -> AnyElement {
    let here = global::place();
    let label = |p: Place| match p { Place::ICloud => "iCloud", Place::Local => tr("本机", "On this device") };
    let icloud = menu::Item::new(PLACE_ICLOUD, "iCloud", "icloud", 0).checked(here == Place::ICloud);
    let items = vec![menu::Item::new(PLACE_LOCAL, label(Place::Local), "iphone", 0).checked(here == Place::Local), if global::icloud_available() { icloud } else { icloud.disabled() }];
    div().relative().w_full().px(px(18.)).py(px(12.)).flex().items_center().gap(px(12.))
        .child(div().flex_1().min_w_0().flex().flex_col().gap(px(2.))
            .child(div().text_size(px(14.)).line_height(px(19.)).text_color(Theme::text_caption()).child(tr("MarginAI 资料位置", "MarginAI Library Location")))
            .child(div().text_size(px(17.)).line_height(px(22.)).text_color(row_ink()).child(label(here)))
            .child(div().pt(px(2.)).text_size(px(14.)).line_height(px(19.)).text_color(Theme::text_caption())
                .child(tr("助手、预设、图像库和语音角色，所有空间共用。", "Assistants, presets, the image library and voice roles, shared by all spaces."))))
        .child(ui::icon("chevron-up-down", 16., row_chevron()))
        .child(menu::button("marginai-place-menu", items))
        .into_any_element()
}

fn set_place(w: &mut Workspace, place: Place, cx: &mut Context<Workspace>) {
    if global::place() == place { return; }
    haptic();
    match global::set_place(place) {
        // 换了目录：各页记着的都重读。
        Ok(_) => { opened(); super::roles::reload(); super::image::opened(); super::prompts::opened(); }
        Err(e) => toast(w, &crate::localized_format!("无法更换存放位置：{}", "Could not change the location: {}", e), cx),
    }
}

// ---- 列表

/// 助手：一张卡片，一行一个（头像、名字、设定的第一行），最后「添加助手」。
pub fn page(cx: &mut Context<Workspace>) -> AnyElement {
    let mut rows: Vec<AnyElement> = all().list.iter().map(|a| {
        let id = a.id.clone();
        let sub = first_line(&a.description);
        div().id(SharedString::from(format!("marginai-assistant-{}", a.id))).w_full().min_h(px(64.)).px(px(18.)).py(px(10.)).flex().items_center().gap(px(12.))
            .active(|d| d.bg(Theme::fill_pressed()))
            .child(face(a, 40.))
            .child(div().flex_1().min_w_0().flex().flex_col().gap(px(1.))
                .child(div().truncate().text_size(px(17.)).line_height(px(22.)).text_color(row_ink()).child(a.name.clone()))
                .children(sub.map(|s| div().truncate().text_size(px(13.)).line_height(px(18.)).text_color(Theme::text_caption()).child(s))))
            .child(ui::icon("chevron-right", 14., row_chevron()))
            .on_click(cx.listener(move |w, _, _, cx| { haptic(); show(w, id.clone(), cx); }))
            .into_any_element()
    }).collect();
    rows.push(add_row("marginai-add", tr("添加助手", "Add Assistant"), add, cx));
    div().w_full().pt(px(18.)).pb(px(24.)).child(general::card(rows)).into_any_element()
}

fn show(w: &mut Workspace, id: String, cx: &mut Context<Workspace>) {
    EDITING.with(|e| *e.borrow_mut() = id);
    w.open_settings(Page::AssistantDetail, cx);
}

fn new_id() -> String { uuid::Uuid::new_v4().simple().to_string()[..12].to_owned() }

/// 起名字：系统的对话框，确定后交给 `done`（空的不算）。
fn name_prompt(w: &mut Workspace, title: &str, value: &str, done: impl Fn(String, &mut Workspace, &mut Context<Workspace>) + 'static, cx: &mut Context<Workspace>) {
    let done = Rc::new(done);
    if !super::prompt(title, None, tr("名称", "Name"), value, Field::Text, None, move |v| {
        let Some(name) = v.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty()) else { return };
        let done = done.clone();
        menu::dispatch(move |w, _, cx| { done(name, w, cx); cx.notify(); });
    }) { toast(w, tr("无法打开名称设置", "Could not open name settings"), cx); }
}

/// 新助手：Agent 核心，其余按默认。
fn add(w: &mut Workspace, cx: &mut Context<Workspace>) {
    name_prompt(w, tr("添加助手", "Add Assistant"), "", |name, w, cx| {
        let id = new_id();
        change(|a| a.list.push(Assistant::new(&id, &name, Core::agent())));
        show(w, id, cx);
    }, cx);
}

/// 复制一份：名字后面加「副本」，复制完就看着这一份。照片头像两份共用一个文件（删的时候还有人用就不删）。
fn duplicate(w: &mut Workspace, cx: &mut Context<Workspace>) {
    let Some(a) = open_one() else { return };
    let id = new_id();
    change(|all| { all.duplicate(&a.id, &id, &crate::localized_format!("{} 副本", "{} Copy", a.name)); });
    super::super::shot::capture_page();
    show(w, id, cx);
}

/// 不再用的照片头像删掉：还有别的助手用着就留着。
fn discard(avatar: Option<String>) {
    if let Some(av) = avatar && !all().list.iter().any(|a| a.avatar.as_deref() == Some(&av)) { super::roles::discard_avatar(Some(&av)); }
}

fn change_avatar(w: &mut Workspace, cx: &mut Context<Workspace>) {
    let Some(a) = open_one() else { return };
    super::roles::choose_avatar(a.avatar, Page::AssistantDetail, Rc::new(|_, next, cx| set_avatar(next, cx)), w, cx);
}
fn set_avatar(next: Option<String>, cx: &mut Context<Workspace>) {
    let mut old = None;
    change_one(|a| old = std::mem::replace(&mut a.avatar, next));
    discard(old);
    cx.notify();
}

// ---- 一个助手

/// 一段长字（设定、开场白）：小标题和预览，点开在系统文字页里改。
fn long_text(id: &'static str, title: &'static str, value: String, save: fn(&mut Assistant, String), cx: &mut Context<Workspace>) -> Div {
    div().w_full().flex().flex_col().child(super::roles::header(title))
        .child(text_block(id, &value, false, false, true).on_click(cx.listener(move |w, _, window, cx| {
            window.blur(cx);
            haptic();
            if !sheet::edit(title, &value, false, move |text| menu::dispatch(move |_, _, cx| { change_one(|a| save(a, text)); cx.notify(); })) {
                toast(w, tr("无法打开编辑", "Could not open the editor"), cx);
            }
        })))
}

/// 酒馆核心的预设：「默认」和导入过的（在写作里导入）。
fn presets() -> Vec<(String, String)> {
    let d = dir();
    let mut out = vec![(String::new(), tr("默认", "Default").to_owned())];
    out.extend(library::presets(&d, &Settings::load(&d).names).into_iter().map(|l| (l.id, l.name)));
    out
}

pub fn detail(cx: &mut Context<Workspace>) -> AnyElement {
    let Some(a) = open_one() else { return div().into_any_element() };
    const SIZE: f32 = 120.;
    let glass = super::super::glass::supported();
    let camera = div().id("marginai-avatar-badge").absolute().right(px(-2.)).bottom(px(-2.)).size(px(38.)).rounded_full()
        .map(|d| if glass { d.child(super::super::glass::page_pane("marginai-avatar-glass", None, Some(("sf:camera.fill", 16., Theme::text())), None)) }
            else { d.bg(Theme::bg_body()).shadow(theme::shadow_button()).flex().items_center().justify_center().child(ui::icon("pen-line", 15., Theme::text())) })
        .on_click(cx.listener(move |w, _, _, cx| { if glass { super::super::glass::pulse("marginai-avatar-glass"); } change_avatar(w, cx); }));
    let kind = a.core.kind();
    let hero = div().w_full().px(px(24.)).flex().flex_col().items_center()
        // 首字的圆底是半透明的，垫阴影会透出一团灰：只给图片垫。
        .child(div().relative().size(px(SIZE)).child(div().rounded_full().when(super::roles::avatar_image(a.avatar.as_deref(), 1.).is_some(), |d| d.shadow(theme::shadow_card())).child(face(&a, SIZE))).child(camera))
        .child(div().mt(px(18.)).max_w_full().truncate().text_size(px(28.)).line_height(px(34.)).font_weight(FontWeight::BOLD).text_color(Theme::text()).child(a.name.clone()))
        .child(div().mt(px(4.)).text_size(px(14.)).line_height(px(19.)).text_color(Theme::text_caption()).child(core_name(kind)));

    let mut basics = vec![
        line("marginai-name", tr("名称", "Name"), a.name.clone(), true).on_click(cx.listener(|w, _, _, cx| {
            haptic();
            let Some(a) = open_one() else { return };
            name_prompt(w, tr("助手名称", "Assistant Name"), &a.name, |name, _, _| change_one(|a| a.name = name), cx);
        })).into_any_element(),
        general::action("marginai-photo", tr("更换头像", "Change Avatar"), false, change_avatar, cx),
    ];
    if a.avatar.is_some() { basics.push(general::action("marginai-photo-reset", tr("恢复默认头像", "Use Default Avatar"), false, |_, cx| set_avatar(None, cx), cx)); }

    // 核心：酒馆再选预设。
    let mut core = vec![general::choice("marginai-core-menu", tr("核心", "Core"), core_name(kind), vec![
        menu::Item::new(CORE_TAVERN, core_name(Kind::Tavern), "theatermasks", 0).checked(kind == Kind::Tavern),
        menu::Item::new(CORE_AGENT, core_name(Kind::Agent), "sparkles", 0).checked(kind == Kind::Agent),
    ])];
    if kind == Kind::Tavern {
        let list = presets();
        let now = a.core.preset();
        let name = list.iter().find(|(id, _)| id == now).map_or_else(|| tr("找不到预设", "Preset missing").to_owned(), |(_, n)| n.clone());
        core.push(general::choice("marginai-preset-menu", tr("预设", "Preset"), name, list.iter().map(|(id, n)| menu::Item::new(key(PRESET, id), n.clone(), "", 0).checked(id == now)).collect()));
    }
    let write = a.permission == Permission::Write;
    let perm = |w: bool| if w { tr("可修改", "Can Edit") } else { tr("只读", "Read Only") };
    core.push(general::choice("marginai-perm-menu", tr("默认权限", "Default Permission"), perm(write), vec![
        menu::Item::new(READ, perm(false), "eye", 0).checked(!write), menu::Item::new(WRITE, perm(true), "pencil", 0).checked(write),
    ]));
    let follow = tr("跟随全局", "Follow Global");
    let model = a.model.as_ref().map_or_else(|| follow.to_owned(), |m| super::text::slot_name(&Slot { provider: m.provider.clone(), model: m.model.clone(), name: m.name.clone(), effort: String::new() }));
    core.push(general::choice("marginai-model-menu", tr("模型", "Model"), model, vec![
        menu::Item::new(MODEL_FOLLOW, follow, "", 0).checked(a.model.is_none()), menu::Item::new(MODEL_PICK, tr("选择模型…", "Choose Model…"), "", 1).checked(a.model.is_some()),
    ]));

    // 形象和声线：可以不要。
    let none = tr("无", "None");
    let people = super::prompts::lib().characters;
    let character = a.character.as_deref().map_or(none.to_owned(), |id| people.iter().find(|c| c.id == id).map_or_else(|| tr("找不到人物", "Person missing").to_owned(), |c| c.name.clone()));
    let mut items = vec![menu::Item::new(NO_CHARACTER, none, "", 0).checked(a.character.is_none())];
    items.extend(people.iter().map(|c| menu::Item::new(key(CHARACTER, &c.id), c.name.clone(), "", 1).checked(a.character.as_deref() == Some(&c.id))));
    let roles = super::roles::all();
    let voice = a.voice.as_deref().map_or(none.to_owned(), |id| roles.iter().find(|r| r.voice_id() == id).map_or_else(|| tr("找不到声线", "Voice missing").to_owned(), |r| r.name().to_owned()));
    let mut voices = vec![menu::Item::new(NO_VOICE, none, "", 0).checked(a.voice.is_none())];
    voices.extend(roles.iter().map(|r| menu::Item::new(key(VOICE, r.voice_id()), r.name().to_owned(), "", 1).picture(super::roles::picture(r)).checked(a.voice.as_deref() == Some(r.voice_id()))));
    let looks = vec![general::choice("marginai-character-menu", tr("形象", "Look"), character, items), general::choice("marginai-voice-menu", tr("声线", "Voice"), voice, voices)];

    let mut files = vec![general::action("marginai-copy", tr("复制一份", "Duplicate"), false, duplicate, cx)];
    if !a.is_builtin() { files.push(delete_row("marginai-delete-menu", DELETE, tr("删除助手", "Delete Assistant"))); }
    div().w_full().pt(px(12.)).pb(px(24.)).flex().flex_col().gap(px(32.))
        .child(hero)
        .child(general::card(basics))
        .child(long_text("marginai-description", tr("设定", "Persona"), a.description.clone(), |a, t| a.description = t, cx))
        .child(long_text("marginai-greeting", tr("开场白", "Greeting"), a.greeting.clone(), |a, t| a.greeting = t, cx))
        .child(general::card(core))
        .child(general::card(looks))
        .child(general::titled(None, files))
        .into_any_element()
}

/// 详情页上的几个菜单、MarginAI 资料的位置，空间设置的「默认助手」（交给 `spaces.rs`）。
pub fn menu_action(action: &str, w: &mut Workspace, cx: &mut Context<Workspace>) -> bool {
    if let Some(id) = action.strip_prefix(SPACE) { super::super::spaces::pick_assistant(w, id, cx); return true; }
    if let Some(id) = action.strip_prefix(PRESET) { change_one(|a| a.core.set_preset(id)); }
    else if let Some(id) = action.strip_prefix(CHARACTER) { change_one(|a| a.character = Some(id.to_owned())); }
    else if let Some(id) = action.strip_prefix(VOICE) { change_one(|a| a.voice = Some(id.to_owned())); }
    else {
        match action {
            PLACE_LOCAL | PLACE_ICLOUD => set_place(w, if action == PLACE_ICLOUD { Place::ICloud } else { Place::Local }, cx),
            CORE_TAVERN | CORE_AGENT => change_one(|a| a.core.set_kind(if action == CORE_TAVERN { Kind::Tavern } else { Kind::Agent })),
            READ | WRITE => change_one(|a| a.permission = if action == WRITE { Permission::Write } else { Permission::Read }),
            MODEL_FOLLOW => change_one(|a| a.model = None),
            MODEL_PICK => {
                let current = open_one().and_then(|a| a.model).map_or_else(Slot::default, |m| Slot { provider: m.provider, model: m.model, name: m.name, effort: String::new() });
                super::text::choose_model(current, Page::AssistantDetail, Rc::new(|_, s, cx| { change_one(|a| a.model = Some(Model { provider: s.provider, model: s.model, name: s.name })); cx.notify(); }), w, cx);
            }
            NO_CHARACTER => change_one(|a| a.character = None),
            NO_VOICE => change_one(|a| a.voice = None),
            DELETE => {
                let id = editing();
                let photo = open_one().and_then(|a| a.avatar);
                change(|a| { a.remove(&id); });
                discard(photo);
                super::super::settings_back(w, cx);
            }
            _ => return false,
        }
    }
    cx.notify();
    true
}
