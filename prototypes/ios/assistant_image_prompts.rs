//! 设置 › 助手设置 › 图像 里的画风和人物（`fdl_ai::imagine`）。
//! 画风：一排横滑的小图（试画过静物的是样张，没画过的写一句话），名字在下面，点一下就是以后生图用的画风；再点选中的那张（或长按选「编辑」）打开它。
//! 内置的几种只能看，可以复制一份再改。人物：一排横滑的圆头像（和语音角色一样），没换头像的是首字；最前面是「添加」；
//! 点开是大头像（右下角换头像）、名称、别名、外貌标签和只压这个人的排除标签（胶囊标签流预览，点开在系统文字页里批量改）。
//! 质量词（「生成」那组里的两行）也在这里：和画风分开，每张图都带，带单行预览与「恢复默认」动作。
//! 都存在空间的 `tavern/imagine.json`，跟着 iCloud 走；每改一处整份写回去。
use std::cell::RefCell;

use fdl_ai::imagine::{self, BUILTIN, Character, Library, Style};
use gpui::{prelude::FluentBuilder, *};

use crate::{app::Workspace, settings::{Page, prefs, row_chevron, row_ink, tr}, theme::{self, Theme, craft}, ui};
use super::{Field, general, menu, sheet, writing::{delete_card, in_use, intern, line}};

const STYLE_EDIT: &str = "image-style-edit";
const STYLE_COPY: &str = "image-style-copy";
const STYLE_DELETE: &str = "image-style-delete";
const DELETE_STYLE: &str = "image-delete-style";
const DELETE_PERSON: &str = "image-delete-person";
/// 画风卡片上下文菜单的键：`image-style:<标识>`。
const KEY: &str = "image-style:";
/// 外貌空着时的示例：写英文标签、逗号分开、以 girl / boy 开头，一眼就明白。
const LOOK_EXAMPLE: &str = "girl, long black hair, red eyes, serafuku";
/// 人物排除空着时的示例。
const NEGATIVE_EXAMPLE: &str = "long hair, glasses, hat";
/// 胶囊标签流最多平铺展示几颗，多出的折成 `+N`。
const MAX_CHIPS: usize = 16;

thread_local! {
    static LIB: RefCell<Option<Library>> = const { RefCell::new(None) };
    /// 正在看的画风、人物的标识。
    static STYLE: RefCell<String> = const { RefCell::new(String::new()) };
    static PERSON: RefCell<String> = const { RefCell::new(String::new()) };
}

fn dir() -> std::path::PathBuf { super::super::assist::dir() }
fn english() -> bool { prefs().english }
pub(crate) fn lib() -> Library { LIB.with(|l| l.borrow_mut().get_or_insert_with(|| Library::load(&dir())).clone()) }
fn change(f: impl FnOnce(&mut Library)) {
    LIB.with(|l| {
        let mut l = l.borrow_mut();
        let lib = l.get_or_insert_with(|| Library::load(&dir()));
        f(lib);
        if let Err(e) = lib.save(&dir()) { log::warn!("Could not save image prompts: {e}"); }
    });
}
/// 配图卡片上「优化描述」开不开（记在 `imagine.json`）。
pub(crate) fn set_optimize(on: bool) { change(|l| l.optimize = on); }
fn new_id() -> String { uuid::Uuid::new_v4().simple().to_string()[..12].to_owned() }
fn haptic() { super::super::glass::haptic(); }
fn toast(w: &mut Workspace, text: &str, cx: &mut Context<Workspace>) { w.show_toast_kind(ui::ToastKind::Error, text, cx); }
fn style_of(id: &str) -> Option<Style> { lib().find_style(id, english()) }
fn open_style() -> Option<Style> { style_of(&STYLE.with(|s| s.borrow().clone())) }
fn open_person() -> Option<Character> { let id = PERSON.with(|p| p.borrow().clone()); lib().characters.into_iter().find(|c| c.id == id) }

/// 内置画风卡片上的一句话。
fn blurb(id: &str) -> Option<&'static str> {
    Some(match id {
        "" => tr("不加画风标签", "No style tags"),
        "builtin-anime" => tr("赛璐璐平涂，动画截图感", "Cel shading, like an anime still"),
        "builtin-painterly" => tr("笔触厚重，光影细腻", "Heavy strokes, rich light"),
        "builtin-watercolor" => tr("低饱和，留白", "Muted colours, open space"),
        "builtin-lineart" => tr("单色线稿与网点", "Monochrome lines and screentone"),
        "builtin-picturebook" => tr("彩铅与柔和的颜色", "Coloured pencil, soft palette"),
        _ => return None,
    })
}

