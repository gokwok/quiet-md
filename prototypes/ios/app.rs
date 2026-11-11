use std::time::Duration;

use gpui::{prelude::FluentBuilder as _, *};

use crate::app::{View, Workspace};
use crate::theme::Theme;

pub mod glass;
pub mod hub;
pub mod keybar;
pub mod menu;

pub fn run() {
    crate::logging::init();
    // 测试传相对路径：放进沙盒的临时目录。真实文档库在应用的 Documents 里。
    if let Some(home) = std::env::var_os("QUIET_MD_HOME") && std::path::Path::new(&home).is_relative() {
        // SAFETY: 还没有别的线程。
        unsafe { std::env::set_var("QUIET_MD_HOME", std::env::temp_dir().join(home)); }
    }
    let root = crate::store::Store::root_dir();
    crate::logging::write(&format!("library {}", root.display()));
    // 冒烟测试用一篇固定的样例笔记，不放「开始指南」。
    if !sample_library() && let Err(err) = crate::guides::initialize(&root) {
        crate::logging::fatal(&format!("无法初始化文档库 / Could not set up the library at {}", root.display()), &err);
    }
    gpui_platform::application().with_assets(crate::assets::Assets).run(|cx| {
        crate::editor::math::load_fonts(cx);
        crate::editor::image::init_pictures(cx);
        crate::input::bind_keys(cx);
        crate::editor::keymap::bind_keys(cx);
        crate::editor::image::bind_keys(cx);
        crate::app::bind_keys(cx);
        crate::settings::load();
        crate::settings::apply_appearance(&crate::settings::prefs().appearance);
        crate::settings::sync_dark(cx);
        crate::themes::load(&crate::settings::prefs().theme_id(), false, || cx.text_system().all_font_names());
        crate::shortcuts::init(cx);
        match crate::shared_store::SharedStore::open() {
            Ok(mut s) => {
                if sample_library() && s.list().is_empty() {
                    let doc = crate::docxml::parse(include_str!("sample.xml")).expect("parse sample note");
                    s.create_from(&doc).expect("seed sample note");
                }
                cx.set_global(s)
            }
            Err(err) => crate::logging::fatal("无法打开文档库 / Could not open the library", &err),
        }
        // 系统随时可能结束后台应用：一进后台就把没写完的改动落盘。
        cx.on_app_lifecycle(|phase, cx| {
            if phase != AppLifecyclePhase::Background { return; }
            for handle in cx.windows() {
                if let Some(handle) = handle.downcast::<Workspace>() { let _ = handle.update(cx, |w, _, cx| { w.flush(cx); }); }
            }
        }).detach();
        cx.open_window(WindowOptions::default(), |window, cx| cx.new(|cx| crate::app::Workspace::new(window, cx))).expect("open iOS window");
    });
}

/// `QUIET_MD_IOS_SAMPLE`：文档库里只放 sample.xml，并直接打开它（冒烟测试）。
pub fn sample_library() -> bool { std::env::var_os("QUIET_MD_IOS_SAMPLE").is_some() }
/// `QUIET_MD_IOS_VERIFY_BACKGROUND`：关掉定时保存，只剩进后台时的那一次。
pub fn background_save_only() -> bool { std::env::var_os("QUIET_MD_IOS_VERIFY_BACKGROUND").is_some() }
/// 手机上从主页开始；宽屏和冒烟测试直接进最近的文档。
pub fn starts_in_document(window: &Window) -> bool { !compact(window) || sample_library() }

/// 手机宽度：单栏布局，侧栏变成抽屉。iPad 横竖屏都够宽，保持两栏。
pub fn compact(window: &Window) -> bool { f32::from(window.viewport_size().width) < 700. }

/// 内容要避开的四边：刘海、Home 指示条，以及弹出时的软键盘。
pub fn safe(window: &Window) -> Edges<Pixels> { window.insets().effective() }

/// 软键盘是否弹出。
pub fn keyboard_up(window: &Window) -> bool { window.insets().ime.bottom > px(0.) }

/// 浮层能用到的最低处：窗口高度减去 Home 指示条或键盘，键盘弹出时再减去工具条。
pub fn visible_height(window: &Window) -> Pixels {
    let bar = if keyboard_up(window) { px(keybar::HEIGHT) } else { px(0.) };
    window.viewport_size().height - safe(window).bottom - bar
}

thread_local! { static LAYOUT: std::cell::Cell<(bool, Edges<Pixels>, Size<Pixels>)> = const { std::cell::Cell::new((false, Edges { top: px(0.), right: px(0.), bottom: px(0.), left: px(0.) }, Size { width: px(0.), height: px(0.) })) }; }
/// 这一帧的版式，给拿不到 `Window` 的渲染函数用（对话框多半只有 `cx`）。
pub fn set_layout(compact: bool, safe: Edges<Pixels>, viewport: Size<Pixels>) {
    LAYOUT.with(|l| l.set((compact, safe, viewport)));
    // 状态栏的字跟着应用的深浅色走（应用可以不跟随系统）。
    thread_local! { static BAR_DARK: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) }; }
    let dark = crate::theme::dark();
    if BAR_DARK.with(|b| b.replace(Some(dark))) != Some(dark) {
        gpui_ios::ios::set_status_bar_style(if dark { gpui_ios::StatusBarContentStyle::Light } else { gpui_ios::StatusBarContentStyle::Dark });
    }
}
pub fn is_compact() -> bool { LAYOUT.with(|l| l.get().0) }
pub fn insets() -> Edges<Pixels> { LAYOUT.with(|l| l.get().1) }

