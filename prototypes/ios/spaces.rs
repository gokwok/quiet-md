//! 手机上的空间（照 Craft）：首页左上角的空间头像点了去设置；设置第一级顶上是大头像和「空间名 ⌄」，
//! 点名字从底部浮起一张卡片（「打开空间…」搜索 + 每个空间一行，当前的打勾）；「空间设置」这一页是
//! 图标（带相机钮）、名称、危险区里的删除。没有空间时整页是「创建你的第一个空间」。逻辑都在 `crate::space`。
use crate::app::{Confirm, View, Workspace};
use crate::settings::tr;
use crate::space::{self, name, space_icon};
use crate::theme::{self, Theme};
use gpui::{prelude::FluentBuilder, *};

const ROW_H: f32 = 52.;

/// 首页左上角：当前空间的头像，点了去设置（第一级）。
pub fn avatar(w: &Workspace, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    let s = space::current(w)?;
    Some(div().id("hub-space").absolute().left(px(20.)).top(super::insets().top + px((crate::app::HEADER_H - 32.) / 2.)).size(px(32.))
        .child(space_icon(s, 32.))
        .on_click(cx.listener(|w, _, _, cx| { super::set_settings_detail(false); w.show_view(View::Settings, cx); }))
        .into_any_element())
}

/// 设置第一级的页头：大头像，下面「空间名 ⌄」，点名字换空间。
pub fn settings_head(w: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let Some(s) = space::current(w) else { return div().into_any_element() };
    div().w_full().pt(px(8.)).pb(px(20.)).flex().flex_col().items_center().gap(px(12.))
        .child(space_icon(s, 84.))
        .child(div().id("settings-space-name").h(px(32.)).px(px(12.)).flex().items_center().gap(px(6.)).rounded(px(10.)).active(|d| d.bg(Theme::fill_pressed()))
            .text_size(px(18.)).line_height(px(24.)).font_weight(FontWeight::SEMIBOLD)
            .child(div().max_w(px(260.)).truncate().child(name(s)))
            .child(crate::ui::icon("chevron-down", 15., Theme::text_caption()))
            .on_click(cx.listener(|w, _, _, cx| { w.space_menu = true; w.space_query.update(cx, |f, cx| f.set_text("", cx)); cx.notify(); })))
        .into_any_element()
}

fn row(id: impl Into<ElementId>, glyph: &str, label: &'static str) -> Stateful<Div> {
    div().id(id).w_full().h(px(ROW_H)).px(px(16.)).flex().items_center().gap(px(12.)).text_size(px(16.)).active(|s| s.bg(Theme::fill_pressed()))
        .child(crate::ui::icon(glyph, 20., Theme::icon())).child(div().flex_1().child(label))
}

/// 设置第一级「空间设置」一节：这个空间的设置、新建空间。
pub fn settings_rows(cx: &mut Context<Workspace>) -> Vec<AnyElement> {
    vec![
        row("settings-space", "settings-gear", tr("空间设置", "Space Settings")).child(crate::ui::icon("chevron-right", 14., Theme::text_caption()))
            .on_click(cx.listener(|w, _, _, cx| { super::set_settings_detail(true); w.open_settings(crate::settings::Page::Library, cx); })).into_any_element(),
        row("settings-space-new", "plus", tr("新建空间…", "New Space…"))
            .on_click(cx.listener(|w, _, window, cx| w.open_space_form(window, cx))).into_any_element(),
    ]
}

