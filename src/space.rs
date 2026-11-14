//! 空间 in the window (照 Craft): the first-space setup when there is none,
//! the switcher at the top of the sidebar (only with two or more spaces),
//! the 新建空间 form, the 空间 settings page, and switching, which saves
//! every window, opens the other library and rebuilds the windows on it.
use crate::app::{Confirm, Workspace};
use crate::input::{TextField, TextFieldEvent};
use crate::settings::tr;
use crate::shared_store::SharedStore;
use crate::theme::{self, Theme};
use crate::ui::{dialog, icon, primary_button, scrim, secondary_button, TitleBarExt, ToastKind};
use fdl_core::spaces::{self, Space};
use gpui::{prelude::FluentBuilder, *};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// No space to open: windows show the setup page until the first one exists.
static SETUP: AtomicBool = AtomicBool::new(false);

pub fn setup_due() -> bool { SETUP.load(Ordering::Relaxed) }

/// The library a window works on while the setup page is up: empty, in the
/// temporary folder, never shown.
fn placeholder_root() -> PathBuf { std::env::temp_dir().join(format!("quiet-md-setup-{}", std::process::id())) }

/// Choose the space to open before any window (and before the library is
/// opened). Tests and scripted runs (`QUIET_MD_HOME`, the harness) keep the
/// old first launch: a library in the home, seeded with 开始指南 unless
/// scripted. `QUIET_MD_SETUP` forces the setup page for tests.
pub fn prepare(scripted: bool) {
    let home = crate::store::Store::home_dir();
    let forced = std::env::var_os("QUIET_MD_SETUP").is_some();
    if spaces::current().is_none() && !forced {
        if scripted {
            if let Err(err) = crate::store::Store::open_at(home.clone()) { crate::logging::fatal(&format!("无法初始化文档库 / Could not set up the library at {}", home.display()), &err); }
        } else if std::env::var_os("QUIET_MD_HOME").is_some() && let Err(err) = crate::guides::initialize(&home) {
            crate::logging::fatal(&format!("无法初始化文档库 / Could not set up the library at {}", home.display()), &err);
        }
    }
    match spaces::current().filter(|_| !forced) {
        Some(s) => crate::store::set_root(s.root),
        None => {
            SETUP.store(true, Ordering::Relaxed);
            crate::store::set_root(placeholder_root());
        }
    }
}

pub fn default_name() -> &'static str { tr("我的空间", "My Space") }

pub fn name(s: &Space) -> String { s.name.clone().unwrap_or_else(|| default_name().to_string()) }

/// A space without an image: a plain square in a colour of its own (照 Craft).
pub fn color(id: &str) -> Hsla {
    const HUES: [&str; 6] = ["blue", "purple", "green", "orange", "red", "yellow"];
    let n = id.bytes().fold(7u32, |a, b| a.wrapping_mul(31).wrapping_add(b as u32));
    crate::theme::callout_border_swatch(HUES[n as usize % HUES.len()]).unwrap_or_else(Theme::primary)
}

/// The space's icon: its image, else its colour square.
pub fn icon_el(image: Option<&Path>, id: &str, size: f32) -> AnyElement {
    let r = (size * 0.24).round();
    match image {
        Some(p) => div().size(px(size)).flex_none().overflow_hidden().rounded(px(r))
            .child(img(p.to_path_buf()).size_full().rounded(px(r)).object_fit(ObjectFit::Cover)).into_any_element(),
        None => div().size(px(size)).flex_none().rounded(px(r)).bg(color(id)).into_any_element(),
    }
}

pub fn space_icon(s: &Space, size: f32) -> AnyElement { icon_el(s.icon.as_deref(), &s.id, size) }

/// A folder for display: `~/…` under the user's home (not on Windows).
fn short(p: &Path) -> String {
    if cfg!(target_os = "windows") { return p.display().to_string(); }
    dirs::home_dir().and_then(|h| p.strip_prefix(h).ok().map(|r| format!("~/{}", r.display()))).unwrap_or_else(|| p.display().to_string())
}