/// 手机上菜单不挂在按钮或手指旁边，而是从底部升起的动作面板：拇指够得着，也不会被手挡住。
/// 宽屏照原样锚在 `pos`。
pub fn place(corner: Anchor, pos: Point<Pixels>) -> Sheet {
    let (compact, safe, viewport) = LAYOUT.with(|l| l.get());
    if !compact { return Sheet { at: anchored().anchor(corner).position(pos).snap_to_window_with_margin(px(8.)), rise: false, kids: vec![] }; }
    Sheet { at: anchored().anchor(Anchor::BottomCenter).position(point(viewport.width / 2., viewport.height - safe.bottom - px(8.))), rise: true, kids: vec![] }
}
/// `place` 给出的浮层。手机上的动作面板出现时从下面升上来。
pub struct Sheet { at: Anchored, rise: bool, kids: Vec<AnyElement> }
impl ParentElement for Sheet {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) { self.kids.extend(elements); }
}
impl IntoElement for Sheet {
    type Element = Anchored;
    fn into_element(self) -> Anchored {
        if !self.rise { return self.at.children(self.kids); }
        self.at.child(div().relative().children(self.kids).with_animation("sheet-rise", Animation::new(Duration::from_millis(260)).with_easing(ease_out_quint()), |d, t| d.top(px((1. - t) * 44.)).opacity(0.3 + 0.7 * t)))
    }
}

thread_local! { static PAGE: std::cell::Cell<(Option<View>, usize, f32)> = const { std::cell::Cell::new((None, 0, 0.)) }; }
/// 手机上换页的过渡：进文档、进文件夹从右边滑入，返回从左边滑入，标签页之间只淡入。
pub fn page(main: AnyElement, view: View) -> AnyElement {
    let depth = |v: View| -> i32 { match v { View::Doc => 2, View::Folder => 1, _ => 0 } };
    let (epoch, dir) = PAGE.with(|p| {
        let (last, mut epoch, mut dir) = p.get();
        if last != Some(view) {
            if let Some(last) = last { epoch += 1; dir = (depth(view) - depth(last)).signum() as f32; }
            p.set((Some(view), epoch, dir));
        }
        (epoch, dir)
    });
    let page = div().flex_1().min_w_0().h_full().flex().relative().child(main);
    if !is_compact() || epoch == 0 { return page.into_any_element(); }
    page.with_animation(("page", epoch), Animation::new(Duration::from_millis(300)).with_easing(ease_out_quint()), move |d, t| d.left(px(dir * (1. - t) * 72.)).opacity(t)).into_any_element()
}

/// 跟着滚动走的一块：在绘制阶段按滚动的偏移摆放。偏移在渲染之后还会变（惯性滚动是在
/// 列表自己布局时结算的），渲染时读它来定位会比列表慢一帧，快速滑动时看起来在抖。
pub struct Follow { handle: ScrollHandle, child: AnyElement }
pub fn follow(handle: &ScrollHandle, child: impl IntoElement) -> Follow { Follow { handle: handle.clone(), child: child.into_any_element() } }
impl IntoElement for Follow { type Element = Self; fn into_element(self) -> Self { self } }
impl Element for Follow {
    type RequestLayoutState = ();
    type PrepaintState = ();
    fn id(&self) -> Option<ElementId> { None }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> { None }
    fn request_layout(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, window: &mut Window, cx: &mut App) -> (LayoutId, ()) { (self.child.request_layout(window, cx), ()) }
    fn prepaint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, _: Bounds<Pixels>, _: &mut (), window: &mut Window, cx: &mut App) {
        let y = self.handle.offset().y;
        window.with_element_offset(point(px(0.), y), |window| self.child.prepaint(window, cx));
    }
    fn paint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, _: Bounds<Pixels>, _: &mut (), _: &mut (), window: &mut Window, cx: &mut App) { self.child.paint(window, cx); }
}