/// 换空间的卡片：从底部浮起，顶上一条「打开空间…」搜索，下面每个空间一行。点外面收起。
pub fn switcher(w: &Workspace, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    if !w.space_menu || !super::is_compact() { return None; }
    let q = w.space_query.read(cx).text.trim().to_lowercase();
    let here = crate::store::Store::root_dir();
    let rows: Vec<AnyElement> = w.spaces.iter().enumerate().filter(|(_, s)| q.is_empty() || name(s).to_lowercase().contains(&q)).map(|(i, s)| {
        let id = s.id.clone();
        let on = s.root == here;
        div().id(("space-card-row", i)).w_full().h(px(48.)).px(px(10.)).flex().items_center().gap(px(12.)).rounded(px(12.))
            .when(on, |d| d.bg(Theme::fill_hover()))
            .active(|d| d.bg(Theme::fill_pressed()))
            .child(space_icon(s, 26.))
            .child(div().flex_1().min_w_0().truncate().text_size(px(16.)).text_color(if s.available { Theme::text() } else { Theme::text_disabled() }).child(name(s)))
            .when(on, |d| d.child(crate::ui::icon("check", 18., Theme::primary())))
            .when(!s.available, |d| d.child(div().text_size(px(13.)).text_color(Theme::text_disabled()).child(tr("找不到文件夹", "Missing"))))
            .when(s.available && !on, |d| d.on_click(move |_, _, cx| { let id = id.clone(); cx.defer(move |cx| { space::switch(id, cx); }); }))
            .into_any_element()
    }).collect();
    let close = |w: &mut Workspace, cx: &mut Context<Workspace>| { w.space_menu = false; cx.notify(); };
    let card = div().id("space-card").occlude().mx(px(12.)).mb(super::insets().bottom.max(px(12.))).p(px(8.)).flex().flex_col().gap(px(4.))
        .rounded(px(super::SHEET_RADIUS)).bg(theme::raised(Theme::popover_bg())).shadow(theme::shadow_dialog())
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(div().h(px(44.)).px(px(12.)).mb(px(4.)).flex().items_center().gap(px(8.)).rounded(px(12.)).bg(Theme::fill_hover())
            .child(crate::ui::icon("search", 17., Theme::text_caption()))
            .child(div().flex_1().min_w_0().child(w.space_query.clone()))
            .child(div().id("space-card-close").size(px(28.)).flex().items_center().justify_center().child(crate::ui::icon("search-clear", 18., Theme::text_caption()))
                .on_click(cx.listener(move |w, _, _, cx| close(w, cx)))))
        .children(rows);
    Some(deferred(div().id("space-card-mask").absolute().inset_0().flex().flex_col().justify_end().font_family(theme::UI_FONT).text_color(Theme::text())
        .on_mouse_down(MouseButton::Left, cx.listener(move |w, _, _, cx| close(w, cx)))
        .child(card)).with_priority(5).into_any_element())
}

/// 设置 › 空间设置（照 Craft）：图标带相机钮，名称，危险区里的删除（第一个空间没有）。
pub fn space_page(w: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let Some(s) = space::current(w).cloned() else { return div().into_any_element() };
    let card = || div().w_full().rounded(px(14.)).bg(Theme::fill_hover()).overflow_hidden();
    let (pick, del, label) = (s.id.clone(), s.id.clone(), name(&s));
    div().w_full().pt(px(8.)).flex().flex_col().gap(px(20.))
        .child(div().w_full().flex().justify_center().child(div().id("space-page-icon").relative().child(space_icon(&s, 96.))
            .child(div().absolute().right(px(-6.)).bottom(px(-6.)).size(px(32.)).rounded_full().flex().items_center().justify_center()
                .bg(theme::raised(Theme::bg_body())).border_1().border_color(Theme::button_border()).shadow(theme::shadow_button()).child(crate::ui::icon("pen-line", 16., Theme::icon())))
            .on_click(cx.listener(move |w, _, window, cx| w.pick_space_icon(pick.clone(), window, cx)))))
        .child(card().px(px(16.)).py(px(10.)).flex().flex_col().gap(px(2.))
            .child(div().text_size(px(13.)).text_color(Theme::text_caption()).child(tr("空间名称", "Name")))
            .child(div().h(px(28.)).flex().items_center().child(w.space_name.clone())))
        .when(s.icon.is_some(), |d| { let id = s.id.clone(); d.child(card().child(div().id("space-icon-reset").h(px(ROW_H)).px(px(16.)).flex().items_center().text_size(px(16.)).text_color(Theme::primary())
            .active(|s| s.bg(Theme::fill_pressed())).child(tr("恢复默认图标", "Use the default icon"))
            .on_click(cx.listener(move |w, _, _, cx| { let _ = fdl_core::spaces::set_icon(&id, None); w.reload_spaces(cx); })))) })
        .when(!s.primary, |d| d
            .child(div().px(px(4.)).mt(px(8.)).text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).child(tr("危险区", "Danger zone")))
            .child(card().child(div().id("space-delete").px(px(16.)).py(px(12.)).flex().flex_col().gap(px(4.)).active(|s| s.bg(Theme::fill_pressed()))
                .child(div().text_size(px(16.)).text_color(Theme::danger()).child(tr("删除空间", "Delete space")))
                .child(div().text_size(px(13.)).line_height(px(18.)).text_color(Theme::text_caption()).child(tr("删除后，这个空间和里面的所有文档都无法恢复。", "Deleting the space removes it and every document in it for good.")))
                .on_click(cx.listener(move |w, _, _, cx| { w.confirm = Some(Confirm::RemoveSpace { id: del.clone(), name: label.clone() }); cx.notify(); })))))
        .into_any_element()
}