/// The open space (`None` only during setup).
pub fn current(w: &Workspace) -> Option<&Space> {
    let root = crate::store::Store::root_dir();
    w.spaces.iter().find(|s| s.root == root)
}

/// The 新建空间 form: the setup page's and the dialog's.
pub struct SpaceForm {
    pub name: Entity<TextField>,
    /// A picked image for the icon.
    pub icon: Option<PathBuf>,
    /// A picked folder; `None` puts the space in the app's folder.
    pub folder: Option<PathBuf>,
    pub error: Option<String>,
    _sub: Subscription,
}

impl SpaceForm {
    pub fn new(cx: &mut Context<Workspace>) -> Self {
        let name = cx.new(|cx| {
            let mut f = TextField::new(default_name(), cx);
            f.placeholder_color = Theme::text_disabled();
            f.max_len = Some(60);
            f
        });
        let sub = cx.subscribe(&name, |w, _, ev: &TextFieldEvent, cx| match ev {
            TextFieldEvent::Submit => cx.defer(create_from_form),
            TextFieldEvent::Cancel if w.setup.is_none() => { w.space_form = None; cx.notify(); }
            _ => {}
        });
        Self { name, icon: None, folder: None, error: None, _sub: sub }
    }
}

const IMAGE_EXTS: [&str; 6] = ["png", "jpg", "jpeg", "gif", "webp", "bmp"];
const ICON_MAX: u64 = 10 * 1024 * 1024;

/// Pick an image file (or the scripted answer).
fn pick_image(cx: &mut App) -> impl std::future::Future<Output = Option<PathBuf>> + use<> {
    let picked = crate::devtools::take_pick();
    let extensions = Some(IMAGE_EXTS.iter().map(|e| SharedString::from(*e)).collect());
    let prompt = picked.is_none().then(|| cx.prompt_for_paths(PathPromptOptions { files: true, directories: false, multiple: false, prompt: Some(tr("选择", "Choose").into()), extensions }));
    async move {
        let paths = match prompt { Some(p) => p.await.ok().and_then(Result::ok).flatten(), None => picked };
        paths.and_then(|p| p.into_iter().next())
    }
}

/// Pick a folder (or the scripted answer).
fn pick_folder(cx: &mut App) -> impl std::future::Future<Output = Option<PathBuf>> + use<> {
    let picked = crate::devtools::take_pick();
    let prompt = picked.is_none().then(|| cx.prompt_for_paths(PathPromptOptions { files: false, directories: true, multiple: false, prompt: Some(tr("选择", "Choose").into()), extensions: None }));
    async move {
        let paths = match prompt { Some(p) => p.await.ok().and_then(Result::ok).flatten(), None => picked };
        paths.and_then(|p| p.into_iter().next())
    }
}

fn read_icon(p: &Path) -> anyhow::Result<(Vec<u8>, String)> {
    let ext = p.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).unwrap_or_default();
    anyhow::ensure!(IMAGE_EXTS.contains(&ext.as_str()), "{}", tr("图片格式仅支持 png、jpg、gif、webp、bmp", "Use a PNG, JPEG, GIF, WebP or BMP image"));
    let bytes = crate::store::read_bounded(p, ICON_MAX)?;
    Ok((bytes, ext))
}