thread_local! { static SETTINGS_DETAIL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
/// 手机上设置是两级：先是一页分类的列表，点进去才是那一类的设置。这里记着现在在不在第二级。
pub fn settings_detail() -> bool { SETTINGS_DETAIL.with(|d| d.get()) }
/// 进出第二级，带上换页的滑入。
pub fn set_settings_detail(on: bool) {
    if SETTINGS_DETAIL.with(|d| d.replace(on)) != on { PAGE.with(|p| { let (view, epoch, _) = p.get(); p.set((view, epoch + 1, if on { 1. } else { -1. })); }); }
}

thread_local! { static KEYBOARD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
/// 每帧记下软键盘是否弹出，给拿不到窗口的地方用。
pub fn set_keyboard(up: bool) { KEYBOARD.with(|k| k.set(up)); }
pub fn keyboard_shown() -> bool { KEYBOARD.with(|k| k.get()) }

/// 打字时文档页右上角的「完成」：一块主色的玻璃，点了收起键盘。
pub fn done_button() -> Stateful<Div> {
    div().id("doc-done").flex_none().size(px(44.)).rounded_full().occlude()
        .child(glass::pane("doc-done", Some(Theme::primary()), Some(("check", 20., gpui::white())), None))
        .on_mouse_down(MouseButton::Left, |_, _, _| glass::pulse("doc-done"))
}
/// 底部面板贴底居中的那个点（窗口坐标）。
pub fn sheet_point() -> Point<Pixels> {
    let (_, safe, viewport) = LAYOUT.with(|l| l.get());
    point(viewport.width / 2., viewport.height - safe.bottom - px(8.))
}
/// 动作面板占满屏宽（两侧各留 8）；宽屏用菜单自己的宽度。
pub fn menu_w(wide: f32) -> Pixels {
    let (compact, _, viewport) = LAYOUT.with(|l| l.get());
    if compact { viewport.width - px(16.) } else { px(wide) }
}
/// 动作面板下面压暗的一层。不接事件：点面板外关闭菜单仍由原来的处理负责。
/// 深色下 `ui::scrim` 会跟着主题反相（发白），这里用不随主题变的黑。
pub fn sheet_scrim() -> impl IntoElement {
    div().absolute().inset_0().bg(crate::theme::rgbaf(0, 0, 0, if crate::theme::dark() { 0.5 } else { 0.25 }))
        .with_animation("sheet-scrim", Animation::new(Duration::from_millis(220)), |d, t| d.opacity(t))
}

/// 手机上有 Liquid Glass 时，外壳的按钮是悬浮在内容之上的玻璃，内容从它们下面滚过。
pub fn floating() -> bool { is_compact() && glass::supported() }

/// 悬浮的玻璃圆钮：玻璃和图标是原生视图，这里只留 44pt 的点击区，按下时让玻璃缩一下。
pub fn glass_button(id: &'static str, icon: &'static str, size: f32) -> Stateful<Div> {
    div().id(id).flex_none().size(px(44.)).rounded_full().occlude()
        .child(glass::pane(id, None, Some((icon, size, Theme::text())), None))
        .on_mouse_down(MouseButton::Left, move |_, _, _| glass::pulse(id))
}

/// 文档页悬浮的那排按钮占的高度（状态栏 + 一行按钮）：正文的开头让到它下面，滚动时从它底下穿过。
pub fn chrome_top() -> f32 { if floating() { f32::from(insets().top) + crate::app::HEADER_H } else { 0. } }

/// 状态栏下面的一道渐隐：正文滚到状态栏底下时，时间和电量仍然看得清。
pub fn top_fade(page: Hsla) -> impl IntoElement {
    div().absolute().top_0().left_0().right_0().h(insets().top + px(28.))
        .bg(linear_gradient(180., linear_color_stop(page, 0.55), linear_color_stop(page.opacity(0.), 1.)))
}

/// 主页底部那排玻璃离屏幕下沿的距离（Home 指示条之上），和列表末尾为它们留出的高度。
fn dock_inset() -> Pixels { insets().bottom.max(px(12.)) + px(4.) }
pub fn dock_bottom() -> f32 { f32::from(dock_inset()) + 72. }

/// 主页的大标题和它下面的工具行一共多高（含悬浮按钮那一段）：网格开头留出这么多。
pub fn home_lead_h() -> f32 { chrome_top() + 56. + LEAD_SEARCH_H + 12. }
/// 大标题下面那条搜索占的高度。
pub const LEAD_SEARCH_H: f32 = 52.;
/// 大标题下面的搜索条，和首页的是同一条。
pub fn lead_search(label: &'static str, cx: &mut Context<Workspace>) -> impl IntoElement { div().h(px(LEAD_SEARCH_H)).pt(px(4.)).child(search_bar_with(label, cx)) }
/// 主页的底色。悬浮版式下是窗口的浅灰，白色的卡片从上面浮出来；否则照旧是页面的白。
pub fn home_bg() -> Hsla { if floating() { Theme::window() } else { Theme::bg_body() } }
/// 跟着内容滚走的大标题。
pub fn large_title(title: impl Into<SharedString>) -> impl IntoElement {
    div().h(px(56.)).px(px(24.)).flex().items_center().text_size(px(30.)).line_height(px(38.)).font_weight(FontWeight::BOLD).text_color(Theme::text())
        .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(title.into()))
}

/// 手机上主页分三页：首页（`hub`）、「所有文档」和「文件夹」（顶层的文件夹）。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HomePage { Hub, All, Folders }
thread_local! { static HOME_PAGE: std::cell::Cell<HomePage> = const { std::cell::Cell::new(HomePage::Hub) }; }
pub fn home_page() -> HomePage { HOME_PAGE.with(|p| p.get()) }
/// 换页，带上滑入：回首页从左边来，往里走从右边来。
pub fn set_home_page(page: HomePage) {
    if HOME_PAGE.with(|p| p.replace(page)) != page { PAGE.with(|p| { let (view, epoch, _) = p.get(); p.set((view, epoch + 1, if page == HomePage::Hub { -1. } else { 1. })); }); }
}

/// 浏览页（所有文档、文件夹、某个文件夹里）是同一套：左上返回，右上「选择」（进批量管理）和「…」
/// （新建文件夹、显示方式、排序），大标题和搜索跟着内容滚走。
pub fn browse_chrome(w: &Workspace, cx: &mut Context<Workspace>, back: impl Fn(&mut Workspace, &mut Window, &mut Context<Workspace>) + 'static) -> impl IntoElement {
    let select = crate::settings::tr("选择", "Select");
    div().absolute().top(insets().top).left_0().right_0().h(px(crate::app::HEADER_H)).px(px(16.)).flex().items_center().gap(px(10.))
        .child(glass_button("browse-back", "editor-icons/chevron-left", 22.).on_click(cx.listener(move |w, _, window, cx| back(w, window, cx))))
        .child(div().flex_1())
        .child(div().id("browse-select").flex_none().h(px(44.)).w(px(if crate::settings::english() { 82. } else { 64. })).rounded_full().occlude()
            .child(glass::pane("browse-select", None, None, Some((select.into(), Theme::text()))))
            .on_mouse_down(MouseButton::Left, |_, _, _| glass::pulse("browse-select"))
            .on_click(cx.listener(|w, _, _, cx| { w.batch = Some(vec![]); w.row_menu = None; w.home.sort_menu = false; cx.notify(); })))
        // 「…」是一颗系统的玻璃钮，挂着系统菜单。
        .child(div().relative().flex_none().size(px(44.)).child(menu::glass_button("browse-options", "ellipsis", Theme::text(), browse_menu_items(w))))
}
/// 浏览页「…」的系统菜单：新建文件夹（所有文档是平铺的，没有）、显示方式、排序。
fn browse_menu_items(w: &Workspace) -> Vec<menu::Item> {
    const SORTS: [&str; 5] = ["browse-sort-0", "browse-sort-1", "browse-sort-2", "browse-sort-3", "browse-sort-4"];
    let mut items = vec![];
    if w.view == View::Folder || home_page() == HomePage::Folders {
        items.push(menu::Item { id: "browse-new-folder", title: crate::settings::tr("新建文件夹", "New folder").into(), symbol: "folder.badge.plus", checked: false, group: 0 });
    }
    items.push(menu::Item { id: "browse-grid", title: crate::settings::tr("宫格视图", "Grid").into(), symbol: "square.grid.2x2", checked: w.home.grid, group: 1 });
    items.push(menu::Item { id: "browse-list", title: crate::settings::tr("列表视图", "List").into(), symbol: "list.bullet", checked: !w.home.grid, group: 1 });
    items.extend(w.sort_choices().into_iter().zip(SORTS).map(|((title, checked), id)| menu::Item { id, title: title.into(), symbol: "", checked, group: 2 }));
    items
}
/// 浏览页开头跟着内容滚走的一段：大标题和搜索。
pub fn browse_lead(title: impl Into<SharedString>, cx: &mut Context<Workspace>) -> AnyElement {
    div().pt(px(chrome_top())).child(large_title(title)).child(lead_search(crate::settings::tr("搜索", "Search"), cx)).into_any_element()
}
/// 浏览页的骨架：内容在下，状态栏的渐隐和那排玻璃在上。
pub fn browse_page(body: AnyElement, chrome: impl IntoElement) -> AnyElement {
    div().flex_1().min_w_0().size_full().flex().flex_col().relative().child(body).child(top_fade(home_bg())).child(chrome).into_any_element()
}
/// 没有内容的浏览页：开头那一段照旧，下面一句话。
pub fn browse_empty(lead: AnyElement, text: &'static str) -> AnyElement {
    div().flex_1().min_h_0().flex().flex_col().child(lead).child(div().pt(px(80.)).text_center().text_size(px(15.)).text_color(Theme::text_caption()).child(text)).into_any_element()
}
/// 批量管理时的浏览页：顶上换成一行固定的工具条（全选、已选几项、创建副本、移动到、删除、完成）。
pub fn batch_page(w: &Workspace, ids: Vec<String>, body: AnyElement, cx: &mut Context<Workspace>) -> AnyElement {
    use crate::sidebar::{Check, checkbox};
    let n = w.batch.as_ref().map_or(0, |s| s.len());
    let all = n > 0 && w.batch.as_ref().is_some_and(|s| ids.iter().all(|id| s.contains(id)));
    let act = |id: &'static str, glyph: &'static str| div().id(id).flex_none().size(px(44.)).flex().items_center().justify_center().rounded(px(10.))
        .child(crate::ui::icon(glyph, 20., if n > 0 { Theme::icon() } else { Theme::text_disabled() }));
    let bar = div().h(px(crate::app::HEADER_H)).flex_none().pl(px(12.)).pr(px(8.)).flex().items_center()
        .child(div().id("batch-all").flex_none().size(px(44.)).flex().items_center().justify_center().group("batch-all")
            .child(checkbox(if all { Check::On } else if n > 0 { Check::Partial } else { Check::Off }, false, "batch-all"))
            .on_click(cx.listener(move |w, _, _, cx| { w.batch = Some(if all { vec![] } else { ids.clone() }); cx.notify(); })))
        .child(div().flex_1().min_w_0().text_size(px(16.)).child(crate::localized_format!("已选 {n} 项", "Selected: {n}")))
        .child(act("batch-copy", "copy").when(n > 0, |d| d.active(|s| s.bg(Theme::fill_pressed())).on_click(cx.listener(|w, _, _, cx| w.batch_copy(cx)))))
        .child(act("batch-move", "folder-input").when(n > 0, |d| d.active(|s| s.bg(Theme::fill_pressed())).on_click(cx.listener(|w, _, window, cx| w.batch_move(window, cx)))))
        .child(act("batch-delete", "bin").when(n > 0, |d| d.active(|s| s.bg(Theme::fill_pressed())).on_click(cx.listener(|w, _, _, cx| w.ask_batch_delete(cx)))))
        .child(div().id("batch-done").flex_none().h(px(44.)).px(px(10.)).flex().items_center().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).text_color(Theme::primary())
            .child(crate::settings::tr("完成", "Done")).on_click(cx.listener(|w, _, _, cx| { w.batch = None; w.row_menu = None; cx.notify(); })));
    div().flex_1().min_w_0().size_full().flex().flex_col().pt(insets().top).pb(insets().bottom).child(bar).child(body).into_any_element()
}

thread_local! { static FOLDERS_FOLDED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
/// 首页的文件夹一栏折叠着没有（只在这次运行里记着）。
pub fn folders_folded() -> bool { FOLDERS_FOLDED.with(|f| f.get()) }
/// 这里自己的底部面板有没有开着（开着时玻璃让开）。
pub fn sheet_open() -> bool { icon_picking() }
/// 首页「文件夹」一栏右边「…」的系统菜单里有什么。
pub fn hub_menu_items() -> Vec<menu::Item> {
    let folded = folders_folded();
    vec![
        menu::Item { id: "hub-new-folder", title: crate::settings::tr("新建文件夹", "New folder").into(), symbol: "folder.badge.plus", checked: false, group: 0 },
        menu::Item { id: "hub-fold", title: if folded { crate::settings::tr("展开文件夹", "Expand folders") } else { crate::settings::tr("折叠文件夹", "Collapse folders") }.into(), symbol: if folded { "arrow.up.left.and.arrow.down.right" } else { "arrow.down.right.and.arrow.up.left" }, checked: false, group: 0 },
    ]
}
/// 系统菜单里的一项被点了（`menu.rs` 把它送回工作区）。
pub fn menu_action(id: &str, w: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    match id {
        "hub-new-folder" => w.new_folder(None, window, cx),
        "hub-fold" => { FOLDERS_FOLDED.with(|f| f.set(!f.get())); cx.notify(); }
        "browse-new-folder" => { let parent = if w.view == View::Folder { w.folder_current.clone() } else { None }; w.new_folder(parent, window, cx); }
        "browse-grid" => w.set_home_grid(true, cx),
        "browse-list" => w.set_home_grid(false, cx),
        // 底栏和新建按钮：玻璃自己接的触摸。
        "tab-home" => { set_home_page(HomePage::Hub); if w.view != View::Home { w.show_view(View::Home, cx) } else { cx.notify() } }
        "tab-trash" => if w.view != View::Trash { w.show_view(View::Trash, cx) },
        "tab-settings" => { set_settings_detail(false); if w.view != View::Settings { w.show_view(View::Settings, cx) } }
        "new-doc" => { glass::haptic(); w.new_doc(window, cx); }
        _ if id.starts_with("browse-sort-") => if let Ok(index) = id["browse-sort-".len()..].parse() { w.pick_home_sort(index, cx); },
        _ => {}
    }
}

/// 手机上的重命名：没有侧栏，名字在屏幕上方的一张卡片里改。回车或点别处保存。
pub fn rename_dialog(w: &Workspace, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    if !floating() { return None; }
    let field = w.rename.as_ref()?.field.clone();
    Some(div().id("rename-scrim").absolute().inset_0().occlude().on_click(cx.listener(|w, _, window, cx| window.focus(&w.focus, cx)))
        .child(sheet_scrim())
        .child(div().absolute().top(insets().top + px(72.)).left(px(16.)).right(px(16.)).child(
            crate::editor_ui::popover().id("rename-dialog").occlude().p(px(16.)).flex().flex_col().gap(px(12.))
                .child(div().text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).child(crate::settings::tr("重命名", "Rename")))
                .child(div().h(px(44.)).px(px(12.)).flex().items_center().rounded(px(10.)).bg(Theme::fill_hover()).text_size(px(16.)).child(field))))
        .into_any_element())
}

thread_local! { static ICON_FOR: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) }; }
/// 正在给哪个文件夹挑图标。
pub fn pick_icon(id: Option<String>) { ICON_FOR.with(|i| *i.borrow_mut() = id); }
pub fn icon_picking() -> bool { ICON_FOR.with(|i| i.borrow().is_some()) }

