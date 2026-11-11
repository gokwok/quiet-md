//! 触屏的编辑菜单：剪切 / 复制 / 粘贴 / 全选。桌面靠快捷键，手机上没有键位，
//! 所以长按选词、双击选词或再点一下光标时，在选区上方浮出这一条。
//! 作为 `editor` 的子模块编译（见 port.patch 里的 `editor/mod.rs`）。

use gpui::{prelude::FluentBuilder, *};

use super::keymap as k;
use super::overlays::at;
use super::view::Editor;
use fdl_core::model::{BlockId, BlockKind};
use crate::editor_ui::menu;
use crate::feishu_theme::Theme;

const H: f32 = 38.;
/// 拖动标记的点击区。
const GRIP: f32 = 36.;

/// 正在拖块的那根手指是怎么开始的：按在右侧的拖动标记上，还是没在编辑时长按了块。
#[derive(Clone, Copy, PartialEq)]
pub enum Lift { Grip, LongPress }

/// 整块的拖动排序。桌面靠悬停时块左边的手柄；手机上是两条路，落点、幽灵和自动滚动都用 `drag.rs` 原有的：
/// 编辑时光标所在的块右侧有一个拖动标记，点它选中整块（并升起块菜单），按住拖动就排序；
/// 没在编辑时，长按一个块直接把它提起来拖。
impl Editor {
    /// 编辑时光标所在的块：右侧画拖动标记。
    fn grip_block(&self, window: &Window) -> Option<BlockId> {
        if !self.compact() || self.read_only || !self.focus.is_focused(window) || self.menu.is_some() || self.slash.is_some() { return None; }
        if self.core.in_block_mode() || self.core.cell_sel.is_some() || self.dragging || self.block_drag.is_some() { return None; }
        let id = self.core.sel.head.block;
        (id != super::core::TITLE && !matches!(self.core.kind_of(id), None | Some(BlockKind::Grid | BlockKind::Column { .. }))).then_some(id)
    }

    /// 拖动标记：两道短横，贴在块第一行的右边（页边里）。只是画出来并记下位置，触摸在 `grip_touch` 里接。
    pub(super) fn touch_grip_mark(&self, window: &Window) -> Option<AnyElement> {
        self.touch_grip.set(None);
        let id = self.grip_block(window)?;
        let (first, lh) = self.point_of(super::core::Pos::new(id, 0)).or_else(|| self.block_rect(id).map(|b| (b.origin, px(26.))))?;
        let view = self.list.viewport_bounds();
        let origin = point(view.right() - px(GRIP - 8.), first.y + (lh - px(GRIP)) / 2.);
        if origin.y < view.top() + px(crate::ios::chrome_top()) || origin.y + px(GRIP) > view.bottom() { return None; }
        self.touch_grip.set(Some((id, Bounds::new(origin, size(px(GRIP), px(GRIP))))));
        let bar = || div().w(px(14.)).h(px(2.)).rounded_full().bg(Theme::text_disabled());
        let mark = div().size(px(GRIP)).flex().flex_col().items_center().justify_center().gap(px(3.)).child(bar()).child(bar());
        Some(at(self.rel(origin), Anchor::TopLeft, mark))
    }

    /// 手指落在拖动标记上：这次触摸归它（不滚动、不放光标）。没怎么动就抬手是点：选中整块，升起块菜单。
    pub(super) fn grip_touch(&mut self, ev: &TouchDragEvent, window: &mut Window, cx: &mut Context<Self>) {
        if ev.phase == TouchPhase::Started {
            let Some((id, bounds)) = self.touch_grip.get() else { return };
            if crate::ios::overlay() || !bounds.contains(&ev.start_position) { return; }
            window.prevent_default();
            self.touch_menu = false;
            self.press_handle(id, ev.start_position);
            if self.block_drag.is_some() { self.touch_lift = Some(Lift::Grip); }
            return;
        }
        if self.touch_lift != Some(Lift::Grip) { return; }
        let tapped = matches!(ev.phase, TouchPhase::Ended) && self.block_drag.as_ref().is_some_and(|d| !d.started);
        let block = self.block_drag.as_ref().and_then(|d| d.ids.first().copied());
        self.lift_move(ev.phase, ev.position, window, cx);
        if tapped && let Some(block) = block { self.open_block_menu(block, false, window, cx); }
    }

