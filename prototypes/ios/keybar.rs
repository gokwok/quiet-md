//! 键盘上方的工具条：触屏没有悬停手柄和快捷键，常用的编辑动作放在这里。
//! 每个按钮只派发编辑器已有的动作，不另写编辑逻辑。

use gpui::{prelude::FluentBuilder, *};

use crate::app::Workspace;
use crate::editor::keymap as k;
use crate::editor_ui::icon;
use crate::theme::Theme;

pub const HEIGHT: f32 = 44.;

fn button(id: &'static str, name: &'static str) -> Stateful<Div> {
    div().id(id).flex_none().w(px(42.)).h(px(36.)).flex().items_center().justify_center().rounded(px(8.))
        .active(|s| s.bg(Theme::fill_pressed())).child(icon(name, 20., Theme::icon()))
}

pub fn render(ws: &Workspace, window: &Window, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    let editor = ws.editor.as_ref()?;
    let ed = editor.read(cx);
    if !super::keyboard_up(window) || ed.read_only || !ed.focus.is_focused(window) { return None; }
    let in_title = ed.core.sel.head.block == crate::editor::core::TITLE;
    let act = |id: &'static str, name: &'static str, action: fn() -> Box<dyn Action>| {
        button(id, name).on_click(move |_, window, cx| window.dispatch_action(action(), cx))
    };
    let menu = |id: &'static str, name: &'static str, add: bool| {
        let editor = editor.clone();
        button(id, name).on_click(move |_, window, cx| editor.update(cx, |ed, cx| {
            let block = ed.core.sel.head.block;
            ed.open_block_menu(block, add, window, cx);
        }))
    };
    let sep = || div().flex_none().w(px(1.)).h(px(20.)).mx(px(4.)).bg(Theme::line_divider());
    let tools = div().id("keybar-tools").flex_1().min_w_0().h_full().flex().items_center().px(px(6.)).overflow_x_scroll().restrict_scroll_to_axis()
        .child(act("kb-undo", "undo", || Box::new(k::Undo)))
        .child(act("kb-redo", "redo", || Box::new(k::Redo)))
        .when(!in_title, |d| d
            .child(sep())
            .child(menu("kb-add", "plus", true))
            .child(menu("kb-block", "grip-vertical", false))
            .child(sep())
            .child(act("kb-h1", "heading-1", || Box::new(k::TurnH1)))
            .child(act("kb-h2", "heading-2", || Box::new(k::TurnH2)))
            .child(act("kb-bullet", "list", || Box::new(k::TurnBullet)))
            .child(act("kb-ordered", "list-ordered", || Box::new(k::TurnOrdered)))
            .child(act("kb-todo", "list-todo", || Box::new(k::TurnTodo)))
            .child(sep())
            .child(act("kb-outdent", "indent-decrease", || Box::new(k::ShiftTab)))
            .child(act("kb-indent", "indent-increase", || Box::new(k::Tab)))
            .child(sep())
            .child(act("kb-bold", "bold", || Box::new(k::Bold)))
            .child(act("kb-italic", "italic", || Box::new(k::Italic)))
            .child(act("kb-strike", "strikethrough", || Box::new(k::Strike)))
            .child(act("kb-code", "code", || Box::new(k::InlineCode)))
            .child(act("kb-link", "link", || Box::new(k::Link))));
    // 收起键盘：焦点交还窗口，再点正文时键盘回来。
    let hide = button("kb-hide", "chevron-down").mx(px(4.)).on_click(cx.listener(|w, _, window, cx| {
        window.focus(&w.focus, cx);
        cx.notify();
    }));
    Some(div().id("keybar").flex_none().h(px(HEIGHT)).w_full().flex().items_center()
        .bg(Theme::bg_body()).border_t_1().border_color(Theme::line_divider())
        // 按在工具条上不抢编辑器的焦点。
        .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
        .child(tools)
        .child(div().flex_none().w(px(1.)).h(px(20.)).bg(Theme::line_divider()))
        .child(hide)
        .into_any_element())
}
