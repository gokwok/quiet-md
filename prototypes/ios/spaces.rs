//! 手机上的空间（照 Craft）：首页左上角的空间头像点了去设置；设置第一级顶上是大头像和「空间名 ⌄」，
//! 点名字从底部浮起一张卡片（「打开空间…」搜索 + 每个空间一行，当前的打勾）；「空间设置」这一页是
//! 图标（带相机钮）、名称、危险区里的删除。没有空间时整页是「创建你的第一个空间」。逻辑都在 `crate::space`。
use crate::app::{Confirm, View, Workspace};
use crate::settings::tr;
use crate::space::{self, name, space_icon};
use crate::theme::{self, Theme};
use gpui::{prelude::FluentBuilder, *};
use fdl_core::spaces::Location;
use objc2::msg_send;
use objc2::runtime::{AnyClass, AnyObject};

const ROW_H: f32 = 52.;

/// 首页左上角：当前空间的头像，点了去设置（第一级）。
pub fn avatar(w: &Workspace, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    let s = space::current(w)?;
    Some(div().id("hub-space").absolute().left(px(20.)).top(super::insets().top + px((crate::app::HEADER_H - 32.) / 2.)).size(px(32.))
        .child(space_icon(s, 32.))
        .on_click(cx.listener(|w, _, _, cx| { super::set_settings_detail(false); w.show_view(View::Settings, cx); }))
        .into_any_element())
}

// 设置页大头像的角度动画：从哪个角度、往哪个角度、什么时候开始。
thread_local! { static TURN: std::cell::Cell<(f32, f32, Option<std::time::Instant>)> = const { std::cell::Cell::new((super::tilt::ANGLE, super::tilt::ANGLE, None)) }; }
const TURN_TIME: f32 = 0.34;

/// 头像右下角的脚标（照 Craft 的「Pro」，截屏 2026-11-15 06.11.36 实测）：这个空间存放在哪里。不跟着头像转，一直是正的。
/// 近黑（32,34,36）的胶囊，22pt 高，没有描边，白色半粗的小字（GPUI 的中等粗细比 Craft 用的系统字细一档，所以用半粗），上下左右都留足空（字只占胶囊高度的一半多一点）。
/// 深色模式反过来：浅色胶囊、深色字。
fn location_badge(location: Location) -> Div {
    let label = match location { Location::ICloud => "iCloud", Location::Local => "Local" };
    let dark = crate::theme::dark();
    let (fill, ink) = if dark { (crate::theme::rgbaf(236, 237, 239, 1.), crate::theme::rgbaf(32, 34, 36, 1.)) } else { (crate::theme::rgbaf(32, 34, 36, 1.), crate::theme::rgbaf(250, 250, 250, 1.)) };
    div().h(px(22.)).px(px(7.5)).flex().items_center().rounded_full().bg(fill)
        .text_size(px(12.5)).line_height(px(15.)).font_weight(FontWeight::SEMIBOLD).text_color(ink).child(label)
}

/// 打开换空间的卡片：轻轻一震，头像转正（`settings_head` 里看 `space_menu`）。
fn open_switcher(w: &mut Workspace, cx: &mut Context<Workspace>) {
    super::glass::haptic();
    w.space_menu = true;
    w.space_query.update(cx, |f, cx| f.set_text("", cx));
    cx.notify();
}