/// 进图像的几页都重读一次（另一台设备可能改过）。
pub fn opened() { LIB.with(|l| *l.borrow_mut() = Some(Library::load(&dir()))); }

pub fn title(page: Page) -> &'static str {
    match page {
        Page::AssistantStyle => open_style().map_or("", |s| intern(&s.name)),
        Page::AssistantPerson => open_person().map_or("", |c| intern(&c.name)),
        _ => "",
    }
}

// ---- 图像页上的两组

/// 画风：「默认」（不加画风）、内置的几种、自己加的，最后一张「新建」。一张是一块 3:2 的小图，名字在下面：
/// 用这个画风试画过静物的是那张图的样张（`image::style_thumb`），没画过的写一句话或它的标签。选中的描一圈、右上角打勾。
pub fn styles(cx: &mut Context<Workspace>) -> AnyElement {
    const W: f32 = 150.;
    const H: f32 = 100.;
    const R: f32 = 18.;
    const GAP: f32 = 10.;
    let l = lib();
    let current = l.current_style(english()).map_or(String::new(), |s| s.id);
    let mut all = vec![Style { name: tr("默认", "Default").into(), ..Default::default() }];
    all.extend(imagine::builtin(english()).into_iter().chain(l.styles));
    let name_line = |name: SharedString, ink: Hsla| div().w(px(W)).px(px(4.)).truncate().text_size(px(14.)).line_height(px(19.)).font_weight(FontWeight::MEDIUM).text_color(ink).child(name);
    let mut cards: Vec<AnyElement> = all.into_iter().map(|Style { id, name, positive, negative }| {
        let on = id == current;
        let thumb = super::image::style_thumb(&id, &positive, &negative);
        let sub: SharedString = blurb(&id).map_or_else(|| {
            if positive.trim().is_empty() { tr("未填写标签", "No tags yet").into() } else { positive.into() }
        }, SharedString::from);
        let builtin = id.is_empty() || id.starts_with(BUILTIN);
        let items = match id.as_str() {
            "" => vec![],
            _ if builtin => vec![menu::Item::new(STYLE_EDIT, tr("查看", "View"), "eye", 0), menu::Item::new(STYLE_COPY, tr("复制一份", "Duplicate"), "plus.square.on.square", 0)],
            _ => vec![menu::Item::new(STYLE_EDIT, tr("编辑", "Edit"), "pencil", 0), menu::Item::new(STYLE_COPY, tr("复制一份", "Duplicate"), "plus.square.on.square", 0),
                menu::Item::new(STYLE_DELETE, tr("删除", "Delete"), "trash", 1).danger()],
        };
        let open = id.clone();
        let tile = div().relative().w(px(W)).h(px(H)).rounded(px(R)).bg(general::card_bg())
            .map(|d| match thumb {
                Some(path) => d.child(img(path).absolute().inset_0().size_full().rounded(px(R)).object_fit(ObjectFit::Cover)),
                None => d.flex().flex_col().justify_end().p(px(12.))
                    .child(div().text_size(px(13.)).line_height(px(18.)).line_clamp(3).text_color(Theme::text_caption()).child(sub)),
            })
            .when(on, |d| d.child(div().absolute().inset_0().rounded(px(R)).border_2().border_color(Theme::text()))
                .child(div().absolute().top(px(8.)).right(px(8.)).size(px(22.)).rounded_full().bg(Theme::text()).flex().items_center().justify_center()
                    .child(ui::icon("check", 13., Theme::bg_body()))))
            .when(!items.is_empty(), |d| d.child(menu::context(SharedString::from(format!("{KEY}{id}")), "", R, items)));
        div().id(SharedString::from(format!("image-style-{id}"))).w(px(W)).flex_none().flex().flex_col().gap(px(8.)).active(|d| d.opacity(0.7))
            .child(tile).child(name_line(name.into(), Theme::text()))
            .on_click(cx.listener(move |w, _, _, cx| {
                haptic();
                if !on { let id = open.clone(); change(|l| l.style = id); cx.notify(); }
                else if !open.is_empty() { show_style(w, open.clone(), cx); }
            }))
            .into_any_element()
    }).collect();
    cards.push(div().id("image-style-new").w(px(W)).flex_none().flex().flex_col().gap(px(8.)).active(|d| d.opacity(0.7))
        .child(div().w(px(W)).h(px(H)).rounded(px(R)).bg(general::card_bg()).flex().items_center().justify_center().child(ui::icon("plus", 22., Theme::text_caption())))
        .child(name_line(tr("新建画风", "New Style").into(), Theme::text_caption()))
        .on_click(cx.listener(|w, _, _, cx| { haptic(); new_style(w, cx) })).into_any_element());
    let width = 20. + cards.len() as f32 * W + cards.len().saturating_sub(1) as f32 * GAP;
    div().w_full().flex().flex_col()
        .child(super::roles::header(tr("画风", "Style")))
        .child(div().id("image-styles").w_full().overflow_x_scroll().restrict_scroll_to_axis()
            .child(div().w(px(width)).flex_none().px(px(10.)).flex().gap(px(GAP)).children(cards)))
        .into_any_element()
}