/// 文件夹可选的图标：一小组常用的 emoji。
const FOLDER_ICONS: [&str; 30] = ["📁", "📂", "🗂️", "📚", "📖", "📝", "✏️", "📌", "⭐", "❤️", "💡", "🎯", "🚀", "🧪", "🛠️", "💼", "🏠", "🎓", "🎨", "🎵", "🎬", "📷", "✈️", "🌱", "🍀", "☕", "🍳", "🏃", "💰", "🔒"];

/// 文件夹的图标面板：从底部升起，一排六个；最下面一行拿掉图标。
pub fn icon_picker(cx: &mut Context<Workspace>) -> Option<AnyElement> {
    let id = ICON_FOR.with(|i| i.borrow().clone())?;
    let set = |icon: Option<String>, id: String| move |w: &mut Workspace, _: &ClickEvent, _: &mut Window, cx: &mut Context<Workspace>| {
        match w.store.set_folder_icon(&id, icon.clone()) {
            Ok(()) => { w.entries = w.store.list(); w.sort(); }
            Err(e) => w.show_toast_kind(crate::ui::ToastKind::Error, &e.to_string(), cx),
        }
        pick_icon(None);
        cx.notify();
    };
    let cell = f32::from(menu_w(0.) - px(24.)) / 6.;
    let grid = div().px(px(12.)).py(px(8.)).flex().flex_wrap().children(FOLDER_ICONS.iter().enumerate().map(|(i, em)|
        div().id(("folder-icon", i)).w(px(cell.floor())).h(px(48.)).flex().items_center().justify_center().rounded(px(10.)).text_size(px(26.))
            .active(|s| s.bg(Theme::fill_pressed())).child(*em).on_click(cx.listener(set(Some(em.to_string()), id.clone())))));
    let panel = crate::editor_ui::popover().id("folder-icons").occlude().w(menu_w(0.)).py(px(crate::ui::MENU_PAD_Y))
        .on_mouse_down_out(cx.listener(|_, _, _, cx| { pick_icon(None); cx.notify(); }))
        .child(div().px(px(20.)).pt(px(8.)).pb(px(4.)).text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(Theme::text_caption()).child(crate::settings::tr("文件夹图标", "Folder icon")))
        .child(grid)
        .child(crate::ui::menu_row("folder-icon-none", SharedString::from("folder-icon-none")).child(crate::ui::icon("folder", 16., Theme::icon())).child(crate::settings::tr("使用默认图标", "Use the default icon"))
            .on_click(cx.listener(set(None, id.clone()))));
    Some(div().absolute().inset_0().child(sheet_scrim()).child(deferred(place(Anchor::BottomCenter, point(px(0.), px(0.))).child(panel)).with_priority(3)).into_any_element())
}

