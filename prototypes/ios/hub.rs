//! 手机上的首页：不是文档库的平铺，而是几个去处——搜索、收藏、所有文档、文件夹、最近删除（照 Craft，IMG_9111/9112）。
//! 文档库的网格 / 列表是从「所有文档」进去的下一级。显示哪几栏、什么顺序由「自定义主页」决定（`customise.rs`）。
use std::cell::RefCell;
use std::collections::HashSet;

use gpui::{prelude::FluentBuilder as _, *};

use crate::app::{RowMenu, View, Workspace};
use crate::settings::{display_title, tr};
use crate::store::Entry;
use crate::theme::Theme;
use super::customise::{self, Quick, Section};

/// 收藏卡片的宽度；高度按主页网格里卡片的比例。
const CARD_W: f32 = 148.;
pub const TOP_GAP: f32 = 12.;
pub const SECTION_TITLE_SIZE: f32 = 18.;
pub const SECTION_TITLE_LINE: f32 = 24.;
/// 左上角空间头像的大小（Craft 实测约 29pt）和左边距（和大标题的字对齐）。
const AVATAR: f32 = 30.;
const AVATAR_X: f32 = 24.;
/// 一行的高度（Craft 实测 48pt）。
const ROW_H: f32 = 48.;
/// 快速访问那排卡片最多几张（最近使用、收藏都是）。
const QUICK_MAX: usize = 10;
/// 卡片之间的空隙。
const QUICK_GAP: f32 = 12.;
thread_local! { static SCROLL: ScrollHandle = ScrollHandle::new(); }

/// 一行去处：图标、名字，右边可选的数目和箭头。
fn row(id: SharedString, icon: impl IntoElement, title: String, count: Option<usize>, chevron: bool) -> Stateful<Div> {
    div().id(id).w_full().h(px(ROW_H)).px(px(16.)).flex().items_center().gap(px(12.)).text_size(px(17.)).rounded(px(10.)).active(|s| s.bg(Theme::fill_pressed()))
        .child(div().size(px(22.)).flex_none().flex().items_center().justify_center().child(icon))
        .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(title))
        .children(count.map(|n| div().flex_none().text_size(px(16.)).text_color(Theme::text_caption()).child(n.to_string())))
        .when(chevron, |d| d.child(crate::ui::icon("chevron-right", 14., Theme::text_caption())))
}

/// 一组行：直接铺在页面的底色上，不进卡片，行与行之间一道细线，左边和字对齐，右边离屏幕 16（走查 V4；设置页的 `settings::group` 是带描边的卡片）。
fn group(rows: Vec<AnyElement>) -> Div { indented_group(rows.into_iter().map(|r| (r, 0)).collect()) }
/// 同上，每行带缩进的层级（展开的文件夹里的东西）：细线和那一行的字对齐。
fn indented_group(rows: Vec<(AnyElement, usize)>) -> Div {
    let n = rows.len();
    div().w_full().px(px(8.)).flex().flex_col()
        .children(rows.into_iter().enumerate().map(move |(i, (r, depth))| div().w_full().flex().flex_col().child(r)
            .when(i + 1 < n, |d| d.child(div().ml(px(50. + depth as f32 * INDENT)).mr(px(8.)).h(px(1.)).bg(crate::ui::list_line())))))
}

/// 展开的文件夹里每深一层往右缩进多少（照 Craft，IMG_9121：约 16pt）。
const INDENT: f32 = 16.;
// 首页上展开着的文件夹（只在这次运行里记着）。
thread_local! { static EXPANDED: RefCell<HashSet<String>> = RefCell::new(HashSet::new()); }
fn expanded(id: &str) -> bool { EXPANDED.with(|e| e.borrow().contains(id)) }
/// 展开或收起一个文件夹，轻轻震一下。
fn toggle(id: &str) {
    EXPANDED.with(|e| { let mut e = e.borrow_mut(); if !e.remove(id) { e.insert(id.to_string()); } });
    super::glass::haptic();
}