/// 人物头像：换过的是自带头像或照片；没换（或照片不在这台设备上）是名字的第一个字。
fn person_avatar(c: &Character, size: f32) -> AnyElement {
    super::roles::avatar_image(c.avatar.as_deref(), size).unwrap_or_else(|| {
        let first: String = c.name.trim().chars().next().map_or_else(String::new, |ch| ch.to_uppercase().collect());
        div().size(px(size)).flex_none().rounded_full().bg(general::card_bg()).border_1().border_color(theme::ca(craft::N1000, 0.06))
            .flex().items_center().justify_center()
            .child(div().text_size(px(size * 0.4)).line_height(px(size * 0.5)).font_weight(FontWeight::MEDIUM).text_color(Theme::text_caption()).child(first))
            .into_any_element()
    })
}

/// 人物：一排横滑的圆头像，名字在下面（照语音角色那一排的尺寸）；最前面是「添加」。
pub fn people(cx: &mut Context<Workspace>) -> AnyElement {
    const CELL: f32 = 76.;
    const GAP: f32 = 12.;
    const PAD: f32 = 16.;
    let caption = |text: SharedString| div().w(px(CELL)).text_center().text_size(px(13.)).line_height(px(17.)).line_clamp(2).text_color(Theme::text()).child(text);
    let add = div().id("image-person-add").w(px(CELL)).flex_none().flex().flex_col().items_center().gap(px(8.))
        .child(div().size(px(64.)).rounded_full().bg(general::card_bg()).border_1().border_color(theme::ca(craft::N1000, 0.06)).flex().items_center().justify_center()
            .child(ui::icon("plus", 24., Theme::text())))
        .child(caption(tr("添加", "Add").into()))
        .on_click(cx.listener(|w, _, _, cx| { haptic(); add_person(w, cx) }))
        .into_any_element();
    let people = lib().characters;
    let cells = people.iter().map(|c| {
        let id = c.id.clone();
        div().id(SharedString::from(format!("image-person-{}", c.id))).w(px(CELL)).flex_none().flex().flex_col().items_center().gap(px(8.)).active(|d| d.opacity(0.7))
            .child(person_avatar(c, 64.))
            .child(caption(c.name.clone().into()))
            .on_click(cx.listener(move |w, _, _, cx| { haptic(); PERSON.with(|p| *p.borrow_mut() = id.clone()); w.open_settings(Page::AssistantPerson, cx); }))
            .into_any_element()
    });
    let width = 2. * PAD + (people.len() + 1) as f32 * (CELL + GAP) - GAP;
    div().w_full().flex().flex_col()
        .child(super::roles::header(tr("人物", "People")))
        .child(div().id("image-people").w_full().overflow_x_scroll().restrict_scroll_to_axis()
            .child(div().w(px(width)).flex_none().px(px(PAD)).flex().items_start().gap(px(GAP)).child(add).children(cells)))
        .into_any_element()
}

// ---- 画风

fn show_style(w: &mut Workspace, id: String, cx: &mut Context<Workspace>) {
    STYLE.with(|s| *s.borrow_mut() = id);
    w.open_settings(Page::AssistantStyle, cx);
}

