use std::time::Duration;

use gpui::*;

use crate::app::{View, Workspace};
use crate::theme::Theme;

pub mod keybar;

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
pub fn place(corner: Anchor, pos: Point<Pixels>) -> Anchored {
    let (compact, safe, viewport) = LAYOUT.with(|l| l.get());
    if !compact { return anchored().anchor(corner).position(pos).snap_to_window_with_margin(px(8.)); }
    anchored().anchor(Anchor::BottomCenter).position(point(viewport.width / 2., viewport.height - safe.bottom - px(8.)))
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
pub fn sheet_scrim() -> Div { div().absolute().inset_0().bg(crate::ui::scrim()) }

/// 主页标题下的搜索条：点了打开搜索。
pub fn search_bar(cx: &mut Context<Workspace>) -> impl IntoElement {
    div().id("home-search-bar").flex_none().mx(px(20.)).mt(px(4.)).h(px(40.)).px(px(12.)).flex().items_center().gap(px(8.))
        .rounded(px(12.)).bg(Theme::fill_hover()).active(|s| s.bg(Theme::fill_pressed()))
        .child(crate::ui::icon("search", 17., Theme::text_caption()))
        .child(div().text_size(px(16.)).text_color(Theme::text_caption()).child(crate::settings::tr("搜索文档", "Search documents")))
        .on_click(cx.listener(|w, _, window, cx| w.open_search(window, cx)))
}

/// 右下角的悬浮新建按钮，在拇指的位置。
pub fn new_button(cx: &mut Context<Workspace>) -> impl IntoElement {
    div().id("home-fab").absolute().right(px(20.)).bottom(px(16.)).size(px(56.)).rounded_full().flex().items_center().justify_center()
        .bg(Theme::primary()).shadow(crate::theme::shadow_dialog()).active(|s| s.bg(crate::theme::c(crate::theme::craft::B700)))
        .occlude().child(crate::ui::icon("plus", 26., gpui::white()))
        .on_click(cx.listener(|w, _, window, cx| w.new_doc(window, cx)))
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
        View::Settings => w.close_settings(cx),
        View::Home => { w.drawer = compact(window); cx.notify(); }
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