/// 一栏的标题：粗体，可选右边的箭头（点了进这一栏的整页）和「…」（系统菜单）。
fn section_head(id: &'static str, title: &'static str, open: Option<Box<dyn Fn(&mut Workspace, &mut Context<Workspace>)>>, more: Option<(&'static str, Vec<super::menu::Item>)>, cx: &mut Context<Workspace>) -> Div {
    let chevron = open.is_some();
    let mut label = div().id(id).h_full().flex().items_center().gap(px(6.)).text_size(px(SECTION_TITLE_SIZE)).font_weight(FontWeight::BOLD).text_color(Theme::text())
        .child(title).when(chevron, |d| d.child(crate::ui::icon("chevron-right", 14., Theme::text_caption())));
    if let Some(open) = open { label = label.on_click(cx.listener(move |w, _, _, cx| open(w, cx))); }
    div().relative().mt(px(16.)).pl(px(24.)).pr(px(12.)).h(px(40.)).flex().items_center()
        .child(label)
        .child(div().flex_1())
        // 「…」是系统菜单（上面盖着透明的原生按钮）。
        .children(more.map(|(key, items)| div().relative().size(px(40.)).flex().items_center().justify_center().child(crate::ui::icon("ellipsis", 20., Theme::text_caption()))
            .child(super::menu::button(key, items))))
}

/// 长按出这一项的菜单，和主页的卡片一样挂在右键上。
fn with_menu(w: &Workspace, row: Stateful<Div>, e: &Entry, cx: &mut Context<Workspace>) -> Stateful<Div> {
    let id = e.id.clone();
    // 菜单开着时这一行浮在菜单上，原处不再留一份。
    // 这一行长按时用自己的键：上面快速访问里同一篇的卡片不跟着藏。
    let key = format!("{}#row", e.id);
    let row = if super::menu::context_hidden(&key) { row.opacity(0.) } else { row };
    row.children(super::row_context_at(w, &e.id, &key, 12.)).on_mouse_down(MouseButton::Right, cx.listener(move |w, ev: &MouseDownEvent, _, cx| {
        w.row_menu = Some(RowMenu { id: id.clone(), pos: ev.position, corner: Anchor::TopLeft, sub: false, batch: false });
        cx.notify();
    }))
}

/// 左上角空间头像的位置：顶栏那一行里竖直居中。
fn avatar_bounds() -> Bounds<Pixels> {
    Bounds::new(point(px(AVATAR_X), super::insets().top + px((crate::app::HEADER_H - AVATAR) / 2.)), size(px(AVATAR), px(AVATAR)))
}

/// 一行的图标：都画在行首那个 22 的格子里、同一个大小（搜索结果的 `entry_icon` 自带 24 的格子和右边距，放进来会偏左、偏小）。
/// 文档没有自己的图标时是一页小缩略图（照 Craft，`Workspace::mini_thumb`）。
fn entry_icon(w: &Workspace, e: &Entry) -> AnyElement {
    match &e.icon {
        _ if e.meta.folder.is_some() => super::folder_icons::glyph(e, 20.).into_any_element(),
        _ if e.meta.markdown => crate::ui::icon("doc-md", 20., Theme::icon()).into_any_element(),
        Some(em) => div().text_size(px(18.)).child(em.clone()).into_any_element(),
        None => w.mini_thumb(e, 20.),
    }
}

/// iCloud 文件夹空间里还没下载（或正在下载）的文档：行尾一行小字（`space_sync::download_label`）。
fn download_label(e: &Entry) -> Option<Div> {
    crate::space_sync::download_label(&e.download).map(|label| div().flex_none().text_size(px(12.)).text_color(Theme::text_caption()).child(label))
}

/// 文件夹那一行，展开着就接着它里面的东西（文件夹也能再展开），`depth` 是缩进的层数。
fn folder_rows(w: &Workspace, e: &Entry, depth: usize, out: &mut Vec<(AnyElement, usize)>, cx: &mut Context<Workspace>) {
    let open_id = e.id.clone();
    let key = if depth == 0 { "folder" } else { "folder-in" };
    let mut r = row(SharedString::from(format!("hub-{key}-{}", e.id)), entry_icon(w, e), display_title(&e.title), None, false);
    if e.meta.folder.is_some() {
        let n = w.entries.iter().filter(|c| c.meta.parent.as_deref() == Some(e.id.as_str())).count();
        let open = expanded(&e.id);
        let flip = e.id.clone();
        r = r.child(div().flex_none().text_size(px(16.)).text_color(Theme::text_caption()).child(n.to_string()))
            // 箭头的点击区比箭头大，伸到行的右边缘；点它不进文件夹。
            .child(div().id(SharedString::from(format!("hub-fold-{}", e.id))).flex_none().h_full().w(px(36.)).mr(px(-12.)).flex().items_center().justify_center()
                .child(crate::ui::icon(if open { "chevron-down" } else { "chevron-right" }, 14., Theme::text_caption()))
                .on_click(cx.listener(move |_, _, _, cx| { cx.stop_propagation(); toggle(&flip); cx.notify(); })));
    }
    let r = with_menu(w, r.children(download_label(e)), e, cx).pl(px(16. + depth as f32 * INDENT))
        .on_click(cx.listener(move |w, _, window, cx| w.open(&open_id, window, cx)));
    out.push((r.into_any_element(), depth));
    if e.meta.folder.is_some() && expanded(&e.id) {
        for c in w.level_entries(Some(&e.id)) { folder_rows(w, &c, depth + 1, out, cx); }
    }
}

pub fn render(w: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let scroll = SCROLL.with(Clone::clone);
    let layout = customise::layout();
    // 最近收藏的在前。
    let mut starred: Vec<Entry> = w.entries.iter().filter(|e| e.meta.folder.is_none() && e.meta.starred > 0).cloned().collect();
    starred.sort_by(|a, b| b.meta.starred.cmp(&a.meta.starred));
    let folders: Vec<Entry> = w.level_entries(None).into_iter().filter(|e| e.meta.folder.is_some()).collect();

    // 快速访问：最上面一排横着滑的卡片，放最近打开的或收藏的文档（「自定义主页」里选）。
    let card_h = CARD_W * (264. / 236.);
    let quick = match layout.quick {
        Quick::Off => None,
        Quick::Starred => Some((starred.iter().take(QUICK_MAX).cloned().collect::<Vec<_>>(), tr("收藏的文档会出现在这里。", "Starred documents show up here."))),
        Quick::Recent => {
            let mut recent: Vec<Entry> = w.entries.iter().filter(|e| e.meta.folder.is_none() && e.meta.opened > 0).cloned().collect();
            recent.sort_by(|a, b| b.meta.opened.cmp(&a.meta.opened));
            recent.truncate(QUICK_MAX);
            Some((recent, tr("最近打开的文档会出现在这里。", "Documents you open show up here.")))
        }
    };
    let quick = quick.map(|(docs, empty)| if docs.is_empty() {
        // 空着时照 Craft 空标签那一块：矮一些，只有一行字。
        div().mx(px(16.)).h(px(98.)).rounded(px(16.)).bg(Theme::fill_hover()).flex().items_center().justify_center()
            .child(div().text_size(px(15.)).text_color(Theme::text_caption()).child(empty))
            .into_any_element()
    } else {
        // 横着拖：里面那一条要有自己的实际宽度（不然它和外框一样宽，卡片只是溢出，滚动区量不出能滚多远）。
        let n = docs.len() as f32;
        let strip = n * CARD_W + (n - 1.).max(0.) * QUICK_GAP + 32.;
        div().id("hub-quick").w_full().overflow_x_scroll().restrict_scroll_to_axis().child(div().w(px(strip)).flex_none().flex().gap(px(QUICK_GAP)).px(px(16.))
            .children(docs.iter().map(|e| div().w(px(CARD_W)).flex_none().child(w.render_home_card(e, card_h, cx)))))
            .into_any_element()
    });
    let open = |e: &Entry, key: &str, count: Option<usize>, cx: &mut Context<Workspace>| {
        let id = e.id.clone();
        with_menu(w, row(SharedString::from(format!("hub-{key}-{}", e.id)), entry_icon(w, e), display_title(&e.title), count, true)
            .children(download_label(e)), e, cx)
            .on_click(cx.listener(move |w, _, window, cx| w.open(&id, window, cx))).into_any_element()
    };

    // 照「自定义主页」的顺序排各栏。
    let mut sections: Vec<AnyElement> = vec![];
    for (section, on) in &layout.sections {
        if !on { continue; }
        match section {
            Section::Folders => {
                let head = section_head("hub-folders", tr("文件夹", "Folders"), Some(Box::new(|_, cx| { super::set_home_page(super::HomePage::Folders); cx.notify(); })), Some(("hub-folders-more", super::hub_menu_items())), cx);
                // 文件夹的箭头可以点：就地展开、收起（照 Craft，IMG_9121）；点行的其余地方照旧进文件夹。
                let mut rows: Vec<(AnyElement, usize)> = vec![];
                if !super::folders_folded() { for e in &folders { folder_rows(w, e, 0, &mut rows, cx); } }
                sections.push(head.into_any_element());
                if !rows.is_empty() { sections.push(indented_group(rows).into_any_element()); }
            }
            Section::Starred => {
                // 照 Craft：标题带箭头，点了进「收藏」整页；右边「…」是收藏文档、自定义主页、折叠收藏。
                sections.push(section_head("hub-starred-head", tr("收藏", "Starred"), Some(Box::new(|_, cx| { super::set_home_page(super::HomePage::Starred); cx.notify(); })), Some(("hub-starred-more", super::hub_starred_items())), cx).into_any_element());
                if super::starred_folded() { continue; }
                let rows: Vec<AnyElement> = starred.iter().map(|e| open(e, "starred", None, cx)).collect();
                sections.push(if rows.is_empty() {
                    div().mx(px(16.)).mt(px(4.)).h(px(64.)).rounded(px(16.)).bg(Theme::fill_hover()).flex().items_center().justify_center()
                        .child(div().text_size(px(15.)).text_color(Theme::text_caption()).child(tr("还没有收藏的文档。", "No starred documents yet."))).into_any_element()
                } else { group(rows).into_any_element() });
            }
        }
    }

    let all = layout.all.then(|| row("hub-all".into(), crate::ui::icon("doc", 20., Theme::icon()), tr("所有文档", "All documents").to_string(), None, false)
        .on_click(cx.listener(|_, _, _, cx| { super::set_home_page(super::HomePage::All); cx.notify(); })));
    // 回收站不在底栏里了：照 Craft 放在主页最后，前面一道通栏的线，右边是篇数。
    // 照 Craft（IMG_9138）：回收站的图标是淡一档的灰（实测约 130/255），不像别的行那样近黑。
    let trash = row("hub-trash".into(), crate::ui::icon("bin", 20., Theme::text_caption()), tr("最近删除", "Recently deleted").to_string(), (!w.trash.is_empty()).then_some(w.trash.len()), false)
        .on_click(cx.listener(|w, _, _, cx| w.show_view(View::Trash, cx)));
    // 照 Craft（IMG_9138 实测）：42pt 高的胶囊，底色比页面只深一点点（247→243），字是浅灰（约 167/255）、中等粗细，不抢眼。
    let customise_button = div().id("hub-customise").mt(px(34.)).mx_auto().h(px(42.)).px(px(19.)).flex().items_center().rounded_full()
        .bg(Theme::fill_hover().opacity(0.4)).active(|s| s.bg(Theme::fill_hover())).text_size(px(16.)).font_weight(FontWeight::MEDIUM).text_color(Theme::text_placeholder()).child(tr("自定义主页", "Customize Home"))
        .on_click(|_, _, _| customise::open());

    let space = crate::space::current(w).map(|s| (s.icon.clone(), crate::space::color(&s.id), crate::space::name(s), s.id.clone()));
    let search_label: SharedString = match &space {
        // 中文里只在西文名字两边空一格（「在 AveMujica 中搜索」「在我的空间中搜索」）。
        Some((_, _, name, _)) if crate::settings::english() => format!("Search in {name}").into(),
        Some((_, _, name, _)) => {
            let pad = |c: Option<char>| if c.is_some_and(|c| c.is_ascii_alphanumeric()) { " " } else { "" };
            format!("在{}{name}{}中搜索", pad(name.chars().next()), pad(name.chars().last())).into()
        }
        None => tr("搜索文档", "Search documents").into(),
    };
    let content = div().w_full().flex().flex_col().pt(px(super::chrome_top() + TOP_GAP)).pb(px(super::dock_bottom()))
        .child(super::large_title(tr("主页", "Home")))
        .child(div().mb(px(20.)).child(super::lead_search(search_label, cx)))
        .children(quick)
        .children(all.map(|r| div().mt(px(12.)).child(group(vec![r.into_any_element()]))))
        .children(sections)
        .child(div().mt(px(24.)).mx(px(20.)).h(px(1.)).bg(crate::ui::list_line()))
        .child(div().mt(px(8.)).child(group(vec![trash.into_any_element()])))
        .child(div().w_full().flex().child(customise_button));
    // 顶栏：左边是当前空间的头像（点了去设置），右边一颗玻璃「…」（导入、自定义）。都钉在顶上，不跟内容滚走。
    // 有顶部玻璃时头像是浮层里那个系统视图（`veil::home_header`），这里只留点击区；没有时（换页滑动、旧系统）照常画。
    // 换页过渡（返回首页的滑动、缩回卡片）里不画：照 Craft，顶上这时还是文档页的返回键，下面不透出头像；
    // 过渡完了系统头像自己长出来（`veil::write_avatar`），也不要 GPUI 这份垫在下面。
    let native = super::veil::avatar_settled() || super::shot::sliding() || super::shot::just_slid();
    let avatar = space.as_ref().map(|(icon, _, _, id)| {
        let b = avatar_bounds();
        div().id("hub-space").absolute().left(b.origin.x).top(b.origin.y).size(b.size.width)
            .when(!native, |d| d.child(super::tilt::element(icon.as_deref(), id, AVATAR, super::tilt::ANGLE))
                // 系统头像还没稳住：接着出帧，等它稳了这份就不画（见 `veil::avatar_settled`）。
                .child(canvas(|_, window, _| if super::veil::on() { window.request_animation_frame() }, |_, _, _, _| {}).absolute().size_0()))
            .on_click(cx.listener(|w, _, _, cx| { super::set_settings_detail(false); w.show_view(View::Settings, cx); }))
    });
    let more = div().absolute().top(super::insets().top + px((crate::app::HEADER_H - 44.) / 2.)).right(px(16.)).size(px(44.))
        .child(super::menu::glass_button("hub-more", "ellipsis", Theme::text(), super::hub_more_items()));
    div().flex_1().min_w_0().h_full().relative()
        // 只认竖着的拖动：不然横着拖那排卡片时，GPUI 会把横向的位移也算成整页往上滚。
        .child(div().id("hub").size_full().overflow_y_scroll().restrict_scroll_to_axis().track_scroll(&scroll).child(content))
        .child(super::veil::home_header(&scroll, tr("主页", "Home").into(), space.map(|(icon, color, _, _)| (avatar_bounds(), icon, color))))
        .children(avatar)
        .child(more)
        .into_any_element()
}

thread_local! { static TRASH: std::cell::RefCell<(Vec<(String, u64)>, Vec<crate::store::Entry>)> = std::cell::RefCell::new(Default::default()); }
/// 回收站里的全部东西（包括跟着文件夹删掉的文档），当成文档库里的条目。按回收站的列表缓存，删、恢复后才重读。
pub fn trash_entries(w: &Workspace) -> Vec<crate::store::Entry> {
    let key: Vec<(String, u64)> = w.trash.iter().map(|t| (t.id.clone(), t.deleted)).collect();
    TRASH.with(|c| {
        let mut c = c.borrow_mut();
        if c.0 != key { *c = (key, w.store.trash_entries()); }
        c.1.clone()
    })
}

pub fn trash_page(w: &Workspace, window: &Window, cx: &mut Context<Workspace>) -> AnyElement {
    // 照 Craft（IMG_9181）：和文件夹页同一套——大标题、搜索、一块说明，下面是同样的卡片网格：删掉的文档是那页纸，
    // 删掉的文件夹是文件夹（露出跟着删掉的文档）。点卡片不打开；长按恢复或彻底删除；右上「…」清空。
    let all = trash_entries(w);
    let roots: Vec<crate::store::Entry> = w.trash.iter().filter_map(|t| all.iter().find(|e| e.id == t.id).cloned()).collect();
    let lead = super::browse_lead_caption(tr("最近删除", "Recently deleted"), tr("文档将保留 30 天。之后，文档将被永久删除。", "Documents are kept for 30 days, then deleted permanently."), cx);
    // 「选择」（照 Craft）：勾选几项，底部是「恢复」和「彻底删除」。
    if w.batch.is_some() && !roots.is_empty() {
        let ids = roots.iter().map(|e| e.id.clone()).collect();
        let n = w.batch.as_ref().map_or(0, |s| s.len());
        let ink = if n > 0 { Theme::text() } else { Theme::text_disabled() };
        let body = w.render_home_grid(&roots, true, Some(lead), window, cx);
        return super::batch_page_with(w, ids, body, vec![("sf:arrow.uturn.backward", ink, "batch-restore"), ("sf:trash", if n > 0 { Theme::danger() } else { ink }, "batch-purge")], |w, _, cx| w.show_view(View::Home, cx), cx);
    }
    let body = if roots.is_empty() { super::browse_empty(lead, tr("没有最近删除的文档", "No recently deleted documents")) } else { w.render_home_grid(&roots, true, Some(lead), window, cx) };
    let mut items = vec![];
    if !w.trash.is_empty() { items.push(super::menu::Item::new("trash-empty-all", tr("清空回收站", "Empty Trash"), "trash", 0).danger()); }
    // 和文件夹页同一排：左边返回，右边「选择」和「…」。
    let chrome = if w.trash.is_empty() {
        div().absolute().top(super::insets().top).left_0().right_0().h(px(crate::app::HEADER_H)).px(px(16.)).flex().items_center()
            .child(super::glass_button("trash-back", "editor-icons/chevron-left", 22.).on_click(cx.listener(|w, _, _, cx| w.show_view(View::Home, cx)))).into_any_element()
    } else {
        super::browse_chrome_with(w, items, cx, |w, _, cx| w.show_view(View::Home, cx)).into_any_element()
    };
    super::browse_page(body, chrome)
}