/// 起名字：系统的对话框，确定后交给 `done`（空的不算）。
fn name_prompt(w: &mut Workspace, title: &str, value: &str, done: impl Fn(String, &mut Workspace, &mut Context<Workspace>) + 'static, cx: &mut Context<Workspace>) {
    let done = std::rc::Rc::new(done);
    if !super::prompt(title, None, tr("名称", "Name"), value, Field::Text, None, move |v| {
        let Some(name) = v.map(|v| v.trim().to_owned()).filter(|v| !v.is_empty()) else { return };
        let done = done.clone();
        menu::dispatch(move |w, _, cx| { done(name, w, cx); cx.notify(); });
    }) { toast(w, tr("无法打开名称设置", "Could not open name settings"), cx); }
}

fn new_style(w: &mut Workspace, cx: &mut Context<Workspace>) {
    name_prompt(w, tr("新建画风", "New Style"), "", |name, w, cx| {
        let id = new_id();
        change(|l| { l.styles.push(Style { id: id.clone(), name, ..Default::default() }); l.style = id.clone(); });
        show_style(w, id, cx);
    }, cx);
}

/// 复制一份：名字后面加「副本」，复制完就看着这一份。
fn duplicate(w: &mut Workspace, id: &str, cx: &mut Context<Workspace>) {
    let Some(s) = style_of(id) else { return };
    let copy = Style { id: new_id(), name: crate::localized_format!("{} 副本", "{} Copy", s.name), ..s };
    let nid = copy.id.clone();
    change(|l| l.styles.push(copy));
    if w.settings_page == Page::AssistantStyle { super::super::shot::capture_page(); }
    show_style(w, nid, cx);
}

fn delete_style(id: &str) { change(|l| { l.styles.retain(|s| s.id != id); if l.style == id { l.style.clear(); } }); }

/// 画风：没在用时最上面是「使用这个画风」；名称；标签、排除两段（胶囊标签流，点开在文字页里改）；复制一份、删除。内置的只能看。
pub fn style_page(cx: &mut Context<Workspace>) -> AnyElement {
    let Some(s) = open_style() else { return div().into_any_element() };
    let builtin = s.id.starts_with(BUILTIN);
    let current = lib().style == s.id;
    let id = s.id.clone();
    let field = |key: &'static str, title: &'static str, value: String, cx: &mut Context<Workspace>| {
        let block = tags_block(key, &value, None, !builtin);
        let block = if builtin { block } else {
            block.on_click(cx.listener(move |w, _, window, cx| {
                window.blur(cx);
                haptic();
                let value = value.clone();
                if !sheet::edit(title, &value, true, move |text| menu::dispatch(move |_, _, cx| {
                    let id = STYLE.with(|s| s.borrow().clone());
                    change(|l| if let Some(s) = l.styles.iter_mut().find(|s| s.id == id) { if key == "positive" { s.positive = text; } else { s.negative = text; } });
                    cx.notify();
                })) { toast(w, tr("无法打开编辑", "Could not open the editor"), cx); }
            }))
        };
        div().w_full().flex().flex_col().child(super::roles::header(title)).child(block)
    };
    let mut files = vec![general::action("image-style-copy-row", tr("复制一份", "Duplicate"), false, move |w, cx| duplicate(w, &STYLE.with(|s| s.borrow().clone()), cx), cx)];
    if !builtin { files.push(super::writing::delete_row("image-style-delete-menu", DELETE_STYLE, tr("删除画风", "Delete Style"))); }
    div().w_full().pt(px(18.)).pb(px(24.)).flex().flex_col().gap(px(28.))
        .child(general::card(vec![if current { in_use() } else {
            general::action("image-style-use", tr("使用这个画风", "Use This Style"), false, move |_, cx| { let id = id.clone(); change(|l| l.style = id); cx.notify(); }, cx)
        }]))
        .child(general::card(vec![line("image-style-name", tr("名称", "Name"), s.name.clone(), !builtin)
            .when(!builtin, |d| d.on_click(cx.listener(|w, _, _, cx| {
                haptic();
                let Some(s) = open_style() else { return };
                name_prompt(w, tr("画风名称", "Style Name"), &s.name, move |name, _, _| change(|l| if let Some(x) = l.styles.iter_mut().find(|x| x.id == s.id) { x.name = name.clone(); }), cx);
            }))).into_any_element()]))
        .child(field("positive", tr("标签", "Tags"), s.positive.clone(), cx))
        .child(field("negative", tr("排除", "Exclude"), s.negative.clone(), cx))
        .child(general::titled(None, files))
        .into_any_element()
}

