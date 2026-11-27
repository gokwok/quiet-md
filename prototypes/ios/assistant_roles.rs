//! ElevenLabs 音色列表和语音角色。角色和自选的头像存在全局库（`fdl_ai::global`）的 `voices.json`、`avatars/`，各空间共用；
//! 旧版存在这台设备的偏好（`Preferences::speech_roles`）和主目录的 `assistant-avatars/` 里，第一次读时并进来（按音色去重），旧的原样留着。
use std::{cell::{Cell, RefCell}, collections::HashSet, fmt::Write, path::{Path, PathBuf}, ptr::null_mut, rc::Rc};

use gpui::{prelude::FluentBuilder, *};
use objc2::{ClassType, class, define_class, msg_send, runtime::{AnyObject, NSObject}};
use serde::{Deserialize, Serialize};

use crate::{app::Workspace, input::TextField, settings::{Page, prefs, row_chevron, row_ink, tr}, theme::{self, Theme, craft}, ui};
use super::{general, menu, ns_string, preview::Phase};

#[derive(Clone, Deserialize, Serialize)]
pub struct Role {
    voice_id: String,
    voice_name: String,
    name: String,
    #[serde(default)]
    avatar: Option<String>,
    /// 试听用的样音地址；旧版存下的角色没有，第一次试听时向 ElevenLabs 要。
    #[serde(default)]
    preview_url: Option<String>,
    /// 音色的特点（性别 · 年龄 · 口音），收藏时记下，卡片上当副标题。
    #[serde(default)]
    traits: String,
    /// 配音前用快速模型给台词加上语气标签（`[叹气]` 这类，只有 v3、v4 认）。
    #[serde(default)]
    pub tags: bool,
    /// 配音时把台词译成哪种语言：空是原文，`zh`、`en`、`ja`。
    #[serde(default)]
    pub lang: String,
}

impl Role {
    pub fn voice_id(&self) -> &str { &self.voice_id }
    pub fn name(&self) -> &str { &self.name }
    pub fn voice_name(&self) -> &str { &self.voice_name }
    pub fn traits(&self) -> &str { &self.traits }
}

thread_local! { static ROLES: RefCell<Option<Vec<Role>>> = const { RefCell::new(None) }; }

fn voices_file() -> PathBuf { fdl_ai::global::dir().join("voices.json") }
fn load_roles() -> Vec<Role> {
    let mut roles: Vec<Role> = fdl_core::file_io::read(voices_file()).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .and_then(|v| serde_json::from_value(v.get("roles")?.clone()).ok()).unwrap_or_default();
    if !fdl_ai::global::device_migrated() {
        let fresh: Vec<Role> = prefs().speech_roles.into_iter().filter(|r| !roles.iter().any(|x| x.voice_id == r.voice_id)).collect();
        let added = !fresh.is_empty();
        roles.extend(fresh);
        let saved = !added || save_roles(&roles).is_ok();
        let copied = fdl_ai::global::copy_files(&old_avatars_dir(), &avatars_dir());
        if saved && copied.is_ok() && let Err(e) = fdl_ai::global::mark_device_migrated() { log::warn!("Could not mark voice roles migrated: {e}"); }
    }
    roles
}
fn save_roles(roles: &[Role]) -> std::io::Result<()> {
    let bytes = serde_json::to_vec_pretty(&serde_json::json!({ "roles": roles })).map_err(std::io::Error::other)?;
    let path = voices_file();
    if let Some(dir) = path.parent() { fdl_core::file_io::create_dir_all(dir)?; }
    fdl_core::file_io::with_write(&path, |p| {
        let tmp = p.with_extension("json.tmp");
        std::fs::write(&tmp, &bytes)?;
        Ok(std::fs::rename(&tmp, p)?)
    }).map_err(std::io::Error::other)
}

/// 所有语音角色，按设置里排的顺序（也是配音菜单的顺序）。
pub fn all() -> Vec<Role> { ROLES.with(|r| r.borrow_mut().get_or_insert_with(load_roles).clone()) }
/// 打开语音设置时重新读一遍：另一台设备可能改过。
pub fn reload() { ROLES.with(|r| *r.borrow_mut() = Some(load_roles())); }
/// 改语音角色：整份写回去；写不了就提示，不改。
pub fn update(w: &mut Workspace, f: impl FnOnce(&mut Vec<Role>), cx: &mut Context<Workspace>) {
    let mut roles = all();
    f(&mut roles);
    if let Err(e) = save_roles(&roles) { return w.show_toast_kind(ui::ToastKind::Error, &format!("{}: {e}", tr("设置保存失败", "Could not save settings")), cx); }
    ROLES.with(|r| *r.borrow_mut() = Some(roles));
    cx.notify();
}

/// 台词语言：(菜单项, 代码, 名称)。
pub const LANGS: [(&str, &str, &str); 4] = [("speech-role-lang-0", "", "原文"), ("speech-role-lang-3", "zh", "中文"), ("speech-role-lang-1", "en", "English"), ("speech-role-lang-2", "ja", "日本語")];
pub fn lang_name(code: &str) -> SharedString {
    match LANGS.iter().find(|l| l.1 == code) { Some(&(_, "", _)) | None => tr("原文", "Original").into(), Some(l) => l.2.into() }
}

/// 只认要用的几项，别的字段 ElevenLabs 加了减了都不影响（`voice_id` 以外都是可选的）。
#[derive(Clone, Deserialize)]
struct Voice {
    voice_id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    preview_url: Option<String>,
    #[serde(default)]
    category: Option<String>,
    #[serde(default)]
    labels: Option<std::collections::HashMap<String, String>>,
}

impl Voice {
    fn name(&self) -> &str { self.name.as_deref().filter(|n| !n.trim().is_empty()).unwrap_or(&self.voice_id) }
    fn label(&self, key: &str) -> Option<&str> { self.labels.as_ref()?.get(key).map(|s| s.trim()).filter(|s| !s.is_empty()) }
    /// 性别 · 年龄 · 口音 · 用途，认得的译成中文，别的把下划线换成空格。
    fn traits(&self) -> String {
        let word = |key: &str| self.label(key).map(|v| match v.to_lowercase().as_str() {
            "female" => tr("女声", "Female").into(),
            "male" => tr("男声", "Male").into(),
            "neutral" | "non-binary" => tr("中性", "Neutral").into(),
            "young" => tr("青年", "Young").into(),
            "middle_aged" | "middle aged" => tr("中年", "Middle-aged").into(),
            "old" => tr("老年", "Old").into(),
            _ => { let v = v.replace('_', " "); let mut c = v.chars(); c.next().map_or_else(String::new, |f| f.to_uppercase().chain(c).collect()) }
        });
        ["gender", "age", "accent", "use_case"].iter().filter_map(|k| word(k)).collect::<Vec<_>>().join(" · ")
    }
    /// 筛选：0 全部、1 女声、2 男声、3 自己的（克隆 / 生成 / 专业克隆）。
    fn matches(&self, filter: u8) -> bool {
        match filter {
            1 => self.label("gender").is_some_and(|g| g.eq_ignore_ascii_case("female")),
            2 => self.label("gender").is_some_and(|g| g.eq_ignore_ascii_case("male")),
            3 => self.category.as_deref().is_some_and(|c| matches!(c, "cloned" | "generated" | "professional")),
            _ => true,
        }
    }
}

#[derive(Deserialize)]
struct VoicePage { voices: Vec<Voice>, has_more: bool, next_page_token: Option<String> }

pub(super) enum LoadError { Key, Network, Http(isize), Response }
enum Catalog { Idle, Loading, Ready(Vec<Voice>), Failed(LoadError) }

const FILTER_GLASS: [&str; 4] = ["speech-filter-0", "speech-filter-1", "speech-filter-2", "speech-filter-3"];

