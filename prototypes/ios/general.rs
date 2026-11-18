//! 设置的子页（手机，照 Craft：IMG_9169–9171、9177）：「外观」「语言」「安全」，以及各子页共用的卡片、开关和选项行。
//! 和第一级不同，子页里的行装在灰色圆角卡片里，卡片上面是粗体的组名。卡片的颜色是 Craft 实测（页面 #F9F8F7、卡片 #F2F2F1，约等于叠一层 3% 的黑），
//! 离屏幕两边 18、圆角 20。
use std::sync::atomic::Ordering;
use std::time::Duration;

use gpui::{prelude::FluentBuilder, *};

use crate::app::Workspace;
use crate::settings::{prefs, row_chevron, row_ink, tr};
use crate::theme::{self, craft, Theme};

use super::lock::{self, Kind};

pub(super) fn card_bg() -> Hsla { theme::ca(craft::N1000, 0.03) }

/// 一张卡片：行之间一道细线，从字的左边开始。
pub fn card(rows: Vec<AnyElement>) -> Div {
    let n = rows.len();
    div().mx(px(10.)).flex().flex_col().rounded(px(20.)).bg(card_bg()).overflow_hidden()
        .children(rows.into_iter().enumerate().map(move |(i, r)| div().w_full().flex().flex_col().child(r)
            .when(i + 1 < n, |d| d.child(div().ml(px(18.)).h(px(1.)).bg(crate::ui::list_line())))))
}

/// 一组：粗体的组名、灰色的说明，下面是卡片。
fn section(title: &'static str, desc: Option<&'static str>, body: impl IntoElement) -> Div {
    div().w_full().flex().flex_col()
        .child(div().px(px(26.)).pb(px(10.)).flex().flex_col().gap(px(4.))
            .child(div().text_size(px(18.)).line_height(px(24.)).font_weight(FontWeight::BOLD).text_color(Theme::text()).child(title))
            .children(desc.map(|d| div().text_size(px(14.)).line_height(px(19.)).text_color(Theme::text_caption()).child(d))))
        .child(body)
}

/// 卡片里的一行：左边标题（可带说明），右边控件。
fn line(title: impl Into<SharedString>, desc: Option<SharedString>, control: impl IntoElement) -> Div {
    div().w_full().min_h(px(56.)).px(px(18.)).py(px(12.)).flex().items_center().gap(px(16.))
        .child(div().flex_1().min_w_0().flex().flex_col().gap(px(2.))
            .child(div().text_size(px(17.)).line_height(px(22.)).text_color(row_ink()).child(title.into()))
            .children(desc.map(|d| div().text_size(px(14.)).line_height(px(19.)).text_color(Theme::text_caption()).child(d))))
        .child(div().flex_none().flex().items_center().child(control))
}

/// 设置子页的一组（`settings::group` 在手机上走这里）：有组名就写在卡片上面。
pub fn titled(title: Option<&'static str>, rows: Vec<AnyElement>) -> Div {
    match title { Some(t) => section(t, None, card(rows)), None => div().w_full().child(card(rows)) }
}

/// 选项行（照 Craft，IMG_9177）：上面一行灰色小字是名称，下面是当前的值，右边一对上下箭头；
/// 点一下就地弹出系统菜单，当前那项打勾。菜单项的 id 在 `app::menu_action` 里接。
pub fn choice(key: &'static str, label: &'static str, value: impl Into<SharedString>, items: Vec<super::menu::Item>) -> AnyElement {
    div().relative().w_full().px(px(18.)).py(px(12.)).flex().items_center().gap(px(12.))
        .child(div().flex_1().min_w_0().flex().flex_col().gap(px(2.))
            .child(div().text_size(px(14.)).line_height(px(19.)).text_color(Theme::text_caption()).child(label))
            .child(div().truncate().text_size(px(17.)).line_height(px(22.)).text_color(row_ink()).child(value.into())))
        .child(crate::ui::icon("chevron-up-down", 16., row_chevron()))
        .child(super::menu::button(key, items))
        .into_any_element()
}