/// 解析权重组 `1.5::a, b::` → `Some(("1.5", "a, b"))`。
fn weight_parts(piece: &str) -> Option<(&str, &str)> {
    let (w, inner) = piece.strip_suffix("::")?.split_once("::")?;
    w.trim().parse::<f32>().ok()?;
    Some((w.trim(), inner.trim()))
}

/// 一颗标签胶囊：普通标签是轻盈的纸白小块；带权重的组带一颗强调色权重角标。
fn tag_chip(piece: &str) -> Div {
    let chip = div().px(px(10.)).py(px(5.)).rounded(px(9.)).flex().items_center().gap(px(5.)).max_w_full();
    if let Some((w, inner)) = weight_parts(piece) {
        chip.bg(Theme::block_hover()).border_1().border_color(Theme::block_selected())
            .child(div().flex_none().font_family(theme::MONO_FONT).text_size(px(11.)).font_weight(FontWeight::SEMIBOLD).text_color(Theme::primary()).child(format!("{w}×")))
            .child(div().min_w_0().text_size(px(13.)).line_height(px(17.)).text_color(row_ink()).child(inner.to_owned()))
    } else {
        chip.bg(Theme::bg_body()).border_1().border_color(Theme::line_divider())
            .child(div().min_w_0().text_size(px(13.)).line_height(px(17.)).text_color(row_ink()).child(piece.to_owned()))
    }
}

/// 一段标签：拆成圆角胶囊流，点开在系统文字页里批量改。空的写灰色的 `example`（没有就写「空」）。
fn tags_block(id: &'static str, text: &str, example: Option<&'static str>, tappable: bool) -> Stateful<Div> {
    let list = imagine::pieces(text);
    let card = div().id(id).mx(px(10.)).px(px(14.)).py(px(14.)).rounded(px(20.)).bg(general::card_bg())
        .when(tappable, |d| d.active(|d| d.bg(Theme::fill_pressed())));
    if list.is_empty() {
        return card.px(px(18.)).child(div().text_size(px(14.)).line_height(px(20.)).font_family(theme::MONO_FONT)
            .text_color(Theme::text_placeholder()).child(example.unwrap_or(tr("空", "Empty")).to_owned()));
    }
    let extra = list.len().saturating_sub(MAX_CHIPS);
    card.child(div().w_full().flex().flex_wrap().gap(px(6.))
        .children(list.into_iter().take(MAX_CHIPS).map(|t| tag_chip(&t)))
        .when(extra > 0, |d| d.child(div().px(px(10.)).py(px(5.)).rounded(px(9.)).bg(Theme::fill_hover())
            .text_size(px(13.)).line_height(px(17.)).font_weight(FontWeight::MEDIUM).text_color(Theme::text_caption()).child(format!("+{extra}")))))
}

// ---- 人物

fn add_person(w: &mut Workspace, cx: &mut Context<Workspace>) {
    name_prompt(w, tr("添加人物", "Add Person"), "", |name, w, cx| {
        let id = new_id();
        change(|l| l.characters.push(Character { id: id.clone(), name, ..Default::default() }));
        PERSON.with(|p| *p.borrow_mut() = id);
        w.open_settings(Page::AssistantPerson, cx);
    }, cx);
}

fn change_person(f: impl FnOnce(&mut Character)) {
    let id = PERSON.with(|p| p.borrow().clone());
    change(|l| if let Some(c) = l.characters.iter_mut().find(|c| c.id == id) { f(c); });
}

fn change_avatar(w: &mut Workspace, cx: &mut Context<Workspace>) {
    let Some(c) = open_person() else { return };
    let id = c.id.clone();
    super::roles::choose_avatar(c.avatar, Page::AssistantPerson, std::rc::Rc::new(move |_, next, cx| set_person_avatar(&id, next, cx)), w, cx);
}
/// 换头像：换下来的照片删掉（自带的不用管）。
fn set_person_avatar(id: &str, next: Option<String>, cx: &mut Context<Workspace>) {
    let mut old = None;
    change(|l| if let Some(c) = l.characters.iter_mut().find(|c| c.id == id) { old = std::mem::replace(&mut c.avatar, next); });
    super::roles::discard_avatar(old.as_deref());
    cx.notify();
}