/// The form's 创建: make the space (the first one gets 开始指南) and open it.
pub(crate) fn create_from_form(cx: &mut App) {
    let Some(handle) = cx.windows().into_iter().filter_map(|h| h.downcast::<Workspace>()).find(|h| h.read(cx).is_ok_and(|w| w.setup.is_some() || w.space_form.is_some())) else { return };
    let made = handle.update(cx, |w, _, cx| {
        let first = w.setup.is_some();
        let f = if first { w.setup.as_mut() } else { w.space_form.as_mut() }?;
        let typed = f.name.read(cx).text.trim().to_string();
        let name = if typed.is_empty() { default_name().to_string() } else { typed };
        let seed = |root: &Path| crate::guides::initialize(root);
        let made = if first { spaces::create_with(&name, f.folder.as_deref(), &seed) } else { spaces::create(&name, f.folder.as_deref()) };
        match made {
            Ok(space) => {
                if let Some(p) = f.icon.clone() && let Ok((bytes, ext)) = read_icon(&p) { let _ = spaces::set_icon(&space.id, Some((&bytes, &ext))); }
                Some(space.id)
            }
            Err(e) => { f.error = Some(crate::settings::diagnostic(&e.to_string()).to_string()); cx.notify(); None }
        }
    }).ok().flatten();
    if let Some(id) = made { switch(id, cx); }
}

/// Open the space `id` in every window: save each window's document first
/// (a save that lands after the switch would write into the other space),
/// then rebuild the windows on the other library. Note windows close.
pub fn switch(id: String, cx: &mut App) -> bool {
    let handles: Vec<WindowHandle<Workspace>> = cx.windows().into_iter().filter_map(|h| h.downcast::<Workspace>()).collect();
    let tell = |msg: String, cx: &mut App| for h in &handles { let _ = h.update(cx, |w, _, cx| { w.space_menu = false; w.show_toast_kind(ToastKind::Error, &msg, cx); }); };
    let mut saved = true;
    for h in &handles { saved &= h.update(cx, |w, _, cx| w.flush(cx)).unwrap_or(true); }
    if !saved { tell(tr("有文档没能保存，处理好再切换空间", "A document could not be saved. Resolve it before switching spaces.").to_string(), cx); return false; }
    let Some(space) = spaces::list().into_iter().find(|s| s.id == id && s.available) else {
        tell(tr("找不到这个空间的文件夹", "This space's folder is missing").to_string(), cx);
        return false;
    };
    let store = match crate::store::Store::open_at(space.root.clone()) {
        Ok(s) => s,
        Err(e) => { tell(crate::localized_format!("无法打开这个空间：{}", "Could not open this space: {}", crate::settings::diagnostic(&e.to_string())), cx); return false; }
    };
    let _ = spaces::set_current(&id);
    crate::store::set_root(space.root.clone());
    if SETUP.swap(false, Ordering::Relaxed) { let _ = std::fs::remove_dir_all(placeholder_root()); }
    cx.global::<SharedStore>().replace(store);
    for h in handles {
        let _ = h.update(cx, |w, window, cx| {
            if w.quick_note { window.remove_window(); return; }
            let (open, width) = (w.sidebar_open, w.sidebar_width);
            window.replace_root(cx, |window, cx| {
                let mut n = Workspace::new(window, cx);
                (n.sidebar_open, n.sidebar_width) = (open, width);
                n
            });
        });
    }
    true
}

#[cfg(target_os = "macos")]
fn move_to_trash(path: &Path) -> anyhow::Result<()> {
    use objc::runtime::Object;
    use objc::{class, msg_send, sel, sel_impl};
    let s = std::ffi::CString::new(path.to_string_lossy().as_bytes())?;
    unsafe {
        let string: *mut Object = msg_send![class!(NSString), stringWithUTF8String: s.as_ptr()];
        let url: *mut Object = msg_send![class!(NSURL), fileURLWithPath: string];
        let fm: *mut Object = msg_send![class!(NSFileManager), defaultManager];
        let nil: *mut Object = std::ptr::null_mut();
        let ok: bool = msg_send![fm, trashItemAtURL: url resultingItemURL: nil error: nil];
        anyhow::ensure!(ok, "{}", tr("无法移到废纸篓", "Could not move it to the Trash"));
    }
    Ok(())
}