/// 底部的玻璃标签栏：主页、回收站、设置，当前页那格垫着底色。右边是新建按钮。
/// 搜索不在这里：它是各页大标题下面的那一条。
pub fn tab_bar(view: View) -> impl IntoElement {
    // 文件夹是主页里的一层，仍算在主页那格。
    let picked = match view { View::Trash => 1, View::Settings => 2, _ => 0 };
    let tabs = ["home", "bin", "settings-gear"].iter().enumerate().map(|(i, name)| (*name, 22., if i == picked { Theme::primary() } else { Theme::text() })).collect();
    // 玻璃自己接触摸（按下的反馈是系统的），各格点了走 `menu_action`；这里只占住位置。
    div().id("tab-bar").absolute().left(px(20.)).w(px(204.)).bottom(dock_inset()).h(px(48.)).rounded_full().occlude()
        .child(glass::tab_pane("tab-bar", tabs, Some((picked, Theme::fill_hover_strong())), vec!["tab-home", "tab-trash", "tab-settings"]))
}
/// 有标签栏的页面，内容的下沿让到它上面。
pub fn tab_bar_h() -> Pixels { dock_inset() + px(56.) }

/// 大标题下的搜索条：点了打开搜索。
pub fn search_bar(cx: &mut Context<Workspace>) -> impl IntoElement { search_bar_with(crate::settings::tr("搜索文档", "Search documents"), cx) }
pub fn search_bar_with(label: &'static str, cx: &mut Context<Workspace>) -> impl IntoElement {
    div().id("home-search-bar").flex_none().mx(px(20.)).mt(px(4.)).h(px(40.)).px(px(12.)).flex().items_center().gap(px(8.))
        .rounded(px(12.)).bg(Theme::fill_hover()).active(|s| s.bg(Theme::fill_pressed()))
        .child(crate::ui::icon("search", 17., Theme::text_caption()))
        .child(div().text_size(px(16.)).text_color(Theme::text_caption()).child(label))
        .on_click(cx.listener(|w, _, window, cx| w.open_search(window, cx)))
}

