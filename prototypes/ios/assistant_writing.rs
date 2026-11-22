//! 设置 › 助手设置 › 写作：预设、助手和人设。
//! 预设在空间的 `tavern/` 目录里，跟着 iCloud 走（`fdl_ai::tavern::library`）；每改一处就整份写回去。内置的「默认」只能看，可以复制一份再改。
//! 第二级：预设（名称、条目、正则、文件）→ 条目列表（搜索、开关、长按拖动排序）→ 一条条目；正则列表 → 一条正则；助手或人设。
//! 长的字（提示词、正则、描述）在系统的文字页里改（`sheet.rs`），引号和破折号原样留着。
use std::{cell::{Cell, RefCell}, collections::HashSet, path::PathBuf, time::{Duration, Instant}};

use fdl_ai::tavern::{library::{self, Card, Listed, Settings}, macros::{Env, Macros}, preset::{Position, Preset, Prompt, Role}};
use gpui::{prelude::FluentBuilder, *};
use serde_json::json;

use crate::{app::Workspace, input::TextField, settings::{Page, prefs, row_chevron, row_ink, tr}, theme::{self, Theme, craft}, ui};
use super::{Field, general, menu, sheet};

/// 正在看的预设：标识（空的是内置的「默认」）和读出来的内容。
#[derive(Clone)]
struct Open { id: String, preset: Preset }

/// 条目一行的高度，加上行间那道线就是一格。拖动排序按格算落点。
const ROW: f32 = 52.;
const STEP: f32 = ROW + 1.;

/// 拖着的那条：原来第几条、手指离这一格顶上多远、手指现在的纵坐标、现在该落在第几条。
#[derive(Clone)]
struct Drag { from: usize, grab: f32, y: f32, at: usize }

const ROLE_ITEMS: [(&str, Role); 3] = [("tavern-role-system", Role::System), ("tavern-role-user", Role::User), ("tavern-role-assistant", Role::Assistant)];
const POS_RELATIVE: &str = "tavern-pos-relative";
const POS_DEPTH: &str = "tavern-pos-depth";
const DELETE_PRESET: &str = "tavern-delete-preset";
const DELETE_ENTRY: &str = "tavern-delete-entry";
const DELETE_REGEX: &str = "tavern-delete-regex";
const DELETE_CARD: &str = "tavern-delete-card";
/// SillyTavern 的 `placement`：1 是我的要求，2 是模型的输出。
const USER_INPUT: i64 = 1;
const AI_OUTPUT: i64 = 2;

thread_local! {
    static SETTINGS: RefCell<Option<Settings>> = const { RefCell::new(None) };
    static LISTED: RefCell<Vec<Listed>> = const { RefCell::new(Vec::new()) };
    static OPEN: RefCell<Option<Open>> = const { RefCell::new(None) };
    static ENTRY: RefCell<String> = const { RefCell::new(String::new()) };
    static REGEX: Cell<usize> = const { Cell::new(0) };
    /// 正在看的是人设（真）还是助手，和它的标识。
    static CARD: RefCell<(bool, String)> = const { RefCell::new((false, String::new())) };
    static SEARCH: RefCell<Option<Entity<TextField>>> = const { RefCell::new(None) };
    static SORTING: Cell<bool> = const { Cell::new(false) };
    static DRAG: RefCell<Option<Drag>> = const { RefCell::new(None) };
    /// 各条现在画在离自己位置多远：让位、落下时缓过去。按第几条记。
    static SHIFT: RefCell<Vec<f32>> = const { RefCell::new(Vec::new()) };
    /// 条目卡片在窗口里的位置（上一帧量的），拖动时算落点。
    static LIST: Cell<Option<Bounds<Pixels>>> = const { Cell::new(None) };
    /// 刚松手：手势补的那次点按不算点开条目。
    static DROPPED: Cell<Option<Instant>> = const { Cell::new(None) };
    /// 从一条条目回到列表时，滚回点开前的位置。
    static LIST_AT: Cell<Option<Pixels>> = const { Cell::new(None) };
    /// 还有预设在从 iCloud 下载：过一会儿再看一眼。
    static POLLING: Cell<bool> = const { Cell::new(false) };
    static TITLES: RefCell<HashSet<&'static str>> = RefCell::new(HashSet::new());
}

fn dir() -> PathBuf { super::super::assist::dir() }
fn settings() -> Settings { SETTINGS.with(|s| s.borrow_mut().get_or_insert_with(|| Settings::load(&dir())).clone()) }
/// 改设置（选中的预设、助手和人设的列表）并存下。
fn change_settings(f: impl FnOnce(&mut Settings)) {
    SETTINGS.with(|s| {
        let mut s = s.borrow_mut();
        let s = s.get_or_insert_with(|| Settings::load(&dir()));
        f(s);
        super::super::assist::save(s);
    });
}
fn open() -> Option<Open> { OPEN.with(|o| o.borrow().clone()) }
fn toast(w: &mut Workspace, text: &str, cx: &mut Context<Workspace>) { w.show_toast_kind(ui::ToastKind::Error, text, cx); }
fn haptic() { super::super::glass::haptic(); }

/// 预设的名字：「默认」是内置的那份。
fn preset_name(id: &str, p: &Preset) -> String { if id.is_empty() { tr("默认", "Default").into() } else { library::display_name(p, id) } }

/// 页名是 `&'static str`：名字记一份，同一个名字不重复记。太长的截短。
pub(super) fn intern(s: &str) -> &'static str {
    let s = s.trim();
    let s = if s.chars().count() > 16 { format!("{}…", s.chars().take(15).collect::<String>()) } else { s.to_owned() };
    TITLES.with(|t| {
        let mut t = t.borrow_mut();
        if let Some(x) = t.get(s.as_str()) { return *x; }
        let x: &'static str = Box::leak(s.into_boxed_str());
        t.insert(x);
        x
    })
}

pub fn title(page: Page) -> &'static str {
    match page {
        Page::AssistantWriting => tr("写作", "Writing"),
        Page::AssistantPreset => open().map_or("", |o| intern(&preset_name(&o.id, &o.preset))),
        Page::AssistantEntries => tr("条目", "Prompts"),
        Page::AssistantEntry => open().and_then(|o| o.preset.prompt(&ENTRY.with(|e| e.borrow().clone()))).map_or("", |p| { let n = entry_name(&p); intern(heading(&n).unwrap_or(n.as_str())) }),
        Page::AssistantRegexes => tr("正则", "Regex"),
        Page::AssistantRegex => open().and_then(|o| o.preset.regex_scripts().into_iter().nth(REGEX.with(Cell::get))).map_or("", |s| if s.name.trim().is_empty() { tr("正则", "Regex") } else { intern(&s.name) }),
        Page::AssistantCard => card_of().map_or("", |c| intern(&c.name)),
        _ => "",
    }
}

/// 第二级回哪一页。
pub fn parent(page: Page) -> Option<Page> {
    match page {
        Page::AssistantPreset | Page::AssistantCard => Some(Page::AssistantWriting),
        Page::AssistantEntries | Page::AssistantRegexes => Some(Page::AssistantPreset),
        Page::AssistantEntry => Some(Page::AssistantEntries),
        Page::AssistantRegex => Some(Page::AssistantRegexes),
        _ => None,
    }
}