/// 设置第一级的页头：大头像（照 Craft 微微斜着立起来），下面「空间名 ⌄」。点头像或名字：头像转正，从底部浮起换空间的卡片；
/// 卡片收起后头像再斜回去。
pub fn settings_head(w: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let Some(s) = space::current(w) else { return div().into_any_element() };
    let target = if w.space_menu { 0. } else { super::tilt::ANGLE };
    let now = std::time::Instant::now();
    let angle = TURN.with(|t| {
        let (from, to, start) = t.get();
        let at = |start: Option<std::time::Instant>| start.map_or(1., |s: std::time::Instant| (now - s).as_secs_f32() / TURN_TIME).min(1.);
        let ease = |k: f32| { let k = 1. - k; 1. - k * k * k };
        let current = from + (to - from) * ease(at(start));
        if to != target { t.set((current, target, Some(now))); current }
        else if at(start) >= 1. { t.set((to, to, None)); to }
        else { current }
    });
    let moving = TURN.with(|t| t.get().2.is_some());
    // 角度按半度取整：画好的图能重复用。
    let angle = (angle * 2.).round() / 2.;
    div().w_full().pt(px(8.)).pb(px(20.)).flex().flex_col().items_center().gap(px(12.))
        .child(div().id("settings-space-icon").relative().child(super::tilt::turning(s.icon.as_deref(), &s.id, 84., angle))
            .child(div().absolute().right(px(-12.)).bottom(px(-2.)).child(location_badge(s.location)))
            .on_click(cx.listener(|w, _, _, cx| open_switcher(w, cx))))
        // 转动时一直出帧；绝对定位，不占列里的位置（不然多一道间距，下面的名字会跳）。
        .when(moving, |d| d.relative().child(canvas(|_, window, _| window.request_animation_frame(), |_, _, _, _| {}).absolute().size_0()))
        .child(div().id("settings-space-name").h(px(32.)).px(px(12.)).flex().items_center().gap(px(6.)).rounded(px(10.)).active(|d| d.bg(Theme::fill_pressed()))
            .text_size(px(18.)).line_height(px(24.)).font_weight(FontWeight::SEMIBOLD)
            .child(div().max_w(px(260.)).truncate().child(name(s)))
            .child(crate::ui::icon("chevron-down", 15., Theme::text_caption()))
            .on_click(cx.listener(|w, _, _, cx| open_switcher(w, cx))))
        .into_any_element()
}

thread_local! { static SETTINGS_SCROLL: ScrollHandle = ScrollHandle::new(); }
/// 设置第一级的滚动位置：顶上的磨砂玻璃和居中的「设置」跟着它出现（`veil::home_header`）。
pub fn settings_scroll() -> ScrollHandle { SETTINGS_SCROLL.with(Clone::clone) }

/// 设置里一行只读的信息：左边名称，右边灰色的值。
fn info_row(label: &'static str, value: impl Into<SharedString>) -> AnyElement {
    crate::settings::row(label, None, div().text_size(px(17.)).text_color(Theme::text_caption()).child(value.into())).into_any_element()
}

/// 设置 › 我的空间（照 Craft）：这台设备上的每个空间，头像、名字、存放位置；点一下换过去。最后一行新建空间。
pub fn spaces_page(w: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let here = crate::store::Store::root_dir();
    let mut rows: Vec<AnyElement> = w.spaces.iter().enumerate().map(|(i, s)| {
        let id = s.id.clone();
        let on = s.root == here;
        let place = match s.location { Location::ICloud => "iCloud", Location::Local => "Local" };
        div().id(("settings-space", i)).w_full().h(px(56.)).px(px(16.)).flex().items_center().gap(px(14.)).rounded(px(10.)).active(|d| d.bg(Theme::fill_pressed()))
            .child(space_icon(s, 30.))
            .child(div().flex_1().min_w_0().flex().flex_col()
                .child(div().truncate().text_size(px(17.)).text_color(if s.available { crate::settings::row_ink() } else { Theme::text_disabled() }).child(name(s)))
                .child(div().text_size(px(13.)).text_color(Theme::text_caption()).child(if s.available { place } else { tr("找不到文件夹", "Missing") })))
            .when(on, |d| d.child(crate::ui::icon("check", 18., Theme::primary())))
            .when(s.available && !on, |d| d.on_click(move |_, _, cx| { super::glass::haptic(); let id = id.clone(); cx.defer(move |cx| { space::switch(id, cx); }); }))
            .into_any_element()
    }).collect();
    rows.push(div().id("settings-space-new").w_full().h(px(52.)).px(px(16.)).flex().items_center().gap(px(14.)).rounded(px(10.)).active(|d| d.bg(Theme::fill_pressed()))
        .text_size(px(17.)).text_color(Theme::primary())
        .child(div().w(px(30.)).flex().justify_center().child(crate::ui::icon("plus", 20., Theme::primary()))).child(tr("新建空间…", "New Space…"))
        .on_click(cx.listener(|w, _, window, cx| { super::glass::haptic(); w.open_space_form(window, cx); })).into_any_element());
    div().w_full().pt(px(8.)).child(super::general::card(rows)).into_any_element()
}