/// 卡片里一行动作：整行可点，字是强调色（危险的是红色）。
pub(super) fn action(id: &'static str, label: &'static str, danger: bool, on: impl Fn(&mut Workspace, &mut Context<Workspace>) + 'static, cx: &mut Context<Workspace>) -> AnyElement {
    div().id(id).w_full().h(px(52.)).px(px(18.)).flex().items_center().active(|d| d.bg(Theme::fill_pressed()))
        .text_size(px(17.)).text_color(if danger { Theme::danger() } else { Theme::primary() }).child(label)
        .on_click(cx.listener(move |w, _, _, cx| { super::glass::haptic(); on(w, cx); })).into_any_element()
}

/// 主题菜单项的 id：按 `themes::themes()` 的顺序，最多这么多个。
pub const THEME_PICKS: [&str; 16] = ["theme-pick-0", "theme-pick-1", "theme-pick-2", "theme-pick-3", "theme-pick-4", "theme-pick-5", "theme-pick-6", "theme-pick-7",
    "theme-pick-8", "theme-pick-9", "theme-pick-10", "theme-pick-11", "theme-pick-12", "theme-pick-13", "theme-pick-14", "theme-pick-15"];

/// iOS 26 的开关（照 Craft 里的实测：轨道 63×28，白色的胶囊滑块 38×24）。
pub fn toggle(id: impl Into<ElementId>, on: bool) -> Stateful<Div> {
    div().id(id).flex_none().w(px(63.)).h(px(28.)).p(px(2.)).rounded_full().flex().items_center()
        .when(on, |d| d.bg(Theme::primary()).justify_end())
        .when(!on, |d| d.bg(theme::ca(craft::N1000, 0.12)).justify_start())
        .child(div().w(px(38.)).h(px(24.)).rounded_full().bg(gpui::white()).shadow(theme::shadow_button()))
}

/// 外观里三种模式的小手机：屏幕上一张卡片、几行字、三个待办。
fn phone(dark: bool) -> Div {
    let (frame, screen, sheet, ink, line_c, box_c) = if dark {
        (theme::rgbaf(0x1c, 0x1c, 0x1e, 1.), theme::rgbaf(0x1c, 0x1c, 0x1e, 1.), theme::rgbaf(0x2c, 0x2c, 0x2e, 1.), theme::rgbaf(0xf2, 0xf2, 0xf2, 1.), theme::rgbaf(0x55, 0x55, 0x58, 1.), theme::rgbaf(0x5a, 0x5a, 0x5e, 1.))
    } else {
        (theme::rgbaf(0xe4, 0xe4, 0xe6, 1.), theme::rgbaf(0xff, 0xff, 0xff, 1.), theme::rgbaf(0xf7, 0xf7, 0xf8, 1.), theme::rgbaf(0x33, 0x33, 0x36, 1.), theme::rgbaf(0xdd, 0xdd, 0xe0, 1.), theme::rgbaf(0xc4, 0xc4, 0xc8, 1.))
    };
    // 勾上的待办跟着强调色。
    let blue = Theme::primary();
    let bar = |w: f32, c: Hsla| div().w(px(w)).h(px(3.)).rounded_full().bg(c);
    let task = |done: bool, w: f32| div().flex().items_center().gap(px(4.))
        .child(div().size(px(6.)).rounded(px(1.5)).when(done, |d| d.bg(blue)).when(!done, |d| d.border_1().border_color(box_c)))
        .child(bar(w, line_c));
    div().w(px(70.)).h(px(136.)).p(px(3.)).rounded(px(16.)).bg(frame)
        .child(div().size_full().rounded(px(13.)).bg(screen).flex().flex_col().items_center().pt(px(5.))
            .child(div().w(px(20.)).h(px(6.)).rounded_full().bg(theme::rgbaf(0, 0, 0, 1.)))
            .child(div().mt(px(6.)).w(px(54.)).flex_1().mb(px(5.)).p(px(6.)).rounded(px(8.)).bg(sheet).flex().flex_col().gap(px(4.))
                .child(bar(18., ink)).child(div().h(px(2.)))
                .child(bar(40., line_c)).child(bar(40., line_c)).child(bar(28., line_c)).child(div().h(px(3.)))
                .child(task(true, 20.)).child(task(true, 26.)).child(task(false, 16.))))
}