/// 进写作的任何一页都重读一次设置（输入卡片上也能换预设、助手和人设）。
pub fn opened(page: Page, cx: &mut Context<Workspace>) {
    SETTINGS.with(|s| *s.borrow_mut() = Some(Settings::load(&dir())));
    DRAG.with(|d| d.borrow_mut().take());
    match page {
        Page::AssistantWriting => refresh_list(),
        Page::AssistantEntries => match LIST_AT.with(Cell::take) {
            // 从一条条目回来：滚回原处（`open_settings` 在这之后才把滚动归零）。
            Some(y) => {
                let ws = cx.entity().downgrade();
                cx.defer(move |cx| { let _ = ws.update(cx, |w, cx| { w.settings_scroll.set_offset(point(px(0.), y)); cx.notify(); }); });
            }
            None => {
                SORTING.with(|s| s.set(false));
                SEARCH.with(|search| {
                    let mut search = search.borrow_mut();
                    match search.as_ref() {
                        Some(field) => field.update(cx, |f, cx| f.set_text("", cx)),
                        None => {
                            let field = cx.new(|cx| TextField::new(tr("搜索条目", "Search prompts"), cx));
                            cx.observe(&field, |_, _, cx| cx.notify()).detach();
                            *search = Some(field);
                        }
                    }
                });
            }
        },
        _ => {}
    }
}

/// 重新列一遍导入的预设；见到的名字记进设置，别的设备上还没下载时也有名字可写。
fn refresh_list() {
    let s = settings();
    let list = library::presets(&dir(), &s.names);
    let fresh: Vec<(String, String)> = list.iter().filter(|l| l.state.ready() && s.names.get(&l.id) != Some(&l.name)).map(|l| (l.id.clone(), l.name.clone())).collect();
    if !fresh.is_empty() { change_settings(|s| s.names.extend(fresh)); }
    LISTED.with(|l| *l.borrow_mut() = list);
}

pub fn page(page: Page, scroll: &ScrollHandle, cx: &mut Context<Workspace>) -> AnyElement {
    match page {
        Page::AssistantWriting => writing(cx),
        Page::AssistantPreset => preset(cx),
        Page::AssistantEntries => entries(scroll, cx),
        Page::AssistantEntry => entry(cx),
        Page::AssistantRegexes => regexes(cx),
        Page::AssistantRegex => regex(cx),
        Page::AssistantCard => card(cx),
        _ => div().into_any_element(),
    }
}

// ---- 共用的行

/// 卡片里的一行：左边名称，右边灰字，能点的有箭头。
pub(super) fn line(id: impl Into<ElementId>, title: impl Into<SharedString>, value: impl Into<SharedString>, tappable: bool) -> Stateful<Div> {
    div().id(id).w_full().h(px(54.)).px(px(18.)).flex().items_center().gap(px(12.))
        .when(tappable, |d| d.active(|d| d.bg(Theme::fill_pressed())))
        .child(div().flex_none().text_size(px(17.)).text_color(row_ink()).child(title.into()))
        .child(div().flex_1().min_w_0().flex().justify_end().child(div().truncate().text_size(px(17.)).text_color(Theme::text_caption()).child(value.into())))
        .when(tappable, |d| d.child(ui::icon("chevron-right", 14., row_chevron())))
}

/// 一行带开关。
fn switch_line(title: &'static str, toggle: impl IntoElement) -> AnyElement {
    div().w_full().h(px(54.)).px(px(18.)).flex().items_center().gap(px(12.))
        .child(div().flex_1().min_w_0().text_size(px(17.)).text_color(row_ink()).child(title))
        .child(toggle).into_any_element()
}

/// 一行带勾（多选：点一下勾上或去掉）。
fn check_line(id: &'static str, title: &'static str, on: bool) -> Stateful<Div> {
    div().id(id).w_full().h(px(54.)).px(px(18.)).flex().items_center().gap(px(12.)).active(|d| d.bg(Theme::fill_pressed()))
        .child(div().flex_1().min_w_0().text_size(px(17.)).text_color(row_ink()).child(title))
        .child(div().w(px(20.)).flex_none().flex().justify_end().when(on, |d| d.child(ui::icon("check", 17., Theme::primary()))))
}

/// 列表里能选的一项（预设、助手、人设）：名字，可带一行灰字；选中的打勾。`state` 是还没下载时的说法，有它就不能点。
fn pick_row(id: impl Into<ElementId>, name: impl Into<SharedString>, sub: Option<String>, on: bool, state: Option<&'static str>) -> Stateful<Div> {
    let ready = state.is_none();
    div().id(id).w_full().min_h(px(54.)).px(px(18.)).py(px(10.)).flex().items_center().gap(px(12.))
        .when(ready, |d| d.active(|d| d.bg(Theme::fill_pressed())))
        .child(div().flex_1().min_w_0().flex().flex_col().gap(px(1.))
            .child(div().truncate().text_size(px(17.)).line_height(px(22.)).text_color(if ready { row_ink() } else { Theme::text_caption() }).child(name.into()))
            .children(sub.filter(|s| !s.is_empty()).map(|s| div().truncate().text_size(px(13.)).line_height(px(18.)).text_color(Theme::text_caption()).child(s))))
        .children(state.map(|s| div().flex_none().text_size(px(15.)).text_color(Theme::text_caption()).child(s)))
        .when(ready, |d| d.child(div().w(px(20.)).flex_none().flex().justify_end().when(on, |d| d.child(ui::icon("check", 17., Theme::primary()))))
            .child(ui::icon("chevron-right", 14., row_chevron())))
}

/// 「＋ 添加…」一行。
pub(super) fn add_row(id: &'static str, label: &'static str, on: impl Fn(&mut Workspace, &mut Context<Workspace>) + 'static, cx: &mut Context<Workspace>) -> AnyElement {
    div().id(id).w_full().h(px(52.)).px(px(18.)).flex().items_center().gap(px(8.)).active(|d| d.bg(Theme::fill_pressed()))
        .child(ui::icon("plus", 16., Theme::primary()))
        .child(div().text_size(px(17.)).text_color(Theme::primary()).child(label))
        .on_click(cx.listener(move |w, _, window, cx| { window.blur(cx); haptic(); on(w, cx); })).into_any_element()
}