// 统计里「占用空间」要把整个文件夹走一遍：按文件夹缓存，打开这一页时才重算。
thread_local! { static SIZE: std::cell::RefCell<Option<(std::path::PathBuf, std::time::Instant, u64)>> = const { std::cell::RefCell::new(None) }; }
fn folder_size(root: &std::path::Path) -> u64 {
    let fresh = SIZE.with(|s| s.borrow().as_ref().filter(|(r, t, _)| r == root && t.elapsed().as_secs() < 30).map(|(_, _, n)| *n));
    if let Some(n) = fresh { return n; }
    fn walk(p: &std::path::Path) -> u64 {
        std::fs::read_dir(p).map(|rd| rd.flatten().map(|e| match e.file_type() { Ok(t) if t.is_dir() => walk(&e.path()), Ok(_) => e.metadata().map_or(0, |m| m.len()), Err(_) => 0 }).sum()).unwrap_or(0)
    }
    let n = walk(root);
    SIZE.with(|s| *s.borrow_mut() = Some((root.to_path_buf(), std::time::Instant::now(), n)));
    n
}
fn bytes(n: u64) -> String {
    let n = n as f64;
    if n >= 1e9 { format!("{:.1} GB", n / 1e9) } else if n >= 1e6 { format!("{:.1} MB", n / 1e6) } else if n >= 1e3 { format!("{:.0} KB", n / 1e3) } else { format!("{n:.0} B") }
}

/// 设置 › 统计：这个空间里有多少东西、占多大。
pub fn stats_page(w: &Workspace, _cx: &mut Context<Workspace>) -> AnyElement {
    let docs = w.entries.iter().filter(|e| e.meta.folder.is_none()).count();
    let folders = w.entries.iter().filter(|e| e.meta.folder.is_some()).count();
    let starred = w.entries.iter().filter(|e| e.meta.folder.is_none() && e.meta.starred > 0).count();
    let root = crate::store::Store::root_dir();
    let place = space::current(w).map_or("Local", |s| match s.location { Location::ICloud => "iCloud", Location::Local => "Local" });
    crate::settings::group(None, vec![
        info_row(tr("文档", "Documents"), docs.to_string()),
        info_row(tr("文件夹", "Folders"), folders.to_string()),
        info_row(tr("收藏", "Starred"), starred.to_string()),
        info_row(tr("最近删除", "Recently deleted"), w.trash.len().to_string()),
        info_row(tr("占用空间", "Storage used"), bytes(folder_size(&root))),
        info_row(tr("存放位置", "Location"), place),
    ]).into_any_element()
}

/// 设置 › 诊断：版本、iCloud 能不能用、日志；日志能导出给人看。
pub fn diagnostics_page(_w: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let log = crate::logging::path();
    let size = std::fs::metadata(&log).map_or(0, |m| m.len());
    let icloud = if fdl_core::spaces::icloud_root().is_some() { tr("可用", "Available") } else { tr("不可用", "Unavailable") };
    let export = div().id("settings-export-log").w_full().h(px(52.)).px(px(16.)).flex().items_center().rounded(px(10.)).active(|d| d.bg(Theme::fill_pressed()))
        .text_size(px(17.)).text_color(Theme::primary()).child(tr("导出日志…", "Export Log…"))
        .on_click(cx.listener(|w, _, _, cx| { super::glass::haptic(); export_log(w, cx); }));
    div().w_full().flex().flex_col().gap(px(24.))
        .child(crate::settings::group(None, vec![
            info_row(tr("版本", "Version"), env!("CARGO_PKG_VERSION")),
            info_row("iCloud", icloud),
            info_row(tr("日志", "Log"), bytes(size)),
        ]))
        .child(crate::settings::group(None, vec![export.into_any_element()]))
        .into_any_element()
}