/// 外观：模式（三台小手机）→ 界面强调色 → 动画与触感 → 应用图标 → 文档主题。照 Craft 不放小字说明。
/// `preview` 是当前主题的一页样张（`settings.rs` 画）。
pub fn appearance(_w: &Workspace, preview: AnyElement, cx: &mut Context<Workspace>) -> AnyElement {
    let p = prefs();
    let at = match p.appearance.as_str() { "light" => 1, "dark" => 2, _ => 0 };
    let modes = [tr("系统", "System"), tr("浅色", "Light"), tr("深色", "Dark")];
    let picker = div().mx(px(10.)).px(px(8.)).py(px(20.)).rounded(px(20.)).bg(card_bg()).flex().justify_around()
        .children(modes.iter().enumerate().map(|(i, label)| {
            let on = i == at;
            let shot: AnyElement = match i {
                // 跟随系统：左半浅、右半深。
                0 => div().relative().child(phone(false)).child(div().absolute().top_0().left(px(35.)).w(px(35.)).h_full().overflow_hidden().child(div().ml(px(-35.)).child(phone(true)))).into_any_element(),
                1 => phone(false).into_any_element(),
                _ => phone(true).into_any_element(),
            };
            div().id(("appearance-mode", i)).flex().flex_col().items_center().gap(px(10.))
                .child(div().p(px(3.)).rounded(px(21.)).border_2().border_color(if on { Theme::primary() } else { gpui::transparent_black() }).child(shot))
                .child(div().text_size(px(17.)).line_height(px(22.)).font_weight(if on { FontWeight::SEMIBOLD } else { FontWeight::NORMAL }).text_color(if on { Theme::primary() } else { row_ink() }).child(*label))
                .on_click(cx.listener(move |w, _, _, cx| w.set_pref(|p| p.appearance = ["", "light", "dark"][i].into(), cx)))
        }));
    // 界面强调色：两排各五个圆（照 Craft：直径 48），选中的外面一圈同色、中间隔一道底色。
    // 不靠换行：宽的手机一行放得下六个，会排成 6 + 4。照 Craft 固定两排、每排五个，两端对齐。
    let swatch = |i: usize, key: &'static str, hex: u32, cx: &mut Context<Workspace>| {
        let on = p.accent == key;
        let fill = theme::c(hex);
        div().id(("accent", i)).size(px(56.)).p(px(3.)).rounded_full().border_2().border_color(if on { fill } else { gpui::transparent_black() })
            .child(div().size_full().rounded_full().bg(fill))
            .on_click(cx.listener(move |w, _, _, cx| { if prefs().accent != key { w.set_pref(|p| p.accent = key.into(), cx); } }))
    };
    let mut accents = div().mx(px(10.)).px(px(18.)).py(px(20.)).rounded(px(20.)).bg(card_bg()).flex().flex_col().gap(px(14.));
    for (r, list) in theme::ACCENTS.chunks(5).enumerate() {
        accents = accents.child(div().w_full().flex().justify_between()
            .children(list.iter().enumerate().map(|(j, &(key, hex, _, _))| swatch(r * 5 + j, key, hex, cx))));
    }
    let motion = p.motion == "high";
    let feel = card(vec![
        line(tr("丰富动画", "Rich animation"), None,
            toggle("pref-motion", motion).on_click(cx.listener(move |w, _, _, cx| w.set_pref(|p| p.motion = if motion { String::new() } else { "high".into() }, cx)))).into_any_element(),
        line(tr("启用触感", "Haptics"), None,
            toggle("pref-haptics", p.haptics).on_click(cx.listener(|w, _, _, cx| w.set_pref(|p| p.haptics = !p.haptics, cx)))).into_any_element(),
    ]);
    let icons = div().mx(px(10.)).py(px(20.)).rounded(px(20.)).bg(card_bg()).child(super::appicon::grid(cx));
    // 文档主题：一行就地选，样张，下面几行动作（编辑 CSS、复制为新主题；改过的能恢复，自己的能删除）。
    if !crate::themes::all_loaded() { crate::themes::load(&p.theme_id(), true, || cx.text_system().all_font_names()); }
    let current = p.theme_id();
    let (items, name, customized, own) = {
        let list = crate::themes::themes();
        let items: Vec<super::menu::Item> = list.iter().zip(THEME_PICKS).map(|(t, id)| super::menu::Item::new(id, t.name().to_string(), "", 0).checked(t.id == current)).collect();
        let sel = list.iter().find(|t| t.id == current).or_else(|| list.first());
        (items, sel.map_or_else(String::new, |t| t.name().to_string()), sel.is_some_and(|t| t.customized()), sel.is_some_and(|t| t.bundled.is_none()))
    };
    let (edit, dup, rm) = (current.clone(), current.clone(), current.clone());
    let mut rows = vec![
        choice("theme-choice", tr("主题", "Theme"), name, items),
        preview,
        action("theme-edit", tr("编辑 CSS", "Edit CSS"), false, move |w, cx| w.edit_theme(&edit, cx), cx),
        action("theme-duplicate", tr("复制为新主题", "Duplicate Theme"), false, move |w, cx| w.duplicate_theme(&dup, cx), cx),
    ];
    if customized || own {
        rows.push(action("theme-remove", if customized { tr("恢复默认", "Restore Default") } else { tr("删除主题", "Delete Theme") }, own && !customized, move |w, cx| w.remove_theme(&rm, cx), cx));
    }
    div().w_full().flex().flex_col().gap(px(32.)).pt(px(8.))
        .child(picker)
        .child(section(tr("界面强调色", "Accent Colour"), None, accents))
        .child(section(tr("动画与触感", "Motion & Haptics"), None, feel))
        .child(section(tr("应用图标", "App Icon"), None, icons))
        .child(section(tr("文档主题", "Document Theme"), None, card(rows)))
        .into_any_element()
}