thread_local! {
    static CATALOG: RefCell<Catalog> = const { RefCell::new(Catalog::Idle) };
    static REQUEST: Cell<u64> = const { Cell::new(0) };
    static SEARCH: RefCell<Option<Entity<TextField>>> = const { RefCell::new(None) };
    static EDITING: RefCell<String> = const { RefCell::new(String::new()) };
    static FILTER: Cell<u8> = const { Cell::new(0) };
    static ORBS: RefCell<Vec<(String, std::sync::Arc<Image>)>> = const { RefCell::new(Vec::new()) };
    /// 刚收藏的那个音色和第几次：星星弹一下。
    static POPPED: RefCell<(String, usize)> = const { RefCell::new((String::new(), 0)) };
}

pub(super) fn key_changed() {
    REQUEST.with(|r| r.set(r.get() + 1));
    CATALOG.with(|c| *c.borrow_mut() = Catalog::Idle);
    super::preview::stop();
}


/// 没换头像的音色：一颗柔光球（照 ElevenLabs 的音色头像）。按 voice_id 定颜色和光斑位置，同一个音色在哪儿都一样。
/// GPUI 没有模糊，光球是 SVG（几团高斯模糊的色块裁在圆里）用 resvg 画成 PNG，按音色缓存。
pub(crate) fn orb(id: &str, size: f32) -> Div {
    let image = ORBS.with(|orbs| {
        if let Some((_, image)) = orbs.borrow().iter().find(|(k, _)| k == id) { return Some(image.clone()); }
        let image = std::sync::Arc::new(Image::from_bytes(ImageFormat::Png, orb_png(id)?));
        orbs.borrow_mut().push((id.to_owned(), image.clone()));
        Some(image)
    });
    div().size(px(size)).flex_none().rounded_full().overflow_hidden().bg(general::card_bg())
        .children(image.map(|i| img(i).size_full().rounded_full()))
}

fn orb_png(id: &str) -> Option<Vec<u8>> {
    const HUES: [&str; 5] = ["blue", "purple", "green", "orange", "red"];
    const SIDE: u32 = 240;
    let n = id.bytes().fold(2166136261u32, |a, b| (a ^ b as u32).wrapping_mul(16777619));
    let bits = |shift: u32, m: u32| (n >> shift) % m;
    // 色板的颜色往亮处、淡处挪，成柔和的一团；底色深浅两种。
    let tone = |name: &str, s: f32, l: f32| {
        let c = theme::callout_border_swatch(name).unwrap_or_else(Theme::primary);
        let c = Rgba::from(hsla(c.h, c.s * s, l, 1.));
        format!("#{:02x}{:02x}{:02x}", (c.r * 255.) as u8, (c.g * 255.) as u8, (c.b * 255.) as u8)
    };
    let pick = |k: u32| HUES[bits(k * 3, 5) as usize];
    let dark = bits(17, 3) == 0;
    let base = if dark { tone(pick(0), 0.55, 0.22) } else { tone(pick(0), 0.35, 0.86) };
    let spot = |k: u32, l: f32| (8. + bits(k * 4 + 1, 84) as f32, 8. + bits(k * 4 + 2, 84) as f32, tone(pick(k), 0.85, l));
    let (a, b, c) = (spot(1, 0.62), spot(2, 0.7), spot(3, 0.82));
    let svg = format!(r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100" viewBox="0 0 100 100">
<defs><filter id="b" x="-60%" y="-60%" width="220%" height="220%"><feGaussianBlur stdDeviation="13"/></filter><clipPath id="c"><circle cx="50" cy="50" r="50"/></clipPath></defs>
<g clip-path="url(#c)"><rect width="100" height="100" fill="{base}"/><g filter="url(#b)">
<circle cx="{}" cy="{}" r="34" fill="{}"/><circle cx="{}" cy="{}" r="28" fill="{}"/><circle cx="{}" cy="{}" r="18" fill="{}" fill-opacity="0.9"/>
<circle cx="32" cy="26" r="12" fill="#fff" fill-opacity="0.45"/></g></g></svg>"##, a.0, a.1, a.2, b.0, b.1, b.2, c.0, c.1, c.2);
    let tree = resvg::usvg::Tree::from_str(&svg, &resvg::usvg::Options::default()).ok()?;
    let mut pixmap = resvg::tiny_skia::Pixmap::new(SIDE, SIDE)?;
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(SIDE as f32 / 100., SIDE as f32 / 100.), &mut pixmap.as_mut());
    pixmap.encode_png().ok()
}

/// 正在放 / 正在载入时盖在头像上的一层：暗一点的底，上面是跳动的竖条或转圈。
fn playing_veil(id: &str, key: &'static str, size: f32) -> Option<AnyElement> {
    let veil = |child: AnyElement| div().absolute().inset_0().rounded_full().bg(theme::ca(craft::N1000, 0.42)).flex().items_center().justify_center().child(child).into_any_element();
    match super::preview::phase(id) {
        Phase::Idle => None,
        Phase::Loading => Some(veil(spinner(key, size * 0.38, gpui::white()))),
        Phase::Playing => Some(veil(super::preview::bars(key, size * 0.34, gpui::white()))),
    }
}

fn spinner(key: &'static str, size: f32, color: Hsla) -> AnyElement {
    svg().path("editor-icons/spinner.svg").size(px(size)).flex_none().text_color(color)
        .with_animation(key, Animation::new(std::time::Duration::from_millis(900)).repeat(), |s, t| s.with_transformation(Transformation::rotate(percentage(t))))
        .into_any_element()
}

/// 玻璃胶囊：有 Liquid Glass 时字和图标是原生的（图标用 SF Symbol），没有时 GPUI 画一个实心的。点击仍在 GPUI 这边。
fn glass_pill(key: &'static str, w: f32, h: f32, inverted: bool, icon: Option<(&'static str, &'static str)>, label: SharedString) -> Div {
    let (tint, ink) = if inverted { (Some(Theme::text()), Theme::bg_body()) } else { (None, Theme::text()) };
    let size = (h * 0.4).round();
    div().relative().flex_none().w(px(w)).h(px(h)).rounded_full()
        .map(|d| if super::super::glass::supported() { d.child(super::super::glass::page_pane(key, tint, icon.map(|i| (i.0, size, ink)), Some((label, ink)))) }
            else {
                d.bg(tint.unwrap_or_else(general::card_bg)).flex().items_center().justify_center().gap(px(8.))
                    .children(icon.map(|i| svg().path(i.1).size(px(size * 0.8)).text_color(ink)))
                    .child(div().text_size(px(16.)).font_weight(FontWeight::MEDIUM).text_color(ink).child(label))
            })
}

/// 组名，和 `general::titled` 一样的字。
pub(super) fn header(title: &'static str) -> Div {
    div().px(px(26.)).pb(px(12.)).text_size(px(18.)).line_height(px(24.)).font_weight(FontWeight::BOLD).text_color(Theme::text()).child(title)
}

/// 「语音角色」那一排：格子宽、间距、两头留白。第一格是「添加」，角色从第二格起。
const CELL: f32 = 76.;
const GAP: f32 = 12.;
const PAD: f32 = 16.;
const STEP: f32 = CELL + GAP;
/// 提起时头像抬高 6pt：这一排上面为它留的空。
const LIFT: f32 = 16.;
fn slot(i: usize) -> f32 { PAD + (i + 1) as f32 * STEP }