/// 把日志拷一份交给系统的「存储到…」面板（`files::new_path` / `export`）。
fn export_log(w: &mut Workspace, cx: &mut Context<Workspace>) {
    let reply = super::files::new_path(Some("quiet-md.log"));
    cx.spawn(async move |this, cx| {
        let Ok(Ok(Some(to))) = reply.await else { return };
        let copied = std::fs::copy(crate::logging::path(), &to);
        let _ = this.update(cx, |w, cx| match copied {
            Ok(_) => super::files::export(&to),
            Err(e) => w.show_toast_kind(crate::ui::ToastKind::Error, &e.to_string(), cx),
        });
    }).detach();
    let _ = w;
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

/// 应用自己的 iCloud 文件夹（容器里的 `Documents`，在 iCloud 云盘里显示成应用名的文件夹）。没有就是空：
/// 没登录 iCloud、关了 iCloud 云盘，或者签名里没有 iCloud 容器的权限。
pub fn icloud_container() -> Option<std::path::PathBuf> {
    unsafe {
        let fm: *mut AnyObject = msg_send![AnyClass::get(c"NSFileManager")?, defaultManager];
        let url: *mut AnyObject = msg_send![fm, URLForUbiquityContainerIdentifier: std::ptr::null_mut::<AnyObject>()];
        if url.is_null() { return None; }
        let path: *mut AnyObject = msg_send![url, path];
        if path.is_null() { return None; }
        let utf8: *const std::ffi::c_char = msg_send![path, UTF8String];
        if utf8.is_null() { return None; }
        Some(std::path::PathBuf::from(std::ffi::CStr::from_ptr(utf8).to_string_lossy().into_owned()).join("Documents"))
    }
}

/// 系统的 iCloud 搬家（`setUbiquitous:itemAtURL:destinationURL:error:`）：本机 → iCloud 是交给 iCloud 管，反过来是
/// 拿回本机。整个文件夹一次搬过去，不是自己复制再删。系统要求不在主线程上调用。
fn system_move(from: &std::path::Path, to: &std::path::Path, ubiquitous: bool) -> anyhow::Result<()> {
    unsafe {
        let url = |p: &std::path::Path| -> *mut AnyObject {
            let s = std::ffi::CString::new(p.to_string_lossy().as_bytes()).unwrap_or_default();
            let ns: *mut AnyObject = msg_send![AnyClass::get(c"NSString").unwrap(), stringWithUTF8String: s.as_ptr()];
            msg_send![AnyClass::get(c"NSURL").unwrap(), fileURLWithPath: ns]
        };
        let fm: *mut AnyObject = msg_send![AnyClass::get(c"NSFileManager").unwrap(), new];
        let mut error: *mut AnyObject = std::ptr::null_mut();
        let ok: bool = msg_send![fm, setUbiquitous: ubiquitous, itemAtURL: url(from), destinationURL: url(to), error: &mut error];
        let _: () = msg_send![fm, release];
        if ok { return Ok(()); }
        let reason = if error.is_null() { String::new() } else {
            let d: *mut AnyObject = msg_send![error, localizedDescription];
            let u: *const std::ffi::c_char = if d.is_null() { std::ptr::null() } else { msg_send![d, UTF8String] };
            if u.is_null() { String::new() } else { std::ffi::CStr::from_ptr(u).to_string_lossy().into_owned() }
        };
        anyhow::bail!("{}", if reason.is_empty() { tr("系统没能移动这个文件夹", "The system could not move the folder").to_string() } else { reason })
    }
}

thread_local! { static MOVING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }

/// 把空间搬到另一个存放位置：先存好打开的文档，在后台让系统搬，搬完重新打开这个空间（它若正打开着）。
fn move_space(w: &mut Workspace, id: String, to: Location, cx: &mut Context<Workspace>) {
    if MOVING.with(|m| m.get()) { return; }
    if !w.flush(cx) { w.show_toast_kind(crate::ui::ToastKind::Error, tr("有文档没能保存，处理好再移动空间", "A document could not be saved. Resolve it before moving the space."), cx); return; }
    MOVING.with(|m| m.set(true));
    super::glass::haptic();
    cx.notify();
    let task = cx.background_executor().spawn(async move {
        let mover = |a: &std::path::Path, b: &std::path::Path| system_move(a, b, to == Location::ICloud);
        fdl_core::spaces::move_space(&id, to, &mover).map(|_| id)
    });
    cx.spawn(async move |this, cx| {
        let result = task.await;
        let _ = this.update(cx, |w, cx| {
            MOVING.with(|m| m.set(false));
            match result {
                Ok(id) => {
                    let open = space::current(w).is_some_and(|s| s.id == id);
                    w.reload_spaces(cx);
                    w.show_toast(if to == Location::ICloud { tr("空间已移到 iCloud", "The space is now in iCloud") } else { tr("空间已移到本机", "The space is now on this device") }, cx);
                    // 正打开着：在新位置重新打开（文档库的路径变了）。
                    if open { cx.defer(move |cx| { space::switch(id, cx); }); }
                }
                Err(e) => w.show_toast_kind(crate::ui::ToastKind::Error, &crate::settings::diagnostic(&e.to_string()), cx),
            }
        });
    }).detach();
}

/// 设置 › 空间设置（照 Craft）：图标带相机钮，名称，危险区里的删除（第一个空间没有）。
pub fn space_page(w: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let Some(s) = space::current(w).cloned() else { return div().into_any_element() };
    let (pick, del, label) = (s.id.clone(), s.id.clone(), name(&s));
    // 和别的子页同一套卡片（`general::card`）：名称、存放位置、恢复默认图标在一张卡片里，删除单独在「危险区」。
    let mut rows = vec![
        div().w_full().px(px(18.)).py(px(12.)).flex().flex_col().gap(px(2.))
            .child(div().text_size(px(14.)).line_height(px(19.)).text_color(Theme::text_caption()).child(tr("空间名称", "Name")))
            .child(div().h(px(26.)).flex().items_center().text_size(px(17.)).child(w.space_name.clone())).into_any_element(),
    ];
    rows.extend(storage(&s));
    if s.icon.is_some() {
        let id = s.id.clone();
        rows.push(div().id("space-icon-reset").w_full().h(px(52.)).px(px(18.)).flex().items_center().text_size(px(17.)).text_color(Theme::primary())
            .active(|s| s.bg(Theme::fill_pressed())).child(tr("恢复默认图标", "Use the default icon"))
            .on_click(cx.listener(move |w, _, _, cx| { super::glass::haptic(); let _ = fdl_core::spaces::set_icon(&id, None); w.reload_spaces(cx); })).into_any_element());
    }
    div().w_full().pt(px(8.)).flex().flex_col().gap(px(28.))
        .child(div().w_full().flex().justify_center().child(div().id("space-page-icon").relative().child(space_icon(&s, 96.))
            .child(div().absolute().right(px(-6.)).bottom(px(-6.)).size(px(32.)).rounded_full().flex().items_center().justify_center()
                .bg(theme::raised(Theme::bg_body())).border_1().border_color(Theme::button_border()).shadow(theme::shadow_button()).child(crate::ui::icon("pen-line", 16., Theme::icon())))
            .on_click(cx.listener(move |w, _, window, cx| w.pick_space_icon(pick.clone(), window, cx)))))
        .child(super::general::card(rows))
        .when(!s.primary, |d| d.child(super::general::titled(Some(tr("危险区", "Danger zone")), vec![
            div().id("space-delete").w_full().px(px(18.)).py(px(12.)).flex().flex_col().gap(px(2.)).active(|s| s.bg(Theme::fill_pressed()))
                .child(div().text_size(px(17.)).line_height(px(22.)).text_color(Theme::danger()).child(tr("删除空间", "Delete space")))
                .child(div().text_size(px(14.)).line_height(px(19.)).text_color(Theme::text_caption()).child(tr("删除后，这个空间和里面的所有文档都无法恢复。", "Deleting the space removes it and every document in it for good.")))
                .on_click(cx.listener(move |w, _, _, cx| { super::glass::haptic(); w.confirm = Some(Confirm::RemoveSpace { id: del.clone(), name: label.clone() }); cx.notify(); }))
                .into_any_element()])))
        .into_any_element()
}

/// 不能移的原因（空就是能移）。
fn move_blocked(s: &fdl_core::spaces::Space, to: Location) -> Option<&'static str> {
    if s.root == fdl_core::spaces::home() { Some(tr("这是应用主文件夹里的第一个空间，暂不能移动。", "This is the first space in the app's main folder and can't be moved yet.")) }
    else if to == Location::ICloud && fdl_core::spaces::icloud_root().is_none() { Some(tr("iCloud 不可用：请登录 iCloud 并打开 iCloud 云盘。", "iCloud is unavailable. Sign in to iCloud and turn on iCloud Drive.")) }
    else { None }
}