/// 标签块上面的小标题和胶囊块；点开在系统文字页里改。
fn tags_section(id: &'static str, title: &'static str, text: String, example: &'static str, save: fn(&mut Character, String), cx: &mut Context<Workspace>) -> Div {
    div().w_full().flex().flex_col().child(super::roles::header(title))
        .child(tags_block(id, &text, Some(example), true).on_click(cx.listener(move |w, _, window, cx| {
            window.blur(cx);
            haptic();
            if !sheet::edit(title, &text, true, move |text| menu::dispatch(move |_, _, cx| { change_person(|c| save(c, text)); cx.notify(); })) {
                toast(w, tr("无法打开编辑", "Could not open the editor"), cx);
            }
        })))
}

/// 人物：上面是圆头像（右下角换头像）、名字和别名；名称、别名、换头像；外貌标签和排除标签（空着时是灰色的示例）；删除。
pub fn person_page(cx: &mut Context<Workspace>) -> AnyElement {
    let Some(c) = open_person() else { return div().into_any_element() };
    let aliases = c.aliases.join("、");
    const SIZE: f32 = 120.;
    let glass = super::super::glass::supported();
    let camera = div().id("image-person-avatar-badge").absolute().right(px(-2.)).bottom(px(-2.)).size(px(38.)).rounded_full()
        .map(|d| if glass { d.child(super::super::glass::page_pane("image-person-avatar-glass", None, Some(("sf:camera.fill", 16., Theme::text())), None)) }
            else { d.bg(Theme::bg_body()).shadow(theme::shadow_button()).flex().items_center().justify_center().child(ui::icon("pen-line", 15., Theme::text())) })
        .on_click(cx.listener(move |w, _, _, cx| { if glass { super::super::glass::pulse("image-person-avatar-glass"); } change_avatar(w, cx); }));
    let hero = div().w_full().px(px(24.)).flex().flex_col().items_center()
        // 首字的圆底是半透明的，垫阴影会透出一团灰：只给图片垫。
        .child(div().relative().size(px(SIZE)).child(div().rounded_full().when(super::roles::avatar_image(c.avatar.as_deref(), 1.).is_some(), |d| d.shadow(theme::shadow_card())).child(person_avatar(&c, SIZE))).child(camera))
        .child(div().mt(px(18.)).max_w_full().truncate().text_size(px(28.)).line_height(px(34.)).font_weight(FontWeight::BOLD).text_color(Theme::text()).child(c.name.clone()))
        .when(!aliases.is_empty(), |d| d.child(div().mt(px(4.)).max_w_full().truncate().text_size(px(14.)).line_height(px(19.)).text_color(Theme::text_caption()).child(aliases.clone())));
    let mut rows = vec![
        line("image-person-name", tr("名称", "Name"), c.name.clone(), true).on_click(cx.listener(|w, _, _, cx| {
            haptic();
            let Some(c) = open_person() else { return };
            name_prompt(w, tr("人物名称", "Name"), &c.name, |name, _, _| change_person(|c| c.name = name), cx);
        })).into_any_element(),
        line("image-person-aliases", tr("别名", "Aliases"), aliases.clone(), true).on_click(cx.listener(move |w, _, _, cx| {
            haptic();
            if !super::prompt(tr("别名", "Aliases"), None, tr("用逗号分开", "Separated by commas"), &aliases, Field::Text, None, |v| {
                let Some(v) = v else { return };
                let all: Vec<String> = v.split([',', '，', '、', ';', '；']).map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect();
                let short = all.iter().any(|a| a.chars().count() < 2);
                let kept: Vec<String> = all.into_iter().filter(|a| a.chars().count() >= 2).collect();
                menu::dispatch(move |w, _, cx| {
                    change_person(|c| c.aliases = kept);
                    if short { w.show_toast_kind(ui::ToastKind::Error, tr("别名至少两个字", "Aliases need at least two characters"), cx); }
                    cx.notify();
                });
            }) { toast(w, tr("无法打开别名设置", "Could not open alias settings"), cx); }
        })).into_any_element(),
        general::action("image-person-photo", tr("更换头像", "Change Avatar"), false, change_avatar, cx),
    ];
    if c.avatar.is_some() {
        let id = c.id.clone();
        rows.push(general::action("image-person-reset", tr("恢复默认头像", "Use Default Avatar"), false, move |_, cx| set_person_avatar(&id, None, cx), cx));
    }
    div().w_full().pt(px(12.)).pb(px(24.)).flex().flex_col().gap(px(32.))
        .child(hero)
        .child(general::card(rows))
        .child(tags_section("image-person-look", tr("外貌", "Appearance"), c.look.clone(), LOOK_EXAMPLE, |c, t| c.look = t, cx))
        .child(tags_section("image-person-negative", tr("排除", "Exclude"), c.negative.clone(), NEGATIVE_EXAMPLE, |c, t| c.negative = t, cx))
        .child(delete_card("image-person-delete-menu", DELETE_PERSON, tr("删除人物", "Delete Person")))
        .into_any_element()
}

