use std::time::Duration;

use gpui::{prelude::FluentBuilder as _, *};

use crate::app::{View, Workspace};
use crate::theme::Theme;

pub mod about;
pub mod alert;
pub mod appicon;
pub mod assistant;
pub mod assist;
pub mod imagine;
pub mod info;
pub mod card;
pub mod cmdmenu;
pub mod editmenu;
pub mod editor_surface;
pub mod files;
pub mod pick;
pub mod press;
pub mod video;
pub mod general;
pub mod lock;
pub mod folder_icons;
pub mod glass;
pub mod hub;
pub mod keybar;
pub mod masonry;
pub mod menu;
pub mod pace;
pub mod panel;
pub mod shot;
pub mod spaces;
pub mod crop;
pub mod coveredit;
pub mod customise;
pub mod starpick;
pub mod tags;
pub mod tilt;
mod astra;
mod neon;
pub mod style;
pub mod search;
pub mod toast;
pub mod veil;
pub mod stylepanel;
pub mod format;
pub mod voice;
pub mod background;
mod washi;

/// 只有 iOS 用的图标（`prototypes/ios/glyphs`），不放进根 `assets/` 以免打进桌面包；路径照 `assets/` 里的写，先于它们查。
pub fn asset(path: &str) -> Option<&'static [u8]> {
    match path { "editor-icons/imagine.svg" => Some(include_bytes!("glyphs/imagine.svg")), _ => None }
}

pub fn run() {
    crate::apple_files::install();
    crate::logging::init();
    // 测试传相对路径：放进沙盒的临时目录。真实文档库在应用的 Documents 里。
    if let Some(home) = std::env::var_os("QUIET_MD_HOME") && std::path::Path::new(&home).is_relative() {
        // SAFETY: 还没有别的线程。
        unsafe { std::env::set_var("QUIET_MD_HOME", std::env::temp_dir().join(home)); }
    }
    files::restore_space_bookmarks();
    // 应用自己的 iCloud 文件夹（iCloud 云盘里的「Quiet.md」）：没登录 iCloud、或这个版本没有 iCloud 的权限时为空，
    // 「存放位置」里的 iCloud 就用不了。要在打开文档库之前知道：iCloud 里的空间的路径靠它解析。
    let icloud = spaces::icloud_container();
    log::info!("iCloud folder: {}", icloud.as_ref().map_or_else(|| "unavailable".to_string(), |p| p.display().to_string()));
    fdl_core::spaces::set_icloud_root(icloud);
    // 要打开的空间，或者「创建第一个空间」（`space::prepare`）。冒烟测试用一篇固定的样例笔记，不放「开始指南」。
    crate::space::prepare(sample_library());
    // 设置 › 安全：进后台盖上、回来验证（偏好要在文档库打开后才读得到）。
    lock::install();
    crate::logging::write(&format!("library {}", crate::store::Store::root_dir().display()));
    gpui_platform::application().with_assets(crate::assets::Assets).run(|cx| {
        pace::install();
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
        crate::space_sync::init(cx);
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
    let bar = if editing() { px(keybar::HEIGHT) } else { px(0.) };
    window.viewport_size().height - safe(window).bottom - bar
}

thread_local! { static LAYOUT: std::cell::Cell<(bool, Edges<Pixels>, Size<Pixels>)> = const { std::cell::Cell::new((false, Edges { top: px(0.), right: px(0.), bottom: px(0.), left: px(0.) }, Size { width: px(0.), height: px(0.) })) }; }
/// 这一帧的版式，给拿不到 `Window` 的渲染函数用（对话框多半只有 `cx`）。
pub fn set_layout(compact: bool, safe: Edges<Pixels>, viewport: Size<Pixels>) {
    LAYOUT.with(|l| l.set((compact, safe, viewport)));
    // 状态栏的字跟着应用的深浅色走（应用可以不跟随系统）。
    thread_local! { static BAR_DARK: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) }; }
    let dark = crate::theme::dark();
    // 跟手返回、旧页的快照还原样盖着整屏时不换：状态栏底下还是旧页（有样式的文档常是浅色的纸）；快照一动就换。
    if shot::covering() { return; }
    if BAR_DARK.with(|b| b.replace(Some(dark))) != Some(dark) {
        gpui_ios::ios::set_status_bar_style(if dark { gpui_ios::StatusBarContentStyle::Light } else { gpui_ios::StatusBarContentStyle::Dark });
    }
}
pub fn is_compact() -> bool { LAYOUT.with(|l| l.get().0) }
pub fn layout() -> (bool, Edges<Pixels>, Size<Pixels>) { LAYOUT.with(|l| l.get()) }
pub fn insets() -> Edges<Pixels> { LAYOUT.with(|l| l.get().1) }

/// 手机上菜单不挂在按钮或手指旁边，而是从底部升起的动作面板：拇指够得着，也不会被手挡住。
/// 宽屏照原样锚在 `pos`。
#[track_caller]
pub fn place(corner: Anchor, pos: Point<Pixels>) -> Sheet {
    let (compact, safe, viewport) = LAYOUT.with(|l| l.get());
    let key = std::panic::Location::caller();
    if !compact { return Sheet { at: anchored().anchor(corner).position(pos).snap_to_window_with_margin(px(8.)), rise: false, key, kids: vec![] }; }
    Sheet { at: anchored().anchor(Anchor::BottomCenter).position(point(viewport.width / 2., viewport.height - safe.bottom - px(SHEET_INSET))), rise: true, key, kids: vec![] }
}
/// `place` 给出的浮层。手机上的动作面板出现时从下面升上来，收起时滑出屏幕（`shot`，按调用的那一处认面板）。
pub struct Sheet { at: Anchored, rise: bool, key: &'static std::panic::Location<'static>, kids: Vec<AnyElement> }
impl ParentElement for Sheet {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) { self.kids.extend(elements); }
}
impl IntoElement for Sheet {
    type Element = Anchored;
    fn into_element(self) -> Anchored {
        if !self.rise { return self.at.children(self.kids); }
        self.at.child(div().relative().children(self.kids).child(shot::sheet_marker(self.key)).with_animation("sheet-rise", Animation::new(Duration::from_millis(260)).with_easing(ease_out_quint()), |d, t| d.top(px((1. - t) * 44.)).opacity(0.3 + 0.7 * t)))
    }
}

/// 编辑器自己的底部面板（块菜单、添加面板、工具栏、链接和公式的面板）也从下面升上来，和 `Sheet` 同一条曲线；
/// 收起时一样滑出屏幕，按调用的那一处认面板（`rise_at` 也带着 `track_caller`）。
#[track_caller]
pub fn rise(key: &'static str, el: impl IntoElement) -> impl IntoElement {
    div().relative().child(el).child(shot::sheet_marker(std::panic::Location::caller())).with_animation(key, Animation::new(Duration::from_millis(260)).with_easing(ease_out_quint()), |d, t| d.top(px((1. - t) * 44.)).opacity(0.3 + 0.7 * t))
}
/// 同 `rise`，但面板是原生玻璃：自己会淡掉，收起时不拍快照滑出去（拍到的是玻璃底下的正文，看着是正文多出一份往下滑）。
pub fn rise_glass(key: &'static str, el: impl IntoElement) -> impl IntoElement {
    div().relative().child(el).with_animation(key, Animation::new(Duration::from_millis(260)).with_easing(ease_out_quint()), |d, t| d.top(px((1. - t) * 44.)).opacity(0.3 + 0.7 * t))
}

/// 换页时谁在动。`Enter`：新页从右（往里走）或原地（标签页之间）淡入；`Back`：返回，旧页的快照往右滑走，
/// 这一页带视差从左边回来（从已经滑过的比例开始）；`Follow`：跟手返回，跟着手指；`Return`：跟手返回没过半，
/// 滑回去；`Still`：取消后恢复原来那页，不动。
#[derive(Clone, Copy, PartialEq)]
enum Motion { Enter(f32), Follow, Return, Still }
/// 一页是哪一页：同一摞（主页、文件夹、文档是一摞；回收站、设置各是一摞）里越往里 `depth` 越大，
/// 文件夹再按它是哪一个分开。
#[derive(Clone, Copy, PartialEq)]
struct PageKey { stack: u8, depth: i32, folder: u64 }
thread_local! { static PAGE: std::cell::Cell<(Option<PageKey>, usize, Motion)> = const { std::cell::Cell::new((None, 0, Motion::Still)) }; }
// 跟手返回时手指滑过的比例；没在跟手时为空。
thread_local! { static GESTURE: std::cell::Cell<Option<f32>> = const { std::cell::Cell::new(None) }; }
// 下一次换页是跟手返回取消后的恢复。
thread_local! { static RESTORING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
/// 换页的过渡，都在 `shot` 里用原生视图做，这一页照停稳的位置画：往里走时新页整张从右边滑进来、旧页左移变暗；
/// 返回时旧页往右滑走，这一页带视差回来；同一层的两页之间旧的淡掉。
/// `folder`：文件夹页是哪一个、在第几层。
pub fn page(main: AnyElement, view: View, folder: Option<(&str, usize)>, sub_depth: i32) -> AnyElement {
    let key = match view {
        View::Doc => PageKey { stack: 0, depth: 100 + sub_depth, folder: 0 },
        View::Folder => PageKey { stack: 0, depth: 2 + folder.map_or(0, |f| f.1) as i32, folder: folder.map_or(0, |f| { use std::hash::{Hash, Hasher}; let mut h = std::hash::DefaultHasher::new(); f.0.hash(&mut h); h.finish() }) },
        View::Home => PageKey { stack: 0, depth: (home_page() != HomePage::Hub) as i32, folder: 0 },
        // 设置从首页进去，分类下的语音角色还有一层。
        View::Settings => PageKey { stack: 0, depth: 1 + settings_detail() as i32 + sub_depth, folder: 2 },
        // 首页「最近删除」点进去的一页：和「所有文档」同一层，用 `folder` 分开。
        View::Trash => PageKey { stack: 0, depth: 1, folder: 1 },
    };
    let restoring = RESTORING.with(|r| r.replace(false));
    let (epoch, motion) = PAGE.with(|p| {
        let (last, mut epoch, mut motion) = p.get();
        if last != Some(key) {
            if let Some(last) = last {
                epoch += 1;
                let dir = if last.stack == key.stack { (key.depth - last.depth).signum() as f32 } else { 0. };
                // 没经过 `capture_page` 的换页（点开一行文档、首页的子页）：屏幕上还是旧页，现在拍还来得及。
                if !restoring && !shot::page_pending() && GESTURE.with(|g| g.get()).is_none() && !shot::opening() { shot::capture_page(); }
                motion = if restoring { Motion::Still } else if GESTURE.with(|g| g.get()).is_some() { Motion::Follow }
                    else if dir < 0. && shot::slide_out(0.) { Motion::Still }
                    // 从卡片打开：文档从卡片放大（`shot::open_zoom`），这一页自己不滑入。
                    else if dir > 0. && (shot::open_zoom() || shot::push()) { Motion::Still }
                    else if dir == 0. && shot::page_pending() { shot::park_page(2); Motion::Still } else { Motion::Enter(dir) };
            }
            p.set((Some(key), epoch, motion));
        }
        (epoch, motion)
    });
    // 拍了快照却不是返回（或者没换成页，比如被「先保存」拦下了）：拿掉。
    if shot::page_pending() && !matches!(motion, Motion::Follow | Motion::Return) { shot::drop_page(); }
    let page = div().flex_1().min_w_0().h_full().flex().relative().child(main)
        // 旧页滑走的这段时间里一直出帧：只属于这一页的玻璃要等它走完才露面。
        .child(canvas(
            |_, window, _| { shot::below_drawn(); let parked = shot::parked_tick(); if parked || shot::sliding() { window.request_animation_frame() } },
            |_, _, window, _| { menu::finish_frame(window); press::finish_frame(); video::finish_frame(); },
        ).absolute().size_0());
    if !is_compact() || epoch == 0 { return page.into_any_element(); }
    match motion {
        // 拍不到快照时的退路。
        Motion::Enter(dir) => page.with_animation(("page", epoch), Animation::new(Duration::from_millis(300)).with_easing(ease_out_quint()), move |d, t| d.left(px(dir * (1. - t) * 72.)).opacity(t)).into_any_element(),
        Motion::Follow | Motion::Return | Motion::Still => page.into_any_element(),
    }
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

/// 整块落在裁剪区外就不画（卡片缩略图里卡片底边以下的块）。gpui-pre 0.3.12 把裁剪区按设备像素往外取整
/// （`Window::cover_bounds`）：自带裁剪的子元素（图片、代码行）整个在外面时，交出来的是一条零高的裁剪区，
/// 取整后变成一像素高，那一行像素会漏出来；滚动到小数位置时时有时无（主页文件夹第一行上面那条线）。
pub struct Cull { child: AnyElement }
pub fn cull(child: impl IntoElement) -> Cull { Cull { child: child.into_any_element() } }
impl IntoElement for Cull { type Element = Self; fn into_element(self) -> Self { self } }
impl Element for Cull {
    type RequestLayoutState = ();
    type PrepaintState = bool;
    fn id(&self) -> Option<ElementId> { None }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> { None }
    fn request_layout(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, window: &mut Window, cx: &mut App) -> (LayoutId, ()) { (self.child.request_layout(window, cx), ()) }
    fn prepaint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, bounds: Bounds<Pixels>, _: &mut (), window: &mut Window, cx: &mut App) -> bool {
        let shown = window.content_mask().bounds.intersects(&bounds);
        if shown { self.child.prepaint(window, cx); }
        shown
    }
    fn paint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, _: Bounds<Pixels>, _: &mut (), shown: &mut bool, window: &mut Window, cx: &mut App) { if *shown { self.child.paint(window, cx); } }
}