/// 长按一个角色拖着排序（照主屏幕的图标）：提起的那个跟着手指走，别的让开位置，松手按新顺序存下；拖到两头这一排自己往那边滚。
/// 这个顺序就是配音菜单里角色的顺序（上次用的提到最前）。`grab` 是手指离这格左边多远，`x` 是手指在窗口里的横坐标。
#[derive(Clone)]
struct Reorder { id: String, from: usize, grab: f32, x: f32, at: usize }
thread_local! {
    static REORDER: RefCell<Option<Reorder>> = const { RefCell::new(None) };
    /// 各格现在画在离自己位置多远：让位、落下时缓过去。
    static SHIFT: RefCell<Vec<(String, f32)>> = const { RefCell::new(Vec::new()) };
    static ROW: ScrollHandle = ScrollHandle::new();
    /// 刚松手的时候：手势识别补的那次点按不算点开角色。
    static DROPPED: Cell<Option<std::time::Instant>> = const { Cell::new(None) };
}
fn row_handle() -> ScrollHandle { ROW.with(|h| h.clone()) }
fn shift_of(id: &str) -> f32 { SHIFT.with(|s| s.borrow().iter().find(|(k, _)| k == id).map_or(0., |(_, v)| *v)) }
fn set_shift(id: &str, v: f32) {
    SHIFT.with(|s| { let mut s = s.borrow_mut(); match s.iter_mut().find(|(k, _)| k == id) { Some(e) => e.1 = v, None => s.push((id.to_owned(), v)) } });
}
/// 提起的那格现在该落在第几个。
fn drop_index(d: &Reorder, n: usize) -> usize {
    let h = row_handle();
    let left = d.x - f32::from(h.bounds().left() + h.offset().x) - d.grab;
    ((left - PAD) / STEP - 1.).round().clamp(0., n.saturating_sub(1) as f32) as usize
}

/// 语音页的「语音角色」（照 ElevenLabs 的 My voices）：一排可以横着滑的圆头像，名字在下面；最前面是「添加」。长按拖动排序（`Reorder`）。
pub(super) fn section(roles: &[Role], cx: &mut Context<Workspace>) -> AnyElement {
    let available = super::KEY_SAVED.with(Cell::get) == Some(true);
    let caption = |text: SharedString| div().w(px(CELL)).text_center().text_size(px(13.)).line_height(px(17.)).line_clamp(2).text_color(Theme::text()).child(text);
    let add = div().id("speech-add-role").w(px(CELL)).flex_none().flex().flex_col().items_center().gap(px(8.))
        .child(div().size(px(64.)).rounded_full().bg(general::card_bg()).border_1().border_color(theme::ca(craft::N1000, 0.06)).flex().items_center().justify_center()
            .child(ui::icon("plus", 24., Theme::text())))
        .child(caption(tr("添加", "Add").into()))
        // 还没有 Key：先把 Key 配上。
        .on_click(cx.listener(move |w, _, _, cx| if available { open_catalog(w, cx) } else { super::open_key(w, cx) }));
    let drag = REORDER.with(|r| r.borrow().clone());
    let to = drag.as_ref().map(|d| drop_index(d, roles.len()));
    // 各格该在的位置：拖着的跟手指，夹在原位和落点之间的让一格；不拖时都回原位。别的格缓过去，还没到就接着出帧。
    let mut moving = false;
    let shifts: Vec<f32> = roles.iter().enumerate().map(|(i, role)| {
        let want = match (&drag, to) {
            (Some(d), Some(_)) if d.id == role.voice_id => { set_shift(&role.voice_id, 0.); return 0. }
            (Some(d), Some(t)) => if d.from < i && i <= t { -STEP } else if t <= i && i < d.from { STEP } else { 0. },
            _ => 0.,
        };
        let cur = shift_of(&role.voice_id);
        let next = if (want - cur).abs() < 0.5 { want } else { moving = true; cur + (want - cur) * 0.3 };
        set_shift(&role.voice_id, next);
        next
    }).collect();
    let cell = |role: &Role, shift: f32, lifted: bool| {
        let open = role.voice_id.clone();
        div().id(SharedString::from(format!("speech-role-{}", role.voice_id))).w(px(CELL)).flex_none().flex().flex_col().items_center().gap(px(8.)).relative().left(px(shift))
            .child(div().relative().rounded_full().when(lifted, |d| d.shadow(theme::shadow_card())).child(avatar(role, 64.)).children(playing_veil(&role.voice_id, "speech-cell-veil", 64.)))
            .child(caption(role.name.clone().into()))
            .on_click(cx.listener(move |w, _, _, cx| {
                if REORDER.with(|r| r.borrow().is_some()) || DROPPED.with(Cell::get).is_some_and(|t| t.elapsed().as_millis() < 600) { return; }
                open_role(w, open.clone(), cx)
            }))
    };
    let mut cells: Vec<AnyElement> = vec![];
    let mut lifted = None;
    for (i, role) in roles.iter().enumerate() {
        match &drag {
            // 提起的那格离开原位（原位留空，别的格让位时补上），画在最上面。
            Some(d) if d.id == role.voice_id => {
                cells.push(div().w(px(CELL)).flex_none().into_any_element());
                let left = d.x - f32::from(row_handle().bounds().left() + row_handle().offset().x) - d.grab;
                lifted = Some(div().absolute().top(px(LIFT - 6.)).left(px(left)).child(cell(role, 0., true)).into_any_element());
            }
            _ => cells.push(cell(role, shifts[i], false).into_any_element()),
        }
    }
    let ids: Vec<String> = roles.iter().map(|r| r.voice_id.clone()).collect();
    let ws = cx.entity().downgrade();
    let watch = canvas(|_, _, _| {}, move |_, _, window, _| {
        let drag = REORDER.with(|r| r.borrow().clone());
        if moving || drag.is_some() { window.request_animation_frame(); }
        // 拖到这一排两头：往那边滚。
        if let Some(d) = &drag {
            let h = row_handle();
            let b = h.bounds();
            let v = if d.x < f32::from(b.left()) + 44. { 6. } else if d.x > f32::from(b.right()) - 44. { -6. } else { 0. };
            if v != 0. { let o = h.offset(); h.set_offset(point((o.x + px(v)).clamp(-h.max_offset().x, px(0.)), o.y)); }
        }
        let (ws, ids) = (ws.clone(), ids.clone());
        window.on_mouse_event(move |ev: &LongPressEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble { return; }
            let active = REORDER.with(|r| r.borrow().is_some());
            match ev.phase {
                TouchPhase::Started => {
                    let h = row_handle();
                    if active || !h.bounds().contains(&ev.start_position) { return; }
                    let left = f32::from(ev.start_position.x - h.bounds().left() - h.offset().x);
                    let k = ((left - PAD) / STEP).floor() as isize - 1;
                    if k < 0 || k as usize >= ids.len() || left - slot(k as usize) > CELL { return; }
                    let k = k as usize;
                    window.prevent_default();
                    crate::ios::take_long_press();
                    REORDER.with(|r| *r.borrow_mut() = Some(Reorder { id: ids[k].clone(), from: k, grab: left - slot(k), x: f32::from(ev.start_position.x), at: k }));
                }
                _ if !active => return,
                TouchPhase::Moved => {
                    let changed = REORDER.with(|r| {
                        let mut r = r.borrow_mut();
                        let Some(d) = r.as_mut() else { return false };
                        d.x = f32::from(ev.position.x);
                        let t = drop_index(d, ids.len());
                        std::mem::replace(&mut d.at, t) != t
                    });
                    if changed { crate::ios::glass::haptic(); }
                }
                TouchPhase::Ended => {
                    let Some(d) = REORDER.with(|r| r.borrow_mut().take()) else { return };
                    let t = drop_index(&d, ids.len());
                    DROPPED.with(|x| x.set(Some(std::time::Instant::now())));
                    // 落下：各格从现在画着的地方缓到新位置（提起的那格从手指下面）。
                    let mut order = ids.clone();
                    let id = order.remove(d.from);
                    order.insert(t, id);
                    let lifted_at = d.x - f32::from(row_handle().bounds().left() + row_handle().offset().x) - d.grab;
                    for (k, id) in order.iter().enumerate() {
                        let Some(i) = ids.iter().position(|x| x == id) else { continue };
                        let visual = if i == d.from { lifted_at } else { slot(i) + shift_of(id) };
                        set_shift(id, visual - slot(k));
                    }
                    if t != d.from {
                        let _ = ws.update(cx, |w, cx| update(w, |roles| roles.sort_by_key(|r| order.iter().position(|id| *id == r.voice_id).unwrap_or(usize::MAX)), cx));
                    }
                }
                _ => { REORDER.with(|r| *r.borrow_mut() = None); }
            }
            let _ = ws.update(cx, |_, cx| cx.notify());
        });
        // 拖着的时候这一排和整页都不跟着手指滚。
        window.on_mouse_event(|ev: &ScrollEvent, phase, _, cx| {
            if phase == DispatchPhase::Capture && ev.source == ScrollSource::Touch && REORDER.with(|r| r.borrow().is_some()) { cx.stop_propagation(); }
        });
    }).absolute().size_0();
    let width = 2. * PAD + (roles.len() + 1) as f32 * STEP - GAP;
    div().w_full().flex().flex_col()
        .child(header(tr("语音角色", "Voice Roles")))
        // 这一排会裁掉超出的部分：上面多留 `LIFT` 给提起时抬高的头像和它的阴影，再整排提回去，不拖时版面不变。
        .child(div().id("speech-roles").w_full().mt(px(-LIFT)).overflow_x_scroll().restrict_scroll_to_axis().track_scroll(&row_handle())
            .child(div().w(px(width)).flex_none().px(px(PAD)).pt(px(LIFT)).relative().flex().items_start().gap(px(GAP)).child(add).children(cells).children(lifted)))
        .child(watch)
        .into_any_element()
}

