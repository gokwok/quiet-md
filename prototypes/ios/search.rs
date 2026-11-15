//! 手机上的搜索（照 Craft，IMG_9193–9199）：一整页，不是对话框。点大标题下面那条搜索，它滑到顶上变成输入框，页面换成结果。
//!
//! 输入框右边一颗「…」（排序方式、显示样式，从文件夹里进来时还有搜索范围），再右边「取消」。
//! 结果默认是列表：小缩略图，标题（收藏的前面一颗星，命中的字垫黄底），第二行「📁 文件夹 · 摘要」，右端一块黄色小牌写这篇里命中了几处；
//! 也可以换成和浏览页一样的卡片网格。没打字时是几道淡淡的占位和「没有结果」。
//! 状态（开没开、打了什么、滚到哪）还是 `Workspace` 原有的那几个字段，桌面的对话框不受影响。
use std::cell::{Cell, RefCell};
use std::time::{Duration, Instant};

use gpui::{prelude::FluentBuilder as _, *};

use crate::app::{View, Workspace};
use crate::settings::{display_title, tr};
use crate::store::Entry;
use crate::theme::Theme;

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum Sort { #[default] Relevance, Name, Kind, Viewed, Created, Modified }
const SORTS: [(Sort, &str, &str, &str); 6] = [
    (Sort::Relevance, "find-sort-relevance", "相关度", "Relevance"), (Sort::Name, "find-sort-name", "名称", "Name"), (Sort::Kind, "find-sort-kind", "类型", "Type"),
    (Sort::Viewed, "find-sort-viewed", "上次浏览", "Last Viewed"), (Sort::Created, "find-sort-created", "创建日期", "Date Created"), (Sort::Modified, "find-sort-modified", "更新日期", "Date Modified"),
];

thread_local! { static SORT: Cell<Sort> = const { Cell::new(Sort::Relevance) }; }
// 显示样式：列表（默认）或网格。
thread_local! { static GRID: Cell<bool> = const { Cell::new(false) }; }
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
const SLIDE: Duration = Duration::from_millis(220);

/// 搜索页已经完全盖住底下那页了（输入框滑到位）：底下那页这时不用画（`app.rs`），打字时不跟着重画满屏的缩略图。
pub fn covering() -> bool { STARTED.with(|s| s.get()) && FROM.with(|f| f.get()).is_none_or(|(_, at)| at.elapsed() >= SLIDE) }

/// 页面上那条搜索每帧报一下自己在哪。
pub fn bar_at(bounds: Bounds<Pixels>) { BAR.with(|b| b.set(Some(bounds))); }
/// 点了那条搜索：记下它这一刻的位置。
pub fn opened() {
    FROM.with(|f| f.set(BAR.with(|b| b.get()).map(|b| (b.top(), Instant::now()))));
    STARTED.with(|s| s.set(false));
    SCOPED.with(|s| s.set(true));
    // 页面上的玻璃按钮跟着淡掉，不是一下没了。
    super::shot::soft_hide();
}
// 滑上来的动画从搜索页第一次画出来那一帧算起（点下去到第一帧之间要准备一会儿，不能把开头几帧吃掉）。
thread_local! { static STARTED: Cell<bool> = const { Cell::new(true) }; }

/// 搜索的范围：从文件夹里进来、又没切到「全部」时，是那个文件夹。
fn scope(w: &Workspace) -> Option<String> { if w.view == View::Folder && SCOPED.with(|s| s.get()) { w.folder_current.clone() } else { None } }

/// 结果：文档的 id 和这篇里命中了几处（没打字时是最近打开的，次数是 0）。
fn results(w: &Workspace, q: &str) -> Vec<(String, usize)> {
    let scope = scope(w);
    let key: Key = (q.to_string(), SORT.with(|s| s.get()), scope.clone(), w.entries.len(), w.entries.iter().map(|e| e.meta.modified.max(e.meta.opened)).max().unwrap_or(0));
    if let Some(hit) = CACHE.with(|c| c.borrow().as_ref().filter(|(k, _)| *k == key).map(|(_, v)| v.clone())) { return hit; }
    let inside = |e: &Entry| scope.as_ref().is_none_or(|f| w.ancestors(&e.id).iter().any(|a| &a.id == f));
    // 没打字：照 Craft 什么都不列。
    let mut out: Vec<(&Entry, bool, usize)> = if q.is_empty() { vec![] } else {
        w.entries.iter().filter(|e| inside(e)).filter_map(|e| {
            let (title, body) = (e.title.to_lowercase().matches(q).count(), e.body.to_lowercase().matches(q).count());
            (title + body > 0).then_some((e, title > 0, title + body))
        }).collect()
    };
    if !q.is_empty() {
        match key.1 {
            // 标题里有的在前，再按命中的次数。
            Sort::Relevance => out.sort_by_key(|(_, title, n)| (!*title, std::cmp::Reverse(*n))),
            Sort::Name => out.sort_by_key(|(e, _, _)| e.title.to_lowercase()),
            // 类型：文件夹在前，再按名称。
            Sort::Kind => out.sort_by_key(|(e, _, _)| (e.meta.folder.is_none(), e.title.to_lowercase())),
            Sort::Viewed => out.sort_by_key(|(e, _, _)| std::cmp::Reverse(e.meta.opened)),
            Sort::Created => out.sort_by_key(|(e, _, _)| std::cmp::Reverse(e.meta.created)),
            Sort::Modified => out.sort_by_key(|(e, _, _)| std::cmp::Reverse(e.meta.modified)),
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

fn row(w: &Workspace, e: &Entry, n: usize, q: &str, last: bool, cx: &mut Context<Workspace>) -> AnyElement {
    let id = e.id.clone();
    let folder = e.meta.parent.as_deref().and_then(|p| w.entries.iter().find(|x| x.id == p)).map(|p| display_title(&p.title));
    // 摘要：命中的那一段，压成一行。
    let summary = crate::search::snippet(&e.body, q).map(|(text, _)| text.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(90).collect::<String>());
    let summary = summary.or_else(|| { let s: String = e.body.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(90).collect(); (!s.is_empty()).then_some(s) });
    let second = match (&folder, summary) { (Some(f), Some(s)) => format!("{f} · {s}"), (Some(f), None) => f.clone(), (None, Some(s)) => s, (None, None) => String::new() };
    // 行首：文档是一页小缩略图（和首页列表的一样），文件夹是它的图标。
    let thumb = if e.meta.folder.is_some() { div().size(px(32.)).flex().items_center().justify_center().child(super::folder_icons::glyph(e, 24.)).into_any_element() } else { w.mini_thumb(e, 30.) };
    // 命中次数的小牌（照 Craft：黄底圆角方块，深色字）：只有正文里也命中了才有。
    let badge = (n > 1).then(|| div().flex_none().h(px(28.)).min_w(px(28.)).px(px(7.)).rounded(px(8.)).bg(badge_fill()).flex().items_center().justify_center()
        .text_size(px(14.)).line_height(px(18.)).font_weight(FontWeight::SEMIBOLD).text_color(rgb(0x3a2e00)).child(if n > 99 { "99+".to_string() } else { n.to_string() }));
    div().id(SharedString::from(format!("find-{}", e.id))).flex_none().w_full().flex().flex_col().active(|s| s.bg(Theme::fill_pressed()))
        .child(div().h(px(62.)).px(px(16.)).flex().items_center().gap(px(14.))
            .child(div().flex_none().w(px(34.)).flex().justify_center().child(thumb))
            .child(div().flex_1().min_w_0().flex().flex_col().gap(px(1.))
                .child(div().flex().items_center().gap(px(5.)).min_w_0()
                    .when(e.meta.starred > 0, |d| d.child(crate::ui::icon("star-fill", 15., super::star_color())))
                    .child(div().min_w_0().text_size(px(17.)).line_height(px(22.)).text_color(Theme::text()).overflow_hidden().whitespace_nowrap().text_ellipsis().child(marked(display_title(&e.title), q))))
                .when(!second.is_empty(), |d| d.child(div().flex().items_center().gap(px(5.)).min_w_0().text_color(Theme::text_caption())
                    .when(folder.is_some(), |d| d.child(crate::ui::icon("folder", 13., Theme::text_caption())))
                    .child(div().min_w_0().text_size(px(13.)).line_height(px(18.)).overflow_hidden().whitespace_nowrap().text_ellipsis().child(marked(second, q))))))
            .children(badge))
        .when(!last, |d| d.child(div().ml(px(64.)).mr(px(16.)).h(px(1.)).bg(crate::ui::list_line())))
        .on_click(cx.listener(move |w, _, window, cx| { super::glass::haptic(); w.close_search(window, cx); w.open(&id, window, cx); }))
        .into_any_element()
}

/// 网格里的文件夹：一块圆角底上它的图标和名字，点了打开。
fn folder_tile(e: &Entry, cx: &mut Context<Workspace>) -> AnyElement {
    let id = e.id.clone();
    div().id(SharedString::from(format!("find-folder-{}", e.id))).size_full().rounded(px(20.)).bg(Theme::fill_hover()).flex().flex_col().items_center().justify_center().gap(px(10.))
        .active(|s| s.bg(Theme::fill_pressed()))
        .child(super::folder_icons::glyph(e, 44.))
        .child(div().px(px(14.)).max_w_full().truncate().text_size(px(15.)).font_weight(FontWeight::SEMIBOLD).text_color(Theme::text()).child(display_title(&e.title)))
        .on_click(cx.listener(move |w, _, window, cx| { w.close_search(window, cx); w.open(&id, window, cx); }))
        .into_any_element()
}

/// 命中次数小牌的黄（照 Craft 实测，IMG_9199：#F6D65C 左右）。
fn badge_fill() -> Hsla { if crate::theme::dark() { rgb(0xc9a62e).into() } else { rgb(0xf6d65c).into() } }

/// 「…」菜单里的一项被点了（`app::menu_action` 转过来）。
pub fn menu_action(id: &str, w: &mut Workspace, cx: &mut Context<Workspace>) -> bool {
    if let Some(s) = SORTS.iter().find(|s| s.1 == id) { SORT.with(|c| c.set(s.0)); }
    else if id == "find-view-grid" || id == "find-view-list" { GRID.with(|g| g.set(id == "find-view-grid")); }
    else if id == "find-scope-here" || id == "find-scope-all" { SCOPED.with(|c| c.set(id == "find-scope-here")); }
    else { return false; }
    super::glass::haptic();
    w.search_scroll.set_offset(point(px(0.), px(0.)));
    cx.notify();
    true
}

/// 「…」：排序方式、显示样式（下面一行小字写着现在是哪个），从文件夹里进来时还有搜索范围。
fn options(in_folder: bool) -> Vec<super::menu::Item> {
    use super::menu::Item;
    let sort = SORT.with(|s| s.get());
    let grid = GRID.with(|g| g.get());
    let now = SORTS.iter().find(|s| s.0 == sort).map_or("", |s| tr(s.2, s.3));
    let mut items = vec![
        Item::new("", tr("排序方式", "Sort By"), "arrow.up.arrow.down", 0).sub(now)
            .kids(SORTS.iter().map(|s| Item::new(s.1, tr(s.2, s.3), "", 0).checked(s.0 == sort)).collect()),
        Item::new("", tr("显示样式", "View As"), if grid { "square.grid.2x2" } else { "rectangle.grid.1x2" }, 0).sub(if grid { tr("网格", "Grid") } else { tr("列表", "List") })
            .kids(vec![Item::new("find-view-grid", tr("网格", "Grid"), "square.grid.2x2", 0).checked(grid), Item::new("find-view-list", tr("列表", "List"), "rectangle.grid.1x2", 0).checked(!grid)]),
    ];
    if in_folder {
        let here = SCOPED.with(|s| s.get());
        items.push(Item::new("", tr("搜索范围", "Search In"), "folder", 1).sub(if here { tr("此文件夹", "This Folder") } else { tr("全部", "Everywhere") })
            .kids(vec![Item::new("find-scope-here", tr("此文件夹", "This Folder"), "", 0).checked(here), Item::new("find-scope-all", tr("全部", "Everywhere"), "", 0).checked(!here)]));
    }
    items
}

/// 搜索这一页。盖在当前页上（底下的玻璃在 `search_open` 时已经让开）。
pub fn page(w: &Workspace, window: &mut Window, cx: &mut Context<Workspace>) -> AnyElement {
    let q = w.query(cx);
    let has_text = !w.search.read(cx).text.is_empty();
    let found = results(w, &q);
    let top = super::insets().top + px(6.);
    let in_folder = w.view == View::Folder && w.folder_current.is_some();
    if !STARTED.with(|s| s.replace(true)) { FROM.with(|f| f.set(f.get().map(|(y, _)| (y, Instant::now())))); }
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
    let more = div().relative().flex_none().size(px(40.)).ml(px(4.)).flex().items_center().justify_center()
        .child(crate::ui::icon("ellipsis", 20., Theme::text())).child(super::menu::button("find-options", options(in_folder)));
    let cancel = div().id("find-cancel").flex_none().h(px(super::SEARCH_H)).pl(px(4.)).flex().items_center().text_size(px(17.)).text_color(Theme::primary()).child(tr("取消", "Cancel"))
        .on_click(cx.listener(|w, _, window, cx| { w.close_search(window, cx); window.focus(&w.focus, cx); }));
    let bar = div().flex_none().h(px(56.)).px(px(16.)).flex().items_center().child(field).child(more).child(cancel);

    let caption = |text: String| div().flex_none().h(px(34.)).px(px(24.)).flex().items_center().text_size(px(13.)).line_height(px(18.)).text_color(Theme::text_caption()).child(text);
    let mut list = div().id("find-results").flex_1().min_h_0().pb(px(16.)).flex().flex_col().overflow_y_scroll().track_scroll(&w.search_scroll);
    if found.is_empty() {
        // 没打字时什么都不放；打了字没找到，正中一行「没有结果」。
        if !q.is_empty() { list = list.child(div().flex_none().pt(px(160.)).text_center().text_size(px(15.)).text_color(Theme::text_placeholder()).child(tr("没有结果", "No Results"))); }
    } else if GRID.with(|g| g.get()) {
        // 网格：和浏览页同一种卡片，两列。
        let vw = f32::from(window.viewport_size().width);
        let card_w = (vw - 12. * 2. - 8.) / 2.;
        let card_h = card_w * (264. / 236.);
        let entries: Vec<Entry> = found.iter().take(SHOWN).filter_map(|(id, _)| w.entries.iter().find(|e| &e.id == id).cloned()).collect();
        let mut grid = div().flex_none().px(px(12.)).pt(px(8.)).flex().flex_col().gap(px(8.));
        for pair in entries.chunks(2) {
            grid = grid.child(div().flex().gap(px(8.)).children(pair.iter().map(|e| div().w(px(card_w)).h(px(card_h)).child(
                if e.meta.folder.is_some() { folder_tile(e, cx) } else { w.render_home_card(e, card_h, cx) }))));
        }
        list = list.child(grid);
    } else {
        let shown = found.len().min(SHOWN);
        for (i, (id, n)) in found.iter().take(SHOWN).enumerate() {
            if let Some(e) = w.entries.iter().find(|e| &e.id == id) { list = list.child(row(w, e, *n, &q, i + 1 == shown, cx)); }
        }
    }
    if found.len() > SHOWN { list = list.child(caption(crate::localized_format!("还有 {} 个，接着输入可以缩小范围", "{} more — keep typing to narrow down", found.len() - SHOWN))); }

    // 整屏盖住；键盘起来时列表只到键盘上沿。
    let keyboard = (window.viewport_size().height - super::visible_height(window)).max(px(0.));
    let rest = div().flex_1().min_h_0().flex().flex_col().child(list);
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