thread_local! { static SETTINGS_DETAIL: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
/// 手机上设置是两级：先是一页分类的列表，点进去才是那一类的设置。这里记着现在在不在第二级。
pub fn settings_detail() -> bool { SETTINGS_DETAIL.with(|d| d.get()) }
/// 进出第二级。先拍下这一页（`ios::page` 把它滑走，或者垫到下面）。
pub fn set_settings_detail(on: bool) {
    if settings_detail() != on { shot::capture_page(); }
    SETTINGS_DETAIL.with(|d| d.set(on));
}

pub fn settings_back(w: &mut Workspace, cx: &mut Context<Workspace>) {
    // 离开语音的几页时，试听跟着停；文本页的测试对话也停下。
    assistant::preview::stop();
    if w.settings_page == crate::settings::Page::AssistantText { assistant::text::stop(); }
    if let Some(page) = assistant::parent(w.settings_page) {
        shot::capture_page();
        w.open_settings(page, cx);
    } else { set_settings_detail(false); cx.notify(); }
}

thread_local! { static KEYBOARD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
/// 每帧记下软键盘是否弹出，给拿不到窗口的地方用。
pub fn set_keyboard(up: bool) { KEYBOARD.with(|k| k.set(up)); }
pub fn keyboard_shown() -> bool { KEYBOARD.with(|k| k.get()) }

thread_local! { static EDITING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
/// 正文有焦点就是在编辑，不管软键盘出没出来（接着硬键盘时没有软键盘）：工具条和「完成」跟着它，不跟着键盘。
pub fn set_editing(on: bool) { EDITING.with(|e| e.set(on)); }
pub fn editing() -> bool { EDITING.with(|e| e.get()) }

/// 文档页右上角的「完成」：收起键盘或退出块选择。
pub fn done_button() -> Stateful<Div> {
    div().id("doc-done").flex_none().size(px(44.)).rounded_full().occlude()
        .when(glass::supported(), |d| d.child(glass::pane("doc-done", Some(Theme::primary()), Some(("check", 20., gpui::white())), None)))
        .when(!glass::supported(), |d| d.bg(Theme::primary()).flex().items_center().justify_center().child(crate::ui::icon("check", 20., gpui::white())))
        .on_mouse_down(MouseButton::Left, |_, _, _| glass::pulse("doc-done"))
}
/// 底部面板贴底居中的那个点（窗口坐标）。
pub fn sheet_point() -> Point<Pixels> {
    let (_, safe, viewport) = LAYOUT.with(|l| l.get());
    point(viewport.width / 2., viewport.height - safe.bottom - px(SHEET_INSET))
}
/// 底部面板是浮着的卡片：四周留这么多，圆角 `SHEET_RADIUS`（走查 V6；没上玻璃）。编辑器的面板顶上有抓手，见 `touch.rs` 的 `pull_sheet`。
pub const SHEET_INSET: f32 = 12.;
pub const SHEET_RADIUS: f32 = 24.;
/// 动作面板占满屏宽（两侧各留 `SHEET_INSET`）；宽屏用菜单自己的宽度。
pub fn menu_w(wide: f32) -> Pixels {
    let (compact, _, viewport) = LAYOUT.with(|l| l.get());
    if compact { viewport.width - px(SHEET_INSET * 2.) } else { px(wide) }
}
/// 动作面板下面压暗的一层。不接事件：点面板外关闭菜单仍由原来的处理负责。
/// 深色下 `ui::scrim` 会跟着主题反相（发白），这里用不随主题变的黑。
pub fn sheet_scrim() -> impl IntoElement {
    let alpha = if crate::theme::dark() { 0.5 } else { 0.25 };
    div().absolute().inset_0().bg(crate::theme::rgbaf(0, 0, 0, alpha)).child(shot::scrim_marker(alpha))
        .with_animation("sheet-scrim", Animation::new(Duration::from_millis(220)), |d, t| d.opacity(t))
}

/// 带抓手的底部面板顶上让出放抓手的地方（盖掉面板原来的上边距）。
pub fn sheet_grab<E: Styled>(e: E) -> E { if is_compact() { e.pt(px(GRAB_ROOM)) } else { e } }
pub const GRAB_ROOM: f32 = 20.;
/// 底部面板不要描边：手机上只靠填充色、阴影和下面的明暗分开（走查 V2），桌面照旧。
pub fn sheet_plane<E: Styled>(e: E) -> E { if is_compact() { e.border_0().rounded(px(SHEET_RADIUS)) } else { e } }

/// 手机上有 Liquid Glass 时，外壳的按钮是悬浮在内容之上的玻璃，内容从它们下面滚过。
pub fn floating() -> bool { is_compact() && glass::supported() }

/// 悬浮的玻璃圆钮：玻璃和图标是原生视图，这里只留 44pt 的点击区，按下时让玻璃缩一下。
pub fn glass_button(id: &'static str, icon: &'static str, size: f32) -> Stateful<Div> {
    div().id(id).flex_none().size(px(44.)).rounded_full().occlude()
        .child(glass::pane(id, None, Some((icon, size, glass::ink())), None))
        .on_mouse_down(MouseButton::Left, move |_, _, _| glass::pulse(id))
}

pub fn catalogue_button() -> impl IntoElement {
    div().flex_none().size(px(44.)).child(glass::tap_pane("doc-catalogue", ("editor-icons/catalogue-list", 18., glass::ink()), "doc-catalogue", crate::settings::tr("目录", "Outline")))
}

/// 文档页悬浮的那排按钮占的高度（状态栏 + 一行按钮）：正文的开头让到它下面，滚动时从它底下穿过。
pub fn chrome_top() -> f32 { if floating() { f32::from(insets().top) + crate::app::HEADER_H } else { 0. } }
/// 同上，给文档的正文用：文档有背景时纸的上沿已经让到顶栏下面（`style::banded`），正文不用再让。
pub fn doc_top() -> f32 { chrome_top() + if style::banded() { style::GAP } else { 0. } }

/// 手机上的滚动条：和系统的一样细，只在内容滚动时出现，停下不久淡出，不接触摸（桌面那条常驻、能拖）。
/// `seen` 记着上次画时滚到哪、最后一次变化的时刻；`top`、`bottom` 是悬浮的外壳占去的两头。
pub fn scroll_thumb(seen: &std::cell::Cell<Option<(Pixels, Option<std::time::Instant>)>>, bounds: Bounds<Pixels>, offset: Pixels, content: Pixels, top: f32, bottom: f32, window: &mut Window) {
    let view = bounds.size.height;
    paint_thumb(seen, bounds, offset, offset / (content-view).max(px(1.)), view / content.max(px(1.)), top, bottom, window);
}

// 虚拟列表只测量访问过的行。用视口两端所在的行和行内比例表示进度，未访问的行也计入总长。
pub fn document_thumb(seen: &std::cell::Cell<Option<(Pixels, Option<std::time::Instant>)>>, bounds: Bounds<Pixels>, list: &ListState, bottom: f32, window: &mut Window) {
    let n = list.item_count();
    let first = list.logical_scroll_top();
    let view = list.viewport_bounds();
    let Some(row) = list.bounds_for_item(first.item_ix) else { return };
    let fraction = |y: Pixels, row: Bounds<Pixels>| ((y-row.top())/row.size.height.max(px(0.01))).clamp(0.,1.);
    let start = first.item_ix as f32 + fraction(view.top(),row);
    let mut end = start;
    for ix in first.item_ix..n {
        let Some(row) = list.bounds_for_item(ix) else { break };
        end = ix as f32 + fraction(view.bottom(),row);
        if row.bottom() >= view.bottom() { break; }
    }
    let span = (end-start).max(0.);
    let travel = n as f32 - span;
    paint_thumb(seen, bounds, list.scroll_px_offset_for_scrollbar().y, start/travel.max(f32::EPSILON), span/n as f32, doc_top(), bottom, window);
}

fn paint_thumb(seen: &std::cell::Cell<Option<(Pixels, Option<std::time::Instant>)>>, bounds: Bounds<Pixels>, offset: Pixels, progress: f32, visible: f32, top: f32, bottom: f32, window: &mut Window) {
    const HOLD: f32 = 0.7;
    const FADE: f32 = 0.25;
    let moved = match seen.get() {
        Some((last, at)) => if last != offset { Some(std::time::Instant::now()) } else { at },
        None => None,
    };
    seen.set(Some((offset, moved)));
    let (Some(at), view) = (moved, bounds.size.height) else { return };
    let t = at.elapsed().as_secs_f32();
    if t >= HOLD + FADE || visible >= 1. { return; }
    window.request_animation_frame();
    let track = (view - px(top + bottom + 8.)).max(px(1.));
    let h = (track * visible).max(px(36.)).min(track);
    let y = bounds.top() + px(top + 4.) + (track - h) * progress.clamp(0., 1.);
    let alpha = if t < HOLD { 1. } else { 1. - (t - HOLD) / FADE };
    window.paint_quad(fill(Bounds::new(point(bounds.right() - px(6.), y), size(px(3.), h)), Theme::scrollbar_thumb().opacity(alpha)).corner_radii(px(1.5)));
}

/// 状态栏下面的一道渐隐：正文滚到状态栏底下时，时间和电量仍然看得清。
/// 标题滚到顶栏底下之后（`titled`），顶栏中间有小字标题，渐隐加高到盖住整排按钮。
pub fn top_fade(page: Hsla, titled: bool) -> impl IntoElement {
    div().absolute().top_0().left_0().right_0().h(insets().top + px(if titled { crate::app::HEADER_H + 24. } else { 28. }))
        .bg(linear_gradient(180., linear_color_stop(page, if titled { 0.8 } else { 0.55 }), linear_color_stop(page.opacity(0.), 1.)))
}

/// 文档页顶栏中间的小字标题：正文的标题滚到顶栏底下后淡入，下面一行是所在的文件夹，前面是这个文件夹的图标（照 Craft，和字同色）。
pub fn doc_title(title: SharedString, folder: Option<SharedString>, icon: Option<&'static str>) -> AnyElement {
    // 有毛玻璃带时标题写在带子上（原生的），这里只留位置。
    if veil::on() { return veil::title(title, folder, icon, Theme::text(), Theme::text_caption()).into_any_element(); }
    let (ink, sub) = style::chrome_ink().unwrap_or((Theme::text(), Theme::text_caption()));
    div().id("doc-title").w_full().flex().flex_col().items_start()
        .child(div().max_w_full().overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(px(15.)).line_height(px(20.)).font_weight(FontWeight::SEMIBOLD).text_color(ink).child(title))
        .children(folder.map(|f| div().max_w_full().flex().items_center().gap(px(3.))
            .children(icon.map(|i| folder_icons::icon(i, 11., sub)))
            .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(px(11.)).line_height(px(14.)).text_color(sub).child(f))))
        .with_animation("doc-title-in", Animation::new(Duration::from_millis(180)), |d, t| d.opacity(t)).into_any_element()
}

/// 手机上的提示：顶上两颗按钮之间的一条玻璃（`toast.rs`）。文档页右边有两颗按钮，让得多一些。
/// `busy`：右上角有两颗钮的页（文档页、浏览页），右边多让一些。
pub fn toast_slot(msg: SharedString, danger: bool, busy: bool) -> AnyElement {
    div().absolute().top(insets().top - px(4.)).left(px(72.)).right(px(if busy { 140. } else { 72. })).h(px(52.)).child(toast::slot(msg, danger)).into_any_element()
}

thread_local! { static HISTORY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
/// 文档页底部那条「撤销 / 粘贴 / 重做」开着。
pub fn history_open() -> bool { HISTORY.with(|h| h.get()) }
pub fn close_history() { HISTORY.with(|h| h.set(false)); }

/// 阅读状态下文档页底部的两组玻璃钮（照 Craft）：左边一条是 撤销… / 样式 / 编辑，右边一颗「+」挂着插入菜单。
/// 编辑就是原来右下角那支铅笔：在文末新起一行并开始写。点了「撤销…」之后换成中间一条 撤销 / 粘贴 / 重做，右上角是「完成」。
pub fn dock() -> impl IntoElement {
    use menu::Item;
    let ink = glass::ink();
    let row = div().id("doc-dock").absolute().left_0().right_0().bottom(dock_inset()).h(px(48.));
    if history_open() {
        let (_, _, viewport) = layout();
        return row.child(div().id("doc-history").absolute().left((viewport.width - px(216.)) / 2.).top_0().w(px(216.)).h_full().rounded_full().occlude()
            .child(glass::tab_pane("doc-history", vec![("sf:arrow.uturn.backward", 22., ink), ("sf:doc.on.clipboard", 22., ink), ("sf:arrow.uturn.forward", 22., ink)], None, vec!["doc-undo", "doc-paste", "doc-redo"])));
    }
    row
        .child(div().id("doc-tools").absolute().left(px(20.)).top_0().w(px(160.)).h_full().rounded_full().occlude()
            .child(glass::tab_pane("doc-tools", vec![("sf:arrow.uturn.backward.circle", 24., ink), ("sf:paintbrush", 23., ink), ("sf:pencil", 24., ink)], None, vec!["doc-history", "doc-style", "doc-append"])))
        .child(div().id("doc-add").absolute().right(px(20.)).top_0().size(px(48.)).rounded_full().occlude()
            .child(menu::glass_button("doc-add", "plus", Theme::text(), vec![
                Item::new("doc-add-more", crate::settings::tr("更多", "More"), "ellipsis", 0),
                Item::new("doc-add-file", crate::settings::tr("文件", "File"), "folder", 1),
                Item::new("doc-add-image", crate::settings::tr("图库", "Photos"), "photo.on.rectangle", 2),
            ])))
}

/// 主页底部那排玻璃离屏幕下沿的距离（Home 指示条之上），和列表末尾为它们留出的高度。
fn dock_inset() -> Pixels { insets().bottom.max(px(12.)) + px(4.) }
pub fn dock_bottom() -> f32 { f32::from(dock_inset()) + 72. }

/// 主页的大标题和它下面的工具行一共多高（含悬浮按钮那一段）：网格开头留出这么多。
pub fn home_lead_h() -> f32 { chrome_top() + 56. + LEAD_SEARCH_H + 12. + if matches!(home_page(), HomePage::Starred | HomePage::Tags) || trash_view() || lead_note() { CAPTION_H + 14. } else { 0. } }
fn lead_note() -> bool { LEAD_NOTE.with(|n| n.get()) }
thread_local! { static TRASH_VIEW: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
/// 这一帧画的是「最近删除」（工作区每帧开头告诉这里）：开头那段多一块说明，网格往下让。
pub fn set_trash_view(on: bool) { TRASH_VIEW.with(|t| t.set(on)); }
pub fn trash_view() -> bool { TRASH_VIEW.with(|t| t.get()) }
/// 大标题下面那条搜索占的高度。
pub const LEAD_SEARCH_H: f32 = 56.;
/// 大标题下面的搜索条，和首页的是同一条。
pub fn lead_search(label: impl Into<SharedString>, cx: &mut Context<Workspace>) -> impl IntoElement { div().h(px(LEAD_SEARCH_H)).pt(px(4.)).child(search_bar_with(label, cx)) }
/// 主页的底色。悬浮版式下是窗口的浅灰，白色的卡片从上面浮出来；否则照旧是页面的白。
/// 深色下浏览页是纯黑，比文档页的深灰暗一档：两种页面一眼能分开（走查 V3）。
pub fn home_bg() -> Hsla { if !floating() { Theme::bg_body() } else if crate::theme::dark() { crate::theme::rgbaf(0, 0, 0, 1.) } else { Theme::window() } }
pub const LARGE_TITLE_H: f32 = 56.;
pub const LARGE_TITLE_LINE: f32 = 34.;
pub const LARGE_TITLE_SIZE: f32 = 26.;
/// 跟着内容滚走的大标题。
pub fn large_title(title: impl Into<SharedString>) -> impl IntoElement { large_title_icon(title, None) }
pub fn large_title_icon(title: impl Into<SharedString>, icon: Option<AnyElement>) -> impl IntoElement {
    div().h(px(LARGE_TITLE_H)).px(px(24.)).flex().items_center().text_size(px(LARGE_TITLE_SIZE)).line_height(px(LARGE_TITLE_LINE)).font_weight(FontWeight::BOLD).text_color(Theme::text()).gap(px(10.)).children(icon)
        .child(div().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().child(title.into()))
}

/// 手机上主页分几页：首页（`hub`）、「所有文档」、「文件夹」（顶层的文件夹）、「收藏」和「所有标签」（`tags`）。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum HomePage { Hub, All, Folders, Starred, Tags }
thread_local! { static HOME_PAGE: std::cell::Cell<HomePage> = const { std::cell::Cell::new(HomePage::Hub) }; }
pub fn home_page() -> HomePage { HOME_PAGE.with(|p| p.get()) }
/// 换页。先拍下这一页（`ios::page` 把它滑走，或者垫到下面）。
pub fn set_home_page(page: HomePage) {
    if home_page() != page { shot::capture_page(); }
    HOME_PAGE.with(|p| p.set(page));
}

/// 浏览页（所有文档、文件夹、某个文件夹里）是同一套：左上返回，右上「选择」（进批量管理）和「…」
/// （新建文件夹、显示方式、排序），大标题和搜索跟着内容滚走。
pub fn browse_chrome(w: &Workspace, cx: &mut Context<Workspace>, back: impl Fn(&mut Workspace, &mut Window, &mut Context<Workspace>) + 'static) -> impl IntoElement {
    browse_chrome_with(w, browse_menu_items(w), cx, back)
}
/// 同上，「…」里是 `items`（「最近删除」：清空回收站）。
pub fn browse_chrome_with(_w: &Workspace, items: Vec<menu::Item>, cx: &mut Context<Workspace>, back: impl Fn(&mut Workspace, &mut Window, &mut Context<Workspace>) + 'static) -> impl IntoElement {
    let select = crate::settings::tr("选择", "Select");
    div().absolute().top(insets().top).left_0().right_0().h(px(crate::app::HEADER_H)).px(px(16.)).flex().items_center().gap(px(10.))
        .child(glass_button("browse-back", "editor-icons/chevron-left", 22.).on_click(cx.listener(move |w, _, window, cx| back(w, window, cx))))
        .child(div().flex_1())
        .child(div().id("browse-select").flex_none().h(px(44.)).w(px(if crate::settings::english() { 82. } else { 64. })).rounded_full().occlude()
            .child(glass::bar_pane("browse-select", select.into(), glass::ink()))
            .on_mouse_down(MouseButton::Left, |_, _, _| glass::pulse("browse-select"))
            .on_click(cx.listener(|w, _, _, cx| { w.batch = Some(vec![]); w.row_menu = None; w.home.sort_menu = false; cx.notify(); })))
        // 「…」是一颗系统的玻璃钮，挂着系统菜单。
        .child(div().relative().flex_none().size(px(44.)).child(menu::glass_button("browse-options", "ellipsis", Theme::text(), items)))
}
/// 浏览页「…」的系统菜单：新建文件夹（所有文档是平铺的，没有）、显示方式、排序。
fn browse_menu_items(w: &Workspace) -> Vec<menu::Item> {
    const SORTS: [&str; 5] = ["browse-sort-0", "browse-sort-1", "browse-sort-2", "browse-sort-3", "browse-sort-4"];
    let mut items = vec![];
    if w.view == View::Folder || home_page() == HomePage::Folders {
        items.push(menu::Item::new("browse-new-folder", crate::settings::tr("新建文件夹", "New folder"), "folder.badge.plus", 0));
    }
    items.push(menu::Item::new("browse-grid", crate::settings::tr("宫格视图", "Grid"), "square.grid.2x2", 1).checked(w.home.grid));
    items.push(menu::Item::new("browse-list", crate::settings::tr("列表视图", "List"), "list.bullet", 1).checked(!w.home.grid));
    items.extend(w.sort_choices().into_iter().zip(SORTS).map(|((title, checked), id)| menu::Item::new(id, title, "", 2).checked(checked)));
    // 桌面版标题栏「导入」菜单里的三项。
    let tr = crate::settings::tr;
    items.push(menu::Item::new("browse-import-md", tr("导入 Markdown 文件…", "Import Markdown Files…"), "square.and.arrow.down", 3));
    items.push(menu::Item::new("browse-import-folder", tr("导入文件夹…", "Import Folder…"), "folder", 3));
    items.push(menu::Item::new("browse-import-xml", tr("导入 DocxXML…", "Import DocxXML…"), "doc", 3));
    items
}
/// 浏览页开头跟着内容滚走的一段：大标题和搜索。
pub fn browse_lead(title: impl Into<SharedString>, cx: &mut Context<Workspace>) -> AnyElement { browse_lead_with(title, None, cx) }
/// 同上，带一句说明（文件夹的描述）。
pub fn browse_lead_with(title: impl Into<SharedString>, note: Option<String>, cx: &mut Context<Workspace>) -> AnyElement { browse_lead_icon(title, note, None, cx) }
/// 照 Craft（文件夹「工作」）：说明不在标题下面，而在搜索下面一块浅色圆角底上，和「收藏」页那块一样。
/// 有说明时网格要多让出这一块（`set_lead_note` → `home_lead_h`）。
pub fn browse_lead_icon(title: impl Into<SharedString>, note: Option<String>, icon: Option<AnyElement>, cx: &mut Context<Workspace>) -> AnyElement {
    let note = note.filter(|n| !n.trim().is_empty());
    set_lead_note(note.is_some());
    div().pt(px(chrome_top())).child(large_title_icon(title, icon))
        .child(lead_search(crate::settings::tr("搜索", "Search"), cx))
        .children(note.map(|n| caption_pill(n)))
        .into_any_element()
}
/// 搜索下面那块说明：浅色圆角底，一行灰字，放不下就省略。
fn caption_pill(text: impl Into<SharedString>) -> Div {
    div().mt(px(10.)).mx(px(16.)).px(px(16.)).h(px(CAPTION_H)).flex().items_center().justify_center().rounded_full().bg(Theme::fill_hover())
        .text_size(px(13.)).text_color(Theme::text_caption()).whitespace_nowrap().overflow_hidden().text_ellipsis().child(text.into())
}
thread_local! { static LEAD_NOTE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
/// 这一帧的文件夹页开头有没有说明那一块。
fn set_lead_note(on: bool) { LEAD_NOTE.with(|n| n.set(on)); }
/// 每帧开头清掉：只有画了带说明的开头的那一页才算。
pub fn clear_lead_note() { set_lead_note(false); }
/// 文件夹图标的颜色：它自己选的那个（`Meta::folder_color`，文档里的颜色名），没选就是平常的图标色。
pub fn folder_tint(e: &crate::store::Entry) -> Hsla { e.meta.folder_color.as_deref().and_then(crate::theme::callout_border_swatch).unwrap_or_else(Theme::icon) }
/// 「收藏」页开头（照 Craft）：大标题、搜索，下面一块浅色圆角底上的一句说明。
pub fn browse_lead_caption(title: impl Into<SharedString>, caption: &'static str, cx: &mut Context<Workspace>) -> AnyElement {
    div().pt(px(chrome_top())).child(large_title(title)).child(lead_search(crate::settings::tr("搜索", "Search"), cx))
        .child(caption_pill(caption))
        .into_any_element()
}
/// 「收藏」页说明那一块的高度；网格开头多留这么多再加上下的空（`home_lead_h`）。
const CAPTION_H: f32 = 40.;
/// 浏览页的骨架：内容在下，状态栏的渐隐和那排玻璃在上。
pub fn browse_page(body: AnyElement, chrome: impl IntoElement) -> AnyElement {
    div().flex_1().min_w_0().size_full().flex().flex_col().relative().child(body).child(top_fade(home_bg(), false)).child(chrome).into_any_element()
}
/// 没有内容的浏览页：开头那一段照旧，下面一句话。
pub fn browse_empty(lead: AnyElement, text: &'static str) -> AnyElement {
    div().flex_1().min_h_0().flex().flex_col().child(lead).child(div().pt(px(80.)).text_center().text_size(px(15.)).text_color(Theme::text_caption()).child(text)).into_any_element()
}
/// 批量管理时的浏览页（照 Craft）：顶上左边「全选」和已选几项，右边一颗主色的 ✓ 收起；动作在底部，标签栏的位置换成一条玻璃动作栏
/// （创建副本 / 移动到 / 删除）。全选和计数是我们多出来的，留着。
/// 「选择」时的浏览页（照 Craft，「如何使用 Craft」）：页面本身不变（大标题、搜索、卡片），选中的卡片外面一圈强调色（`pick_ring`）；
/// 顶上左边照常返回，右边「选择」和「…」换成一颗强调色的 ✓（收起）；底部左边一条玻璃动作栏，右边一颗玻璃「…」（全选 / 取消全选）。
/// `body` 是整页内容（含开头），`back` 是左上角返回做的事。
pub fn batch_page(w: &Workspace, ids: Vec<String>, body: AnyElement, back: impl Fn(&mut Workspace, &mut Window, &mut Context<Workspace>) + 'static, cx: &mut Context<Workspace>) -> AnyElement {
    let n = w.batch.as_ref().map_or(0, |s| s.len());
    let ink = if n > 0 { glass::ink() } else { Theme::text_disabled() };
    // 照 Craft：收藏（全都收藏着时是取消收藏）、导出、删除。
    let all_starred = n > 0 && w.batch.as_ref().is_some_and(|s| s.iter().all(|id| w.entries.iter().any(|e| &e.id == id && e.meta.starred > 0)));
    batch_page_with(w, ids, body, vec![(if all_starred { "sf:star.slash" } else { "sf:star" }, ink, "batch-star"), ("sf:square.and.arrow.up", ink, "batch-export"), ("sf:trash", if n > 0 { Theme::danger() } else { ink }, "batch-delete")], back, cx)
}
thread_local! { static BATCH_IDS: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) }; }
/// 同上，底部那条换成别的动作（「最近删除」：恢复、彻底删除）。每格：图标、颜色、菜单项 id（`menu_action` 里接）。
pub fn batch_page_with(w: &Workspace, ids: Vec<String>, body: AnyElement, actions: Vec<(&'static str, Hsla, &'static str)>, back: impl Fn(&mut Workspace, &mut Window, &mut Context<Workspace>) + 'static, cx: &mut Context<Workspace>) -> AnyElement {
    let all = !ids.is_empty() && w.batch.as_ref().is_some_and(|s| ids.iter().all(|id| s.contains(id)));
    BATCH_IDS.with(|b| *b.borrow_mut() = ids);
    let chrome = div().absolute().top(insets().top).left_0().right_0().h(px(crate::app::HEADER_H)).px(px(16.)).flex().items_center()
        .child(glass_button("batch-back", "editor-icons/chevron-left", 22.).on_click(cx.listener(move |w, _, window, cx| { w.batch = None; w.row_menu = None; back(w, window, cx); })))
        .child(div().flex_1())
        .child(done_button().on_click(cx.listener(|w, _, _, cx| { glass::haptic(); w.batch = None; w.row_menu = None; cx.notify(); })));
    // 没选东西时几格是灰的，点了也不做事（`menu_action` 里看 `batch` 空不空）。
    let width = px(54. * actions.len() as f32 + 12.);
    let (icons, keys): (Vec<_>, Vec<_>) = actions.into_iter().map(|(sym, color, key)| ((sym, 22., color), key)).unzip();
    let bar = div().id("batch-bar").absolute().left(px(20.)).w(width).bottom(dock_inset()).h(px(52.)).rounded_full().occlude()
        .child(glass::tab_pane("batch-bar", icons, None, keys));
    let more = vec![menu::Item::new("batch-all", if all { crate::settings::tr("取消全选", "Deselect All") } else { crate::settings::tr("全选", "Select All") }, if all { "circle" } else { "checkmark.circle" }, 0)];
    let more = div().absolute().right(px(20.)).bottom(dock_inset()).size(px(52.)).occlude().child(menu::glass_button("batch-more", "ellipsis", Theme::text(), more));
    // 和 `browse_page` 一样是竖着的 flex：网格（`uniform_list`）靠它撑出高度，不然有文件夹的那种网格高度是 0、整页空掉。
    div().flex_1().min_w_0().size_full().flex().flex_col().relative().child(body).child(top_fade(home_bg(), false)).child(chrome).child(bar).child(more).into_any_element()
}
/// 选中的卡片外面那一圈（强调色，压在卡片的边上）。
pub fn pick_ring(radius: f32) -> Div {
    div().absolute().inset_0().rounded(px(radius)).border(px(3.)).border_color(Theme::primary())
}

thread_local! { static FOLDERS_FOLDED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
thread_local! { static STARRED_FOLDED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
/// 首页的收藏一栏折叠着没有（只在这次运行里记着）。
pub fn starred_folded() -> bool { STARRED_FOLDED.with(|f| f.get()) }
/// 收藏的金色星。照 Craft 实测（截屏 2026-11-14 16.46.22 卡片上的星 197,165,54，压在面板的暗下面；按同图页面底色 199→242 还原）。
pub fn star_color() -> Hsla { crate::theme::rgbaf(240, 201, 66, 1.) }
/// 首页「收藏」一栏右边「…」的系统菜单（照 Craft）：收藏文档；自定义主页、折叠 / 展开收藏。
pub fn hub_starred_items() -> Vec<menu::Item> {
    let tr = crate::settings::tr;
    let folded = starred_folded();
    vec![
        menu::Item::new("hub-star-pick", tr("收藏文档", "Star documents"), "star", 0),
        menu::Item::new("hub-customise", tr("自定义主页", "Customize Home"), "slider.horizontal.3", 1),
        menu::Item::new("hub-starred-fold", if folded { tr("展开收藏", "Expand starred") } else { tr("折叠收藏", "Collapse starred") }, if folded { "arrow.up.left.and.arrow.down.right" } else { "arrow.down.right.and.arrow.up.left" }, 1),
    ]
}
/// 首页的文件夹一栏折叠着没有（只在这次运行里记着）。
pub fn folders_folded() -> bool { FOLDERS_FOLDED.with(|f| f.get()) }
/// 这里自己的底部面板有没有开着（开着时玻璃让开）。
pub fn sheet_open() -> bool { icon_picking() }
/// 首页「文件夹」一栏右边「…」的系统菜单里有什么。
pub fn hub_menu_items() -> Vec<menu::Item> {
    let folded = folders_folded();
    vec![
        menu::Item::new("hub-new-folder", crate::settings::tr("新建文件夹", "New folder"), "folder.badge.plus", 0),
        menu::Item::new("hub-fold", if folded { crate::settings::tr("展开文件夹", "Expand folders") } else { crate::settings::tr("折叠文件夹", "Collapse folders") }, if folded { "arrow.up.left.and.arrow.down.right" } else { "arrow.down.right.and.arrow.up.left" }, 0),
    ]
}
/// 首页右上角玻璃「…」的系统菜单（照 Craft，IMG_9114）：导入（子菜单），自定义主页。
pub fn hub_more_items() -> Vec<menu::Item> {
    let tr = crate::settings::tr;
    vec![
        menu::Item::new("hub-import", tr("导入", "Import"), "square.and.arrow.down", 0).kids(vec![
            menu::Item::new("browse-import-md", tr("导入 Markdown 文件…", "Import Markdown Files…"), "doc.text", 0),
            menu::Item::new("browse-import-folder", tr("导入文件夹…", "Import Folder…"), "folder", 0),
            menu::Item::new("browse-import-xml", tr("导入 DocxXML…", "Import DocxXML…"), "doc", 0),
        ]),
        menu::Item::new("hub-customise", tr("自定义", "Customize"), "slider.horizontal.3", 1),
    ]
}
/// 文档页「…」的系统菜单：和桌面的文档菜单同样的项、顺序和文案，子菜单就地展开。
pub fn doc_menu_items(w: &Workspace) -> Vec<menu::Item> {
    use crate::settings::text;
    use crate::store::PageWidth;
    use menu::Item;
    let entry = w.current.as_ref().and_then(|id| w.entries.iter().find(|e| &e.id == id));
    let pinned = entry.is_some_and(|e| e.meta.pinned);
    let starred = entry.is_some_and(|e| e.meta.starred > 0);
    let width = entry.map(|e| e.meta.width).unwrap_or_default();
    let refs_shown = !entry.is_some_and(|e| e.meta.refs_hidden);
    let mut download = vec![];
    if crate::printing::SUPPORTED { download.push(Item::new("doc-pdf", "PDF", "", 0)); }
    download.push(Item::new("doc-md", "Markdown", "", 0));
    let mut items = vec![
        Item::new("doc-style", crate::settings::tr("样式", "Style"), "paintpalette", 0),
        Item::new("doc-text-size", crate::settings::tr("文字大小", "Text size"), "textformat.size", 0),
        Item::new("", text("页宽设置"), "arrow.left.and.right", 0).kids(vec![
            Item::new("doc-width-standard", text("默认"), "", 0).checked(width == PageWidth::Standard),
            Item::new("doc-width-wide", text("较宽"), "", 0).checked(width == PageWidth::Wide),
            Item::new("doc-width-full", text("全宽"), "", 0).checked(width == PageWidth::Full),
        ]),
        Item::new("doc-find", crate::settings::tr("在页面中查找","Find in page"), "magnifyingglass", 1),
        Item::new("doc-outline",text("目录"),"list.bullet",1),
        Item::new("doc-move", text("移动到"), "folder", 2),
        Item::new("doc-pin", if pinned { text("从“置顶”移除") } else { text("添加到“置顶”") }, if pinned { "pin.slash" } else { "pin" }, 2),
        Item::new("doc-star", if starred { text("取消收藏") } else { text("添加到收藏") }, if starred { "star.slash" } else { "star" }, 2),
        Item::new("doc-copy", text("创建副本"), "plus.square.on.square", 3),
        Item::new("", text("下载为"), "square.and.arrow.up", 3).kids(download),
    ];
    if crate::printing::SUPPORTED { items.push(Item::new("doc-print", text("打印"), "printer", 4)); }
    items.push(Item::new("doc-info", text("文档信息"), "info.circle", 5));
    let mut more = vec![Item::new("doc-refs", if refs_shown { crate::settings::tr("隐藏引用列表", "Hide references") } else { text("显示引用列表") }, "link", 0)];
    if !w.reading() {
        more.push(Item::new("doc-select-all-blocks", crate::settings::tr("选中所有块", "Select all blocks"), "checklist", 0));
        more.push(Item::new("doc-clean-empty", crate::settings::tr("清理空段落", "Remove empty paragraphs"), "eraser", 0));
    }
    items.push(Item::new("", text("更多"), "ellipsis.circle", 6).kids(more));
    items.push(Item::new("doc-delete", text("删除"), "trash", 6).danger());
    items
}

pub fn select_all_blocks(w: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    if let Some(ed) = w.editor.clone().filter(|e| !e.read(cx).read_only) {
        ed.update(cx, |e,cx| {
            e.core.select_all_blocks();
            e.menu = None; e.slash = None;
            window.blur(cx); window.refresh(); cx.notify();
        });
    }
}

pub fn clean_empty_paragraphs(w: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    if let Some(editor) = w.editor.clone().filter(|e| !e.read(cx).read_only) {
        let n = editor.update(cx, |ed, cx| {
            let n = ed.core.remove_empty_paragraphs();
            if n > 0 { ed.changed(window, cx); }
            n
        });
        if n == 0 { w.show_toast(crate::settings::tr("没有可清理的空段落", "No empty paragraphs to remove"), cx); }
        else {
            w.show_toast(&crate::localized_format!("已清理 {} 个空段落", "Removed {} empty paragraphs", n), cx);
            if toast::native() { w.toast_link(crate::settings::tr("撤销", "Undo"), "toast-undo", cx); }
        }
    }
}

/// 长按卡片、行出的系统上下文菜单：项、顺序和文案照桌面右键菜单（`render_row_menu`、`folder_row_menu`）。
/// 「本地文件」不在这里（仍是动作面板），也就不放占位。
pub fn row_context(w: &Workspace, id: &str, radius: f32) -> Option<AnyElement> { row_context_at(w, id, id, radius) }
/// 同上，长按的这一处用 `key`（「id#位置」）区分：同一篇文档出现在别处的那一块不会跟着藏起来。
pub fn row_context_at(w: &Workspace, id: &str, key: &str, radius: f32) -> Option<AnyElement> {
    if !floating() { return None; }
    // 「最近删除」的卡片：恢复、彻底删除。点了不打开，没有点触。
    if w.view == View::Trash {
        let title = hub::trash_entries(w).into_iter().find(|e| e.id == id).map(|e| crate::settings::display_title(&e.title))?;
        return trash_context_r(id, title.into(), radius);
    }
    use crate::settings::{text, tr};
    use menu::Item;
    let e = w.entries.iter().find(|e| e.id == id)?;
    let title = crate::settings::display_title(&e.title);
    // 按住缩小一点（`press`）。文件夹卡片的影子和前片画到格子外面。
    let mark = press::mark(SharedString::from(key.to_string()), if e.meta.folder.is_some() && radius == FOLDER_CARD_R { 8. } else { 0. });
    let items = if e.meta.folder.is_some() {
        // 照 Craft（IMG_9173–9175）：没有标题；编辑… / 排序方式（下面一行是当前的排法，点开就地选）/ 移动到…、
        // 把里面的东西导出为 Markdown / 删除。排法和文件夹页「…」里的是同一个。
        // Craft 的「复制为 › 深层链接」我们还没有（没有 App 的链接），不放。
        let sorts = w.sort_choices();
        // 照 Craft 的顺序和短名字：手动、名称、上次浏览、创建日期、更新日期（`sort_choices` 的顺序是 手动、上次浏览、更新、创建、名称）。
        let order: [(usize, &'static str, &'static str); 5] = [
            (0, "ctx-sort-0", tr("手动", "Manual")), (4, "ctx-sort-4", tr("名称", "Name")), (1, "ctx-sort-1", tr("上次浏览", "Last Viewed")),
            (3, "ctx-sort-3", tr("创建日期", "Date Created")), (2, "ctx-sort-2", tr("更新日期", "Date Modified")),
        ];
        let current = order.iter().find(|o| sorts.get(o.0).is_some_and(|s| s.1)).map_or("", |o| o.2);
        let sort_items = order.iter().map(|&(i, id, name)| Item::new(id, name, "", 0).checked(sorts.get(i).is_some_and(|s| s.1))).collect();
        let n = w.entries.iter().filter(|c| c.meta.parent.as_deref() == Some(id)).count();
        let export = if n == 1 { tr("将 1 个项目导出为", "Export 1 Item As").to_string() } else { crate::localized_format!("将 {} 个项目导出为", "Export {} Items As", n) };
        let mut items = vec![
            Item::new("ctx-edit-folder", tr("编辑…", "Edit…"), "character.cursor.ibeam", 0),
            Item::new("", tr("排序方式", "Sort By"), "arrow.up.arrow.down", 1).sub(current).kids(sort_items),
            Item::new("ctx-move", tr("移动到…", "Move to…"), "arrowshape.turn.up.right.fill", 2),
        ];
        if n > 0 { items.push(Item::new("", export, "square.and.arrow.up", 2).kids(vec![Item::new("ctx-export-md", "Markdown", "", 0)])); }
        items.push(Item::new("ctx-delete", tr("删除", "Delete"), "trash", 3).danger());
        items
    } else if e.meta.local() {
        return Some(mark.into_any_element());
    } else {
        let (pinned, starred) = (e.meta.pinned, e.meta.starred > 0);
        let mut download = vec![];
        if crate::printing::SUPPORTED { download.push(Item::new("ctx-pdf", "PDF", "", 0)); }
        download.push(Item::new("ctx-md", "Markdown", "", 0));
        vec![
            Item::new("ctx-copy", text("创建副本"), "plus.square.on.square", 0),
            Item::new("ctx-move", text("移动到"), "folder", 0),
            Item::new("ctx-pin", if pinned { text("从“置顶”移除") } else { text("添加到“置顶”") }, if pinned { "pin.slash" } else { "pin" }, 0),
            Item::new("ctx-star", if starred { text("取消收藏") } else { tr("收藏", "Add to favorites") }, if starred { "star.slash" } else { "star" }, 0),
            Item::new("", text("下载"), "square.and.arrow.up", 1).kids(download),
            Item::new("ctx-rename", text("重命名"), "pencil", 1),
            Item::new("ctx-delete", text("删除"), "trash", 2).danger(),
        ]
    };
    // 文件夹的菜单照 Craft 不带标题。
    let title = if e.meta.folder.is_some() { String::new() } else { title };
    Some(div().absolute().inset_0().child(menu::context(SharedString::from(key.to_string()), title, radius, items)).child(mark).into_any_element())
}
/// `render_folder_card` 报的圆角。
const FOLDER_CARD_R: f32 = 10.;
/// 回收站的行：恢复、彻底删除。
pub fn trash_context(id: &str, title: SharedString) -> Option<AnyElement> { trash_context_r(id, title, 10.) }
/// 同上，浮起来的那块圆角是 `radius`（「最近删除」的卡片是 20）。
pub fn trash_context_r(id: &str, title: SharedString, radius: f32) -> Option<AnyElement> {
    if !floating() { return None; }
    use crate::settings::text;
    let items = vec![
        menu::Item::new("ctx-restore", text("恢复"), "arrow.uturn.backward", 0),
        menu::Item::new("ctx-purge", text("彻底删除"), "trash", 0).danger(),
    ];
    Some(menu::context(SharedString::from(id.to_string()), title, radius, items).into_any_element())
}

/// 块动作栏剪掉 / 删掉了 `n` 块：这时正文没有焦点、键盘条（带撤销）不在，顶上一条提示带「撤销」。
/// 没有原生提示的旧系统只出一行字。
pub fn blocks_removed(n: usize, cut: bool) {
    menu::dispatch(move |w, _, cx| {
        let msg = match (cut, n) {
            (true, 1) => crate::settings::tr("已剪切 1 个块", "Cut 1 block").to_string(),
            (true, _) => crate::localized_format!("已剪切 {} 个块", "Cut {} blocks", n),
            (false, 1) => crate::settings::tr("已删除 1 个块", "Deleted 1 block").to_string(),
            (false, _) => crate::localized_format!("已删除 {} 个块", "Deleted {} blocks", n),
        };
        w.show_toast(&msg, cx);
        if toast::native() { w.toast_link(crate::settings::tr("撤销", "Undo"), "toast-undo", cx); }
    });
}

/// 系统菜单里的一项被点了（`menu.rs` 把它送回工作区）。
pub fn menu_action(id: &str, w: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    if coveredit::menu_action(id, w, window, cx) { return; }
    match id {
        "doc-catalogue" if w.view == View::Doc => {
            if let Some(editor) = &w.editor { editor.update(cx, |e, cx| e.toggle_catalogue(cx)); }
        }
        "hub-new-folder" => w.new_folder(None, window, cx),
        "hub-fold" => { FOLDERS_FOLDED.with(|f| f.set(!f.get())); cx.notify(); }
        "hub-customise" => customise::open(),
        "hub-star-pick" => starpick::open(w),
        "hub-starred-fold" => { STARRED_FOLDED.with(|f| f.set(!f.get())); cx.notify(); }
        "hub-tag-pick" => tags::open_pick(w, tags::Mode::Pin),
        "hub-tags-fold" => { tags::toggle_folded(); cx.notify(); }
        "browse-new-folder" => { let parent = if w.view == View::Folder { w.folder_current.clone() } else { None }; w.new_folder(parent, window, cx); }
        "browse-import-md" | "browse-import-folder" | "browse-import-xml" => {
            use crate::transfer::ImportKind;
            let kind = match id { "browse-import-md" => ImportKind::Markdown, "browse-import-folder" => ImportKind::Folder, _ => ImportKind::DocxXml };
            let parent = if w.view == View::Folder { w.folder_current.clone() } else { None };
            w.open_import_kind(kind, parent, window, cx);
        }
        // 批量管理底部的动作栏：没选东西时不做事。
        "batch-copy" | "batch-move" | "batch-delete" | "batch-restore" | "batch-purge" | "batch-star" | "batch-export" if w.batch.as_ref().is_none_or(|s| s.is_empty()) => {}
        // 收藏：选中的有没收藏的就全收藏上，全都收藏着就全取消（文件夹不能收藏，跳过）。
        "batch-star" => {
            glass::haptic();
            let ids = w.batch.clone().unwrap_or_default();
            let docs: Vec<String> = ids.into_iter().filter(|id| w.entries.iter().any(|e| &e.id == id && e.meta.folder.is_none())).collect();
            let on = docs.iter().any(|id| w.entries.iter().any(|e| &e.id == id && e.meta.starred == 0));
            for id in &docs {
                if let Ok(t) = w.store.set_starred(id, on) && let Some(e) = w.entries.iter_mut().find(|e| &e.id == id) { e.meta.starred = t; }
            }
            w.batch = None;
            let msg = if on { crate::localized_format!("已收藏 {} 篇文档", "Starred {} documents", docs.len()) } else { crate::localized_format!("已取消收藏 {} 篇文档", "Unstarred {} documents", docs.len()) };
            w.show_toast(&msg, cx);
            cx.notify();
        }
        "batch-export" => { glass::haptic(); let ids = w.batch.take().unwrap_or_default(); w.export_selected_md(&ids, cx); cx.notify(); }
        "batch-restore" => { glass::haptic(); w.restore_batch(cx); }
        "batch-all" => {
            let ids = BATCH_IDS.with(|b| b.borrow().clone());
            let all = w.batch.as_ref().is_some_and(|s| ids.iter().all(|id| s.contains(id)));
            w.batch = Some(if all { vec![] } else { ids.into_iter().take(crate::sidebar::BATCH_LIMIT).collect() });
            cx.notify();
        }
        "batch-purge" => { let ids = w.batch.clone().unwrap_or_default(); w.confirm = Some(crate::app::Confirm::PurgeMany(ids)); cx.notify(); }
        // 文件夹不能「创建副本」：只复制选中的文档。
        "batch-copy" => { let docs: Vec<String> = w.batch.clone().unwrap_or_default().into_iter().filter(|id| w.entries.iter().any(|e| &e.id == id && e.meta.folder.is_none())).collect(); w.batch = Some(docs); w.batch_copy(cx) }
        "batch-move" => w.batch_move(window, cx),
        "batch-delete" => w.ask_batch_delete(cx),
        "browse-grid" => w.set_home_grid(true, cx),
        "browse-list" => w.set_home_grid(false, cx),
        // 删除后提示里的「打开回收站」。新建按钮是玻璃自己接的触摸。
        "tab-trash" => if w.view != View::Trash { w.show_view(View::Trash, cx) },
        "trash-empty-all" => { w.trash_menu = None; w.confirm = Some(crate::app::Confirm::EmptyTrash); cx.notify(); }
        _ if id.starts_with("find-") && search::menu_action(id, w, cx) => {}
        _ if search::set_mode(id) => { cx.notify(); }
        _ if keybar::menu_action(id, w, window, cx) => {}
        _ if voice::menu_action(id, w, window, cx) => {}
        _ if assist::menu_action(id, w, window, cx) => {}
        _ if imagine::menu_action(id, w, window, cx) => {}
        _ if video::menu_action(id, w, window, cx) => {}
        // 设置子页的选项行（`general::choice`）。
        "pref-lang-zh" | "pref-lang-en" => { let en = id == "pref-lang-en"; if crate::settings::prefs().english != en { w.set_pref(|p| p.english = en, cx); } }
        _ if assistant::menu_action(id, w, cx) => {}
        _ if id.starts_with("theme-pick-") => {
            let picked = id["theme-pick-".len()..].parse::<usize>().ok().and_then(|i| crate::themes::themes().get(i).map(|t| t.id.clone()));
            if let Some(theme) = picked { crate::settings::apply_theme(w, theme, cx); }
        }
        "space-loc-local" | "space-loc-icloud" => spaces::pick_location(w, id == "space-loc-icloud", cx),
        "new-doc" | "fab-new-doc" => { glass::haptic(); w.new_doc(window, cx); }
        // 原生的新建文件夹卡片（`card.rs`）。
        "new-folder-create" => w.finish_new_folder(true, window, cx),
        "new-folder-cancel" => w.finish_new_folder(false, window, cx),
        // 系统提示框（`alert.rs`）的两颗按钮。
        "confirm-hide-refs-always" => if let Some(crate::app::Confirm::Refs { show: false, no_ask }) = &mut w.confirm {
            *no_ask = true;
            w.confirm_ok(window, cx);
        }
        "confirm-ok" => confirm_ok(w, window, cx),
        "confirm-cancel" => { w.confirm = None; w.after_save = None; cx.notify(); }
        "fab-new-folder" => { let parent = if w.view == View::Folder { w.folder_current.clone() } else { None }; w.new_folder(parent, window, cx); }
        // 文档页底部的铅笔：在文末新起一行开始写。
        "doc-append" => if let Some(editor) = w.editor.clone() { glass::haptic(); editor.update(cx, |ed, cx| ed.append_line(window, cx)); },
        "doc-history" => { glass::haptic(); HISTORY.with(|h| h.set(true)); cx.notify(); }
        "doc-undo" | "doc-redo" => if let Some(editor) = w.editor.clone() { glass::haptic(); editor.update(cx, |ed, cx| ed.history_step(id == "doc-redo", window, cx)); },
        // 提示上的「撤销」：撤回之后提示也收起。
        "toast-undo" => { w.toast = None; if let Some(editor) = w.editor.clone() { editor.update(cx, |ed, cx| ed.history_step(false, window, cx)); } cx.notify(); }
        "doc-paste" => if let Some(editor) = w.editor.clone() { editor.update(cx, |ed, cx| ed.paste_at_end(window, cx)); },
        // 「+」：都是在文末新起一行，再往里放东西。
        "doc-add-more" | "doc-add-file" | "doc-add-image" => if let Some(editor) = w.editor.clone() { editor.update(cx, |ed, cx| ed.append_with(id, window, cx)); },
        "page-restore" => restore(w, window, cx),
        // 文档页「…」：对当前文档。
        _ if id.starts_with("doc-") => {
            let Some(cur) = w.current.clone() else { return };
            use crate::store::PageWidth;
            match id {
                "doc-width-standard" => w.set_page_width(PageWidth::Standard, cx),
                "doc-width-wide" => w.set_page_width(PageWidth::Wide, cx),
                "doc-width-full" => w.set_page_width(PageWidth::Full, cx),
                "doc-outline" => if let Some(ed) = w.editor.clone() { ed.update(cx,|e,cx| { e.cat_pinned = false; e.toggle_catalogue(cx); }); },
                "doc-find" => w.find_in_doc(false, window, cx),
                "doc-text-size" => { style::set_sizing(true); if let Some(editor) = w.editor.clone() { let focus = w.focus.clone(); if editor.read(cx).focus.is_focused(window) { window.focus(&focus, cx); } } cx.notify(); }
                "doc-style" => { glass::haptic(); format::close(); stylepanel::open(); if let Some(editor) = w.editor.clone() { let focus = w.focus.clone(); if editor.read(cx).focus.is_focused(window) { window.focus(&focus, cx); } } cx.notify(); }
                "doc-move" => w.open_move(vec![cur], window, cx),
                "doc-pin" => w.toggle_pin(&cur, cx),
                "doc-star" => w.toggle_star(&cur, cx),
                "doc-copy" => w.duplicate_doc(&cur, window, cx),
                "doc-pdf" => w.export_pdf(&cur, window, cx),
                "doc-md" => w.open_export_md(&cur, cx),
                "doc-print" => w.print_doc(&cur, window, cx),
                "doc-select-all-blocks" => select_all_blocks(w, window, cx),
                "doc-clean-empty" => clean_empty_paragraphs(w, window, cx),
                "doc-info" => { stylepanel::close(); format::close(); w.open_doc_info(window, cx) }
                "doc-refs" => { let shown = !w.entries.iter().find(|e| e.id == cur).is_some_and(|e| e.meta.refs_hidden); w.ask_refs(!shown, cx); }
                "doc-delete" => w.ask_delete(&cur, cx),
                _ => {}
            }
        }
        // 长按出的上下文菜单：对长按的那一项。
        _ if id.starts_with("ctx-") => {
            let target = menu::context_target().map(|t| t.to_string()).unwrap_or_default();
            match id {
                "ctx-batch-copy" => w.batch_copy(cx),
                "ctx-batch-move" => w.batch_move(window, cx),
                "ctx-batch-delete" => w.ask_batch_delete(cx),
                _ if target.is_empty() => {}
                "ctx-copy" => w.duplicate_doc(&target, window, cx),
                "ctx-move" => w.open_move(vec![target], window, cx),
                "ctx-pin" => w.toggle_pin(&target, cx),
                "ctx-star" => w.toggle_star(&target, cx),
                "ctx-pdf" => w.export_pdf(&target, window, cx),
                "ctx-md" => w.open_export_md(&target, cx),
                "ctx-rename" => w.start_rename(&target, window, cx),
                "ctx-delete" => w.ask_delete(&target, cx),
                // 文件夹的「排序方式」：和浏览页「…」里的是同一个排法。
                "ctx-sort-0" => w.pick_home_sort(0, cx),
                "ctx-sort-1" => w.pick_home_sort(1, cx),
                "ctx-sort-2" => w.pick_home_sort(2, cx),
                "ctx-sort-3" => w.pick_home_sort(3, cx),
                "ctx-sort-4" => w.pick_home_sort(4, cx),
                "ctx-export-md" => w.export_folder_md(&target, cx),
                "ctx-new-doc" => w.new_child(&target, window, cx),
                "ctx-new-folder" => w.new_folder(Some(target), window, cx),
                "ctx-icon" => { pick_icon(Some(target)); cx.notify(); }
                "ctx-edit-folder" => edit_folder(w, &target, cx),
                "ctx-restore" => { w.confirm = Some(crate::app::Confirm::Restore(target)); cx.notify(); }
                "ctx-purge" => { w.confirm = Some(crate::app::Confirm::Purge(target)); cx.notify(); }
                _ => {}
            }
        }
        // 系统的编辑菜单（`editmenu.rs`）：交给编辑器。
        _ if id.starts_with("edit-") => if let Some(editor) = w.editor.clone() { editor.update(cx, |ed, cx| ed.touch_menu_action(id, window, cx)); },
        _ if id.starts_with("browse-sort-") => if let Ok(index) = id["browse-sort-".len()..].parse() { w.pick_home_sort(index, cx); },
        _ => {}
    }
}

/// 手机上的确认框：系统提示框的样子，居中的圆角卡片，标题、一段说明，下面两颗等宽的胶囊按钮（取消在左）。
/// 文案和桌面的确认框是同一份，危险的动作字是红的。
pub fn alert(title: String, body: String, action: &'static str, danger: bool, cx: &mut Context<Workspace>) -> impl IntoElement {
    let button = |id: &'static str, label: &'static str, color: Hsla| div().id(id).flex_1().h(px(46.)).rounded_full().flex().items_center().justify_center()
        .bg(Theme::fill_hover()).active(|s| s.bg(Theme::fill_pressed())).text_size(px(16.)).font_weight(FontWeight::MEDIUM).text_color(color).child(label);
    sheet_plane(crate::editor_ui::popover()).shadow(crate::theme::shadow_dialog()).rounded(px(30.)).id("confirm-dialog").occlude().relative().w(px(296.)).p(px(16.)).flex().flex_col()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(div().px(px(12.)).pt(px(6.)).text_size(px(16.)).line_height(px(22.)).font_weight(FontWeight::SEMIBOLD).text_color(Theme::text()).child(title))
        .when(!body.is_empty(), |d| d.child(div().px(px(12.)).mt(px(4.)).text_size(px(13.)).line_height(px(19.)).text_color(Theme::text_caption()).child(body)))
        .child(div().mt(px(16.)).flex().gap(px(8.))
            .child(button("confirm-cancel", crate::settings::text("取消"), Theme::text()).on_click(cx.listener(|w, _, _, cx| { w.confirm = None; w.after_save = None; cx.notify(); })))
            .child(button("confirm-ok", action, if danger { Theme::danger() } else { Theme::primary() }).on_click(cx.listener(|w, _, window, cx| confirm_ok(w, window, cx)))))
}

/// 确认框点了确定：删除文档（进最近删除、彻底删除、清空）先来一下厚一点的删除触感（同样式面板里的删除）。
fn confirm_ok(w: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    use crate::app::Confirm;
    if matches!(w.confirm, Some(Confirm::Delete { .. } | Confirm::Purge(_) | Confirm::PurgeMany(_) | Confirm::EmptyTrash)) { glass::delete_haptic(); }
    w.confirm_ok(window, cx);
}

thread_local! { static NEW_FOLDER: std::cell::RefCell<Option<Option<String>>> = const { std::cell::RefCell::new(None) }; }
/// 正在填、还没建出来的新文件夹建在哪一层（`Some(None)` 是根目录）。手机上新建文件夹先在卡片（`card.rs`）里填好，点「创建」才真的建，
/// 建好就进到里面；关掉卡片什么也不留下（桌面是先建一个「未命名文件夹」再就地改名）。
pub fn new_folder_parent() -> Option<Option<String>> { NEW_FOLDER.with(|n| n.borrow().clone()) }
pub fn set_new_folder(parent: Option<Option<String>>) { if parent.is_none() { EDIT_FOLDER.with(|e| e.borrow_mut().take()); } NEW_FOLDER.with(|n| *n.borrow_mut() = parent); }
thread_local! { static EDIT_FOLDER: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) }; }
/// 卡片正在编辑的那个已有文件夹（没有就是在新建）。
pub fn editing_folder() -> Option<String> { EDIT_FOLDER.with(|e| e.borrow().clone()) }
/// 长按文件夹的「编辑…」：同一张卡片，填上它现在的名称、描述、图标、颜色（照 Craft，IMG_9145）。
fn edit_folder(w: &mut Workspace, id: &str, cx: &mut Context<Workspace>) {
    let Some(e) = w.entries.iter().find(|e| e.id == id && e.meta.folder.is_some()) else { return };
    card::prefill(Some(card::Draft { name: e.title.clone(), note: e.meta.folder_note.clone().unwrap_or_default(), icon: e.meta.folder_icon.clone(), color: e.meta.folder_color.clone() }));
    let parent = e.meta.parent.clone();
    set_new_folder(Some(parent));
    EDIT_FOLDER.with(|f| *f.borrow_mut() = Some(id.to_string()));
    w.row_menu = None;
    cx.notify();
}

/// 手机上的重命名：没有侧栏，名字在屏幕上方的一张卡片里改。回车或点别处保存。
/// 新建文件夹是另一张原生的卡片（`card.rs`），也从这里挂出来。
pub fn rename_dialog(w: &Workspace, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    if !floating() { return None; }
    // 新建文件夹：卡片是原生的（`card.rs`），这里只有压暗的遮罩和它的占位。点遮罩是取消。
    if new_folder_parent().is_some() {
        return Some(div().id("rename-scrim").absolute().inset_0().occlude().on_click(cx.listener(|w, _, window, cx| w.finish_new_folder(false, window, cx)))
            .child(sheet_scrim())
            // 贴在键盘上面（`insets().bottom` 在键盘弹出时就是键盘的高度）。
            .child(div().id("new-folder-card").absolute().bottom(insets().bottom + px(12.)).left(px(16.)).right(px(16.)).h(px(card::HEIGHT)).occlude()
                .on_click(|_, _, cx| cx.stop_propagation()).child(card::slot()))
            .into_any_element());
    }
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
    let grid = div().px(px(12.)).py(px(8.)).flex().flex_wrap().children(folder_icons::CHOICES.iter().enumerate().map(|(i, choice)|
        div().id(("folder-icon", i)).w(px(cell.floor())).h(px(48.)).flex().items_center().justify_center().rounded(px(10.))
            .active(|s| s.bg(Theme::fill_pressed())).child(folder_icons::icon(choice.glyph, 24., Theme::icon()))
            .on_click(cx.listener(set(Some(choice.value.to_string()), id.clone())))));
    let panel = sheet_plane(crate::editor_ui::popover()).id("folder-icons").occlude().w(menu_w(0.)).py(px(crate::ui::MENU_PAD_Y))
        .on_mouse_down_out(cx.listener(|_, _, _, cx| { pick_icon(None); cx.notify(); }))
        .child(div().px(px(20.)).pt(px(8.)).pb(px(4.)).text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).text_color(Theme::text_caption()).child(crate::settings::tr("文件夹图标", "Folder icon")))
        .child(grid)
        .child(crate::ui::menu_row("folder-icon-none", SharedString::from("folder-icon-none")).child(crate::ui::icon("folder", 16., Theme::icon())).child(crate::settings::tr("使用默认图标", "Use the default icon"))
            .on_click(cx.listener(set(None, id.clone()))));
    Some(div().absolute().inset_0().child(sheet_scrim()).child(deferred(place(Anchor::BottomCenter, point(px(0.), px(0.))).child(panel)).with_priority(3)).into_any_element())
}


/// 搜索条的高度，搜索页的输入框也是这么高（从它这里滑上去）。
pub const SEARCH_H: f32 = 44.;
/// 搜索条的底色。深色下 `fill_hover`（约 11/255）在纯黑上几乎看不出来；Craft 深色实测约 29/255（IMG_9111），取同样的亮度。
pub fn search_fill() -> Hsla { if crate::theme::dark() { crate::theme::ca(crate::theme::craft::N1000, 0.11) } else { Theme::fill_hover() } }

/// 大标题下的搜索条：点了打开搜索。
pub fn search_bar(cx: &mut Context<Workspace>) -> impl IntoElement { search_bar_with(crate::settings::tr("搜索文档", "Search documents"), cx) }
/// 照 Craft（IMG_9111）：高 44 的胶囊，左右离屏幕 16，字 17。
pub fn search_bar_with(label: impl Into<SharedString>, cx: &mut Context<Workspace>) -> impl IntoElement {
    let label: SharedString = label.into();
    div().id("home-search-bar").relative().flex_none().mx(px(16.)).mt(px(4.)).h(px(SEARCH_H)).px(px(14.)).flex().items_center().gap(px(8.))
        .rounded_full().bg(search_fill()).active(|s| s.bg(Theme::fill_pressed()))
        .child(crate::ui::icon("search", 18., Theme::text_caption()))
        .child(div().flex_1().min_w_0().overflow_hidden().whitespace_nowrap().text_ellipsis().text_size(px(17.)).text_color(Theme::text_caption()).child(label))
        // 右边的搜索方式（照 Craft，IMG_9193）：点它弹菜单，点条上别处才打开搜索（系统的按钮盖在这一块上，先接住手指）。
        .child(div().relative().flex_none().h_full().pl(px(10.)).mr(px(-4.)).pr(px(4.)).flex().items_center().gap(px(3.))
            .text_size(px(15.)).text_color(Theme::text_placeholder())
            .child(search::mode_label())
            .child(crate::ui::icon("chevron-down", 12., Theme::text_placeholder()))
            .child(menu::button("search-mode", search::mode_items())))
        // 记下自己画在哪：点了以后搜索页的输入框从这儿滑上去。
        .child(canvas(|bounds, _, _| search::bar_at(bounds), |_, _, _, _| {}).absolute().inset_0())
        .on_click(cx.listener(|w, _, window, cx| { search::opened(); w.open_search(window, cx) }))
}

/// 右下角的悬浮新建按钮，在拇指的位置。有 Liquid Glass 时是和搜索并排的一块玻璃，GPUI 这边只留点击区。
/// 在文件夹这一层（文件夹列表、某个文件夹里）它是一个小菜单：新建文档 / 新建文件夹，都建在眼前这一层；别处点了直接新建文档。
pub fn new_button(view: View, cx: &mut Context<Workspace>) -> impl IntoElement {
    let pick = view == View::Folder || (view == View::Home && home_page() == HomePage::Folders);
    div().id("home-fab").absolute().right(px(20.)).rounded_full().flex().items_center().justify_center()
        .map(|d| if floating() && pick { d.bottom(dock_inset()).size(px(48.)).child(menu::glass_button("fab-menu", "plus", Theme::text(), vec![
                menu::Item::new("fab-new-doc", crate::settings::tr("新建文档", "New document"), "doc.badge.plus", 0),
                menu::Item::new("fab-new-folder", crate::settings::tr("新建文件夹", "New folder"), "folder.badge.plus", 0),
            ])) }
            else if floating() { d.bottom(dock_inset()).size(px(48.)).child(menu::glass_tap("fab", "plus", Theme::text(), "new-doc", crate::settings::text("新建文档"))) }
            else { d.bottom(px(16.)).size(px(56.)).bg(Theme::primary()).shadow(crate::theme::shadow_dialog()).active(|s| s.bg(crate::theme::c(crate::theme::craft::B700))).child(crate::ui::icon("plus", 26., gpui::white())) })
        .occlude().on_click(cx.listener(|w, _, window, cx| w.new_doc(window, cx)))
}

thread_local! { static OVERLAY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
/// 正文之上盖着别的东西（抽屉、搜索、菜单、对话框），或当前不在文档页：长按不该选正文里的字。
pub fn set_overlay(on: bool) { OVERLAY.with(|o| o.set(on)); }
pub fn overlay() -> bool { OVERLAY.with(|o| o.get()) }

thread_local! { static LONG_PRESS_TAKEN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
/// 这次长按已经有人用了（编辑器把块提起来拖）：不要再补一次右键，那会让正文拿到焦点、弹出键盘。
pub fn take_long_press() { LONG_PRESS_TAKEN.with(|t| t.set(true)); }

/// 长按当右键：侧栏的行、主页的卡片和回收站的行，菜单原本都挂在右键上。
pub fn long_press_layer() -> impl IntoElement {
    canvas(|_, _, _| {}, |_, _, window, _| {
        window.on_mouse_event(|ev: &ScrollEvent, phase, window, _| {
            if phase == DispatchPhase::Capture && ev.delta.pixel_delta(px(1.)) != Point::default() { menu::before_scroll(window); }
        });
        // 手指抬起后没有「悬停」：把指针移出窗口，免得刚点过的行一直亮着。
        window.on_mouse_event(|ev: &MouseUpEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble || ev.button != MouseButton::Left { return; }
            window.defer(cx, |window, cx| {
                window.dispatch_event(PlatformInput::MouseMove(MouseMoveEvent { position: point(px(-100.), px(-100.)), pressed_button: None, modifiers: Modifiers::default() }), cx);
            });
        });
        // TRACE-TOUCH（临时）：看按下落在哪。
        if std::env::var_os("QUIET_MD_IOS_TRACE").is_some() {
            window.on_mouse_event(|ev: &MouseDownEvent, phase, _, _| if phase == DispatchPhase::Capture { eprintln!("TRACE down {:?} {:?}", ev.button, ev.position); });
            window.on_mouse_event(|ev: &MouseUpEvent, phase, _, _| if phase == DispatchPhase::Capture { eprintln!("TRACE up {:?} {:?}", ev.button, ev.position); });
        }
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
            // 这一块有系统的上下文菜单（卡片、行）：交给系统，这里不补右键，也不吃掉之后的点按。
            if menu::context_at(ev.position) { return; }
            if overlay() { EAT.with(|e| e.set((true, true, Some(std::time::Instant::now())))); }
            glass::haptic();
            let position = ev.position;
            // 当前事件还在分发，右键放到它结束之后。
            window.defer(cx, move |window, cx| {
                if LONG_PRESS_TAKEN.with(|t| t.replace(false)) { return; }
                let modifiers = Modifiers::default();
                window.dispatch_event(PlatformInput::MouseDown(MouseDownEvent { button: MouseButton::Right, position, modifiers, click_count: 1, first_mouse: false }), cx);
                window.dispatch_event(PlatformInput::MouseUp(MouseUpEvent { button: MouseButton::Right, position, modifiers, click_count: 1 }), cx);
            });
        });
    }).absolute().size_0()
}

thread_local! { static DOC_FROM: std::cell::Cell<View> = const { std::cell::Cell::new(View::Home) }; }
/// 文档里打开了几层页面（页面 / 卡片，没打开是 0）：往里一层从右边滑入，回外面一层往右滑走。
pub fn sub_depth(w: &Workspace, cx: &App) -> i32 {
    if w.view == View::Settings { return (settings_detail() && assistant::parent(w.settings_page).is_some()) as i32; }
    w.editor.as_ref().map_or(0, |e| e.read(cx).outer.len() as i32)
}
/// 打开着的那一页（页面 / 卡片）在外面那一层里的块。
fn open_page(w: &Workspace, cx: &App) -> Option<fdl_core::model::BlockId> {
    w.editor.as_ref().and_then(|e| e.read(cx).outer.last().map(|o| o.page))
}
/// 从别的页进文档时记下是哪一页（文档里点链接去另一篇不改）。
pub fn entered_doc(from: View) { DOC_FROM.with(|f| f.set(from)); }
/// 文档页的返回：回到进来的那一页。从文件夹进的回那个文件夹（滚动位置还在），文件夹没了就回主页；
/// 从主页进的回主页，首页、「所有文档」「文件夹」哪一页由 `home_page` 记着。
/// 打开着页面（页面 / 卡片）时先回外面那一层。
pub fn doc_back(w: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    // 先拍下这一页：回外面一层时它往右滑走，和回主页一样。
    if open_page(w, cx).is_some() && let Some(ed) = w.editor.clone() {
        shot::capture_page();
        ed.update(cx, |e, cx| e.close_page(window, cx));
        return cx.notify();
    }
    shot::leaving(w.current.clone());
    let folder = DOC_FROM.with(|f| f.get()) == View::Folder && w.folder_current.as_ref().is_some_and(|id| w.entries.iter().any(|e| &e.id == id && e.meta.folder.is_some()));
    w.show_view(if folder { View::Folder } else { View::Home }, cx);
}

/// 跟手返回取消后要回到哪一页。
#[derive(Clone)]
enum Restore { Doc(Option<fdl_core::model::BlockId>), Folder(String), Home(HomePage), Settings(crate::settings::Page), SettingsRoot, Trash }
thread_local! { static RESTORE: std::cell::RefCell<Option<Restore>> = const { std::cell::RefCell::new(None) }; }

/// 左缘右滑：文档回进来的那一页，文件夹、回收站和设置回主页，设置的第二级回第一级，主页拉出抽屉。文档、文件夹、主页的子页、回收站和设置
/// 跟着手指：一动就切到下面那一页，旧页的快照跟着手指走（`shot::follow`）；松手过半就滑走，没过半滑回来、
/// 再恢复原来那页。
pub fn back(w: &mut Workspace, ev: &BackGestureEvent, window: &mut Window, cx: &mut Context<Workspace>) {
    let following = GESTURE.with(|g| g.get()).is_some();
    match ev.phase {
        BackGesturePhase::Progressed if following => {
            GESTURE.with(|g| g.set(Some(ev.progress)));
            shot::follow(ev.progress);
            cx.notify();
            return;
        }
        BackGesturePhase::Progressed => {
            if ev.progress <= 0. || !floating() || w.search_open || w.rename.is_some() || new_folder_parent().is_some() { return; }
            // 「选择」时跟手返回和左上角的返回一样：先退出选择，再回上一页。
            if w.batch.is_some() { w.batch = None; w.row_menu = None; }
            let restore = match w.view {
                View::Doc => Restore::Doc(open_page(w, cx)),
                View::Folder => match w.folder_current.clone() { Some(id) => Restore::Folder(id), None => return },
                View::Home if home_page() != HomePage::Hub => Restore::Home(home_page()),
                View::Settings if settings_detail() => Restore::Settings(w.settings_page),
                // 设置第一级和「最近删除」都是从首页点进去的，同样跟手回首页。
                View::Settings => Restore::SettingsRoot,
                View::Trash => Restore::Trash,
                View::Home => return,
            };
            // 切到下面那一页：这几条路都会先拍下这一页。
            pop(w, window, cx);
            if !shot::page_pending() { return; }
            RESTORE.with(|r| *r.borrow_mut() = Some(restore));
            GESTURE.with(|g| g.set(Some(ev.progress)));
            shot::follow(ev.progress);
            cx.notify();
            return;
        }
        BackGesturePhase::Committed if following => {
            GESTURE.with(|g| g.take());
            RESTORE.with(|r| r.borrow_mut().take());
            shot::slide_out(shot::release_speed().max(0.));
            PAGE.with(|s| { let (key, epoch, _) = s.get(); s.set((key, epoch + 1, Motion::Still)); });
            cx.notify();
            return;
        }
        BackGesturePhase::Cancelled if following => {
            GESTURE.with(|g| g.take());
            // 快照没了就没有滑回去的动画来接恢复：直接恢复。
            if !shot::page_pending() { return restore(w, window, cx); }
            PAGE.with(|s| { let (key, epoch, _) = s.get(); s.set((key, epoch + 1, Motion::Return)); });
            shot::slide_back((-shot::release_speed()).max(0.), "page-restore");
            cx.notify();
            return;
        }
        BackGesturePhase::Committed => {}
        _ => return,
    }
    if w.search_open { return w.close_search(window, cx); }
    w.batch = None;
    match w.view {
        View::Doc => doc_back(w, window, cx),
        View::Folder | View::Trash => w.show_view(View::Home, cx),
        View::Home if home_page() != HomePage::Hub => { set_home_page(HomePage::Hub); cx.notify(); }
        View::Settings if settings_detail() => settings_back(w, cx),
        View::Settings => w.show_view(View::Home, cx),
        // 有标签栏时主页不拉抽屉。
        View::Home => { w.drawer = compact(window) && !floating(); cx.notify(); }
    }
}
/// 跟手返回切到下面那一页：和各页左上角的返回一样（文件夹回上一级）。
fn pop(w: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    match w.view {
        View::Folder => match w.folder_current.as_ref().and_then(|id| w.entries.iter().find(|e| &e.id == id)).and_then(|e| e.meta.parent.clone()) {
            Some(parent) => { w.open(&parent, window, cx); }
            None => w.show_view(View::Home, cx),
        },
        View::Home => { set_home_page(HomePage::Hub); cx.notify(); }
        View::Settings if settings_detail() => settings_back(w, cx),
        View::Doc => doc_back(w, window, cx),
        _ => w.show_view(View::Home, cx),
    }
}
/// 跟手返回取消、快照滑回原处之后：恢复原来那页，快照随后淡掉。
fn restore(w: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let Some(to) = RESTORE.with(|r| r.borrow_mut().take()) else { return shot::fade_page(); };
    // 快照先盖着，等原来那页画出来几帧再淡（`shot::park_page`）。
    shot::park_page(8);
    RESTORING.with(|r| r.set(true));
    match to {
        // 从页面回外面一层又取消了：重新打开那一页。
        Restore::Doc(Some(page)) => if let Some(ed) = w.editor.clone() { ed.update(cx, |e, cx| e.open_page(page, window, cx)); },
        Restore::Doc(None) => w.show_view(View::Doc, cx),
        Restore::Folder(id) => { w.open(&id, window, cx); }
        Restore::Home(page) => { set_home_page(page); cx.notify(); }
        Restore::Settings(page) => { set_settings_detail(true); if w.settings_page != page { w.open_settings(page, cx); } else { cx.notify(); } }
        Restore::SettingsRoot => { set_settings_detail(false); w.show_view(View::Settings, cx); }
        Restore::Trash => w.show_view(View::Trash, cx),
    }
}

/// 选文件、存文件都经过这里（上游 gpui-pre-ios 0.3.12 没有实现，见 `files.rs`）。
pub fn prompt_for_paths(_: &mut App, options: PathPromptOptions) -> files::Reply<Option<Vec<std::path::PathBuf>>> { files::open(options) }
/// 选一张图片（插入图片、上传封面）：照片图库或文件。
pub fn prompt_for_images(_: &mut App, _: PathPromptOptions) -> files::Reply<Option<Vec<std::path::PathBuf>>> { files::open_image() }
/// 「视频或文件」：先问从哪里选（照片图库里的视频，或文件）。
pub fn prompt_for_attachments(_: &mut App, _: PathPromptOptions) -> files::Reply<Option<Vec<std::path::PathBuf>>> { files::open_attachment() }
/// 给出的是沙盒里的临时路径：调用方写完后要调 `exported`，系统的面板才把它存到使用者选的地方。
pub fn prompt_for_new_path(_: &mut App, _: &std::path::Path, name: Option<&str>) -> files::Reply<Option<std::path::PathBuf>> { files::new_path(name) }
/// `prompt_for_new_path` 给的路径写好了。
pub fn exported(path: &std::path::Path) { files::export(path) }

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
