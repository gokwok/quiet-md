//! 键盘上方的一条（照 Craft，录屏 2026-11-15 19:25）：左边「/」（弹出插入菜单）、「格式」（键盘换成格式面板，`format.rs`）、
//! 「⋮」（撤销、重做和这一块的动作），右边粘贴、收起键盘。和正文同一个底色、不画分隔线，不再挤一排图标。
//! 每个按钮只派发编辑器已有的动作，不另写编辑逻辑。
//! 「——」（录屏 2026-11-19 20:23）：工具条上方浮起一条玻璃，五种分隔线（三个点、点线、分页、细线、粗线），点一种插在光标处。

use gpui::{prelude::FluentBuilder, *};

use crate::app::{View, Workspace};
use crate::editor::keymap as k;
use crate::settings::tr;
use crate::theme::Theme;

pub const HEIGHT: f32 = 44.;

// 「——」的样式板开着：开的时候光标在哪一块。光标换了块（点到别处）就当它关了。
thread_local! { static OPEN: std::cell::Cell<Option<crate::model::BlockId>> = const { std::cell::Cell::new(None) }; }

/// 样式板上的五种，从左到右（照 Craft）：动作、三套图标（线、和纸胶带、涂鸦）、插哪一种。
/// 文档样式换成胶带或涂鸦时，同样五格换成那一套（录屏 2026-11-19 20:32）：胶带从小到大，涂鸦各是一种画法，分页不变。
const RULES: [(&str, [&str; 3], crate::model::Rule); 5] = [
    ("kb-rule-dots", ["rule-dots", "washi-1", "doodle-1"], crate::model::Rule::Dots),
    ("kb-rule-dotted", ["rule-dotted", "washi-2", "doodle-2"], crate::model::Rule::Dotted),
    ("kb-rule-page", ["rule-page", "rule-page", "rule-page"], crate::model::Rule::Page),
    ("kb-rule-line", ["rule-line", "washi-3", "doodle-3"], crate::model::Rule::Line),
    ("kb-rule-thick", ["rule-thick", "washi-4", "doodle-4"], crate::model::Rule::Thick),
];

fn button(id: &'static str, name: &'static str) -> Stateful<Div> {
    div().id(id).relative().flex_none().h(px(40.)).w(px(44.)).flex().items_center().justify_center().rounded(px(10.))
        .active(|s| s.bg(Theme::fill_pressed())).child(crate::ui::icon(name, 20., Theme::icon()))
}