/// 右下角的悬浮新建按钮，在拇指的位置。有 Liquid Glass 时是和搜索并排的一块玻璃，GPUI 这边只留点击区。
pub fn new_button(cx: &mut Context<Workspace>) -> impl IntoElement {
    div().id("home-fab").absolute().right(px(20.)).rounded_full().flex().items_center().justify_center()
        .map(|d| if floating() { d.bottom(dock_inset()).size(px(48.)).child(glass::tap_pane("fab", ("plus", 24., Theme::text()), "new-doc")) }
            else { d.bottom(px(16.)).size(px(56.)).bg(Theme::primary()).shadow(crate::theme::shadow_dialog()).active(|s| s.bg(crate::theme::c(crate::theme::craft::B700))).child(crate::ui::icon("plus", 26., gpui::white())) })
        .occlude().on_click(cx.listener(|w, _, window, cx| w.new_doc(window, cx)))
}

thread_local! { static OVERLAY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
/// 正文之上盖着别的东西（抽屉、搜索、菜单、对话框），或当前不在文档页：长按不该选正文里的字。
pub fn set_overlay(on: bool) { OVERLAY.with(|o| o.set(on)); }
pub fn overlay() -> bool { OVERLAY.with(|o| o.get()) }

/// 长按当右键：侧栏的行、主页的卡片和回收站的行，菜单原本都挂在右键上。
pub fn long_press_layer() -> impl IntoElement {
    canvas(|_, _, _| {}, |_, _, window, _| {
        // 手指抬起后没有「悬停」：把指针移出窗口，免得刚点过的行一直亮着。
        window.on_mouse_event(|ev: &MouseUpEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble || ev.button != MouseButton::Left { return; }
            window.defer(cx, |window, cx| {
                window.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent { position: point(px(-100.), px(-100.)), pressed_button: None, modifiers: Modifiers::default() }), cx);
            });
        });
        // 长按结束、手指抬起时，手势识别还会补一次点按（左键按下加松开）：在正文之外它会被当成点击（打开文档）。
        // 长按开始时记下「各吞一次」，十秒内有效；结束阶段的事件不一定送到这里，所以不依赖它。
        thread_local! { static EAT: std::cell::Cell<(bool, bool, Option<std::time::Instant>)> = const { std::cell::Cell::new((false, false, None)) }; }
        fn live(at: Option<std::time::Instant>) -> bool { at.is_some_and(|t| t.elapsed() < Duration::from_secs(10)) }
        window.on_mouse_event(|ev: &MouseDownEvent, phase, _, cx| {
            if phase != DispatchPhase::Capture || ev.button != MouseButton::Left { return; }
            let (down, up, at) = EAT.with(|e| e.get());
            if down && live(at) { EAT.with(|e| e.set((false, up, at))); cx.stop_propagation(); }
        });
        window.on_mouse_event(|ev: &MouseUpEvent, phase, _, cx| {
            if phase != DispatchPhase::Capture || ev.button != MouseButton::Left { return; }
            let (_, up, at) = EAT.with(|e| e.get());
            if up && live(at) { EAT.with(|e| e.set((false, false, None))); cx.stop_propagation(); }
        });
        window.on_mouse_event(|ev: &LongPressEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble || ev.phase != TouchPhase::Started { return; }
            if overlay() { EAT.with(|e| e.set((true, true, Some(std::time::Instant::now())))); }
            glass::haptic();
            let position = ev.position;
            // 当前事件还在分发，右键放到它结束之后。
            window.defer(cx, move |window, cx| {
                let modifiers = Modifiers::default();
                window.dispatch_event(PlatformInput::MouseDown(MouseDownEvent { button: MouseButton::Right, position, modifiers, click_count: 1, first_mouse: false }), cx);
                window.dispatch_event(PlatformInput::MouseUp(MouseUpEvent { button: MouseButton::Right, position, modifiers, click_count: 1 }), cx);
            });
        });
    }).absolute().size_0()
}