/// What 删除空间 does with the folder: the Trash on macOS; on iOS it is
/// deleted (照 Craft; only a folder the app made in `<home>/spaces/`);
/// elsewhere the space is only taken off the list.
#[derive(Clone, Copy, PartialEq)]
pub enum Deletion { Trash, Permanent, ListOnly }
pub const DELETION: Deletion = if cfg!(target_os = "macos") { Deletion::Trash } else if cfg!(target_os = "ios") { Deletion::Permanent } else { Deletion::ListOnly };

/// Take a space off the list and deal with its folder (`DELETION`). The
/// open space is left first, for the first one.
pub fn delete(id: String, cx: &mut App) {
    let here = crate::store::Store::root_dir();
    let all = spaces::list();
    let Some(target) = all.iter().find(|s| s.id == id).cloned() else { return };
    if target.root == here {
        let Some(first) = all.iter().find(|s| s.primary && s.available) else { return };
        if !switch(first.id.clone(), cx) { return; }
    }
    let done = spaces::remove(&id).and_then(|s| {
        if !s.available { return Ok(()); }
        match DELETION {
            #[cfg(target_os = "macos")]
            Deletion::Trash => move_to_trash(&s.root)?,
            Deletion::Permanent if s.root.starts_with(crate::store::Store::home_dir().join("spaces")) => std::fs::remove_dir_all(&s.root)?,
            _ => {}
        }
        Ok(())
    });
    for h in cx.windows().into_iter().filter_map(|h| h.downcast::<Workspace>()) {
        let _ = h.update(cx, |w, _, cx| match &done {
            Ok(()) => { w.reload_spaces(cx); w.show_toast(tr("已删除空间", "Space deleted"), cx); }
            Err(e) => w.show_toast_kind(ToastKind::Error, &crate::settings::diagnostic(&e.to_string()), cx),
        });
    }
}

impl Workspace {
    pub(crate) fn reload_spaces(&mut self, cx: &mut Context<Self>) {
        self.spaces = spaces::list();
        if let Some(s) = current(self) { let n = name(s); self.space_name.update(cx, |f, cx| if f.text != n { f.set_text(&n, cx) }); }
        cx.notify();
    }