/// 名字输入框外面那一圈。
fn field(f: &space::SpaceForm) -> Div {
    div().h(px(48.)).px(px(14.)).flex().items_center().rounded(px(12.)).bg(Theme::fill_hover()).child(f.name.clone())
}

/// 图标（点了换图）和名字：创建页和新建面板共用。
fn form_head(f: &space::SpaceForm, size: f32, cx: &mut Context<Workspace>) -> Div {
    div().w_full().flex().flex_col().items_center().gap(px(16.))
        .child(div().id("space-form-icon").relative().child(space::icon_el(f.icon.as_deref(), "new", size))
            .child(div().absolute().right(px(-6.)).bottom(px(-6.)).size(px(30.)).rounded_full().flex().items_center().justify_center()
                .bg(theme::raised(Theme::bg_body())).border_1().border_color(Theme::button_border()).shadow(theme::shadow_button()).child(crate::ui::icon("pen-line", 15., Theme::icon())))
            .on_click(cx.listener(|w, _, window, cx| w.pick_form_icon(window, cx))))
        .child(div().w_full().child(field(f)))
        .children(f.error.clone().map(|e| div().w_full().text_size(px(13.)).text_color(Theme::danger()).child(e)))
}

fn location(f: &space::SpaceForm, cx: &mut Context<Workspace>) -> Div {
    div().w_full().flex().flex_col().gap(px(6.))
        .child(div().text_size(px(13.)).text_color(Theme::text_caption()).child(tr("位置", "Location")))
        .child(div().id("space-choose-folder").w_full().py(px(12.)).px(px(14.)).rounded(px(12.)).bg(Theme::fill_hover()).flex().items_center().gap(px(8.))
            .child(div().flex_1().text_size(px(15.)).child(f.folder.as_ref().map(|p| p.file_name().unwrap_or_default().to_string_lossy().into_owned()).unwrap_or_else(|| tr("仅此设备", "On this device").into())))
            .child(crate::ui::icon("folder", 18., Theme::icon()))
            .on_click(cx.listener(|w, _, window, cx| { window.blur(cx); w.pick_form_folder(window, cx); })))
        .child(div().text_size(px(12.)).line_height(px(17.)).text_color(Theme::text_caption()).child(tr("选择 iCloud Drive 文件夹即可在 Apple 设备间同步。", "Choose an iCloud Drive folder to sync between Apple devices.")))
        .when(f.folder.is_some(), |d| d.child(div().id("space-use-local").py(px(6.)).text_size(px(13.)).text_color(Theme::primary()).child(tr("改为仅此设备", "Use this device instead"))
            .on_click(cx.listener(|w, _, _, cx| if let Some(f) = w.form_mut() { f.folder = None; cx.notify(); }))))
}