/// 试听一个角色：存着样音地址就直接放，没存（旧版收藏的）先向 ElevenLabs 问一次，问到了顺手存下。
fn preview_role(role: &Role, cx: &mut Context<Workspace>) {
    let (id, known) = (role.voice_id.clone(), role.preview_url.clone());
    let key = if known.is_none() { super::read_key() } else { None };
    let lookup = id.clone();
    super::preview::toggle(id, async move {
        if known.is_some() { return known; }
        let url = format!("https://api.elevenlabs.io/v1/voices/{lookup}");
        let voice: Voice = serde_json::from_slice(&fetch_bytes(&url, Some(&key?)).await.ok()?.ok()?.ok()?).ok()?;
        let found = voice.preview_url.clone().filter(|u| !u.is_empty())?;
        let save = found.clone();
        menu::dispatch(move |w, _, cx| update(w, |roles| if let Some(r) = roles.iter_mut().find(|r| r.voice_id == lookup) {
            r.preview_url = Some(save.clone());
            if r.traits.is_empty() { r.traits = voice.traits(); }
        }, cx));
        Some(found)
    }, cx);
}

fn item(id: impl Into<ElementId>, name: impl Into<SharedString>, subtitle: Option<SharedString>, icon: Option<AnyElement>, control: impl IntoElement) -> Stateful<Div> {
    div().id(id).w_full().min_h(px(54.)).px(px(18.)).py(px(11.)).flex().items_center().gap(px(12.)).active(|d| d.bg(Theme::fill_pressed()))
        .children(icon)
        .child(div().flex_1().min_w_0().flex().flex_col().gap(px(2.))
            .child(div().truncate().text_size(px(17.)).line_height(px(22.)).text_color(row_ink()).child(name.into()))
            .children(subtitle.map(|s| div().truncate().text_size(px(14.)).line_height(px(19.)).text_color(Theme::text_caption()).child(s))))
        .child(div().flex_none().flex().items_center().child(control))
}

fn open_catalog(w: &mut Workspace, cx: &mut Context<Workspace>) {
    super::super::glass::haptic();
    super::preview::stop();
    FILTER.with(|f| f.set(0));
    SEARCH.with(|search| {
        let mut search = search.borrow_mut();
        let placeholder = tr("搜索音色", "Search voices");
        if let Some(field) = search.as_ref() {
            field.update(cx, |f, cx| { f.set_placeholder(placeholder, cx); f.set_text("", cx); });
        } else {
            let field = cx.new(|cx| TextField::new(placeholder, cx));
            cx.observe(&field, |_, _, cx| cx.notify()).detach();
            *search = Some(field);
        }
    });
    w.open_settings(Page::AssistantVoices, cx);
    fetch(cx);
}

/// 载入时的几行骨架：头像和两道字的位置，一起轻轻地明暗。
fn skeleton() -> AnyElement {
    let block = |w: f32, h: f32| div().w(px(w)).h(px(h)).rounded_full().bg(theme::ca(craft::N1000, 0.07));
    general::card((0..7).map(|i| div().w_full().h(px(64.)).px(px(16.)).flex().items_center().gap(px(14.))
        .child(div().size(px(40.)).rounded_full().bg(theme::ca(craft::N1000, 0.07)))
        .child(div().flex().flex_col().gap(px(8.)).child(block(96. + (i * 37 % 60) as f32, 12.)).child(block(150. + (i * 53 % 70) as f32, 9.)))
        .into_any_element()).collect())
        .with_animation("speech-skeleton", Animation::new(std::time::Duration::from_millis(1100)).repeat(), |d, t| d.opacity(0.55 + 0.45 * (0.5 - 0.5 * (t * std::f32::consts::TAU).cos())))
        .into_any_element()
}

pub fn catalog(cx: &mut Context<Workspace>) -> AnyElement {
    let search = SEARCH.with(|s| s.borrow().clone());
    let query = search.as_ref().map_or_else(String::new, |f| f.read(cx).text.trim().to_lowercase());
    let filter = FILTER.with(Cell::get);
    let message = |text: &'static str| div().px(px(18.)).py(px(18.)).text_center().text_size(px(15.)).line_height(px(21.)).text_color(Theme::text_caption()).child(text).into_any_element();
    let (ready, body) = CATALOG.with(|catalog| match &*catalog.borrow() {
        Catalog::Idle | Catalog::Loading => (false, skeleton()),
        Catalog::Failed(error) => (false, general::card(vec![message(match error {
            LoadError::Key | LoadError::Http(401 | 403) => tr("请检查 API Key 和音色读取权限。", "Check your API Key and its voices read permission."),
            LoadError::Http(429) => tr("请求过多，请稍后重试。", "Too many requests. Try again shortly."),
            LoadError::Network => tr("无法连接 ElevenLabs，请检查网络。", "Could not connect to ElevenLabs. Check your connection."),
            _ => tr("无法获取 ElevenLabs 音色。", "Could not load ElevenLabs voices."),
        }), general::action("speech-retry-voices", tr("重试", "Retry"), false, |_, cx| fetch(cx), cx)]).into_any_element()),
        Catalog::Ready(voices) => {
            let roles = all();
            let local = |v: &Voice| roles.iter().find(|r| r.voice_id == v.voice_id);
            let shown: Vec<_> = voices.iter().filter(|v| v.matches(filter) && (query.is_empty() || v.name().to_lowercase().contains(&query)
                || v.traits().to_lowercase().contains(&query) || local(v).is_some_and(|r| r.name.to_lowercase().contains(&query)))).collect();
            let rows: Vec<_> = if shown.is_empty() { vec![message(tr("没有找到音色。", "No voices found."))] }
                else { shown.into_iter().map(|v| voice_row(v, local(v), cx)).collect() };
            (true, general::card(rows).into_any_element())
        }
    });
    let chips = [tr("全部", "All"), tr("女声", "Female"), tr("男声", "Male"), tr("我的", "Mine")];
    let glass = super::super::glass::supported();
    div().w_full().pt(px(18.)).flex().flex_col().gap(px(14.))
        .when(ready, |d| d.children(search.map(|f| general::card(vec![div().px(px(18.)).h(px(50.)).flex().items_center().gap(px(10.))
            .child(ui::icon("search", 17., Theme::text_caption())).child(div().flex_1().min_w_0().child(f)).into_any_element()])))
            // 筛选：一排玻璃胶囊，选中的那个反色。
            .child(div().mx(px(10.)).flex().gap(px(8.)).children(chips.iter().enumerate().map(|(i, label)| {
                let on = i as u8 == filter;
                let (tint, ink) = if on { (Some(Theme::text()), Theme::bg_body()) } else { (None, Theme::text()) };
                div().id(FILTER_GLASS[i]).relative().h(px(36.)).px(px(16.)).rounded_full().flex().items_center().justify_center()
                    .map(|d| if glass { d.min_w(px(64.)).child(super::super::glass::page_pane(FILTER_GLASS[i], tint, None, Some(((*label).into(), ink)))) }
                        else { d.bg(tint.unwrap_or_else(general::card_bg)).text_size(px(15.)).font_weight(FontWeight::MEDIUM).text_color(ink).child(*label) })
                    .on_click(cx.listener(move |_, _, window, cx| {
                        window.blur(cx);
                        if FILTER.with(|f| f.replace(i as u8)) != i as u8 { super::super::glass::haptic(); super::super::glass::pulse(FILTER_GLASS[i]); cx.notify(); }
                    }))
            }))))
        .child(body).into_any_element()
}

