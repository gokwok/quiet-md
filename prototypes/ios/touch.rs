//! 触屏的编辑菜单：剪切 / 复制 / 粘贴 / 全选。桌面靠快捷键，手机上没有键位，
//! 所以长按选词、双击选词或再点一下光标时，在选区上方浮出这一条。
//! 作为 `editor` 的子模块编译（见 port.patch 里的 `editor/mod.rs`）。

use gpui::{prelude::FluentBuilder, *};

use super::keymap as k;
use super::overlays::at;
use super::view::Editor;
use crate::editor_ui::menu;
use crate::feishu_theme::Theme;

const H: f32 = 38.;

impl Editor {
    /// 按下前的状态：松手时用来判断是不是「点在原来的光标上」。
    pub(super) fn touch_press(&mut self, window: &Window) {
        let idle = self.focus.is_focused(window) && self.core.sel.is_collapsed() && self.menu.is_none() && self.slash.is_none();
        self.touch_before = idle.then_some((self.core.sel.head, self.touch_menu));
        self.touch_menu = false;
    }

    /// 松手：双击选了词就出菜单；点在原光标上则开合菜单。
    pub(super) fn touch_release(&mut self) {
        let before = self.touch_before.take();
        if self.core.in_block_mode() || self.core.cell_sel.is_some() { return; }
        self.touch_menu = if !self.core.sel.is_collapsed() { true } else {
            matches!(before, Some((pos, shown)) if pos == self.core.sel.head && !shown)
        };
    }

    pub(super) fn touch_callout(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.touch_menu || self.dragging || self.menu.is_some() || self.slash.is_some() || !self.focus.is_focused(window) { return None; }
        if self.core.in_block_mode() || self.core.cell_sel.is_some() { return None; }
        let collapsed = self.core.sel.is_collapsed();
        let (s, e) = self.core.ordered();
        let (sp, _) = self.point_of(s)?;
        let (ep, elh) = self.point_of(e)?;
        let view = self.list.viewport_bounds();
        // 选区在同一行时居中，跨行时对着选区开头那一行的中间。
        let x = if sp.y == ep.y { (sp.x + ep.x) / 2. } else { (sp.x + view.right() - px(20.)) / 2. };
        let above = sp.y - px(H + 10.) >= view.top() + px(crate::ios::chrome_top());
        let (pos, anchor) = if above { (point(x, sp.y - px(8.)), Anchor::BottomCenter) } else { (point(x, ep.y + elh + px(8.)), Anchor::TopCenter) };
        let item = |id: &'static str, label: &'static str, action: fn() -> Box<dyn Action>, keep: bool| {
            div().id(id).h(px(H - 8.)).px(px(11.)).flex().items_center().rounded(px(7.)).text_size(px(14.)).whitespace_nowrap()
                .active(|s| s.bg(Theme::fill_hover_strong())).child(crate::settings::label(label))
                .on_mouse_down(MouseButton::Left, |_, window, cx| { cx.stop_propagation(); window.prevent_default(); })
                .on_click(cx.listener(move |ed, _, window, cx| {
                    window.dispatch_action(action(), cx);
                    ed.touch_menu = keep;
                    cx.notify();
                }))
        };
        let tr = crate::settings::tr;
        let edit = !self.read_only;
        let bar = menu().id("touch-callout").occlude().h(px(H)).px(px(4.)).flex().items_center()
            .on_mouse_down(MouseButton::Left, |_, window, cx| { cx.stop_propagation(); window.prevent_default(); })
            .when(edit && !collapsed, |d| d.child(item("tc-cut", tr("剪切", "Cut"), || Box::new(k::Cut), false)))
            .when(!collapsed, |d| d.child(item("tc-copy", tr("复制", "Copy"), || Box::new(k::Copy), false)))
            .when(edit, |d| d.child(item("tc-paste", tr("粘贴", "Paste"), || Box::new(k::Paste), false)))
            .child(item("tc-all", tr("全选", "Select All"), || Box::new(k::SelectAll), true))
            // 「格式」换成原有的选区工具栏。
            .when(edit && !collapsed && s.block != super::core::TITLE, |d| d
                .child(div().flex_none().w(px(1.)).h(px(18.)).mx(px(3.)).bg(Theme::line_border()))
                .child(div().id("tc-format").h(px(H - 8.)).px(px(11.)).flex().items_center().rounded(px(7.)).text_size(px(14.)).whitespace_nowrap()
                    .active(|s| s.bg(Theme::fill_hover_strong())).child(crate::settings::label(tr("格式", "Format")))
                    .on_mouse_down(MouseButton::Left, |_, window, cx| { cx.stop_propagation(); window.prevent_default(); })
                    .on_click(cx.listener(|ed, _, _, cx| { ed.touch_menu = false; ed.toolbar_hidden = false; cx.notify(); }))));
        Some(at(self.rel(pos), anchor, bar))
    }
}
