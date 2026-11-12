//! 手机上的首页：不是文档库的平铺，而是几个去处——搜索、收藏、所有文档、文件夹、最近打开。
//! 文档库的网格 / 列表是从「所有文档」进去的下一级。
use gpui::{prelude::FluentBuilder as _, *};

use crate::app::{RowMenu, Workspace};
use crate::settings::{display_title, tr};
use crate::store::Entry;
use crate::theme::Theme;

/// 最近打开只列这么多篇。
const RECENT: usize = 5;
/// 收藏卡片的宽度；高度按主页网格里卡片的比例。
const CARD_W: f32 = 148.;
pub const TOP_GAP: f32 = 12.;
pub const SECTION_TITLE_SIZE: f32 = 18.;
pub const SECTION_TITLE_LINE: f32 = 24.;
thread_local! { static SCROLL: ScrollHandle = ScrollHandle::new(); }

fn section(title: &'static str) -> Div {
    div().mt(px(24.)).mb(px(4.)).px(px(24.)).text_size(px(SECTION_TITLE_SIZE)).line_height(px(SECTION_TITLE_LINE)).font_weight(FontWeight::BOLD).text_color(Theme::text()).child(title)
}

/// 一行去处：图标、名字、右边的数目和箭头。
fn row(id: SharedString, icon: impl IntoElement, title: String, count: Option<usize>) -> Stateful<Div> {
    div().id(id).w_full().h(px(50.)).px(px(16.)).flex().items_center().gap(px(12.)).text_size(px(16.)).rounded(px(10.)).active(|s| s.bg(Theme::fill_pressed()))
        .child(div().size(px(22.)).flex_none().flex().items_center().justify_center().child(icon))
        .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(title))
        .children(count.map(|n| div().flex_none().text_size(px(15.)).text_color(Theme::text_caption()).child(n.to_string())))
        .child(crate::ui::icon("chevron-right", 14., Theme::text_caption()))
}

/// 一组行：直接铺在页面的底色上，不进卡片，行与行之间一道缩进的细线（走查 V4；设置页的 `settings::group` 是带描边的卡片）。
fn group(rows: Vec<AnyElement>) -> Div {
    let n = rows.len();
    div().w_full().flex().flex_col()
        .children(rows.into_iter().enumerate().map(move |(i, r)| div().w_full().flex().flex_col().child(r).when(i + 1 < n, |d| d.child(div().ml(px(50.)).mr(px(16.)).h(px(1.)).bg(crate::ui::list_line())))))
}

/// 长按出这一项的菜单，和主页的卡片一样挂在右键上。
fn with_menu(w: &Workspace, row: Stateful<Div>, e: &Entry, cx: &mut Context<Workspace>) -> Stateful<Div> {
    let id = e.id.clone();
    row.children(super::row_context(w, &e.id, 12.)).on_mouse_down(MouseButton::Right, cx.listener(move |w, ev: &MouseDownEvent, _, cx| {
        w.row_menu = Some(RowMenu { id: id.clone(), pos: ev.position, corner: Anchor::TopLeft, sub: false, batch: false });
        cx.notify();
    }))
}