/// 红色的删除：点一下弹出只有一项的系统菜单，再点才删，免得误触。
pub(super) fn delete_row(key: &'static str, id: &'static str, label: &'static str) -> AnyElement {
    div().relative().w_full().h(px(52.)).px(px(18.)).flex().items_center().text_size(px(17.)).text_color(Theme::danger())
        .child(label).child(menu::button(key, vec![menu::Item::new(id, label, "trash", 0).danger()])).into_any_element()
}
pub(super) fn delete_card(key: &'static str, id: &'static str, label: &'static str) -> Div { general::card(vec![delete_row(key, id, label)]) }

/// 已经在用：和「使用这个…」同一个位置，灰字加一个勾，点了没反应。版面不跳。
pub(super) fn in_use() -> AnyElement {
    div().w_full().h(px(52.)).px(px(18.)).flex().items_center().gap(px(12.))
        .child(div().flex_1().text_size(px(17.)).text_color(Theme::text_caption()).child(tr("正在使用", "In Use")))
        .child(ui::icon("check", 17., Theme::primary())).into_any_element()
}

/// 组名，和 `general::titled` 一样的字。
fn header(title: &'static str) -> Div { super::roles::header(title) }

/// 一段长字的预览：点开在文字页里改。空的写灰色的「空」。`error` 时描一圈红色。
fn text_block(id: &'static str, text: &str, mono: bool, error: bool, tappable: bool) -> Stateful<Div> {
    let empty = text.trim().is_empty();
    // 预览只取前面一截：整段提示词可能有几千字。
    let shown: String = text.chars().take(1200).collect();
    div().id(id).mx(px(10.)).px(px(18.)).py(px(14.)).rounded(px(20.)).bg(general::card_bg())
        .when(error, |d| d.border_1().border_color(Theme::danger()))
        .when(tappable, |d| d.active(|d| d.bg(Theme::fill_pressed())))
        // 按高度截，不按行数（`line_clamp` 遇到换行后的段落不折行）。
        .child(div().text_size(px(if mono { 14. } else { 15. })).line_height(px(if mono { 20. } else { 22. })).max_h(px(if mono { 160. } else { 220. })).overflow_hidden()
            .when(mono, |d| d.font_family(theme::MONO_FONT))
            .text_color(if empty { Theme::text_caption() } else { row_ink() })
            .child(if empty { tr("空", "Empty").to_owned() } else { shown }))
}

// ---- 写作（第一级）

/// 写作：预设（内置的「默认」在最前，导入的按名字排；没下载好的写着状态）、助手、人设。选中的打勾，点开看详情。
fn writing(cx: &mut Context<Workspace>) -> AnyElement {
    let s = settings();
    let listed = LISTED.with(|l| l.borrow().clone());
    let mut presets = vec![pick_row("tavern-preset-default", tr("默认", "Default"), None, s.preset.is_empty(), None)
        .on_click(cx.listener(|w, _, _, cx| { haptic(); open_preset(w, String::new(), cx) })).into_any_element()];
    for l in &listed {
        let label = crate::space_sync::download_label(&l.state);
        let id = l.id.clone();
        presets.push(pick_row(SharedString::from(format!("tavern-preset-{}", l.id)), l.name.clone(), None, s.preset == l.id, label)
            .when(label.is_none(), |d| d.on_click(cx.listener(move |w, _, _, cx| { haptic(); open_preset(w, id.clone(), cx) }))).into_any_element());
    }
    presets.push(add_row("tavern-import", tr("导入预设", "Import Preset"), import, cx));
    // 还在下载的：过两秒再看一眼，下好了就能点。
    if listed.iter().any(|l| !l.state.ready()) && !POLLING.with(Cell::get) {
        POLLING.with(|p| p.set(true));
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(2)).await;
            POLLING.with(|p| p.set(false));
            let _ = this.update(cx, |w, cx| if w.settings_page == Page::AssistantWriting { refresh_list(); cx.notify(); });
        }).detach();
    }
    let cards = |persona: bool, cx: &mut Context<Workspace>| {
        let (list, current) = if persona { (&s.personas, &s.persona) } else { (&s.chars, &s.char) };
        let mut rows: Vec<AnyElement> = list.iter().map(|c| {
            let id = c.id.clone();
            let sub = c.description.lines().map(str::trim).find(|l| !l.is_empty()).map(str::to_owned);
            pick_row(SharedString::from(format!("tavern-card-{persona}-{}", c.id)), c.name.clone(), sub, *current == c.id, None)
                .on_click(cx.listener(move |w, _, _, cx| { haptic(); open_card(w, persona, id.clone(), cx) })).into_any_element()
        }).collect();
        rows.push(if persona { add_row("tavern-add-persona", tr("添加人设", "Add Persona"), |w, cx| add_card(w, true, cx), cx) }
            else { add_row("tavern-add-char", tr("添加助手", "Add Assistant"), |w, cx| add_card(w, false, cx), cx) });
        rows
    };
    let (chars, personas) = (cards(false, cx), cards(true, cx));
    div().w_full().pt(px(18.)).pb(px(24.)).flex().flex_col().gap(px(32.))
        .child(general::titled(Some(tr("预设", "Presets")), presets))
        .child(general::titled(Some(tr("助手", "Assistants")), chars))
        .child(general::titled(Some(tr("人设", "Personas")), personas))
        .into_any_element()
}

fn open_preset(w: &mut Workspace, id: String, cx: &mut Context<Workspace>) {
    match library::load(&dir(), &id) {
        Ok(preset) => { OPEN.with(|o| *o.borrow_mut() = Some(Open { id, preset })); w.open_settings(Page::AssistantPreset, cx); }
        Err(e) => toast(w, &e, cx),
    }
}

/// 导入：系统的文件选择器，可以一次选好几份。只导入一份时直接打开它。
fn import(_: &mut Workspace, cx: &mut Context<Workspace>) {
    let reply = super::super::files::open(PathPromptOptions { files: true, directories: false, multiple: true, prompt: None });
    cx.spawn(async move |this, cx| {
        let Some(paths) = reply.await.ok().and_then(Result::ok).flatten() else { return };
        let _ = this.update(cx, |w, cx| {
            let dir = dir();
            let (mut done, mut failed) = (Vec::new(), None);
            for path in paths {
                let file = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                let result = std::fs::read(&path).map_err(|e| e.to_string()).and_then(|b| library::import(&dir, &b, &file)).and_then(|id| Ok((library::load(&dir, &id)?, id)));
                match result { Ok((preset, id)) => done.push(Open { id, preset }), Err(e) => failed = Some(e) }
            }
            change_settings(|s| for o in &done { s.names.insert(o.id.clone(), library::display_name(&o.preset, &o.id)); });
            refresh_list();
            if let Some(e) = failed { toast(w, &crate::localized_format!("无法导入：{}", "Could not import: {}", e), cx); }
            match done.len() {
                0 => {}
                1 => { OPEN.with(|o| *o.borrow_mut() = done.pop()); w.open_settings(Page::AssistantPreset, cx); }
                n => w.show_toast_kind(ui::ToastKind::Success, &crate::localized_format!("已导入 {} 份预设", "Imported {} presets", n), cx),
            }
            cx.notify();
        });
    }).detach();
}

// ---- 预设（第二级）

/// 改正在看的预设并整份写回。「默认」不能改：界面上已经拦着，这里再拦一次。
fn change(w: &mut Workspace, cx: &mut Context<Workspace>, f: impl FnOnce(&mut Preset)) {
    let result = OPEN.with(|o| {
        let mut o = o.borrow_mut();
        let Some(open) = o.as_mut() else { return Ok(()) };
        if open.id.is_empty() { return Err(tr("「默认」不能改，先复制一份", "Duplicate the default to edit it").to_owned()); }
        f(&mut open.preset);
        library::save(&dir(), &open.id, &open.preset)
    });
    if let Err(e) = result { toast(w, &e, cx); }
    cx.notify();
}

fn read_only(w: &mut Workspace, cx: &mut Context<Workspace>) { toast(w, tr("「默认」不能改，先复制一份", "Duplicate the default to edit it"), cx); }