    /// 没在编辑时长按：把手指下面的块提起来。编辑时返回假，长按照旧选词。
    pub(super) fn lift_block(&mut self, at: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.compact() || self.read_only || self.focus.is_focused(window) { return false; }
        // 手指下面最里层的块（引用、高亮块里的那一段，而不是整个容器）。
        let hit = self.block_bounds.borrow().iter().filter(|(id, b)| **id != super::core::TITLE && b.contains(&at))
            .min_by(|a, b| f32::from(a.1.size.height).total_cmp(&f32::from(b.1.size.height))).map(|(id, _)| *id);
        let Some(id) = hit else { return false };
        self.press_handle(id, at);
        if self.block_drag.is_none() { return false; }
        window.prevent_default();
        crate::ios::take_long_press();
        self.touch_lift = Some(Lift::LongPress);
        // 提起来就是拖动的开始，不用等手指再挪 2pt。
        self.block_drag_move(point(at.x, at.y + px(3.)), true, window, cx);
        cx.notify();
        true
    }

    /// 拖着块的手指动了或抬起了。
    pub(super) fn lift_move(&mut self, phase: TouchPhase, at: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        match phase {
            TouchPhase::Moved => self.block_drag_move(at, true, window, cx),
            TouchPhase::Ended => { self.touch_lift = None; self.end_block_drag(Some(at), window, cx); }
            TouchPhase::Cancelled => { self.touch_lift = None; self.block_drag = None; self.toolbar_hidden = false; cx.notify(); }
            _ => {}
        }
    }
}

impl Editor {
    /// 系统的编辑菜单里点了一项（或系统把菜单收起了）。
    pub fn touch_menu_action(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let action: Box<dyn Action> = match id {
            "edit-cut" => Box::new(k::Cut),
            "edit-copy" => Box::new(k::Copy),
            "edit-paste" => Box::new(k::Paste),
            "edit-all" => Box::new(k::SelectAll),
            // 「格式」换成原有的选区工具栏。
            "edit-format" => { self.touch_menu = false; self.toolbar_hidden = false; return cx.notify(); }
            // 系统收起了菜单。全选之后菜单要留着（换成带剪切、复制的那一条），那一次收起不算。
            _ => { if !self.touch_keep.is_some_and(|t| t.elapsed() < std::time::Duration::from_millis(600)) { self.touch_menu = false; cx.notify(); } return; }
        };
        window.dispatch_action(action, cx);
        self.touch_menu = id == "edit-all";
        self.touch_keep = (id == "edit-all").then(std::time::Instant::now);
        cx.notify();
    }

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

    /// 编辑菜单：有系统的就交给系统（`ios::editmenu`），这里什么都不画；没有时才自己画一条。
    pub(super) fn touch_callout(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let shown = self.touch_menu && !self.dragging && self.menu.is_none() && self.slash.is_none() && self.focus.is_focused(window)
            && !self.core.in_block_mode() && self.core.cell_sel.is_none() && self.block_drag.is_none();
        let native = crate::ios::editmenu::supported();
        if !shown { if native { crate::ios::editmenu::dismiss(); } return None; }
        let collapsed = self.core.sel.is_collapsed();
        let (s, e) = self.core.ordered();
        let (Some((sp, slh)), Some((ep, elh))) = (self.point_of(s), self.point_of(e)) else { if native { crate::ios::editmenu::dismiss(); } return None; };
        let view = self.list.viewport_bounds();
        let tr = crate::settings::tr;
        let edit = !self.read_only;
        if native {
            // 菜单对着选区：同一行就是选中的那一段，跨行时是从第一行到最后一行的整宽。
            let (left, right) = if sp.y == ep.y { (sp.x, ep.x.max(sp.x + px(2.))) } else { (view.left() + px(20.), view.right() - px(20.)) };
            let rect = Bounds::from_corners(point(left, sp.y), point(right, ep.y + elh.max(slh)));
            let mut items: Vec<crate::ios::editmenu::Item> = vec![];
            if edit && !collapsed { items.push(("edit-cut", tr("剪切", "Cut").into())); }
            if !collapsed { items.push(("edit-copy", tr("复制", "Copy").into())); }
            if edit { items.push(("edit-paste", tr("粘贴", "Paste").into())); }
            items.push(("edit-all", tr("全选", "Select All").into()));
            if edit && !collapsed && s.block != super::core::TITLE { items.push(("edit-format", tr("格式", "Format").into())); }
            crate::ios::editmenu::present(rect, items, window);
            return None;
        }
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