/// 「存放位置」（照 Craft 的选项行）：点一下就地选本机或 iCloud，选了另一边就把空间移过去。两边都在应用自己的文件夹里。
/// 移不了时不出菜单，下面一行说为什么；移动中值写「正在移动…」。
fn storage(s: &fdl_core::spaces::Space) -> Vec<AnyElement> {
    let here = s.location;
    let to = if here == Location::ICloud { Location::Local } else { Location::ICloud };
    let label = |l: Location| match l { Location::ICloud => "iCloud", Location::Local => tr("本机", "On this device") };
    let blocked = move_blocked(s, to);
    let moving = MOVING.with(|m| m.get());
    let value = if moving { tr("正在移动…", "Moving…") } else { label(here) };
    let mut rows = vec![];
    if blocked.is_some() || moving {
        rows.push(div().w_full().px(px(18.)).py(px(12.)).flex().flex_col().gap(px(2.))
            .child(div().text_size(px(14.)).line_height(px(19.)).text_color(Theme::text_caption()).child(tr("存放位置", "Location")))
            .child(div().text_size(px(17.)).line_height(px(22.)).text_color(crate::settings::row_ink()).child(value))
            .children(blocked.filter(|_| !moving).map(|why| div().pt(px(4.)).text_size(px(14.)).line_height(px(19.)).text_color(Theme::text_caption()).child(why)))
            .into_any_element());
    } else {
        let items = vec![
            super::menu::Item::new("space-loc-local", label(Location::Local), "iphone", 0).checked(here == Location::Local),
            super::menu::Item::new("space-loc-icloud", "iCloud", "icloud", 0).checked(here == Location::ICloud),
        ];
        rows.push(super::general::choice("space-location", tr("存放位置", "Location"), value, items));
    }
    rows
}

/// 「存放位置」菜单里选了一边。
pub fn pick_location(w: &mut Workspace, icloud: bool, cx: &mut Context<Workspace>) {
    let Some(s) = space::current(w).cloned() else { return };
    let to = if icloud { Location::ICloud } else { Location::Local };
    if s.location == to || MOVING.with(|m| m.get()) || move_blocked(&s, to).is_some() { return; }
    super::glass::haptic();
    move_space(w, s.id.clone(), to, cx);
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