/// 预设：没在用时最上面是「使用这个预设」；名称；内容（条目、正则、不会运行的扩展）；文件（导出、复制一份、删除）。
fn preset(cx: &mut Context<Workspace>) -> AnyElement {
    let Some(Open { id, preset: p }) = open() else { return div().into_any_element() };
    let builtin = id.is_empty();
    let current = settings().preset == id;
    let order = p.order();
    let scripts = p.regex_scripts();
    let count = |on: usize, all: usize| -> SharedString { format!("{on}/{all}").into() };
    let mut content = vec![
        line("tavern-preset-entries", tr("条目", "Prompts"), count(order.iter().filter(|e| e.enabled).count(), order.len()), true)
            .on_click(cx.listener(|w, _, _, cx| { haptic(); w.open_settings(Page::AssistantEntries, cx) })).into_any_element(),
        line("tavern-preset-regex", tr("正则", "Regex"), count(scripts.iter().filter(|s| !s.disabled).count(), scripts.len()), true)
            .on_click(cx.listener(|w, _, _, cx| { haptic(); w.open_settings(Page::AssistantRegexes, cx) })).into_any_element(),
    ];
    // 预设带的脚本之类不会运行，只原样保留（导出时还在）。
    let ext = p.ignored_extensions();
    if let Some(first) = ext.iter().find(|e| *e == "tavern_helper").or_else(|| ext.first()) {
        let value = if ext.len() == 1 { first.clone() } else { crate::localized_format!("{} 等 {} 个", "{} and {} more", first, if prefs().english { ext.len() - 1 } else { ext.len() }) };
        content.push(line("tavern-preset-ext", tr("不运行的扩展", "Extensions Not Run"), value, false).into_any_element());
    }
    let name = preset_name(&id, &p);
    let mut files = vec![
        general::action("tavern-preset-export", tr("导出", "Export"), false, export, cx),
        general::action("tavern-preset-copy", tr("复制一份", "Duplicate"), false, duplicate, cx),
    ];
    if !builtin { files.push(delete_row("tavern-preset-delete-menu", DELETE_PRESET, tr("删除预设", "Delete Preset"))); }
    let use_id = id.clone();
    div().w_full().pt(px(18.)).pb(px(24.)).flex().flex_col().gap(px(28.))
        .child(general::card(vec![if current { in_use() } else { general::action("tavern-preset-use", tr("使用这个预设", "Use This Preset"), false, move |_, cx| {
            let id = use_id.clone();
            change_settings(|s| s.preset = id);
            cx.notify();
        }, cx) }]))
        .child(general::card(vec![line("tavern-preset-name", tr("名称", "Name"), name, !builtin)
            .when(!builtin, |d| d.on_click(cx.listener(|w, _, _, cx| rename_preset(w, cx)))).into_any_element()]))
        .child(general::titled(Some(tr("内容", "Contents")), content))
        .child(general::titled(Some(tr("文件", "File")), files))
        .into_any_element()
}

fn rename_preset(w: &mut Workspace, cx: &mut Context<Workspace>) {
    haptic();
    let Some(o) = open() else { return };
    let shown = preset_name(&o.id, &o.preset);
    if !super::prompt(tr("预设名称", "Preset Name"), None, tr("名称", "Name"), &shown, Field::Text, None, move |value| {
        let Some(name) = value.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty()) else { return };
        menu::dispatch(move |w, _, cx| {
            change(w, cx, |p| p.set_setting("name", json!(name)));
            if let Some(o) = open() { change_settings(|s| { s.names.insert(o.id.clone(), name); }); }
        });
    }) { toast(w, tr("无法打开名称设置", "Could not open name settings"), cx); }
}

/// 导出：整份 JSON 交给系统的「存储到…」面板，文件名是预设的名字。
fn export(w: &mut Workspace, cx: &mut Context<Workspace>) {
    let Some(o) = open() else { return };
    let (file, bytes) = match library::export(&dir(), &o.id) { Ok(x) => x, Err(e) => return toast(w, &e, cx) };
    let reply = super::super::files::new_path(Some(&file));
    cx.spawn(async move |this, cx| {
        let Ok(Ok(Some(to))) = reply.await else { return };
        let written = std::fs::write(&to, &bytes);
        let _ = this.update(cx, |w, cx| match written {
            Ok(_) => super::super::files::export(&to),
            Err(e) => toast(w, &e.to_string(), cx),
        });
    }).detach();
}

/// 复制一份（「默认」要改就从这里来）：名字后面加「副本」，复制完就看着这一份。
fn duplicate(w: &mut Workspace, cx: &mut Context<Workspace>) {
    let Some(o) = open() else { return };
    let mut copy = o.preset.clone();
    let name = crate::localized_format!("{} 副本", "{} Copy", preset_name(&o.id, &o.preset));
    copy.set_setting("name", json!(name));
    match library::create(&dir(), &copy) {
        Ok(id) => {
            change_settings(|s| { s.names.insert(id.clone(), name.clone()); });
            refresh_list();
            OPEN.with(|o| *o.borrow_mut() = Some(Open { id, preset: copy }));
            super::super::shot::capture_page();
            w.open_settings(Page::AssistantPreset, cx);
            w.show_toast_kind(ui::ToastKind::Success, &crate::localized_format!("已复制为「{}」", "Duplicated as “{}”", name), cx);
        }
        Err(e) => toast(w, &e, cx),
    }
}

fn delete_preset(w: &mut Workspace, cx: &mut Context<Workspace>) {
    let Some(o) = open() else { return };
    if let Err(e) = library::delete(&dir(), &o.id) { return toast(w, &e, cx); }
    change_settings(|s| { if s.preset == o.id { s.preset.clear(); } s.names.remove(&o.id); });
    OPEN.with(|x| x.borrow_mut().take());
    super::super::settings_back(w, cx);
}

// ---- 条目（第三级）

/// SillyTavern 的位置（标记）没改过名字时用中文说。
fn entry_name(p: &Prompt) -> String {
    if p.marker && p.name.is_ascii() && !prefs().english {
        let zh = match p.id.as_str() {
            "chatHistory" => "聊天记录", "worldInfoBefore" => "世界书（前）", "worldInfoAfter" => "世界书（后）", "charDescription" => "角色描述",
            "charPersonality" => "角色性格", "scenario" => "情景", "personaDescription" => "人设描述", "dialogueExamples" => "对话示例", _ => "",
        };
        if !zh.is_empty() { return zh.into(); }
    }
    p.name.trim().to_owned()
}

/// 「━━━━ 文风 ━━━━」这种用线围起来的名字是分组的小标题：取中间的字。它照样是一条能开关的条目。
fn heading(name: &str) -> Option<&str> {
    let deco = |c: char| ('\u{2500}'..='\u{257f}').contains(&c) || c == '=';
    let t = name.trim();
    if !(t.chars().next().is_some_and(deco) && t.chars().last().is_some_and(deco)) { return None; }
    Some(t.trim_matches(|c: char| deco(c) || c.is_whitespace())).filter(|s| !s.is_empty())
}

fn role_label(r: Option<Role>) -> &'static str {
    match r { Some(Role::User) => tr("用户", "User"), Some(Role::Assistant) => tr("助手", "Assistant"), _ => tr("系统", "System") }
}

/// 名字后面的小标签：位置（标记）、按深度插入的写深度，其余写角色。
fn tag(p: &Prompt) -> SharedString {
    if p.marker { return tr("位置", "Slot").into(); }
    match p.position { Position::Depth { depth, .. } => crate::localized_format!("深度 {}", "Depth {}", depth).into(), _ => role_label(p.role).into() }
}

fn drop_index(d: &Drag, n: usize) -> usize {
    let top = LIST.with(Cell::get).map_or(0., |b| f32::from(b.top()));
    ((d.y - d.grab - top) / STEP).round().clamp(0., n.saturating_sub(1) as f32) as usize
}