pub fn render(ws: &Workspace, window: &Window, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    // 格式面板开着：键盘收下去了，这里让出面板那么高，选中的块留在面板上面看得见的地方。
    // 文档有背景时纸接着铺到底，面板浮在纸上（不然纸在面板上方断开，底色露出来、玻璃也透成底色）。
    if ws.view == View::Doc && super::format::is_open() {
        return Some(div().flex_none().w_full().h(px(super::format::reserved()))
            .when(super::style::banded(), |d| d.px(px(super::style::SIDE)).child(div().size_full().bg(crate::editor_theme::Theme::bg_body())))
            .into_any_element());
    }
    let editor = ws.editor.as_ref()?;
    let ed = editor.read(cx);
    let head = ed.core.sel.head.block;
    if OPEN.with(|r| r.get()).is_some_and(|b| b != head) { OPEN.with(|r| r.set(None)); }
    if ed.read_only || !ed.focus.is_focused(window) { OPEN.with(|r| r.set(None)); return None; }
    // 选中了文字：这一条让给选区工具栏（`Editor::toolbar` 升在同一个位置）。
    if ed.selection_bar() { return None; }
    // 选着单元格：让给表格工具栏。
    if ed.compact() && ed.core.cell_sel.is_some() { return None; }
    let in_table = ed.caret_cell().is_some();
    let in_title = ed.core.sel.head.block == crate::editor::core::TITLE;
    // 没有软键盘（接着硬键盘）：贴在屏幕底边，下面让出 Home 指示条。
    let foot = if super::keyboard_up(window) { px(0.) } else { super::safe(window).bottom };
    let slash = { let editor = editor.clone(); button("kb-slash", "slash-command").on_click(move |_, window, cx| editor.update(cx, |ed, cx| ed.type_slash(window, cx))) };
    let format = div().id("kb-format").flex_none().h(px(40.)).px(px(10.)).flex().items_center().rounded(px(10.)).active(|s| s.bg(Theme::fill_pressed()))
        .text_size(px(16.)).font_weight(FontWeight::SEMIBOLD).text_color(Theme::icon()).child(tr("格式", "Format"))
        .on_click(cx.listener(|w, _, window, cx| super::format::open(w, window, cx)));
    // 「⋮」：系统菜单。撤销 / 重做在最上面（工具条上不再单放），下面是光标所在这一块的动作。
    let mut items = vec![
        super::menu::Item::new("kb-undo", tr("撤销", "Undo"), "arrow.uturn.backward", 0),
        super::menu::Item::new("kb-redo", tr("重做", "Redo"), "arrow.uturn.forward", 0),
    ];
    if in_table { items.push(super::menu::Item::new("kb-cells", tr("选择单元格", "Select Cells"), "tablecells", 1)); }
    if !in_title {
        items.push(super::menu::Item::new("kb-copy", tr("拷贝块", "Copy Block"), "doc.on.doc", 2));
        items.push(super::menu::Item::new("kb-cut", tr("剪切块", "Cut Block"), "scissors", 2));
        items.push(super::menu::Item::new("kb-delete", tr("删除块", "Delete Block"), "trash", 2).danger());
    }
    let more = button("kb-more", "more-vertical").child(super::menu::button("kb-more", items));
    // 「——」：再点一下收起样式板。标题里和表格里不插分隔线。
    let rules_open = OPEN.with(|r| r.get()).is_some();
    // 照 Craft（截图 2026-11-19）：按钮就是一道细长的横线，摆在工具条正中；文档的分隔符换成胶带、涂鸦时换成那一种的样子。
    let ink = Theme::icon();
    let (set, tint) = match super::style::rule_glyphs() { super::style::RuleGlyphs::Line => (0, ink), // 胶带色和工具条底色太近（深色模式里的深胶带、浅色模式里的浅胶带）就用图标色。
        super::style::RuleGlyphs::Tape(c) => (1, if (crate::theme::dark() && c.l < 0.35) || (!crate::theme::dark() && c.l > 0.88) { ink } else { c }), super::style::RuleGlyphs::Doodle => (2, ink) };
    let rule = div().id("kb-rule").flex_none().h(px(40.)).w(px(64.)).flex().items_center().justify_center().rounded(px(10.))
        .active(|s| s.bg(Theme::fill_pressed())).when(rules_open, |d| d.bg(Theme::fill_pressed()))
        .map(|d| match set {
            0 => d.child(div().w(px(48.)).h(px(1.5)).rounded_full().bg(ink)),
            1 => d.child(crate::ui::icon("washi-3", 40., tint)),
            _ => d.child(crate::ui::icon("doodle-3", 40., ink)),
        })
        .on_click(cx.listener(move |_, _, _, cx| {
        OPEN.with(|r| r.set(if rules_open { None } else { Some(head) }));
        cx.notify();
    }));
    // 样式板：一条玻璃浮在工具条上方正中，等分五格，点了交给 `menu_action`。
    let palette = (rules_open && !in_title && !in_table).then(|| {
        // 胶带、涂鸦的图标是扁长的，画大一些才和 Craft 一样宽（最小的约 14pt，最大的约 33pt）。
        let glyph = move |r: &(&'static str, [&'static str; 3], crate::model::Rule)| if r.2 == crate::model::Rule::Page || set == 0 { (r.1[set], 24., ink) } else { (r.1[set], 36., tint) };
        div().absolute().left_0().right_0().bottom(px(HEIGHT + 6.) + foot).flex().justify_center().child(
            div().id("kb-rules").relative().w(px(360.)).max_w(px(f32::from(window.viewport_size().width) - 16.)).h(px(58.))
                .occlude().on_mouse_down(MouseButton::Left, |_, window, cx| { window.prevent_default(); cx.stop_propagation(); })
                .map(|d| if super::glass::supported() {
                    d.child(super::glass::tab_pane("kb-rules", RULES.iter().map(glyph).collect(), None, RULES.iter().map(|r| r.0).collect()))
                } else {
                    // 没有玻璃的系统：同样一条，用菜单的外壳画。
                    d.child(crate::ui::popover().rounded_full().size_full().flex().items_center().px(px(6.)).children(RULES.iter().map(|r| {
                        let (id, (icon, size, color)) = (r.0, glyph(r));
                        div().id(id).flex_1().h(px(46.)).flex().items_center().justify_center().rounded_full().active(|s| s.bg(Theme::fill_pressed()))
                            .child(crate::ui::icon(icon, size, color))
                            .on_click(cx.listener(move |w, _, window, cx| { menu_action(id, w, window, cx); }))
                    })))
                }))
    });
    let paste = button("kb-paste", "paste").on_click(|_, window, cx| window.dispatch_action(Box::new(k::Paste), cx));
    // 收起键盘：焦点交还窗口，再点正文时键盘回来。
    let hide = button("kb-hide", "keyboard-down").on_click(cx.listener(|w, _, window, cx| {
        window.focus(&w.focus, cx);
        cx.notify();
    }));
    Some(div().id("keybar").relative().flex_none().h(px(HEIGHT) + foot).pb(foot).w_full().px(px(8.)).flex().items_center()
        .bg(crate::editor_theme::Theme::bg_body())
        // 按在工具条上不抢编辑器的焦点。
        .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
        // 左右两组一样宽，中间的分隔线按钮正好在正中。
        .child(div().flex_1().min_w_0().flex().items_center().when(!in_title, |d| d.child(slash).child(format)).child(more))
        .when(!in_title && !in_table, |d| d.child(rule))
        .child(div().flex_1().min_w_0().flex().items_center().justify_end().child(paste).child(hide))
        .children(palette)
        .into_any_element())
}

/// 「⋮」菜单里选了一项。
pub fn menu_action(id: &str, w: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) -> bool {
    use crate::editor::view::BlockAct;
    if let Some(&(_, _, rule)) = RULES.iter().find(|r| r.0 == id) {
        OPEN.with(|r| r.set(None));
        super::glass::haptic();
        if let Some(editor) = w.editor.clone() {
            editor.update(cx, |ed, cx| { ed.core.insert_divider(rule); ed.changed(window, cx); });
        }
        cx.notify();
        return true;
    }
    if !matches!(id, "kb-undo" | "kb-redo" | "kb-cells" | "kb-copy" | "kb-cut" | "kb-delete") { return false; }
    let Some(editor) = w.editor.clone() else { return true };
    editor.update(cx, |ed, cx| {
        let block = ed.core.sel.head.block;
        match id {
            "kb-undo" | "kb-redo" => ed.history_step(id == "kb-redo", window, cx),
            "kb-cells" => ed.cell_select_start(cx),
            "kb-copy" => ed.block_menu_action(block, BlockAct::Copy, window, cx),
            "kb-cut" => ed.block_menu_action(block, BlockAct::Cut, window, cx),
            _ => ed.block_menu_action(block, BlockAct::Delete, window, cx),
        }
    });
    true
}