pub fn render(w: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let scroll = SCROLL.with(Clone::clone);
    // 最近收藏的在前。
    let mut starred: Vec<Entry> = w.entries.iter().filter(|e| e.meta.folder.is_none() && e.meta.starred > 0).cloned().collect();
    starred.sort_by(|a, b| b.meta.starred.cmp(&a.meta.starred));
    let folders: Vec<Entry> = w.level_entries(None).into_iter().filter(|e| e.meta.folder.is_some()).collect();
    let mut recent: Vec<Entry> = w.entries.iter().filter(|e| e.meta.folder.is_none() && e.meta.opened > 0).cloned().collect();
    recent.sort_by(|a, b| b.meta.opened.cmp(&a.meta.opened));
    recent.truncate(RECENT);

    let card_h = CARD_W * (264. / 236.);
    let favourites = if starred.is_empty() {
        div().mx(px(16.)).h(px(132.)).rounded(px(16.)).bg(Theme::fill_hover()).flex().flex_col().items_center().justify_center().gap(px(10.))
            .child(crate::ui::icon("star", 24., Theme::text_caption()))
            .child(div().text_size(px(14.)).text_color(Theme::text_caption()).child(tr("收藏的文档会出现在这里。", "Starred documents show up here.")))
            .into_any_element()
    } else {
        div().id("hub-starred").w_full().overflow_x_scroll().restrict_scroll_to_axis().child(div().flex().gap(px(12.)).px(px(16.))
            .children(starred.iter().map(|e| div().w(px(CARD_W)).flex_none().child(w.render_home_card(e, card_h, cx)))))
            .into_any_element()
    };
    let all = row("hub-all".into(), crate::ui::icon("doc", 20., Theme::icon()), tr("所有文档", "All documents").to_string(), Some(w.entries.iter().filter(|e| e.meta.folder.is_none()).count()))
        .on_click(cx.listener(|_, _, _, cx| { super::set_home_page(super::HomePage::All); cx.notify(); })).into_any_element();
    let open = |e: &Entry, key: &str, count: Option<usize>, cx: &mut Context<Workspace>| {
        let id = e.id.clone();
        // 都画在行首那个 22 的格子里、同一个大小（搜索结果的 `entry_icon` 自带 24 的格子和右边距，放进来会偏左、偏小）。
        let icon = match &e.icon {
            None if e.meta.folder.is_some() => crate::ui::icon("folder", 20., super::folder_tint(e)).into_any_element(),
            _ if e.meta.markdown => crate::ui::icon("doc-md", 20., Theme::icon()).into_any_element(),
            Some(em) => div().text_size(px(18.)).child(em.clone()).into_any_element(),
            None => crate::ui::icon("doc", 20., Theme::icon()).into_any_element(),
        };
        with_menu(w, row(SharedString::from(format!("hub-{key}-{}", e.id)), icon, display_title(&e.title), count), e, cx)
            .on_click(cx.listener(move |w, _, window, cx| w.open(&id, window, cx))).into_any_element()
    };
    let folder_rows: Vec<AnyElement> = folders.iter().map(|e| { let n = w.entries.iter().filter(|c| c.meta.parent.as_deref() == Some(e.id.as_str())).count(); open(e, "folder", Some(n), cx) }).collect();
    let recent_rows: Vec<AnyElement> = recent.iter().map(|e| open(e, "recent", None, cx)).collect();

    // 「文件夹」一栏的标题：点了进文件夹页；右边的「…」是系统菜单（上面盖着透明的原生按钮）：新建文件夹、折叠 / 展开这一栏。
    let folders_head = div().relative().mt(px(16.)).mb(px(0.)).pl(px(24.)).pr(px(12.)).h(px(40.)).flex().items_center()
        .child(div().id("hub-folders").h_full().flex().items_center().gap(px(6.)).text_size(px(SECTION_TITLE_SIZE)).font_weight(FontWeight::BOLD).text_color(Theme::text())
            .child(tr("文件夹", "Folders")).child(crate::ui::icon("chevron-right", 14., Theme::text_caption()))
            .on_click(cx.listener(|_, _, _, cx| { super::set_home_page(super::HomePage::Folders); cx.notify(); })))
        .child(div().flex_1())
        .child(div().relative().size(px(40.)).flex().items_center().justify_center().child(crate::ui::icon("ellipsis", 20., Theme::text_caption()))
            .child(super::menu::button("hub-folders-more", super::hub_menu_items())));
    let content = div().w_full().flex().flex_col().pt(px(super::chrome_top() + TOP_GAP)).pb(px(super::dock_bottom()))
        .child(super::large_title(tr("主页", "Home")))
        .child(div().mb(px(16.)).child(super::lead_search(tr("搜索文档", "Search documents"), cx)))
        .child(favourites)
        .child(div().mt(px(12.)).px(px(8.)).child(group(vec![all])))
        .child(folders_head)
        .when(!folder_rows.is_empty() && !super::folders_folded(), |d| d.child(div().px(px(8.)).child(group(folder_rows))))
        .when(!recent_rows.is_empty(), |d| d.child(section(tr("最近打开", "Recently opened"))).child(div().px(px(8.)).child(group(recent_rows))));
    div().flex_1().min_w_0().h_full().relative()
        .child(div().id("hub").size_full().overflow_y_scroll().track_scroll(&scroll).child(content))
        .child(super::veil::home_header(&scroll, tr("主页", "Home").into()))
        .into_any_element()
}