/// 条目：上面搜索和「排序」，下面一张卡片列着全部条目（名字、标签、开关）。长按一条拖动排序；排序时开关换成把手、不能搜索。
/// 内置的「默认」能看不能改。
fn entries(scroll: &ScrollHandle, cx: &mut Context<Workspace>) -> AnyElement {
    let Some(Open { id: pid, preset: p }) = open() else { return div().into_any_element() };
    let ro = pid.is_empty();
    let prompts = p.prompts();
    let order = p.order();
    let n = order.len();
    let sorting = SORTING.with(Cell::get) && !ro;
    let search = SEARCH.with(|s| s.borrow().clone());
    let query = if sorting { String::new() } else { search.as_ref().map_or_else(String::new, |f| f.read(cx).text.trim().to_lowercase()) };
    let drag = DRAG.with(|d| d.borrow().clone());
    let to = drag.as_ref().map(|d| drop_index(d, n));
    // 各条该在的位置：拖着的跟手指，夹在原位和落点之间的让一格；别的缓过去，还没到就接着出帧。
    let mut moving = false;
    let shifts: Vec<f32> = SHIFT.with(|s| {
        let mut s = s.borrow_mut();
        s.resize(n, 0.);
        (0..n).map(|i| {
            let want = match (&drag, to) {
                (Some(d), Some(_)) if d.from == i => 0.,
                (Some(d), Some(t)) => if d.from < i && i <= t { -STEP } else if t <= i && i < d.from { STEP } else { 0. },
                _ => 0.,
            };
            let next = if (want - s[i]).abs() < 0.5 { want } else { moving = true; s[i] + (want - s[i]) * 0.3 };
            s[i] = next;
            next
        }).collect()
    });
    let list_top = LIST.with(Cell::get).map_or(0., |b| f32::from(b.top()));
    let mut rows: Vec<AnyElement> = Vec::new();
    let mut lifted = None;
    for (i, e) in order.iter().enumerate() {
        let pr = prompts.iter().find(|x| x.id == e.id);
        let name = pr.map_or_else(|| e.id.clone(), entry_name);
        // 名字和内容都搜：大预设里常要找「哪一条写了这句」。
        if !query.is_empty() && !name.to_lowercase().contains(&query) && !pr.is_some_and(|p| p.content.to_lowercase().contains(&query)) { continue; }
        let last = i + 1 == n || !query.is_empty();
        match &drag {
            Some(d) if d.from == i => {
                rows.push(div().w_full().h(px(STEP)).into_any_element());
                lifted = Some(div().absolute().left_0().right_0().top(px(d.y - d.grab - list_top)).rounded(px(14.)).bg(Theme::bg_body()).shadow(theme::shadow_card())
                    .child(entry_row(i, e.enabled, pr, &name, true, ro, true, cx)).into_any_element());
            }
            _ => rows.push(div().relative().top(px(shifts[i])).w_full().flex().flex_col()
                .child(entry_row(i, e.enabled, pr, &name, sorting, ro, false, cx))
                .when(!last, |d| d.child(div().ml(px(18.)).h(px(1.)).bg(ui::list_line()))).into_any_element()),
        }
    }
    let found = !rows.is_empty();
    let measure = canvas(|bounds, _, _| LIST.with(|l| l.set(Some(bounds))), |_, _, _, _| {}).absolute().top_0().left_0().size_full();
    let list = div().relative().mx(px(10.)).flex().flex_col().rounded(px(20.)).bg(general::card_bg()).overflow_hidden()
        .child(measure).children(rows).children(lifted);
    let tools = div().mx(px(10.)).h(px(44.)).flex().items_center().gap(px(12.))
        // 排序时搜索框留着、变淡、点不动（只是一张画），头上不空一块。
        .map(|d| match search {
            Some(field) => d.child(div().flex_1().min_w_0().h_full().px(px(14.)).rounded(px(14.)).bg(general::card_bg()).flex().items_center().gap(px(8.))
                .when(sorting, |d| d.opacity(0.45))
                .child(ui::icon("search", 16., Theme::text_caption()))
                .child(if sorting { div().flex_1().min_w_0().text_size(px(14.)).text_color(Theme::text_caption()).child(tr("搜索条目", "Search prompts")) } else { div().flex_1().min_w_0().child(field) })),
            None => d.child(div().flex_1()),
        })
        .when(!ro, |d| d.child(div().id("tavern-sort").h_full().px(px(6.)).flex().items_center().text_size(px(17.)).text_color(Theme::primary())
            .font_weight(if sorting { FontWeight::SEMIBOLD } else { FontWeight::NORMAL })
            .child(if sorting { tr("完成", "Done") } else { tr("排序", "Sort") })
            .on_click(cx.listener(move |_, _, window, cx| {
                window.blur(cx);
                haptic();
                SORTING.with(|s| s.set(!sorting));
                if let Some(f) = SEARCH.with(|s| s.borrow().clone()) { f.update(cx, |f, cx| f.set_text("", cx)); }
                cx.notify();
            }))));
    let ws = cx.entity().downgrade();
    let scroll = scroll.clone();
    let can_drag = !ro && query.is_empty();
    let watch = canvas(|_, _, _| {}, move |_, _, window, _| {
        let drag = DRAG.with(|d| d.borrow().clone());
        if moving || drag.is_some() { window.request_animation_frame(); }
        // 拖到屏幕上下边：整页往那边滚。
        if let Some(d) = &drag {
            let b = scroll.bounds();
            let v = if d.y < f32::from(b.top()) + 120. { 9. } else if d.y > f32::from(b.bottom()) - 90. { -9. } else { 0. };
            if v != 0. { let o = scroll.offset(); scroll.set_offset(point(o.x, (o.y + px(v)).clamp(-scroll.max_offset().y, px(0.)))); }
        }
        if !can_drag { return; }
        let ws = ws.clone();
        window.on_mouse_event(move |ev: &LongPressEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble { return; }
            let active = DRAG.with(|d| d.borrow().is_some());
            match ev.phase {
                TouchPhase::Started => {
                    let Some(b) = LIST.with(Cell::get) else { return };
                    if active || !b.contains(&ev.start_position) { return; }
                    let at = f32::from(ev.start_position.y - b.top());
                    let k = (at / STEP).floor() as usize;
                    if k >= n { return; }
                    window.prevent_default();
                    crate::ios::take_long_press();
                    haptic();
                    DRAG.with(|d| *d.borrow_mut() = Some(Drag { from: k, grab: at - k as f32 * STEP, y: f32::from(ev.start_position.y), at: k }));
                }
                _ if !active => return,
                TouchPhase::Moved => {
                    let changed = DRAG.with(|d| {
                        let mut d = d.borrow_mut();
                        let Some(d) = d.as_mut() else { return false };
                        d.y = f32::from(ev.position.y);
                        let t = drop_index(d, n);
                        std::mem::replace(&mut d.at, t) != t
                    });
                    if changed { haptic(); }
                }
                TouchPhase::Ended => {
                    let Some(d) = DRAG.with(|x| x.borrow_mut().take()) else { return };
                    let t = drop_index(&d, n);
                    DROPPED.with(|x| x.set(Some(Instant::now())));
                    // 落下：各条从现在画着的地方缓到新位置（拖着的那条从手指下面）。
                    let top = LIST.with(Cell::get).map_or(0., |b| f32::from(b.top()));
                    let mut idx: Vec<usize> = (0..n).collect();
                    let moved = idx.remove(d.from);
                    idx.insert(t, moved);
                    SHIFT.with(|s| {
                        let mut s = s.borrow_mut();
                        let old = s.clone();
                        for (k, &i) in idx.iter().enumerate() {
                            let visual = if i == d.from { d.y - d.grab - top } else { i as f32 * STEP + old.get(i).copied().unwrap_or(0.) };
                            if let Some(x) = s.get_mut(k) { *x = visual - k as f32 * STEP; }
                        }
                    });
                    if t != d.from { let _ = ws.update(cx, |w, cx| change(w, cx, |p| p.move_entry(d.from, t))); }
                }
                _ => { DRAG.with(|d| d.borrow_mut().take()); }
            }
            let _ = ws.update(cx, |_, cx| cx.notify());
        });
        // 拖着的时候整页不跟着手指滚。
        window.on_mouse_event(|ev: &ScrollEvent, phase, _, cx| {
            if phase == DispatchPhase::Capture && ev.source == ScrollSource::Touch && DRAG.with(|d| d.borrow().is_some()) { cx.stop_propagation(); }
        });
    }).absolute().size_0();
    div().w_full().pt(px(12.)).pb(px(24.)).flex().flex_col().gap(px(16.))
        .child(tools)
        .when(found, |d| d.child(list))
        .when(!found, |d| d.child(general::card(vec![div().px(px(18.)).py(px(16.)).text_center().text_size(px(15.)).text_color(Theme::text_caption())
            .child(tr("没有找到条目", "No prompts found")).into_any_element()])))
        .when(!ro && !sorting && query.is_empty(), |d| d.child(general::card(vec![add_row("tavern-add-entry", tr("添加条目", "Add Prompt"), add_entry, cx)])))
        .child(watch)
        .into_any_element()
}