/// 左缘右滑：文档、文件夹和回收站回主页，设置返回，主页拉出抽屉。
pub fn back(w: &mut Workspace, ev: &BackGestureEvent, window: &mut Window, cx: &mut Context<Workspace>) {
    if ev.phase != BackGesturePhase::Committed { return; }
    if w.search_open { return w.close_search(window, cx); }
    match w.view {
        View::Doc | View::Folder | View::Trash => w.show_view(View::Home, cx),
        View::Home if home_page() != HomePage::Hub => { set_home_page(HomePage::Hub); cx.notify(); }
        View::Settings if settings_detail() => { set_settings_detail(false); cx.notify(); }
        View::Settings => w.close_settings(cx),
        // 有标签栏时主页不拉抽屉。
        View::Home => { w.drawer = compact(window) && !floating(); cx.notify(); }
    }
}

/// 系统的文件选择器在 gpui-pre-ios 0.3.12 里还没有实现，调用只会返回错误。
/// 所有选文件、存文件的入口都经过这两个函数：先如实告诉使用者，日后接上原生选择器也只改这里。
pub fn prompt_for_paths(cx: &mut App, options: PathPromptOptions) -> impl Future<Output = Result<anyhow::Result<Option<Vec<std::path::PathBuf>>>, impl std::fmt::Debug + use<>>> + use<> {
    files_unavailable(cx);
    cx.prompt_for_paths(options)
}