/// 音色一行：头像（收藏过的用角色自己的头像）、名字（收藏过的写本地名称，下面注明 ElevenLabs 上的名字）、星标。
fn voice_row(v: &Voice, role: Option<&Role>, cx: &mut Context<Workspace>) -> AnyElement {
    let (voice, star, id) = (v.clone(), v.clone(), v.voice_id.clone());
    let phase = super::preview::phase(&v.voice_id);
    let traits = v.traits();
    let (title, subtitle): (String, String) = match role {
        Some(r) if r.name != v.name() => (r.name.clone(), [format!("ElevenLabs · {}", v.name()), traits].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ")),
        _ => (v.name().to_owned(), traits),
    };
    let popped = POPPED.with(|p| { let p = p.borrow(); (p.0 == v.voice_id).then_some(p.1) });
    let star_icon = ui::icon(if role.is_some() { "star-fill" } else { "star" }, 20., if role.is_some() { Theme::text() } else { Theme::text_placeholder() });
    let star_icon = match popped {
        Some(n) => star_icon.with_animation(("speech-star-pop", n), Animation::new(std::time::Duration::from_millis(460)).with_easing(ease_out_quint()), |s, t| {
            let k = 1. + 0.5 * (t * std::f32::consts::PI).sin();
            s.with_transformation(Transformation::scale(size(k, k)))
        }).into_any_element(),
        None => star_icon.into_any_element(),
    };
    let face = match role { Some(r) => avatar(r, 40.), None => orb(&v.voice_id, 40.).into_any_element() };
    div().id(SharedString::from(format!("speech-voice-{}", v.voice_id))).relative().w_full().min_h(px(64.)).pl(px(16.)).pr(px(8.)).py(px(12.)).flex().items_center().gap(px(14.))
        .active(|d| d.bg(Theme::fill_pressed()))
        .child(div().relative().flex_none().child(face).children(playing_veil(&v.voice_id, "speech-row-veil", 40.)))
        .child(div().flex_1().min_w_0().flex().flex_col().gap(px(2.))
            .child(div().truncate().text_size(px(17.)).line_height(px(22.)).font_weight(if phase == Phase::Idle { FontWeight::NORMAL } else { FontWeight::MEDIUM }).text_color(row_ink()).child(title))
            .when(!subtitle.is_empty(), |d| d.child(div().truncate().text_size(px(13.)).line_height(px(18.)).text_color(Theme::text_caption()).child(subtitle))))
        .child(div().id(SharedString::from(format!("speech-star-{}", v.voice_id))).flex_none().size(px(44.)).rounded_full().flex().items_center().justify_center()
            .active(|d| d.bg(Theme::fill_pressed()))
            .child(star_icon)
            .on_click(cx.listener(move |w, _, window, cx| { cx.stop_propagation(); window.blur(cx); add_role(w, &star, cx); })))
        .children(super::preview::progress(&v.voice_id, 180.).map(|bar| div().absolute().left(px(70.)).bottom(px(4.)).child(bar)))
        .on_click(cx.listener(move |_, _, window, cx| { window.blur(cx); let url = voice.preview_url.clone(); super::preview::toggle(id.clone(), async move { url }, cx); }))
        .into_any_element()
}

/// 收藏为角色：没收藏过的存下来，星星弹一下，留在列表里接着挑；收藏过的进它的角色设置（改名字、换头像、移除）。
fn add_role(w: &mut Workspace, voice: &Voice, cx: &mut Context<Workspace>) {
    if all().iter().any(|r| r.voice_id == voice.voice_id) { return open_role(w, voice.voice_id.clone(), cx); }
    super::super::glass::haptic();
    let name = voice.name().to_owned();
    let role = Role { voice_id: voice.voice_id.clone(), voice_name: name.clone(), name: name.clone(), avatar: None, preview_url: voice.preview_url.clone(), traits: voice.traits(), tags: false, lang: String::new() };
    update(w, |roles| roles.push(role), cx);
    if all().iter().any(|r| r.voice_id == voice.voice_id) {
        POPPED.with(|p| { let mut p = p.borrow_mut(); p.0 = voice.voice_id.clone(); p.1 += 1; });
        w.show_toast_kind(ui::ToastKind::Success, tr("已收藏", "Saved"), cx);
    }
}

fn open_role(w: &mut Workspace, id: String, cx: &mut Context<Workspace>) {
    super::super::glass::haptic();
    super::preview::stop();
    EDITING.with(|r| *r.borrow_mut() = id);
    w.open_settings(Page::AssistantRole, cx);
}

/// 角色设置：上面是圆头像（右下角换头像的玻璃钮，进「选择头像」）、本地名称、对应的 ElevenLabs 音色和「试听」；
/// 下面「本地资料」是名称和头像，存在全局库里，ElevenLabs 那边的音色不改。
pub fn detail(cx: &mut Context<Workspace>) -> AnyElement {
    let Some(role) = EDITING.with(|id| all().into_iter().find(|r| r.voice_id == *id.borrow())) else { return div().into_any_element() };
    let (name, clear, play) = (role.clone(), role.voice_id.clone(), role.clone());
    let mut rows = vec![
        item("speech-role-name", tr("名称", "Name"), Some(role.name.clone().into()), None, ui::icon("chevron-right", 14., row_chevron()))
            .on_click(cx.listener(move |w, _, _, cx| rename(w, &name, cx))).into_any_element(),
        item("speech-role-source", tr("ElevenLabs 音色", "ElevenLabs Voice"), Some(role.voice_name.clone().into()), None, div()).into_any_element(),
        general::action("speech-role-photo", tr("更换头像", "Change Avatar"), false, open_avatars, cx),
    ];
    if role.avatar.is_some() { rows.push(general::action("speech-role-reset", tr("恢复默认头像", "Use Default Avatar"), false, move |w, cx| set_avatar(w, &clear, None, cx), cx)); }
    let phase = super::preview::phase(&role.voice_id);
    const SIZE: f32 = 120.;
    // 正在放时头像外一圈细环，跟着呼吸。
    let halo = (phase == Phase::Playing).then(|| div().absolute().top(px(-7.)).left(px(-7.)).size(px(SIZE + 14.)).rounded_full().border_2().border_color(Theme::text())
        .with_animation(("speech-halo", super::preview::epoch() as usize), Animation::new(std::time::Duration::from_millis(1400)).repeat(), |d, t| d.opacity(0.15 + 0.55 * (0.5 - 0.5 * (t * std::f32::consts::TAU).cos()))));
    let camera = div().id("speech-avatar-badge").absolute().right(px(-2.)).bottom(px(-2.)).size(px(38.)).rounded_full()
        .map(|d| if super::super::glass::supported() { d.child(super::super::glass::page_pane("speech-avatar-glass", None, Some(("sf:camera.fill", 16., Theme::text())), None)) }
            else { d.bg(Theme::bg_body()).shadow(theme::shadow_button()).flex().items_center().justify_center().child(ui::icon("pen-line", 15., Theme::text())) })
        .on_click(cx.listener(move |w, _, _, cx| { super::super::glass::pulse("speech-avatar-glass"); open_avatars(w, cx); }));
    let (label, icon) = match phase {
        Phase::Idle => (tr("试听", "Preview"), ("sf:play.fill", "editor-icons/play-filled.svg")),
        Phase::Loading => (tr("载入中", "Loading"), ("sf:ellipsis", "editor-icons/ellipsis.svg")),
        Phase::Playing => (tr("停止", "Stop"), ("sf:stop.fill", "editor-icons/pause-filled.svg")),
    };
    // 胶囊按字的宽度定：原生玻璃上的字靠左排，宽了右边会空出一截。
    let text_w: f32 = label.chars().map(|c| if c.is_ascii() { 9. } else { 16. }).sum();
    let listen = glass_pill("speech-listen-glass", 14. + 19. + 8. + text_w + 20., 48., phase != Phase::Idle, Some(icon), label.into())
        .id("speech-role-listen")
        .on_click(cx.listener(move |_, _, _, cx| { super::super::glass::pulse("speech-listen-glass"); preview_role(&play, cx); }));
    let source = [role.voice_name.as_str(), role.traits.as_str()].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
    let hero = div().w_full().px(px(24.)).flex().flex_col().items_center()
        .child(div().relative().size(px(SIZE)).children(halo).child(div().rounded_full().shadow(theme::shadow_card()).child(avatar(&role, SIZE)))
            .children(playing_veil(&role.voice_id, "speech-hero-veil", SIZE).filter(|_| phase == Phase::Loading)).child(camera))
        .child(div().mt(px(18.)).max_w_full().truncate().text_size(px(28.)).line_height(px(34.)).font_weight(FontWeight::BOLD).text_color(Theme::text()).child(role.name.clone()))
        .child(div().mt(px(4.)).max_w_full().truncate().text_size(px(14.)).line_height(px(19.)).text_color(Theme::text_caption()).child(source))
        .child(div().mt(px(22.)).child(listen))
        .child(div().mt(px(12.)).h(px(3.)).flex().justify_center().children(super::preview::progress(&role.voice_id, 120.)));
    div().pt(px(12.)).flex().flex_col().gap(px(32.))
        .child(hero)
        .child(general::card(rows))
        .child(script_rows(&role, cx))
        .into_any_element()
}

/// 「台本」：配音前要不要让快速模型加语气标签、把台词译成哪种语言。每段配音的「…」里还能单独改。
fn script_rows(role: &Role, cx: &mut Context<Workspace>) -> AnyElement {
    let id = role.voice_id.clone();
    let tags = item("speech-role-tags", tr("语气标签", "Audio Tags"), None, None, general::toggle("speech-role-tags-toggle", role.tags))
        .on_click(cx.listener(move |w, _, _, cx| {
            super::super::glass::haptic();
            update(w, |roles| if let Some(r) = roles.iter_mut().find(|r| r.voice_id == id) { r.tags = !r.tags; }, cx);
        })).into_any_element();
    let items = LANGS.iter().map(|&(action, code, _)| menu::Item::new(action, lang_name(code), "", 0).checked(role.lang == code)).collect();
    let lang = div().relative().w_full().h(px(54.)).px(px(18.)).flex().items_center().gap(px(12.))
        .child(div().flex_1().min_w_0().text_size(px(17.)).text_color(row_ink()).child(tr("台词语言", "Line Language")))
        .child(div().flex_none().text_size(px(17.)).text_color(Theme::text_caption()).child(lang_name(&role.lang)))
        .child(ui::icon("chevron-up-down", 16., row_chevron()))
        .child(menu::button("speech-role-lang-menu", items))
        .into_any_element();
    general::titled(Some(tr("台本", "Script")), vec![tags, lang]).into_any_element()
}

/// 角色页「台词语言」菜单里的一项。
pub fn menu_action(action: &str, w: &mut Workspace, cx: &mut Context<Workspace>) -> bool {
    let Some(&(_, code, _)) = LANGS.iter().find(|l| l.0 == action) else { return false };
    let id = EDITING.with(|e| e.borrow().clone());
    update(w, |roles| if let Some(r) = roles.iter_mut().find(|r| r.voice_id == id) { r.lang = code.into(); }, cx);
    true
}

fn rename(w: &mut Workspace, role: &Role, cx: &mut Context<Workspace>) {
    let id = role.voice_id.clone();
    let title = tr("角色名称", "Role Name");
    if !super::prompt(title, None, title, &role.name, super::Field::Text, Some(tr("移除角色", "Remove Role")), move |value| {
        let id = id.clone();
        menu::dispatch(move |w, _, cx| match value {
            Some(name) if name.is_empty() => w.show_toast_kind(ui::ToastKind::Error, tr("请输入角色名称", "Enter a role name"), cx),
            Some(name) => update(w, |roles| { if let Some(role) = roles.iter_mut().find(|r| r.voice_id == id) { role.name = name; } }, cx),
            None => remove_role(w, &id, cx),
        });
    }) { w.show_toast_kind(ui::ToastKind::Error, tr("无法打开角色设置", "Could not open role settings"), cx); }
}

fn remove_role(w: &mut Workspace, id: &str, cx: &mut Context<Workspace>) {
    super::preview::stop();
    let old = all().into_iter().find(|r| r.voice_id == id).and_then(|r| r.avatar);
    update(w, |roles| roles.retain(|r| r.voice_id != id), cx);
    if all().iter().any(|r| r.voice_id == id) { return; }
    discard_avatar(old.as_deref());
    super::super::settings_back(w, cx);
}

fn old_avatars_dir() -> PathBuf { crate::store::Store::home_dir().join("assistant-avatars") }
fn avatars_dir() -> PathBuf { fdl_ai::global::dir().join("avatars") }
/// 照片头像的文件：全局库的 `avatars/`；还没搬过来的（旧版存在主目录）用原处的。
fn avatar_path(id: &str) -> Option<PathBuf> {
    let file = format!("{}.png", uuid::Uuid::parse_str(id).ok()?);
    let (path, old) = (avatars_dir().join(&file), old_avatars_dir().join(&file));
    Some(if !path.is_file() && old.is_file() { old } else { path })
}

/// 自带的头像（`avatars/*.webp`，语音角色和图像的人物用，编进二进制）。选了的话 `Role::avatar` 存 `builtin-NN`（从 1 数）。
const BUILTIN: [&[u8]; 38] = [
    include_bytes!("avatars/01.webp"),
    include_bytes!("avatars/02.webp"),
    include_bytes!("avatars/03.webp"),
    include_bytes!("avatars/04.webp"),
    include_bytes!("avatars/05.webp"),
    include_bytes!("avatars/06.webp"),
    include_bytes!("avatars/07.webp"),
    include_bytes!("avatars/08.webp"),
    include_bytes!("avatars/09.webp"),
    include_bytes!("avatars/10.webp"),
    include_bytes!("avatars/11.webp"),
    include_bytes!("avatars/12.webp"),
    include_bytes!("avatars/13.webp"),
    include_bytes!("avatars/14.webp"),
    include_bytes!("avatars/15.webp"),
    include_bytes!("avatars/16.webp"),
    include_bytes!("avatars/17.webp"),
    include_bytes!("avatars/18.webp"),
    include_bytes!("avatars/19.webp"),
    include_bytes!("avatars/20.webp"),
    include_bytes!("avatars/21.webp"),
    include_bytes!("avatars/22.webp"),
    include_bytes!("avatars/23.webp"),
    include_bytes!("avatars/24.webp"),
    include_bytes!("avatars/25.webp"),
    include_bytes!("avatars/26.webp"),
    include_bytes!("avatars/27.webp"),
    include_bytes!("avatars/28.webp"),
    include_bytes!("avatars/29.webp"),
    include_bytes!("avatars/30.webp"),
    include_bytes!("avatars/31.webp"),
    include_bytes!("avatars/32.webp"),
    include_bytes!("avatars/33.webp"),
    include_bytes!("avatars/34.webp"),
    include_bytes!("avatars/35.webp"),
    include_bytes!("avatars/36.webp"),
    include_bytes!("avatars/37.webp"),
    include_bytes!("avatars/38.webp"),
];
const BUILTIN_PREFIX: &str = "builtin-";

fn builtin(id: &str) -> Option<usize> { id.strip_prefix(BUILTIN_PREFIX)?.parse().ok().filter(|n| (1..=BUILTIN.len()).contains(n)) }

/// 自带头像解出来的图，按编号缓存。
fn builtin_image(n: usize) -> std::sync::Arc<Image> {
    thread_local! { static IMAGES: RefCell<Vec<Option<std::sync::Arc<Image>>>> = const { RefCell::new(Vec::new()) }; }
    IMAGES.with(|images| {
        let mut images = images.borrow_mut();
        if images.len() < BUILTIN.len() { images.resize(BUILTIN.len(), None); }
        images[n - 1].get_or_insert_with(|| std::sync::Arc::new(Image::from_bytes(ImageFormat::Webp, BUILTIN[n - 1].to_vec()))).clone()
    })
}

fn circle(size: f32, image: impl Into<ImageSource>) -> AnyElement {
    div().size(px(size)).flex_none().rounded_full().overflow_hidden().bg(general::card_bg()).child(img(image).size_full().rounded_full().object_fit(ObjectFit::Cover)).into_any_element()
}

/// 头像标识（自带的 `builtin-NN` 或照片）画成圆；照片不在这台设备上（比如人物从别的设备同步过来）是 `None`。
pub(super) fn avatar_image(id: Option<&str>, size: f32) -> Option<AnyElement> {
    let id = id?;
    if let Some(n) = builtin(id) { return Some(circle(size, builtin_image(n))); }
    avatar_path(id).filter(|path| path.is_file()).map(|path| circle(size, path))
}

/// 角色头像：换过的是圆形照片或自带头像，没换的是这个音色的光球。
pub(crate) fn avatar(role: &Role, size: f32) -> AnyElement {
    if let Some(n) = role.avatar.as_deref().and_then(builtin) { return circle(size, builtin_image(n)); }
    match role.avatar.as_deref().and_then(avatar_path).filter(|path| path.is_file()) {
        Some(path) => circle(size, path),
        None => orb(&role.voice_id, size).into_any_element(),
    }
}

/// 角色头像的图片文件，给系统菜单用（`menu::Item::picture`）：自带头像、换上的照片，没换的是光球。按头像缓存。
pub(crate) fn picture(role: &Role) -> Option<(SharedString, std::sync::Arc<[u8]>)> {
    thread_local! { static PICTURES: RefCell<Vec<(SharedString, std::sync::Arc<[u8]>)>> = const { RefCell::new(Vec::new()) }; }
    let key: SharedString = role.avatar.clone().unwrap_or_else(|| format!("orb-{}", role.voice_id)).into();
    if let Some(found) = PICTURES.with(|p| p.borrow().iter().find(|(k, _)| *k == key).cloned()) { return Some(found); }
    let bytes: Vec<u8> = match role.avatar.as_deref() {
        Some(a) if let Some(n) = builtin(a) => BUILTIN[n - 1].to_vec(),
        Some(a) => std::fs::read(avatar_path(a)?).ok()?,
        None => orb_png(&role.voice_id)?,
    };
    let found = (key, std::sync::Arc::<[u8]>::from(bytes));
    PICTURES.with(|p| p.borrow_mut().push(found.clone()));
    Some(found)
}

/// 换上一个头像（`None` 是恢复默认）。
pub(super) type SetAvatar = Rc<dyn Fn(&mut Workspace, Option<String>, &mut Context<Workspace>)>;
thread_local! {
    /// 选择头像页不是给语音角色选时：现在的头像、换上的办法、返回到哪一页（图像的人物）。
    static CHOOSING: RefCell<Option<(Option<String>, SetAvatar, Page)>> = const { RefCell::new(None) };
}

fn open_avatars(w: &mut Workspace, cx: &mut Context<Workspace>) {
    super::super::glass::haptic();
    CHOOSING.with(|c| *c.borrow_mut() = None);
    w.open_settings(Page::AssistantAvatar, cx);
}

/// 别处（图像的人物）借选择头像页：`back` 是选完回到的那页。
pub(super) fn choose_avatar(current: Option<String>, back: Page, set: SetAvatar, w: &mut Workspace, cx: &mut Context<Workspace>) {
    super::super::glass::haptic();
    CHOOSING.with(|c| *c.borrow_mut() = Some((current, set, back)));
    w.open_settings(Page::AssistantAvatar, cx);
}
/// 选择头像页的上一页。
pub(super) fn avatars_parent() -> Page { CHOOSING.with(|c| c.borrow().as_ref().map_or(Page::AssistantRole, |c| c.2)) }

/// 选择头像（角色设置或人物的下一级）：四个一排的圆，第一个是「从照片选」，后面是自带的头像，现在用的那个外面一圈细环。
/// 点一下就换上并回到上一页。
pub fn avatars(cx: &mut Context<Workspace>) -> AnyElement {
    let chosen = CHOOSING.with(|c| c.borrow().as_ref().map(|(current, set, _)| (current.clone(), set.clone())));
    let (current, set): (Option<String>, SetAvatar) = match chosen {
        Some(found) => found,
        None => {
            let Some(role) = EDITING.with(|id| all().into_iter().find(|r| r.voice_id == *id.borrow())) else { return div().into_any_element() };
            let id = role.voice_id.clone();
            (role.avatar, Rc::new(move |w, next, cx| set_avatar(w, &id, next, cx)))
        }
    };
    let current = current.as_deref().and_then(builtin);
    const CELL: f32 = 80.;
    let ring = |id: ElementId, on: bool| div().id(id).size(px(CELL)).flex_none().rounded_full().p(px(4.)).border_2()
        .border_color(if on { Theme::text() } else { gpui::transparent_black() }).active(|d| d.opacity(0.7));
    let photo = set.clone();
    let mut cells = vec![ring("speech-avatar-photo".into(), false)
        .child(div().size_full().rounded_full().bg(general::card_bg()).flex().items_center().justify_center().child(svg().path("editor-icons/image.svg").size(px(26.)).flex_none().text_color(Theme::text_caption())))
        .on_click(cx.listener(move |_, _, _, cx| { super::super::glass::haptic(); pick_avatar(photo.clone(), cx); }))
        .into_any_element()];
    cells.extend((1..=BUILTIN.len()).map(|n| {
        let set = set.clone();
        ring(("speech-builtin-avatar", n).into(), current == Some(n)).child(circle(CELL - 12., builtin_image(n)))
            .on_click(cx.listener(move |w, _, _, cx| {
                super::super::glass::haptic();
                set(w, Some(format!("{BUILTIN_PREFIX}{n:02}")), cx);
                super::super::settings_back(w, cx);
            }))
            .into_any_element()
    }));
    let mut cells = cells.into_iter();
    // 最后一排不满时用空位补齐，圆还是和上面对齐。
    let rows = std::iter::from_fn(move || {
        let row: Vec<AnyElement> = cells.by_ref().take(4).collect();
        let gap = 4 - row.len();
        (!row.is_empty()).then(|| div().w_full().flex().justify_between().children(row).children((0..gap).map(|_| div().size(px(CELL)).flex_none())))
    });
    div().w_full().px(px(20.)).pt(px(18.)).pb(px(32.)).flex().flex_col().gap(px(14.)).children(rows).into_any_element()
}

/// 从照片选一张，裁成正方形存下来；在「选择头像」页选的，换上后回到上一页。
fn pick_avatar(set: SetAvatar, cx: &mut Context<Workspace>) {
    let pick = super::super::files::open_image();
    cx.spawn(async move |this, cx| {
        let Some(path) = pick.await.ok().and_then(Result::ok).flatten().and_then(|p| p.into_iter().next()) else { return };
        let Some(path) = super::super::crop::square(path).await.ok().and_then(Result::ok).flatten() else { return };
        let _ = this.update(cx, |w, cx| match write_avatar(&path) {
            Ok(image) => {
                set(w, Some(image), cx);
                if w.settings_page == Page::AssistantAvatar { super::super::settings_back(w, cx); }
            }
            Err(_) => w.show_toast_kind(ui::ToastKind::Error, tr("无法保存头像", "Could not save avatar"), cx),
        });
    }).detach();
}

fn write_avatar(from: &Path) -> anyhow::Result<String> {
    let id = uuid::Uuid::new_v4().to_string();
    let to = avatar_path(&id).unwrap();
    let image = image::open(from)?.thumbnail(384, 384);
    std::fs::create_dir_all(to.parent().unwrap())?;
    if let Err(err) = image.save(&to) { let _ = std::fs::remove_file(to); return Err(err.into()); }
    Ok(id)
}

pub(super) fn discard_avatar(id: Option<&str>) { if let Some(path) = id.and_then(avatar_path) { let _ = std::fs::remove_file(path); } }

fn set_avatar(w: &mut Workspace, id: &str, next: Option<String>, cx: &mut Context<Workspace>) {
    let Some(old) = all().into_iter().find(|r| r.voice_id == id) else { discard_avatar(next.as_deref()); return; };
    update(w, |roles| { if let Some(r) = roles.iter_mut().find(|r| r.voice_id == id) { r.avatar = next.clone(); } }, cx);
    if all().iter().any(|r| r.voice_id == id && r.avatar == next) { discard_avatar(old.avatar.as_deref()); }
    else { discard_avatar(next.as_deref()); }
}

fn fetch(cx: &mut Context<Workspace>) {
    if CATALOG.with(|c| matches!(*c.borrow(), Catalog::Loading)) { return; }
    let Some(key) = super::read_key() else { CATALOG.with(|c| *c.borrow_mut() = Catalog::Failed(LoadError::Key)); return cx.notify(); };
    CATALOG.with(|c| *c.borrow_mut() = Catalog::Loading);
    let request = REQUEST.with(|r| { r.set(r.get() + 1); r.get() });
    cx.notify();
    cx.spawn(async move |this, cx| {
        let result = fetch_pages(&key).await;
        if REQUEST.with(Cell::get) == request {
            CATALOG.with(|c| *c.borrow_mut() = match result { Ok(voices) => Catalog::Ready(voices), Err(e) => Catalog::Failed(e) });
            let _ = this.update(cx, |_, cx| cx.notify());
        }
    }).detach();
}

async fn fetch_pages(key: &str) -> Result<Vec<Voice>, LoadError> {
    let mut voices = Vec::new();
    let mut ids = HashSet::new();
    let mut cursors = HashSet::new();
    let mut token = None;
    loop {
        let mut url = "https://api.elevenlabs.io/v2/voices?page_size=100&include_total_count=false".to_owned();
        if let Some(cursor) = token.as_deref() {
            url.push_str("&next_page_token=");
            for b in str::bytes(cursor) {
                if b.is_ascii_alphanumeric() || b"-._~".contains(&b) { url.push(b as char); }
                else { let _ = write!(url, "%{b:02X}"); }
            }
        }
        let bytes = fetch_bytes(&url, Some(key)).await.unwrap().unwrap()?;
        let page: VoicePage = serde_json::from_slice(&bytes).map_err(|_| LoadError::Response)?;
        voices.extend(page.voices.into_iter().filter(|v| !v.voice_id.is_empty() && ids.insert(v.voice_id.clone())));
        if !page.has_more { return Ok(voices); }
        token = page.next_page_token.filter(|t| !t.is_empty() && cursors.insert(t.clone()));
        if token.is_none() { return Err(LoadError::Response); }
    }
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "QuietMdElevenLabsSessionDelegate"]
    struct SessionDelegate;

    impl SessionDelegate {
        #[unsafe(method(URLSession:task:willPerformHTTPRedirection:newRequest:completionHandler:))]
        fn redirect(&self, _session: *mut AnyObject, _task: *mut AnyObject, _response: *mut AnyObject, _request: *mut AnyObject, reply: &block2::Block<dyn Fn(*mut AnyObject)>) {
            // API Key 只发往最初的 ElevenLabs URL。
            reply.call((null_mut(),));
        }
    }
);

