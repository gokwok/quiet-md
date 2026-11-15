//! 键盘上方的一条（照 Craft，录屏 2026-11-15 19:25）：左边「/」（弹出插入菜单）、「格式」（键盘换成格式面板，`format.rs`）、
//! 「⋮」（撤销、重做和这一块的动作），右边粘贴、收起键盘。和正文同一个底色、不画分隔线，不再挤一排图标。
//! 每个按钮只派发编辑器已有的动作，不另写编辑逻辑。

use gpui::{prelude::FluentBuilder, *};

use crate::app::{View, Workspace};
use crate::editor::keymap as k;
use crate::settings::tr;
use crate::theme::Theme;

pub const HEIGHT: f32 = 44.;

fn button(id: &'static str, name: &'static str) -> Stateful<Div> {
    div().id(id).relative().flex_none().h(px(40.)).w(px(44.)).flex().items_center().justify_center().rounded(px(10.))
        .active(|s| s.bg(Theme::fill_pressed())).child(crate::ui::icon(name, 20., Theme::icon()))
}

pub fn render(ws: &Workspace, window: &Window, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    // 格式面板开着：键盘收下去了，这里让出面板那么高，选中的块留在面板上面看得见的地方。
    if ws.view == View::Doc && super::format::is_open() {
        return Some(div().flex_none().w_full().h(px(super::format::reserved())).into_any_element());
    }
    let editor = ws.editor.as_ref()?;
    let ed = editor.read(cx);
    if ed.read_only || !ed.focus.is_focused(window) { return None; }
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
    let paste = button("kb-paste", "paste").on_click(|_, window, cx| window.dispatch_action(Box::new(k::Paste), cx));
    // 收起键盘：焦点交还窗口，再点正文时键盘回来。
    let hide = button("kb-hide", "keyboard-down").on_click(cx.listener(|w, _, window, cx| {
        window.focus(&w.focus, cx);
        cx.notify();
    }));
    Some(div().id("keybar").flex_none().h(px(HEIGHT) + foot).pb(foot).w_full().px(px(8.)).flex().items_center()
        .bg(crate::editor_theme::Theme::bg_body())
        // 按在工具条上不抢编辑器的焦点。
        .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
        .when(!in_title, |d| d.child(slash).child(format))
        .child(more)
        .child(div().flex_1())
        .child(paste)
        .child(hide)
        .into_any_element())
}

/// 「⋮」菜单里选了一项。
pub fn menu_action(id: &str, w: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) -> bool {
    use crate::editor::view::BlockAct;
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