pub fn prompt_for_new_path(cx: &mut App, directory: &std::path::Path, name: Option<&str>) -> impl Future<Output = Result<anyhow::Result<Option<std::path::PathBuf>>, impl std::fmt::Debug + use<>>> + use<> {
    files_unavailable(cx);
    cx.prompt_for_new_path(directory, name)
}

fn files_unavailable(cx: &mut App) {
    let Some(handle) = cx.windows().first().copied() else { return };
    cx.defer(move |cx| {
        let _ = handle.update(cx, |_, window, cx| {
            let title = crate::settings::tr("iOS 版暂不支持选择文件", "Choosing files is not available on iOS yet");
            let detail = crate::settings::tr("导入、导出和插入文件暂不可用。", "Import, export and file insertion are unavailable.");
            drop(window.prompt(PromptLevel::Info, title, Some(detail), &[crate::settings::tr("好", "OK")], cx));
        });
    });
}

/// 抽屉：侧栏从左侧滑入盖在内容上，点右侧的遮罩收起。
pub fn drawer(sidebar: AnyElement, safe: Edges<Pixels>, cx: &mut Context<Workspace>) -> AnyElement {
    let slide = Animation::new(Duration::from_millis(220)).with_easing(ease_out_quint());
    let fade = Animation::new(Duration::from_millis(220));
    div().absolute().inset_0()
        .child(div().id("drawer-scrim").absolute().inset_0().occlude().bg(crate::ui::scrim())
            .on_click(cx.listener(|w, _, _, cx| { w.drawer = false; cx.notify(); }))
            .with_animation("drawer-fade", fade, |d, t| d.opacity(t)))
        // 侧栏的行在根节点的 mouse-up 里完成点击，而 occlude 会挡住根节点，所以这里补上同样的两个监听。
        .child(div().id("drawer").absolute().top_0().bottom_0().left_0().occlude().bg(Theme::window())
            .pt(safe.top).pb(safe.bottom).pl(safe.left).shadow(crate::theme::shadow_dialog())
            .on_mouse_move(cx.listener(|w, ev: &MouseMoveEvent, window, cx| w.track_sidebar_press(ev, window, cx)))
            .on_mouse_up(MouseButton::Left, cx.listener(|w, _, window, cx| w.finish_sidebar_click(window, cx)))
            .child(sidebar)
            .with_animation("drawer-slide", slide, |d, t| d.left(px(-320. * (1. - t)))))
        .into_any_element()
}

pub fn paint_rows(
    line: &WrappedLine, origin: Point<Pixels>, height: Pixels, tops: &[Pixels], align: TextAlign,
    bounds: Option<Bounds<Pixels>>, window: &mut Window, cx: &mut App,
) -> anyhow::Result<()> {
    paint_line_rows(WrappedLine::paint, line, origin, height, tops, align, bounds, window, cx)
}

pub fn paint_background_rows(
    line: &WrappedLine, origin: Point<Pixels>, height: Pixels, tops: &[Pixels], align: TextAlign,
    bounds: Option<Bounds<Pixels>>, window: &mut Window, cx: &mut App,
) -> anyhow::Result<()> {
    paint_line_rows(WrappedLine::paint_background, line, origin, height, tops, align, bounds, window, cx)
}

fn paint_line_rows(
    paint: fn(&WrappedLine, Point<Pixels>, Pixels, TextAlign, Option<Bounds<Pixels>>, &mut Window, &mut App) -> anyhow::Result<()>,
    line: &WrappedLine, mut origin: Point<Pixels>, height: Pixels, tops: &[Pixels], align: TextAlign,
    bounds: Option<Bounds<Pixels>>, window: &mut Window, cx: &mut App,
) -> anyhow::Result<()> {
    let first = tops.first().copied().unwrap_or(px(0.));
    let offsets = tops.iter().enumerate().map(|(i, top)| *top - height * i as f32);
    if offsets.clone().all(|offset| offset == first) {
        origin.y += first;
        return paint(line, origin, height, align, bounds, window, cx);
    }
    let mask = window.content_mask().bounds;
    for (top, offset) in tops.iter().zip(offsets) {
        let mask = ContentMask { bounds: Bounds::new(point(mask.left(), origin.y + *top), size(mask.size.width, height)) };
        let row = point(origin.x, origin.y + offset);
        window.with_content_mask(Some(mask), |window| paint(line, row, height, align, bounds, window, cx))?;
    }
    Ok(())
}