/// GET 一个地址，拿回原始字节。`key` 只发往 ElevenLabs 的接口，带着它时不跟随重定向；样音文件不带 Key。
pub(super) fn fetch_bytes(url: &str, key: Option<&str>) -> super::super::files::Reply<Result<Vec<u8>, LoadError>> {
    use super::super::files::Reply;
    unsafe {
        let url: *mut AnyObject = msg_send![class!(NSURL), URLWithString: ns_string(url)];
        if url.is_null() { return Reply::ready(Err(LoadError::Response)); }
        let request: *mut AnyObject = msg_send![class!(NSMutableURLRequest), requestWithURL: url];
        if let Some(key) = key { let _: () = msg_send![request, setValue: ns_string(key), forHTTPHeaderField: ns_string("xi-api-key")]; }
        let _: () = msg_send![request, setTimeoutInterval: 15f64];
        let config: *mut AnyObject = msg_send![class!(NSURLSessionConfiguration), ephemeralSessionConfiguration];
        let queue: *mut AnyObject = msg_send![class!(NSOperationQueue), mainQueue];
        let delegate: Option<objc2::rc::Retained<SessionDelegate>> = key.is_some().then(|| msg_send![SessionDelegate::class(), new]);
        let delegate: *const AnyObject = delegate.as_deref().map_or(null_mut::<AnyObject>(), |d| (d as *const SessionDelegate).cast_mut().cast());
        let session: *mut AnyObject = msg_send![class!(NSURLSession), sessionWithConfiguration: config, delegate: delegate, delegateQueue: queue];
        let reply = Reply::pending();
        let handle = reply.handle();
        let complete = block2::RcBlock::new(move |data: *mut AnyObject, response: *mut AnyObject, error: *mut AnyObject| {
            let result = (|| {
                if !error.is_null() || response.is_null() { return Err(LoadError::Network); }
                let status: isize = msg_send![response, statusCode];
                if status != 200 { return Err(LoadError::Http(status)); }
                let len: usize = msg_send![data, length];
                let bytes: *const std::ffi::c_void = msg_send![data, bytes];
                if len == 0 || bytes.is_null() { return Err(LoadError::Response); }
                Ok(std::slice::from_raw_parts(bytes.cast::<u8>(), len).to_vec())
            })();
            handle.send(result);
        });
        let task: *mut AnyObject = msg_send![session, dataTaskWithRequest: request, completionHandler: &*complete];
        let _: () = msg_send![task, resume];
        let _: () = msg_send![session, finishTasksAndInvalidate];
        reply
    }
}