/// 条目一行。分组小标题用灰色的粗体字、不带标签。标记（应用填内容的位置）点不开。
#[allow(clippy::too_many_arguments)]
fn entry_row(i: usize, on: bool, pr: Option<&Prompt>, name: &str, sorting: bool, ro: bool, lifted: bool, cx: &mut Context<Workspace>) -> Stateful<Div> {
    let head = heading(name);
    let id = pr.map(|p| p.id.clone());
    let tappable = !sorting && !lifted && pr.is_some_and(|p| !p.marker);
    let title = match head {
        Some(h) => div().flex_1().min_w_0().truncate().text_size(px(14.)).font_weight(FontWeight::SEMIBOLD).text_color(Theme::text_caption()).child(h.to_owned()),
        None => div().flex_1().min_w_0().truncate().text_size(px(17.)).text_color(row_ink()).child(name.to_owned()),
    };
    let chip = pr.filter(|_| head.is_none()).map(|p| div().flex_none().px(px(7.)).h(px(20.)).rounded_full().flex().items_center().bg(theme::ca(craft::N1000, 0.06))
        .text_size(px(12.)).font_weight(FontWeight::MEDIUM).text_color(Theme::text_caption()).child(tag(p)));
    let control = if sorting { svg().path("editor-icons/grip-vertical.svg").size(px(18.)).flex_none().text_color(row_chevron()).into_any_element() } else {
        let entry = id.clone().unwrap_or_default();
        general::toggle(("tavern-entry-on", i), on).when(ro, |d| d.opacity(0.5)).on_click(cx.listener(move |w, _, _, cx| {
            cx.stop_propagation();
            if ro { return read_only(w, cx); }
            haptic();
            let entry = entry.clone();
            change(w, cx, |p| p.set_enabled(&entry, !on));
        })).into_any_element()
    };
    div().id(("tavern-entry", i)).w_full().h(px(ROW)).px(px(18.)).flex().items_center().gap(px(10.))
        .when(tappable, |d| d.active(|d| d.bg(Theme::fill_pressed())))
        .child(title).children(chip).child(control)
        .when(tappable, |d| d.on_click(cx.listener(move |w, _, window, cx| {
            if DRAG.with(|d| d.borrow().is_some()) || DROPPED.with(Cell::get).is_some_and(|t| t.elapsed().as_millis() < 600) { return; }
            window.blur(cx);
            haptic();
            if let Some(id) = id.clone() { open_entry(w, id, cx); }
        })))
}

fn open_entry(w: &mut Workspace, id: String, cx: &mut Context<Workspace>) {
    LIST_AT.with(|l| l.set(Some(w.settings_scroll.offset().y)));
    ENTRY.with(|e| *e.borrow_mut() = id);
    w.open_settings(Page::AssistantEntry, cx);
}

fn add_entry(w: &mut Workspace, cx: &mut Context<Workspace>) {
    if !super::prompt(tr("添加条目", "Add Prompt"), None, tr("名称", "Name"), "", Field::Text, None, |value| {
        let Some(name) = value.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty()) else { return };
        menu::dispatch(move |w, _, cx| {
            let id = uuid::Uuid::new_v4().to_string();
            change(w, cx, |p| p.add_prompt(&id, &name, Role::System, ""));
            open_entry(w, id, cx);
        });
    }) { toast(w, tr("无法打开名称设置", "Could not open name settings"), cx); }
}

fn entry_id() -> String { ENTRY.with(|e| e.borrow().clone()) }

/// 一条条目：名称和开关；角色、位置（按深度插入时还有深度）；内容；自己加的可以删。标记只有名称和开关。
fn entry(cx: &mut Context<Workspace>) -> AnyElement {
    let Some(Open { id: pid, preset: p }) = open() else { return div().into_any_element() };
    let eid = entry_id();
    let Some(e) = p.prompt(&eid) else { return div().into_any_element() };
    let ro = pid.is_empty();
    let on = p.order().iter().find(|x| x.id == eid).is_some_and(|x| x.enabled);
    let switch = general::toggle("tavern-entry-enabled", on).when(ro, |d| d.opacity(0.5)).on_click(cx.listener(move |w, _, _, cx| {
        if ro { return read_only(w, cx); }
        haptic();
        let id = entry_id();
        change(w, cx, |p| p.set_enabled(&id, !on));
    }));
    let mut body = div().w_full().pt(px(18.)).pb(px(24.)).flex().flex_col().gap(px(28.))
        .child(general::card(vec![
            line("tavern-entry-name", tr("名称", "Name"), entry_name(&e), !ro).when(!ro, |d| d.on_click(cx.listener(|w, _, _, cx| rename_entry(w, cx)))).into_any_element(),
            switch_line(tr("启用", "Enabled"), switch),
        ]));
    if !e.marker {
        let role = role_label(e.role);
        let depth = match e.position { Position::Depth { depth, .. } => Some(depth), _ => None };
        let place = match e.position { Position::Relative => tr("按顺序", "In Order"), Position::Depth { .. } => tr("按深度插入", "At Depth"), Position::Attach => tr("附加到消息", "Attached") };
        let mut rows = if ro { vec![line("tavern-entry-role", tr("角色", "Role"), role, false).into_any_element(), line("tavern-entry-pos", tr("位置", "Position"), place, false).into_any_element()] } else { vec![
            general::choice("tavern-entry-role-menu", tr("角色", "Role"), role, ROLE_ITEMS.iter().map(|&(id, r)| menu::Item::new(id, role_label(Some(r)), "", 0).checked(e.role.unwrap_or(Role::System) == r)).collect()),
            general::choice("tavern-entry-pos-menu", tr("位置", "Position"), place, vec![
                menu::Item::new(POS_RELATIVE, tr("按顺序", "In Order"), "", 0).checked(e.position == Position::Relative),
                menu::Item::new(POS_DEPTH, tr("按深度插入", "At Depth"), "", 0).checked(depth.is_some()),
            ]),
        ] };
        if let Some(depth) = depth {
            rows.push(line("tavern-entry-depth", tr("深度", "Depth"), depth.to_string(), !ro).when(!ro, |d| d.on_click(cx.listener(move |w, _, _, cx| edit_depth(w, depth, cx)))).into_any_element());
        }
        let content = e.content.clone();
        body = body.child(general::card(rows))
            .child(div().w_full().flex().flex_col().child(header(tr("内容", "Content")))
                .child(text_block("tavern-entry-content", &e.content, false, false, !ro).when(!ro, |d| d.on_click(cx.listener(move |w, _, window, cx| {
                    window.blur(cx);
                    haptic();
                    let id = entry_id();
                    if !sheet::edit(tr("内容", "Content"), &content, false, move |text| {
                        let id = id.clone();
                        menu::dispatch(move |w, _, cx| change(w, cx, |p| { p.set_prompt_field(&id, "content", json!(text)); }));
                    }) { toast(w, tr("无法打开编辑", "Could not open the editor"), cx); }
                })))));
        if !ro && !e.system_prompt { body = body.child(delete_card("tavern-entry-delete-menu", DELETE_ENTRY, tr("删除条目", "Delete Prompt"))); }
    }
    body.into_any_element()
}