/// 「生成」里的质量词与质量排除：带单行标签预览，右边状态字是「默认」「已修改」或「无」；有修改时底下出现一键「恢复默认质量词」。
pub fn quality_rows(cx: &mut Context<Workspace>) -> Vec<AnyElement> {
    let l = lib();
    let edited = l.quality.is_some() || l.quality_negative.is_some();
    let mut rows: Vec<AnyElement> = [(false, "image-quality", tr("质量词", "Quality Tags"), l.quality.clone(), l.quality().to_owned()),
        (true, "image-quality-negative", tr("质量排除", "Quality Exclusions"), l.quality_negative.clone(), l.quality_negative().to_owned())]
        .into_iter().map(|(negative, id, title, custom, preview)| {
            let state = match custom.as_deref() { None => tr("默认", "Default"), Some(s) if s.trim().is_empty() => tr("无", "None"), Some(_) => tr("已修改", "Edited") };
            let sub = preview.trim().to_owned();
            div().id(id).w_full().min_h(px(58.)).px(px(18.)).py(px(11.)).flex().items_center().gap(px(10.))
                .active(|d| d.bg(Theme::fill_pressed()))
                .child(div().flex_1().min_w_0().flex().flex_col().gap(px(2.))
                    .child(div().text_size(px(17.)).line_height(px(22.)).text_color(row_ink()).child(title))
                    .when(!sub.is_empty(), |d| d.child(div().truncate().text_size(px(12.5)).line_height(px(16.)).font_family(theme::MONO_FONT).text_color(Theme::text_placeholder()).child(sub))))
                .child(div().flex_none().text_size(px(15.)).text_color(if custom.is_some() { Theme::primary() } else { Theme::text_caption() }).child(state))
                .child(ui::icon("chevron-right", 14., row_chevron()))
                .on_click(cx.listener(move |w, _, window, cx| {
                    window.blur(cx);
                    haptic();
                    let l = lib();
                    let value = if negative { l.quality_negative() } else { l.quality() }.to_owned();
                    if !sheet::edit(title, &value, true, move |text| menu::dispatch(move |_, _, cx| { change(|l| l.set_quality(negative, &text)); cx.notify(); })) {
                        toast(w, tr("无法打开编辑", "Could not open the editor"), cx);
                    }
                })).into_any_element()
        }).collect();
    if edited {
        rows.push(general::action("image-quality-reset", tr("恢复默认质量词", "Reset Quality Tags"), false, |_, cx| {
            change(|l| { l.quality = None; l.quality_negative = None; });
            cx.notify();
        }, cx));
    }
    rows
}

/// 画风卡片的上下文菜单、两页上的删除。
pub fn menu_action(action: &str, w: &mut Workspace, cx: &mut Context<Workspace>) -> bool {
    let target = || menu::context_target().and_then(|t| t.strip_prefix(KEY).map(str::to_owned));
    match action {
        STYLE_EDIT => if let Some(id) = target() { show_style(w, id, cx); },
        STYLE_COPY => if let Some(id) = target() { duplicate(w, &id, cx); },
        STYLE_DELETE => if let Some(id) = target() { super::super::glass::delete_haptic(); delete_style(&id); },
        DELETE_STYLE => { delete_style(&STYLE.with(|s| s.borrow().clone())); super::super::settings_back(w, cx); }
        DELETE_PERSON => {
            let id = PERSON.with(|p| p.borrow().clone());
            let mut photo = None;
            change(|l| l.characters.retain(|c| if c.id == id { photo = c.avatar.clone(); false } else { true }));
            super::roles::discard_avatar(photo.as_deref());
            super::super::settings_back(w, cx);
        }
        _ => return false,
    }
    cx.notify();
    true
}