    pub(crate) fn open_space_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.space_menu = false;
        let f = SpaceForm::new(cx);
        let focus = f.name.read(cx).focus_handle(cx);
        self.space_form = Some(f);
        window.focus(&focus);
        cx.notify();
    }

    fn form_mut(&mut self) -> Option<&mut SpaceForm> { if self.setup.is_some() { self.setup.as_mut() } else { self.space_form.as_mut() } }

    pub(crate) fn pick_form_icon(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pick = pick_image(cx);
        cx.spawn_in(window, async move |this, cx| {
            let Some(p) = pick.await else { return };
            let _ = this.update(cx, |w, cx| if let Some(f) = w.form_mut() {
                match read_icon(&p) { Ok(_) => { f.icon = Some(p); f.error = None; } Err(e) => f.error = Some(e.to_string()) }
                cx.notify();
            });
        }).detach();
    }

    fn pick_form_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pick = pick_folder(cx);
        cx.spawn_in(window, async move |this, cx| {
            let Some(p) = pick.await else { return };
            let _ = this.update(cx, |w, cx| if let Some(f) = w.form_mut() { f.folder = Some(p); f.error = None; cx.notify(); });
        }).detach();
    }

    pub(crate) fn pick_space_icon(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let pick = pick_image(cx);
        cx.spawn_in(window, async move |this, cx| {
            let Some(p) = pick.await else { return };
            let _ = this.update(cx, |w, cx| {
                match read_icon(&p).and_then(|(bytes, ext)| spaces::set_icon(&id, Some((&bytes, &ext)))) {
                    Ok(()) => w.reload_spaces(cx),
                    Err(e) => w.show_toast_kind(ToastKind::Error, &crate::settings::diagnostic(&e.to_string()), cx),
                }
            });
        }).detach();
    }

    /// The rows of the form: icon and name, then where it lives.
    fn form_body(&self, f: &SpaceForm, cx: &mut Context<Self>) -> Div {
        let radio = |on: bool| div().size(px(16.)).flex_none().rounded_full().border_1()
            .when(on, |d| d.border_color(Theme::primary()).child(div().m(px(3.)).size(px(8.)).rounded_full().bg(Theme::primary())))
            .when(!on, |d| d.border_color(Theme::button_border()));
        let place = |id: &'static str, on: bool, title: &'static str, detail: String| div().id(id).w_full().flex().items_start().gap(px(10.)).py(px(8.)).px(px(10.)).rounded(px(8.)).cursor_pointer()
            .hover(|s| s.bg(Theme::fill_hover()))
            .child(div().pt(px(2.)).child(radio(on)))
            .child(div().flex_1().min_w_0().flex().flex_col()
                .child(div().text_size(px(13.)).line_height(px(20.)).child(title))
                .child(div().text_size(px(12.)).line_height(px(17.)).text_color(Theme::text_caption()).child(detail)));
        let home = crate::store::Store::home_dir();
        let here = if cfg!(target_os = "macos") { tr("这台 Mac 上", "On this Mac") } else { tr("这台电脑上", "On this computer") };
        let picked = f.folder.as_ref().map(|p| p.display().to_string())
            .unwrap_or_else(|| tr("空文件夹，或已有的 Quiet.md 文档库。放在同步盘里时，同一时间只在一台设备上使用。", "An empty folder or an existing Quiet.md library. In a synced folder, use it on one device at a time.").to_string());
        div().w_full().flex().flex_col().gap(px(16.))
            .child(div().w_full().flex().items_center().gap(px(14.))
                .child(div().id("space-form-icon").relative().cursor_pointer().child(icon_el(f.icon.as_deref(), "new", 52.))
                    .child(div().absolute().right(px(-4.)).bottom(px(-4.)).size(px(20.)).rounded_full().flex().items_center().justify_center()
                        .bg(crate::theme::raised(Theme::bg_body())).border_1().border_color(Theme::button_border()).child(icon("pen-line", 12., Theme::icon())))
                    .on_click(cx.listener(|w, _, window, cx| w.pick_form_icon(window, cx))))
                .child(div().flex_1().min_w_0().flex().flex_col().gap(px(4.))
                    .child(div().text_size(px(12.)).line_height(px(16.)).text_color(Theme::text_caption()).child(tr("空间名称", "Name")))
                    .child(div().h(px(34.)).px(px(10.)).flex().items_center().rounded(px(8.)).border_1().border_color(Theme::button_border()).bg(crate::theme::raised(Theme::bg_body())).child(f.name.clone()))))
            .child(div().w_full().flex().flex_col().gap(px(2.))
                .child(div().px(px(10.)).text_size(px(12.)).line_height(px(16.)).text_color(Theme::text_caption()).child(tr("位置", "Location")))
                .child(place("space-form-here", f.folder.is_none(), here, crate::localized_format!("放在 {} 里", "In {}", short(&home)))
                    .on_click(cx.listener(|w, _, _, cx| if let Some(f) = w.form_mut() { f.folder = None; cx.notify(); })))
                .child(place("space-form-folder", f.folder.is_some(), tr("选择文件夹…", "Choose a folder…"), picked)
                    .on_click(cx.listener(|w, _, window, cx| w.pick_form_folder(window, cx)))))
            .children(f.error.clone().map(|e| div().px(px(10.)).text_size(px(12.)).line_height(px(17.)).text_color(Theme::danger()).child(e)))
    }

    /// The whole window while there is no space: create the first one.
    pub(crate) fn render_setup(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let f = self.setup.as_ref().expect("setup form");
        div().id("space-setup").size_full().flex().flex_col().bg(Theme::bg_body()).font_family(theme::UI_FONT).text_color(Theme::text())
            .child(div().h(px(crate::app::HEADER_H)).flex_none().title_bar())
            .child(div().id("space-setup-scroll").flex_1().min_h_0().overflow_y_scroll().flex().justify_center()
                .child(div().w(px(440.)).pt(px(72.)).pb(px(48.)).flex().flex_col().gap(px(28.))
                    .child(div().flex().flex_col().gap(px(8.))
                        .child(div().text_size(px(24.)).line_height(px(32.)).font_weight(FontWeight::SEMIBOLD).child(tr("创建你的第一个空间", "Create your first space")))
                        .child(div().text_size(px(13.)).line_height(px(20.)).text_color(Theme::text_caption())
                            .child(tr("空间是一个独立的文档库。以后可以再建更多空间，在侧栏顶上切换。", "A space is a library of its own. You can add more later and switch at the top of the sidebar."))))
                    .child(self.form_body(f, cx))
                    .child(div().flex().justify_end().child(primary_button("space-setup-create", tr("创建", "Create")).on_click(|_, _, cx| cx.defer(create_from_form))))))
            .children(crate::ui::window_controls(window))
            .into_any_element()
    }

    /// 新建空间… as a dialog over the window.
    pub(crate) fn render_space_dialog(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let f = self.space_form.as_ref()?;
        let body = dialog("space-dialog").w(px(460.)).px(px(24.)).pt(px(20.)).pb(px(24.)).flex().flex_col().gap(px(20.))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(div().text_size(px(15.)).line_height(px(22.)).font_weight(FontWeight::SEMIBOLD).child(tr("新建空间", "New space")))
            .child(self.form_body(f, cx))
            .child(div().flex().justify_end().gap(px(12.))
                .child(secondary_button("space-dialog-cancel", tr("取消", "Cancel")).on_click(cx.listener(|w, _, _, cx| { w.space_form = None; cx.notify(); })))
                .child(primary_button("space-dialog-create", tr("创建", "Create")).on_click(|_, _, cx| cx.defer(create_from_form))));
        Some(deferred(div().id("space-dialog-mask").absolute().inset_0().flex().items_center().justify_center().bg(scrim())
            .on_mouse_down(MouseButton::Left, cx.listener(|w, _, _, cx| { w.space_form = None; cx.notify(); }))
            .child(body)).with_priority(3).into_any_element())
    }

    /// The switcher row above 主页: only once there are two spaces.
    pub(crate) fn render_space_row(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.spaces.len() < 2 { return None; }
        let s = current(self)?;
        let open = self.space_menu;
        Some(div().flex_none().pt(px(4.)).px(px(crate::sidebar::SB_X)).child(
            div().id("space-switcher").h(px(crate::sidebar::ROW_H)).pl(px(6.)).pr(px(8.)).flex().items_center().gap(px(9.)).rounded(px(crate::sidebar::ROW_R)).cursor_pointer()
                .text_size(px(13.)).line_height(px(20.)).font_weight(FontWeight::MEDIUM).text_color(Theme::text())
                .when(open, |d| d.bg(Theme::sidebar_hover()))
                .hover(|d| d.bg(Theme::sidebar_hover()))
                .child(div().size(px(20.)).flex_none().flex().items_center().justify_center().child(space_icon(s, 18.)))
                .child(div().flex_1().min_w_0().truncate().child(name(s)))
                .child(icon("chevron-down", 14., Theme::icon()))
                .on_click(cx.listener(|w, _, _, cx| { w.space_menu = !w.space_menu; cx.notify(); }))
        ).into_any_element())
    }

    /// The switcher's menu: every space (the open one ticked), then 新建空间… and 空间设置….
    pub(crate) fn render_space_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.space_menu || !self.sidebar_open { return None; }
        let here = crate::store::Store::root_dir();
        let top = crate::app::HEADER_H + 4. + crate::sidebar::ROW_H + 4.;
        let group: SharedString = "space-menu".into();
        let mut menu = crate::ui::popover().w(px(260.)).py(px(crate::ui::MENU_PAD_Y)).flex().flex_col()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation());
        for (i, s) in self.spaces.iter().enumerate() {
            let id = s.id.clone();
            let on = s.root == here;
            let mut row = crate::ui::menu_row(("space-menu-row", i), group.clone())
                .child(space_icon(s, 18.))
                .child(div().flex_1().min_w_0().truncate().child(name(s)))
                .when(on, |d| d.child(icon("check", 16., Theme::primary())));
            if s.available { row = row.on_click(cx.listener(move |_, _, _, cx| { let id = id.clone(); cx.defer(move |cx| { switch(id, cx); }); })); }
            else { row = row.text_color(Theme::text_disabled()).child(div().text_size(px(12.)).text_color(Theme::text_disabled()).child(tr("找不到文件夹", "Missing"))); }
            menu = menu.child(row);
        }
        menu = menu.child(div().my(px(5.)).h(px(1.)).bg(Theme::line_border()))
            .child(crate::ui::menu_row("space-menu-new", group.clone()).child(crate::ui::menu_glyph("plus", 16., group.clone())).child(tr("新建空间…", "New Space…"))
                .on_click(cx.listener(|w, _, window, cx| w.open_space_form(window, cx))))
            .child(crate::ui::menu_row("space-menu-settings", group.clone()).child(crate::ui::menu_glyph("settings-gear", 16., group.clone())).child(tr("空间设置…", "Space Settings…"))
                .on_click(cx.listener(|w, _, _, cx| { w.space_menu = false; w.open_settings(crate::settings::Page::Library, cx); })));
        Some(deferred(div().id("space-menu-mask").absolute().inset_0()
            .on_mouse_down(MouseButton::Left, cx.listener(|w, _, _, cx| { w.space_menu = false; cx.notify(); }))
            .child(div().absolute().top(px(top)).left(px(crate::sidebar::SB_X)).child(menu))).with_priority(3).into_any_element())
    }

    /// 设置 › 空间: this space (icon, name, folder, trash), then every space.
    pub(crate) fn space_page(&self, cx: &mut Context<Self>) -> AnyElement {
        use crate::settings::{button, group, row};
        let root = crate::store::Store::root_dir();
        let location = short(&root);
        let reveal = if cfg!(target_os = "macos") { tr("在访达中显示", "Show in Finder") } else if cfg!(target_os = "windows") {
            tr("在文件资源管理器中显示", "Show in File Explorer")
        } else { tr("在文件管理器中显示", "Show in File Manager") };
        let this = current(self).cloned();
        let mut page = div().w_full().flex().flex_col().gap(px(28.));
        if let Some(s) = &this {
            let (pick_id, reset_id) = (s.id.clone(), s.id.clone());
            page = page.child(group(None, vec![
                div().w_full().px(px(16.)).py(px(16.)).flex().items_center().gap(px(14.))
                    .child(div().id("space-icon").cursor_pointer().child(space_icon(s, 52.)).on_click(cx.listener(move |w, _, window, cx| w.pick_space_icon(pick_id.clone(), window, cx))))
                    .child(div().flex_1().min_w_0().flex().flex_col().gap(px(4.))
                        .child(div().text_size(px(12.)).line_height(px(16.)).text_color(Theme::text_caption()).child(tr("空间名称", "Name")))
                        .child(div().h(px(32.)).px(px(10.)).flex().items_center().rounded(px(8.)).border_1().border_color(Theme::button_border()).child(self.space_name.clone())))
                    .child(div().flex().gap(px(8.))
                        .child(button("space-icon-pick", tr("更换图标…", "Change Icon…")).on_click(cx.listener({ let id = s.id.clone(); move |w, _, window, cx| w.pick_space_icon(id.clone(), window, cx) })))
                        .when(s.icon.is_some(), |d| d.child(button("space-icon-reset", tr("恢复默认", "Default")).on_click(cx.listener(move |w, _, _, cx| { let _ = spaces::set_icon(&reset_id, None); w.reload_spaces(cx); })))))
                    .into_any_element(),
                row(tr("文件夹", "Folder"), Some(location.into()),
                    button("reveal-library", reveal).on_click(|_, _, cx| cx.reveal_path(&crate::store::Store::root_dir()))).into_any_element(),
                row(tr("回收站", "Trash"), Some(if crate::settings::english() { format!("Deleted documents are kept for {} days.", crate::store::TRASH_DAYS) } else { format!("删除的文档保留 {} 天后自动清除。", crate::store::TRASH_DAYS) }.into()),
                    button("open-trash", tr("打开", "Open")).on_click(cx.listener(|w, _, _, cx| w.show_view(crate::app::View::Trash, cx)))).into_any_element(),
            ]));
        }
        let mut rows: Vec<AnyElement> = self.spaces.iter().enumerate().map(|(i, s)| {
            let on = s.root == root;
            let (id, rm, label) = (s.id.clone(), s.id.clone(), name(s));
            let detail: SharedString = if s.available { short(&s.root).into() } else { tr("找不到文件夹", "Folder missing").into() };
            div().w_full().min_h(px(52.)).px(px(16.)).py(px(10.)).flex().items_center().gap(px(12.))
                .child(space_icon(s, 28.))
                .child(div().flex_1().min_w_0().flex().flex_col().gap(px(2.))
                    .child(div().text_size(px(13.)).line_height(px(20.)).truncate().child(label.clone()))
                    .child(div().text_size(px(12.)).line_height(px(17.)).text_color(if s.available { Theme::text_caption() } else { Theme::danger() }).truncate().child(detail)))
                .when(on, |d| d.child(div().text_size(px(12.)).text_color(Theme::text_caption()).child(tr("当前", "Open"))))
                .when(!on && s.available, |d| d.child(button(("space-open", i), tr("切换", "Switch")).on_click(move |_, _, cx| { let id = id.clone(); cx.defer(move |cx| { switch(id, cx); }); })))
                .when(!s.primary && !on, |d| d.child(button(("space-remove", i), if s.available && DELETION != Deletion::ListOnly { tr("删除…", "Delete…") } else { tr("移除", "Remove") })
                    .text_color(Theme::danger())
                    .on_click(cx.listener(move |w, _, _, cx| { w.confirm = Some(Confirm::RemoveSpace { id: rm.clone(), name: label.clone() }); cx.notify(); }))))
                .into_any_element()
        }).collect();
        rows.push(div().w_full().px(px(16.)).py(px(10.)).flex().justify_start()
            .child(button("space-new", tr("新建空间…", "New Space…")).on_click(cx.listener(|w, _, window, cx| w.open_space_form(window, cx)))).into_any_element());
        page.child(group(Some(tr("所有空间", "All spaces")), rows))
            .child(div().px(px(4.)).text_size(px(12.)).line_height(px(17.)).text_color(Theme::text_caption())
                .child(tr("第一个空间不能删除。删除别的空间时，它的文件夹会移到废纸篓；打开着的空间要先切换走。", "The first space cannot be deleted. Deleting another space moves its folder to the Trash; switch away from a space before deleting it.")))
            .into_any_element()
    }

    /// 删除空间 / 移除 confirmed (`delete`).
    pub(crate) fn remove_space(&mut self, id: &str, cx: &mut Context<Self>) {
        let id = id.to_string();
        cx.defer(move |cx| delete(id, cx));
    }

    /// The name field on 设置 › 空间 saves as it is typed (照 Craft).
    pub(crate) fn space_name_changed(&mut self, cx: &mut Context<Self>) {
        let Some(id) = current(self).map(|s| s.id.clone()) else { return };
        let typed = self.space_name.read(cx).text.clone();
        if spaces::rename(&id, &typed).is_ok() {
            self.spaces = spaces::list();
            cx.notify();
        }
    }
}