fn rename_entry(w: &mut Workspace, cx: &mut Context<Workspace>) {
    haptic();
    let Some(e) = open().and_then(|o| o.preset.prompt(&entry_id())) else { return };
    let id = e.id.clone();
    if !super::prompt(tr("条目名称", "Prompt Name"), None, tr("名称", "Name"), &e.name, Field::Text, None, move |value| {
        let Some(name) = value.filter(|v| !v.trim().is_empty()) else { return };
        let id = id.clone();
        menu::dispatch(move |w, _, cx| change(w, cx, |p| { p.set_prompt_field(&id, "name", json!(name)); }));
    }) { toast(w, tr("无法打开名称设置", "Could not open name settings"), cx); }
}

fn edit_depth(w: &mut Workspace, depth: usize, cx: &mut Context<Workspace>) {
    haptic();
    let id = entry_id();
    let body = tr("插在倒数第几条消息之前：0 是最后。", "How many messages from the end: 0 is the very end.");
    if !super::prompt(tr("深度", "Depth"), Some(body), "4", &depth.to_string(), Field::Url, None, move |value| {
        let id = id.clone();
        match value.and_then(|v| v.trim().parse::<u32>().ok()) {
            Some(d) => menu::dispatch(move |w, _, cx| change(w, cx, |p| { p.set_prompt_field(&id, "injection_depth", json!(d)); })),
            None => menu::dispatch(|w, _, cx| toast(w, tr("请输入一个整数", "Enter a whole number"), cx)),
        }
    }) { toast(w, tr("无法打开深度设置", "Could not open depth settings"), cx); }
}

// ---- 正则（第三级）

/// 正则：每条一行，名字、下面一行灰字写作用于哪里，右边开关；最后可以添加。
fn regexes(cx: &mut Context<Workspace>) -> AnyElement {
    let Some(Open { id: pid, preset: p }) = open() else { return div().into_any_element() };
    let ro = pid.is_empty();
    let mut rows: Vec<AnyElement> = p.regex_scripts().into_iter().enumerate().map(|(i, s)| {
        let on = !s.disabled;
        let mut facts: Vec<&str> = Vec::new();
        if s.placement.contains(&AI_OUTPUT) { facts.push(tr("模型的输出", "Model output")); }
        if s.placement.contains(&USER_INPUT) { facts.push(tr("我的要求", "My request")); }
        if s.markdown_only { facts.push(tr("只改显示", "Display only")); }
        if s.prompt_only { facts.push(tr("只改发出去的", "Prompt only")); }
        let name = if s.name.trim().is_empty() { tr("未命名", "Untitled").to_owned() } else { s.name.clone() };
        div().id(("tavern-regex", i)).w_full().min_h(px(58.)).px(px(18.)).py(px(9.)).flex().items_center().gap(px(12.)).active(|d| d.bg(Theme::fill_pressed()))
            .child(div().flex_1().min_w_0().flex().flex_col().gap(px(1.))
                .child(div().truncate().text_size(px(17.)).line_height(px(22.)).text_color(row_ink()).child(name))
                .when(!facts.is_empty(), |d| d.child(div().truncate().text_size(px(13.)).line_height(px(18.)).text_color(Theme::text_caption()).child(facts.join(" · ")))))
            .child(general::toggle(("tavern-regex-on", i), on).when(ro, |d| d.opacity(0.5)).on_click(cx.listener(move |w, _, _, cx| {
                cx.stop_propagation();
                if ro { return read_only(w, cx); }
                haptic();
                change(w, cx, |p| { p.set_regex_field(i, "disabled", json!(on)); });
            })))
            .on_click(cx.listener(move |w, _, _, cx| { haptic(); REGEX.with(|r| r.set(i)); w.open_settings(Page::AssistantRegex, cx); }))
            .into_any_element()
    }).collect();
    if !ro {
        rows.push(add_row("tavern-add-regex", tr("添加正则", "Add Regex"), |w, cx| {
            let mut at = None;
            change(w, cx, |p| at = Some(p.add_regex(&uuid::Uuid::new_v4().to_string(), tr("新正则", "New Regex"))));
            if let Some(i) = at { REGEX.with(|r| r.set(i)); w.open_settings(Page::AssistantRegex, cx); }
        }, cx));
    }
    div().w_full().pt(px(18.)).pb(px(24.)).child(general::card(rows)).into_any_element()
}

/// 一条正则：名称和开关；查找（编译不了时描红，下面一行红字写原因）；替换为；作用于哪里。
fn regex(cx: &mut Context<Workspace>) -> AnyElement {
    let Some(Open { id: pid, preset: p }) = open() else { return div().into_any_element() };
    let i = REGEX.with(Cell::get);
    let Some(s) = p.regex_scripts().into_iter().nth(i) else { return div().into_any_element() };
    let ro = pid.is_empty();
    let error = if s.find.is_empty() { None } else { s.compile(&mut Macros::new(Env::default())).err().map(|e| crate::localized_format!("无法编译：{}", "Can’t compile: {}", e)) };
    let field = move |w: &mut Workspace, key: &'static str, title: &'static str, value: String, cx: &mut Context<Workspace>| {
        haptic();
        if !sheet::edit(title, &value, true, move |text| menu::dispatch(move |w, _, cx| change(w, cx, |p| { p.set_regex_field(i, key, json!(text)); }))) {
            toast(w, tr("无法打开编辑", "Could not open the editor"), cx);
        }
    };
    let flag = move |id: &'static str, key: &'static str, on: bool, cx: &mut Context<Workspace>| general::toggle(id, on).when(ro, |d| d.opacity(0.5))
        .on_click(cx.listener(move |w, _, _, cx| { if ro { return read_only(w, cx); } haptic(); change(w, cx, |p| { p.set_regex_field(i, key, json!(!on)); }); }));
    let placement = s.placement.clone();
    let place = move |id: &'static str, title: &'static str, which: i64, cx: &mut Context<Workspace>| {
        let list = placement.clone();
        let on = list.contains(&which);
        check_line(id, title, on).on_click(cx.listener(move |w, _, _, cx| {
            if ro { return read_only(w, cx); }
            haptic();
            let mut list = list.clone();
            if on { list.retain(|x| *x != which); } else { list.push(which); list.sort(); }
            change(w, cx, |p| { p.set_regex_field(i, "placement", json!(list)); });
        })).into_any_element()
    };
    let (find, replace, disabled) = (s.find.clone(), s.replace.clone(), s.disabled);
    let name = if s.name.trim().is_empty() { tr("未命名", "Untitled").to_owned() } else { s.name.clone() };
    let scope = vec![
        place("tavern-regex-output", tr("模型的输出", "Model Output"), AI_OUTPUT, cx),
        place("tavern-regex-input", tr("我的要求", "My Request"), USER_INPUT, cx),
        switch_line(tr("只改显示", "Display Only"), flag("tavern-regex-display", "markdownOnly", s.markdown_only, cx)),
        switch_line(tr("只改发出去的", "Prompt Only"), flag("tavern-regex-prompt", "promptOnly", s.prompt_only, cx)),
    ];
    div().w_full().pt(px(18.)).pb(px(24.)).flex().flex_col().gap(px(28.))
        .child(general::card(vec![
            line("tavern-regex-name", tr("名称", "Name"), name, !ro).when(!ro, |d| d.on_click(cx.listener(|w, _, _, cx| rename_regex(w, cx)))).into_any_element(),
            // 存的是「停用」，开关是「启用」：反过来。
            switch_line(tr("启用", "Enabled"), general::toggle("tavern-regex-enabled", !s.disabled).when(ro, |d| d.opacity(0.5)).on_click(cx.listener(move |w, _, _, cx| {
                if ro { return read_only(w, cx); }
                haptic();
                let off = !disabled;
                change(w, cx, |p| { p.set_regex_field(i, "disabled", json!(off)); });
            }))),
        ]))
        .child(div().w_full().flex().flex_col().child(header(tr("查找", "Find")))
            .child(text_block("tavern-regex-find", &s.find, true, error.is_some(), !ro).when(!ro, |d| d.on_click(cx.listener(move |w, _, _, cx| field(w, "findRegex", tr("查找", "Find"), find.clone(), cx)))))
            .children(error.map(|e| div().px(px(28.)).pt(px(8.)).text_size(px(13.)).line_height(px(18.)).text_color(Theme::danger()).child(e))))
        .child(div().w_full().flex().flex_col().child(header(tr("替换为", "Replace With")))
            .child(text_block("tavern-regex-replace", &s.replace, true, false, !ro).when(!ro, |d| d.on_click(cx.listener(move |w, _, _, cx| field(w, "replaceString", tr("替换为", "Replace With"), replace.clone(), cx))))))
        .child(general::titled(Some(tr("作用于", "Applies To")), scope))
        .when(!ro, |d| d.child(delete_card("tavern-regex-delete-menu", DELETE_REGEX, tr("删除正则", "Delete Regex"))))
        .into_any_element()
}

