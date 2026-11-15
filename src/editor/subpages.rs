//! 子页面列表 block (`<sub-page-list/>`): the page's name as a small
//! header, then one row per child page in a crisp box (1pt hairline,
//! radius 5). The list itself comes from the library (`Editor::sub_pages`).
//! Also the 页面 / 卡片 block, and the way back out of an open page.

use super::render::{Env, R};
use super::view::{Editor, EditorEvent, Menu};
use crate::settings::display_title;
use crate::model::Block;
use crate::editor_theme::Theme;
use crate::editor_ui::{icon, ui_text};
use gpui::{prelude::FluentBuilder, *};

/// Containers inside the page keep crisp corners: the box at 5, the
/// hover under a row at 4.
const RADIUS: f32 = 5.;
const ROW: f32 = 4.;

/// 16px page glyph, or the page's emoji icon.
fn page_icon(emoji: &Option<String>) -> AnyElement {
    match emoji {
        Some(e) => div().text_size(px(16.)).line_height(px(18.)).child(e.clone()).into_any_element(),
        None => icon("doc", 16., Theme::icon()).into_any_element(),
    }
}

impl R<'_> {
    pub(super) fn sub_page_list(&self, b: &Block, cx: &mut Context<Editor>) -> AnyElement {
        let ed = self.ed;
        let id = b.id;
        // Handle menu open: the accent-tinted ring of the code block;
        // block-selected: an accent rim; otherwise the card hairline.
        let menu_on = matches!(&ed.menu, Some(Menu::Block { block, .. }) | Some(Menu::Add { block, .. }) if *block == id);
        let selected = !menu_on && self.selected.contains(&id);
        let header = div().min_h(px(24.)).mb(px(8.)).flex().items_center().child(
            div()
                .h(px(20.))
                .min_w_0()
                .pr(px(4.))
                .flex()
                .items_center()
                .rounded(px(ROW))
                .hover(|s| s.bg(Theme::fill_hover()))
                .child(div().size(px(20.)).p(px(2.)).flex_none().flex().items_center().justify_center().child(page_icon(&ed.core.doc.icon)))
                .child(
                    div()
                        .min_w_0()
                        .max_w(px(476.))
                        .text_size(px(12.))
                        .line_height(px(20.))
                        .text_color(Theme::text_caption())
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(ui_text(display_title(&ed.core.doc.title.text))),
                ),
        );
        let mut content = div()
            .relative()
            .p(px(4.))
            .rounded(px(RADIUS))
            .border_1()
            .border_color(if menu_on {
                Theme::code_hover_ring()
            } else if selected {
                Theme::primary()
            } else {
                Theme::button_border()
            })
            .when(menu_on, |d| {
                d.child(div().absolute().top(px(-3.)).left(px(-3.)).right(px(-3.)).bottom(px(-3.)).rounded(px(RADIUS + 3.)).border_2().border_color(Theme::code_hover_ring()))
            });
        if ed.sub_pages.is_empty() {
            content = content.child(
                div().h(px(240.)).flex().items_center().justify_center().text_size(px(13.)).line_height(px(20.)).text_color(Theme::text_caption()).child(crate::settings::tr("此文件夹中没有其他文档", "No other documents in this folder")),
            );
        }
        for p in &ed.sub_pages {
            let doc = p.id.clone();
            content = content.child(
                div()
                    .id(SharedString::from(format!("subpage-{}-{}", id.0, p.id)))
                    .min_h(px(36.))
                    .px(px(6.))
                    .py(px(7.))
                    .flex()
                    .items_start()
                    .rounded(px(ROW))
                    .cursor_pointer()
                    .hover(|s| s.bg(Theme::fill_hover()))
                    .child(div().w(px(20.)).h(px(22.)).mr(px(6.)).p(px(2.)).flex_none().flex().items_center().justify_center().child(page_icon(&p.icon)))
                    // A page block's title: body size.
                    .child(div().flex_1().min_w_0().text_size(px(15.)).line_height(px(22.)).text_color(Theme::text()).line_clamp(2).child(display_title(&p.title)))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |_, _, _, cx| {
                            cx.stop_propagation();
                            cx.emit(EditorEvent::OpenDoc(doc.clone()));
                        }),
                    ),
            );
        }
        div().max_w(px(720.)).child(header).child(content).into_any_element()
    }
}