/// 没有空间时的整页：创建第一个空间。空间放在应用自己的文件夹里（手机上不选位置）。
pub fn setup(w: &Workspace, window: &mut Window, cx: &mut Context<Workspace>) -> AnyElement {
    let f = w.setup.as_ref().expect("setup form");
    let safe = super::safe(window);
    div().id("space-setup").size_full().flex().flex_col().bg(Theme::bg_body()).font_family(theme::UI_FONT).text_color(Theme::text())
        .pt(safe.top + px(56.)).pb(safe.bottom + px(16.)).px(px(24.))
        .child(div().text_size(px(28.)).line_height(px(36.)).font_weight(FontWeight::BOLD).child(tr("创建你的第一个空间", "Create your first space")))
        .child(div().mt(px(8.)).text_size(px(15.)).line_height(px(22.)).text_color(Theme::text_caption())
            .child(tr("空间是一个独立的文档库。以后可以再建更多空间，在设置里切换。", "A space is a library of its own. You can add more later and switch in Settings.")))
        .child(div().mt(px(36.)).child(form_head(f, 88., cx)))
        .child(div().mt(px(20.)).child(location(f, cx)))
        .child(div().flex_1())
        .child(div().id("space-setup-create").h(px(50.)).rounded(px(14.)).flex().items_center().justify_center().bg(Theme::primary())
            .active(|d| d.bg(theme::c(theme::craft::B700))).text_size(px(17.)).font_weight(FontWeight::SEMIBOLD).text_color(gpui::white())
            .child(tr("创建", "Create")).on_click(|_, _, cx| cx.defer(space::create_from_form)))
        .into_any_element()
}

/// 新建空间：从底部浮起的面板，和创建页一样的图标和名字，下面「取消 / 创建」。
pub fn new_space(w: &Workspace, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    let f = w.space_form.as_ref()?;
    let close = |w: &mut Workspace, cx: &mut Context<Workspace>| { w.space_form = None; cx.notify(); };
    let button = |id: &'static str, label: &'static str, primary: bool| div().id(id).flex_1().h(px(48.)).rounded(px(14.)).flex().items_center().justify_center()
        .text_size(px(16.)).font_weight(FontWeight::SEMIBOLD)
        .map(|d| if primary { d.bg(Theme::primary()).text_color(gpui::white()).active(|d| d.bg(theme::c(theme::craft::B700))) } else { d.bg(Theme::fill_hover()).active(|d| d.bg(Theme::fill_pressed())) })
        .child(label);
    let card = div().id("space-new-card").occlude().mx(px(12.)).mb(super::insets().bottom.max(px(12.))).px(px(20.)).pt(px(20.)).pb(px(16.)).flex().flex_col().gap(px(20.))
        .rounded(px(super::SHEET_RADIUS)).bg(theme::raised(Theme::popover_bg())).shadow(theme::shadow_dialog())
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(div().text_size(px(17.)).font_weight(FontWeight::SEMIBOLD).child(tr("新建空间", "New space")))
        .child(form_head(f, 72., cx))
        .child(location(f, cx))
        .child(div().flex().gap(px(12.))
            .child(button("space-new-cancel", tr("取消", "Cancel"), false).on_click(cx.listener(move |w, _, _, cx| close(w, cx))))
            .child(button("space-new-create", tr("创建", "Create"), true).on_click(|_, _, cx| cx.defer(space::create_from_form))));
    Some(deferred(div().id("space-new-mask").absolute().inset_0().flex().flex_col().justify_end().bg(crate::ui::scrim()).font_family(theme::UI_FONT).text_color(Theme::text())
        .on_mouse_down(MouseButton::Left, cx.listener(move |w, _, _, cx| close(w, cx)))
        .child(card)).with_priority(5).into_any_element())
}