fn rename_regex(w: &mut Workspace, cx: &mut Context<Workspace>) {
    haptic();
    let i = REGEX.with(Cell::get);
    let Some(s) = open().and_then(|o| o.preset.regex_scripts().into_iter().nth(i)) else { return };
    if !super::prompt(tr("正则名称", "Regex Name"), None, tr("名称", "Name"), &s.name, Field::Text, None, move |value| {
        let Some(name) = value.filter(|v| !v.trim().is_empty()) else { return };
        menu::dispatch(move |w, _, cx| change(w, cx, |p| { p.set_regex_field(i, "scriptName", json!(name)); }));
    }) { toast(w, tr("无法打开名称设置", "Could not open name settings"), cx); }
}

// ---- 助手和人设（第二级）

fn card_of() -> Option<Card> {
    let (persona, id) = CARD.with(|c| c.borrow().clone());
    let s = settings();
    (if persona { s.personas } else { s.chars }).into_iter().find(|c| c.id == id)
}

fn open_card(w: &mut Workspace, persona: bool, id: String, cx: &mut Context<Workspace>) {
    CARD.with(|c| *c.borrow_mut() = (persona, id));
    w.open_settings(Page::AssistantCard, cx);
}

fn add_card(w: &mut Workspace, persona: bool, cx: &mut Context<Workspace>) {
    let title = if persona { tr("添加人设", "Add Persona") } else { tr("添加助手", "Add Assistant") };
    if !super::prompt(title, None, tr("名称", "Name"), "", Field::Text, None, move |value| {
        let Some(name) = value.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty()) else { return };
        menu::dispatch(move |w, _, cx| {
            let card = Card { id: uuid::Uuid::new_v4().to_string(), name, description: String::new() };
            let id = card.id.clone();
            change_settings(|s| if persona { s.personas.push(card) } else { s.chars.push(card) });
            open_card(w, persona, id, cx);
        });
    }) { toast(w, tr("无法打开名称设置", "Could not open name settings"), cx); }
}

/// 改正在看的助手或人设。
fn change_card(f: impl FnOnce(&mut Card)) {
    let (persona, id) = CARD.with(|c| c.borrow().clone());
    change_settings(|s| if let Some(c) = (if persona { &mut s.personas } else { &mut s.chars }).iter_mut().find(|c| c.id == id) { f(c); });
}

/// 助手或人设：没在用时最上面是「使用」；名称；描述（助手的描述填进 `{{description}}`，人设的填进 `{{persona}}`）；不止一个时可以删。
fn card(cx: &mut Context<Workspace>) -> AnyElement {
    let (persona, id) = CARD.with(|c| c.borrow().clone());
    let s = settings();
    let list = if persona { &s.personas } else { &s.chars };
    let Some(c) = list.iter().find(|c| c.id == id).cloned() else { return div().into_any_element() };
    let current = if persona { s.persona == id } else { s.char == id };
    let use_label = if persona { tr("使用这个人设", "Use This Persona") } else { tr("使用这个助手", "Use This Assistant") };
    let description = c.description.clone();
    div().w_full().pt(px(18.)).pb(px(24.)).flex().flex_col().gap(px(28.))
        .child(general::card(vec![if current { in_use() } else { general::action("tavern-card-use", use_label, false, move |_, cx| {
            let id = id.clone();
            change_settings(|s| if persona { s.persona = id } else { s.char = id });
            cx.notify();
        }, cx) }]))
        .child(general::card(vec![line("tavern-card-name", tr("名称", "Name"), c.name.clone(), true).on_click(cx.listener(|w, _, _, cx| rename_card(w, cx))).into_any_element()]))
        .child(div().w_full().flex().flex_col().child(header(tr("描述", "Description")))
            .child(text_block("tavern-card-description", &c.description, false, false, true).on_click(cx.listener(move |w, _, window, cx| {
                window.blur(cx);
                haptic();
                if !sheet::edit(tr("描述", "Description"), &description, false, |text| {
                    menu::dispatch(move |_, _, cx| { change_card(|c| c.description = text); cx.notify(); });
                }) { toast(w, tr("无法打开编辑", "Could not open the editor"), cx); }
            }))))
        .when(list.len() > 1, |d| d.child(delete_card("tavern-card-delete-menu", DELETE_CARD, if persona { tr("删除人设", "Delete Persona") } else { tr("删除助手", "Delete Assistant") })))
        .into_any_element()
}

fn rename_card(w: &mut Workspace, cx: &mut Context<Workspace>) {
    haptic();
    let Some(c) = card_of() else { return };
    if !super::prompt(tr("名称", "Name"), None, tr("名称", "Name"), &c.name, Field::Text, None, |value| {
        let Some(name) = value.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty()) else { return };
        menu::dispatch(move |_, _, cx| { change_card(|c| c.name = name); cx.notify(); });
    }) { toast(w, tr("无法打开名称设置", "Could not open name settings"), cx); }
}

// ---- 菜单

pub fn menu_action(action: &str, w: &mut Workspace, cx: &mut Context<Workspace>) -> bool {
    if let Some(&(_, role)) = ROLE_ITEMS.iter().find(|r| r.0 == action) {
        let id = entry_id();
        change(w, cx, |p| { p.set_prompt_field(&id, "role", json!(role.as_str())); });
        return true;
    }
    match action {
        POS_RELATIVE | POS_DEPTH => {
            let id = entry_id();
            change(w, cx, |p| { p.set_prompt_field(&id, "injection_position", json!(if action == POS_DEPTH { 1 } else { 0 })); });
        }
        DELETE_PRESET => delete_preset(w, cx),
        DELETE_ENTRY => {
            let id = entry_id();
            change(w, cx, |p| { p.remove_prompt(&id); });
            super::super::settings_back(w, cx);
        }
        DELETE_REGEX => {
            let i = REGEX.with(Cell::get);
            change(w, cx, |p| { p.remove_regex(i); });
            super::super::settings_back(w, cx);
        }
        DELETE_CARD => {
            let (persona, id) = CARD.with(|c| c.borrow().clone());
            change_settings(|s| {
                let list = if persona { &mut s.personas } else { &mut s.chars };
                if list.len() > 1 { list.retain(|c| c.id != id); }
                let first = list[0].id.clone();
                let current = if persona { &mut s.persona } else { &mut s.char };
                if *current == id { *current = first; }
            });
            super::super::settings_back(w, cx);
        }
        _ => return false,
    }
    true
}
