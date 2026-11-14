//! 手机上的搜索：一整页，不是对话框。点大标题下面那条搜索，它滑到顶上变成输入框，页面换成结果。
//!
//! 结果是列表：图标，标题（命中的字垫黄底），第二行「文件夹 · 摘要」，右端一颗黄色小牌写这篇里命中了几处。
//! 输入框下面一排是排序（相关度 / 最近修改 / 标题）；从某个文件夹里进来时多一组范围（全部 / 这个文件夹）。
//! 没打字时列最近打开的。状态（开没开、打了什么、滚到哪）还是 `Workspace` 原有的那几个字段，桌面的对话框不受影响。
use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};

use gpui::{prelude::FluentBuilder as _, *};

use crate::app::{View, Workspace};
use crate::settings::{display_title, tr};
use crate::store::Entry;
use crate::theme::Theme;

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum Sort { #[default] Relevance, Modified, Title }

thread_local! { static SORT: Cell<Sort> = const { Cell::new(Sort::Relevance) }; }
// 只搜眼前这个文件夹（从文件夹里进来时默认开着）。
thread_local! { static SCOPED: Cell<bool> = const { Cell::new(true) }; }
// 页面上那条搜索画在哪（每帧记），和点它那一刻的位置、时刻：输入框从那儿滑上来。
thread_local! { static BAR: Cell<Option<Bounds<Pixels>>> = const { Cell::new(None) }; }
thread_local! { static FROM: Cell<Option<(Pixels, Instant)>> = const { Cell::new(None) }; }
// 算好的结果：按（关键词，排序，范围，文档数，最近一次修改）记着，没变就不重算（每帧都会来要）。
type Key = (String, Sort, Option<String>, usize, u64);
thread_local! { static CACHE: RefCell<Option<(Key, Vec<(String, usize)>)>> = const { RefCell::new(None) }; }

/// 一页最多摆这么多条：再多就该接着打字了。
const SHOWN: usize = 60;
/// 命中的字垫的黄底，和右端小牌的底、字（自己配的，深浅色各一套）。
fn marker() -> Hsla { if crate::theme::dark() { rgb(0x7a5c00).into() } else { rgb(0xffe58a).into() } }
fn badge() -> (Hsla, Hsla) { if crate::theme::dark() { (rgb(0x5c4500).into(), rgb(0xffd666).into()) } else { (rgb(0xfff1b8).into(), rgb(0x8a6100).into()) } }
const SLIDE: Duration = Duration::from_millis(280);

/// 页面上那条搜索每帧报一下自己在哪。
pub fn bar_at(bounds: Bounds<Pixels>) { BAR.with(|b| b.set(Some(bounds))); }
/// 点了那条搜索：记下它这一刻的位置。
pub fn opened() { FROM.with(|f| f.set(BAR.with(|b| b.get()).map(|b| (b.top(), Instant::now())))); SCOPED.with(|s| s.set(true)); }

/// 搜索的范围：从文件夹里进来、又没切到「全部」时，是那个文件夹。
fn scope(w: &Workspace) -> Option<String> { if w.view == View::Folder && SCOPED.with(|s| s.get()) { w.folder_current.clone() } else { None } }

/// 结果：文档的 id 和这篇里命中了几处（没打字时是最近打开的，次数是 0）。
fn results(w: &Workspace, q: &str) -> Vec<(String, usize)> {
    let scope = scope(w);
    let key: Key = (q.to_string(), SORT.with(|s| s.get()), scope.clone(), w.entries.len(), w.entries.iter().map(|e| e.meta.modified.max(e.meta.opened)).max().unwrap_or(0));
    if let Some(hit) = CACHE.with(|c| c.borrow().as_ref().filter(|(k, _)| *k == key).map(|(_, v)| v.clone())) { return hit; }
    let inside = |e: &Entry| scope.as_ref().is_none_or(|f| w.ancestors(&e.id).iter().any(|a| &a.id == f));
    let mut out: Vec<(&Entry, bool, usize)> = if q.is_empty() {
        let mut recent: Vec<&Entry> = w.entries.iter().filter(|e| e.meta.folder.is_none() && inside(e)).collect();
        recent.sort_by_key(|e| std::cmp::Reverse(e.meta.opened.max(e.meta.modified)));
        recent.truncate(20);
        recent.into_iter().map(|e| (e, false, 0)).collect()
    } else {
        w.entries.iter().filter(|e| inside(e)).filter_map(|e| {
            let (title, body) = (e.title.to_lowercase().matches(q).count(), e.body.to_lowercase().matches(q).count());
            (title + body > 0).then_some((e, title > 0, title + body))
        }).collect()
    };
    if !q.is_empty() {
        match key.1 {
            // 标题里有的在前，再按命中的次数。
            Sort::Relevance => out.sort_by_key(|(_, title, n)| (!*title, std::cmp::Reverse(*n))),
            Sort::Modified => out.sort_by_key(|(e, _, _)| std::cmp::Reverse(e.meta.modified)),
            Sort::Title => out.sort_by_key(|(e, _, _)| e.title.to_lowercase()),
        }
    }
    let out: Vec<(String, usize)> = out.into_iter().map(|(e, _, n)| (e.id.clone(), n)).collect();
    CACHE.with(|c| *c.borrow_mut() = Some((key, out.clone())));
    out
}

/// 一行字，命中的地方垫黄底。
fn marked(text: String, q: &str) -> StyledText {
    let mut hits = vec![];
    let lower = text.to_lowercase();
    // 小写之后字节数变了的（少数文字）就不标，免得切在字中间。
    if !q.is_empty() && lower.len() == text.len() {
        let mut at = 0;
        while let Some(i) = lower[at..].find(q) {
            let (s, e) = (at + i, at + i + q.len());
            if text.is_char_boundary(s) && text.is_char_boundary(e) { hits.push((s..e, HighlightStyle { background_color: Some(marker()), ..Default::default() })); }
            at = e;
        }
    }
    StyledText::new(text).with_highlights(hits)
}

fn chip(id: &'static str, label: &'static str, on: bool) -> Stateful<Div> {
    div().id(id).flex_none().h(px(30.)).px(px(12.)).rounded_full().flex().items_center().text_size(px(14.)).line_height(px(20.))
        .when(on, |d| d.bg(Theme::primary().opacity(0.14)).text_color(Theme::primary()).font_weight(FontWeight::SEMIBOLD))
        .when(!on, |d| d.bg(Theme::fill_hover()).text_color(Theme::text_caption()))
        .child(label)
}

fn row(w: &Workspace, e: &Entry, n: usize, q: &str, cx: &mut Context<Workspace>) -> AnyElement {
    let id = e.id.clone();
    let folder = e.meta.parent.as_deref().and_then(|p| w.entries.iter().find(|x| x.id == p)).map(|p| display_title(&p.title));
    // 摘要：命中的那一段，压成一行。
    let summary = if q.is_empty() { None } else { crate::search::snippet(&e.body, q).map(|(text, _)| text.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(90).collect::<String>()) };
    let summary = summary.or_else(|| { let s: String = e.body.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(90).collect(); (!s.is_empty()).then_some(s) });
    let second = match (folder, summary) { (Some(f), Some(s)) => format!("{f} · {s}"), (Some(f), None) => f, (None, Some(s)) => s, (None, None) => String::new() };
    let glyph = if e.meta.folder.is_some() { super::folder_icons::glyph(e, 20.).into_any_element() } else { crate::search::entry_icon(e).mr(px(0.)).into_any_element() };
    let (fill, ink) = badge();
    div().id(SharedString::from(format!("find-{}", e.id))).flex_none().h(px(64.)).mx(px(12.)).px(px(8.)).flex().items_center().gap(px(12.)).rounded(px(12.)).active(|s| s.bg(Theme::fill_hover()))
        .child(div().flex_none().size(px(40.)).rounded(px(10.)).bg(Theme::fill_hover()).flex().items_center().justify_center().child(glyph))
        .child(div().flex_1().min_w_0().flex().flex_col().gap(px(2.))
            .child(div().text_size(px(16.)).line_height(px(22.)).font_weight(FontWeight::SEMIBOLD).text_color(Theme::text()).overflow_hidden().whitespace_nowrap().text_ellipsis().child(marked(display_title(&e.title), q)))
            .when(!second.is_empty(), |d| d.child(div().text_size(px(13.)).line_height(px(18.)).text_color(Theme::text_caption()).overflow_hidden().whitespace_nowrap().text_ellipsis().child(marked(second, q)))))
        .when(n > 0, |d| d.child(div().flex_none().h(px(22.)).min_w(px(22.)).px(px(7.)).rounded_full().bg(fill).flex().items_center().justify_center()
            .text_size(px(12.)).line_height(px(16.)).font_weight(FontWeight::SEMIBOLD).text_color(ink).child(if n > 50 { "50+".to_string() } else { n.to_string() })))
        .on_click(cx.listener(move |w, _, window, cx| { w.close_search(window, cx); w.open(&id, window, cx); }))
        .into_any_element()
}

/// 搜索这一页。盖在当前页上（底下的玻璃在 `search_open` 时已经让开）。
pub fn page(w: &Workspace, window: &mut Window, cx: &mut Context<Workspace>) -> AnyElement {
    let q = w.query(cx);
    let has_text = !w.search.read(cx).text.is_empty();
    let found = results(w, &q);
    let top = super::insets().top + px(6.);
    let (sort, scoped, in_folder) = (SORT.with(|s| s.get()), SCOPED.with(|s| s.get()), w.view == View::Folder && w.folder_current.is_some());
    let from = FROM.with(|f| f.get());

    // 和主页的搜索条同一个样子（`ios::search_bar_with`）：它从那里滑上来。
    let field = div().flex_1().min_w_0().h(px(super::SEARCH_H)).px(px(14.)).flex().items_center().gap(px(8.)).rounded_full().bg(super::search_fill())
        .child(crate::ui::icon("search", 18., Theme::text_caption()))
        .child(div().flex_1().min_w_0().flex().child(w.search.clone()))
        .when(has_text, |d| d.child(div().id("find-clear").flex_none().size(px(28.)).mr(px(-6.)).flex().items_center().justify_center().child(crate::ui::icon("search-clear", 18., Theme::text_disabled()))
            .on_click(cx.listener(|w, _, window, cx| {
                w.search.update(cx, |s, cx| s.set_text("", cx));
                window.focus(&w.search.focus_handle(cx), cx);
                w.search_scroll.set_offset(point(px(0.), px(0.)));
                cx.notify();
            }))));
    let cancel = div().id("find-cancel").flex_none().h(px(super::SEARCH_H)).pl(px(12.)).flex().items_center().text_size(px(16.)).text_color(Theme::primary()).child(tr("取消", "Cancel"))
        .on_click(cx.listener(|w, _, window, cx| { w.close_search(window, cx); window.focus(&w.focus, cx); }));
    let bar = div().flex_none().h(px(56.)).px(px(16.)).flex().items_center().child(field).child(cancel);

    let set_sort = |s: Sort| cx.listener(move |w: &mut Workspace, _: &ClickEvent, _, cx| { SORT.with(|c| c.set(s)); w.search_scroll.set_offset(point(px(0.), px(0.))); cx.notify(); });
    let set_scope = |on: bool| cx.listener(move |w: &mut Workspace, _: &ClickEvent, _, cx| { SCOPED.with(|c| c.set(on)); w.search_scroll.set_offset(point(px(0.), px(0.))); cx.notify(); });
    let chips = div().id("find-chips").flex_none().h(px(42.)).px(px(20.)).flex().items_center().gap(px(8.)).overflow_x_scroll()
        .when(in_folder, |d| d.child(chip("find-here", tr("此文件夹", "This folder"), scoped).on_click(set_scope(true)))
            .child(chip("find-all", tr("全部", "Everywhere"), !scoped).on_click(set_scope(false)))
            .child(div().flex_none().w(px(1.)).h(px(16.)).bg(Theme::line_divider())))
        .child(chip("find-rel", tr("相关度", "Relevance"), sort == Sort::Relevance).on_click(set_sort(Sort::Relevance)))
        .child(chip("find-new", tr("最近修改", "Modified"), sort == Sort::Modified).on_click(set_sort(Sort::Modified)))
        .child(chip("find-name", tr("标题", "Title"), sort == Sort::Title).on_click(set_sort(Sort::Title)));

    let caption = |text: String| div().flex_none().h(px(34.)).px(px(24.)).flex().items_center().text_size(px(13.)).line_height(px(18.)).text_color(Theme::text_caption()).child(text);
    let mut list = div().id("find-results").flex_1().min_h_0().pb(px(16.)).flex().flex_col().overflow_y_scroll().track_scroll(&w.search_scroll);
    if q.is_empty() {
        if !found.is_empty() { list = list.child(caption(tr("最近浏览", "Recently viewed").to_string())); }
    } else if found.is_empty() {
        list = list.child(div().flex_none().pt(px(72.)).px(px(32.)).text_center().text_size(px(15.)).line_height(px(22.)).text_color(Theme::text_caption()).child(crate::settings::text("没有找到匹配的结果，尝试其他关键词进行搜索")));
    } else {
        list = list.child(caption(crate::localized_format!("{} 个结果", "{} results", found.len())));
    }
    for (id, n) in found.iter().take(SHOWN) {
        if let Some(e) = w.entries.iter().find(|e| &e.id == id) { list = list.child(row(w, e, *n, &q, cx)); }
    }
    if found.len() > SHOWN { list = list.child(caption(crate::localized_format!("还有 {} 个，接着输入可以缩小范围", "{} more — keep typing to narrow down", found.len() - SHOWN))); }

    // 整屏盖住；键盘起来时列表只到键盘上沿。
    let keyboard = (window.viewport_size().height - super::visible_height(window)).max(px(0.));
    let rest = div().flex_1().min_h_0().flex().flex_col().when(!q.is_empty(), |d| d.child(chips)).child(list);
    let body = |lift: Pixels, shown: f32| div().id("find-page").occlude().absolute().inset_0().bg(super::home_bg().opacity(shown)).flex().flex_col()
        .pt(top + lift).pb(keyboard);
    // 输入框从页面上那条搜索的位置滑到顶上，结果跟着淡入。
    match from.filter(|(_, at)| at.elapsed() < SLIDE) {
        Some((y, at)) => {
            let far = (y - top).max(px(0.));
            let t = 1. - (1. - (at.elapsed().as_secs_f32() / SLIDE.as_secs_f32()).clamp(0., 1.)).powi(4);
            window.request_animation_frame();
            body(far * (1. - t), t).child(bar).child(rest.opacity(t)).into_any_element()
        }
        None => body(px(0.), 1.).child(bar).child(rest).into_any_element(),
    }
}