/// What a page's first lines say, for the grey line under its title.
fn preview(b: &Block, max: usize) -> String {
    fn walk(list: &[Block], out: &mut String, max: usize) {
        for c in list {
            if out.chars().count() >= max {
                return;
            }
            let t = c.text.text.replace('\u{fffc}', "");
            let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
            if !t.is_empty() {
                if !out.is_empty() {
                    out.push(' ');
                }
                out.push_str(&t);
            }
            if !c.kind.is_page() {
                walk(&c.children, out, max);
            }
        }
    }
    let mut out = String::new();
    walk(&b.children, &mut out, max);
    out.chars().take(max).collect()
}

/// A page's thumbnail: a little sheet with three lines of text on it.
fn sheet(w: f32, h: f32) -> Div {
    let bar = |frac: f32| div().h(px((h / 14.).max(1.5))).w(relative(frac)).rounded_full().bg(Theme::line_border());
    div()
        .w(px(w))
        .h(px(h))
        .flex_none()
        .p(px(w / 5.))
        .flex()
        .flex_col()
        .gap(px(h / 9.))
        .rounded(px(w / 6.))
        .border_1()
        .border_color(Theme::button_border())
        .bg(Theme::bg_overlay())
        .child(bar(1.))
        .child(bar(0.8))
        .child(bar(0.9))
}

impl R<'_> {
    /// 页面 / 卡片 block (Craft): a page thumbnail, the title (the block's own
    /// text, editable like any line) and its first words in grey; as a card,
    /// a raised box with more of the text. A click opens the page.
    pub(super) fn page_block(&self, b: &Block, card: bool, env: Env, cx: &mut Context<Editor>) -> AnyElement {
        let ed = self.ed;
        let id = b.id;
        let menu_on = matches!(&ed.menu, Some(Menu::Block { block, .. }) | Some(Menu::Add { block, .. }) if *block == id);
        let selected = !menu_on && self.selected.contains(&id);
        let mut spec = self.spec_for(b, env);
        spec.weight = FontWeight::MEDIUM;
        let lh = spec.line_height;
        if card {
            spec.size += 1.;
            spec.weight = FontWeight::SEMIBOLD;
        }
        let title = self.text_el(id, &b.text, spec, b.align, Some(crate::settings::tr("无标题页面", "Untitled page").into()));
        let more = preview(b, if card { 160 } else { 80 });
        // The preview: smaller, grey, secondary.
        let caption = |lines: usize| div().text_size(px(13.)).line_height(px(18.)).text_color(Theme::text_caption()).line_clamp(lines).overflow_hidden().child(more.clone());
        let el = div()
            .id(SharedString::from(format!("page-{}", id.0)))
            .relative()
            .flex()
            .items_start()
            .cursor_pointer()
            // A tap only opens the page: no caret lands in the title on the way.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(cx.listener(move |e, _, window, cx| e.open_page(id, window, cx)));
        if card {
            return el
                .flex_col()
                .gap(px(10.))
                .p(px(14.))
                .rounded(px(10.))
                .border_1()
                .border_color(if menu_on { Theme::code_hover_ring() } else if selected { Theme::primary() } else { Theme::button_border() })
                .bg(Theme::bg_body())
                .shadow(crate::theme::shadow_card())
                .child(sheet(26., 32.))
                .child(div().w_full().min_w_0().child(title).when(!more.is_empty(), |d| d.child(caption(3))))
                .into_any_element();
        }
        el.gap(px(10.))
            .py(px(4.))
            .rounded(px(8.))
            .hover(|s| s.bg(Theme::fill_hover()))
            // The thumbnail sits on the title's first line.
            .child(div().h(px(lh)).flex().items_center().child(sheet(((lh - 4.) * 0.8).round(), lh - 4.)))
            .child(div().flex_1().min_w_0().child(title).when(!more.is_empty(), |d| d.child(caption(1))))
            .into_any_element()
    }
}