/// 语言：照 Craft 的选项行，点一下就地选。
pub fn language(_w: &Workspace, _cx: &mut Context<Workspace>) -> AnyElement {
    let english = prefs().english;
    let items = vec![
        super::menu::Item::new("pref-lang-zh", "简体中文", "", 0).checked(!english),
        super::menu::Item::new("pref-lang-en", "English", "", 0).checked(english),
    ];
    div().w_full().pt(px(8.)).child(card(vec![choice("language-choice", tr("界面语言", "Language"), if english { "English" } else { "简体中文" }, items)])).into_any_element()
}

/// 安全：用面容 ID 锁定留白。打开和关闭都先验证一次，免得别人拿到手机就把锁关了。
pub fn security(_w: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let k = lock::kind();
    let on = prefs().lock;
    let how = lock::method(k);
    let title = crate::localized_format!("使用{}锁定留白", "Lock Margin with {}", how);
    let desc: SharedString = if k == Kind::Unavailable {
        tr("这台设备还没有设置面容 ID 或密码，请先在系统设置里设置。", "Set up Face ID or a passcode in the Settings app first.").into()
    } else {
        crate::localized_format!("打开留白时需要{}解锁。你的{}数据只由系统处理，留白不会读取或保存。", "Margin asks for {} when it opens. Your {} data is handled by iOS; Margin never reads or stores it.", how, how).into()
    };
    let switch = toggle("pref-lock", on).when(k == Kind::Unavailable && !on, |d| d.opacity(0.4))
        .when(k != Kind::Unavailable || on, |d| d.on_click(cx.listener(move |_, _, _, cx| {
            let reason = if on { tr("关闭锁定", "Turn off the lock") } else { tr("开启锁定", "Turn on the lock") };
            let result = lock::evaluate(reason);
            cx.spawn(async move |this, cx| {
                let ok = loop {
                    cx.background_executor().timer(Duration::from_millis(60)).await;
                    match result.load(Ordering::SeqCst) { 0 => continue, v => break v == 1 }
                };
                if !ok { return }
                let _ = this.update(cx, |w, cx| { lock::mark_unlocked(); w.set_pref(|p| p.lock = !on, cx); });
            }).detach();
        })));
    div().w_full().pt(px(8.)).child(card(vec![line(title, Some(desc), switch).into_any_element()])).into_any_element()
}
