//! 触屏的编辑菜单：剪切 / 复制 / 粘贴 / 全选。桌面靠快捷键，手机上没有键位，
//! 所以长按选词、双击选词或再点一下光标时，在选区上方浮出这一条。
//! 作为 `editor` 的子模块编译（见 port.patch 里的 `editor/mod.rs`）。

use gpui::{prelude::FluentBuilder, *};

use super::keymap as k;
use super::overlays::{at, rise_at};
use super::view::{BlockSub, Editor, Menu};
use fdl_core::editor::table_ops::{CellSel, CellSelKind};
use fdl_core::model::{BlockId, BlockKind};
use crate::editor_ui::{icon, menu};
use crate::feishu_theme::Theme;

const H: f32 = 38.;
/// 拖动标记的点击区。
const GRIP: f32 = 36.;
/// 手机上正文右边多留的一条道（`catalogue.rs` 加在右页边上）：拖动标记和选择圈居中放在页边和这条道合起来的 44pt 里。
pub const GUTTER: f32 = 24.;
/// 标记（36pt 见方）的左边离正文区右缘多远。
const MARK_X: f32 = 40.;

/// 正在拖块的那根手指是怎么开始的：按在右侧的拖动标记上，还是长按了块。长按时记着当时在不在编辑、按的是不是光标所在的那一块，
/// 还有编辑是不是已经因为拖动结束了。
#[derive(Clone, Copy, PartialEq)]
pub enum Lift { Grip, LongPress { editing: bool, caret: bool } }

/// 正文拉过头的回弹（GPUI 的列表到头就停，系统的滚动视图会跟着手指再走一段、松手弹回）。
/// `raw` 是手指在边缘之外又拖了多远（向下为正），显示出来的位移按系统的阻尼曲线打折；
/// 松手或惯性撞到边缘后是一段临界阻尼的弹簧 `anim`（起始时刻、起始位移、起始速度）。
#[derive(Default)]
pub struct Bounce { ok: bool, raw: f32, finger: bool, spent: bool, anim: Option<(std::time::Instant, f32, f32)>, last: Option<std::time::Instant> }
/// 弹簧的角频率（1/秒）、动画时长、阻尼系数（UIScrollView 的 0.55）、惯性撞边时带进来的速度上限（pt/秒）。
const BOUNCE_W: f32 = 14.;
const BOUNCE_END: f32 = 0.6;
const BOUNCE_C: f32 = 0.55;
const BOUNCE_V: f32 = 2000.;
fn rubber(raw: f32, dim: f32) -> f32 { raw.signum() * (1. - 1. / (raw.abs() * BOUNCE_C / dim + 1.)) * dim }
fn unrubber(x: f32, dim: f32) -> f32 { x.signum() * (dim / BOUNCE_C) * (1. / (1. - x.abs().min(dim * 0.9) / dim) - 1.) }

/// 右侧选择区拖动开始时的选区；每次移动从这个基准重算，折返即可缩短范围。
pub(super) struct PickDrag {
    base: Vec<BlockId>, anchor: BlockId, end: Option<BlockId>,
    moved: bool, y: Pixels, tick: std::time::Instant,
}

/// 在块上横着滑的一次手势：哪一行、手指横向走了多远；松手后记下时刻和当时挪开的距离，弹回原位。
pub struct Swipe { id: BlockId, dx: f32, released: Option<(std::time::Instant, f32)> }
/// 行最多跟着手指挪这么远；滑过 `SWIPE_COMMIT` 松手才算数；弹回用时。
const SWIPE_MAX: f32 = 44.;
const SWIPE_COMMIT: f32 = 28.;
const SWIPE_BACK: f32 = 0.24;

/// 整块的拖动排序。桌面靠悬停时块左边的手柄；手机上是两条路，落点、幽灵和自动滚动都用 `drag.rs` 原有的：
/// 编辑时光标所在的块右侧有一个拖动标记，点它选中整块（并升起块动作栏），按住拖动就排序；
/// 长按一个块直接把它提起来拖（照 Craft，编辑时也是：选词用双击，或在光标上再点一下出编辑菜单）。
impl Editor {
    // 在正文与浮层之间放命中区域。窗口级监听也必须遵守上层控件的遮挡。
    pub(super) fn phone_gesture_layer(&self, cx: &Context<Self>) -> impl IntoElement + use<> {
        let editor = cx.entity();
        canvas(|b, w, _| w.insert_hitbox(b, HitboxBehavior::Normal), move |_, body, window, cx| {
            if editor.read(cx).phone_panel_open() || crate::ios::overlay() {
                editor.read(cx).body_pan.set(false);
                return;
            }
            let ed = editor.clone();
            let hit = body.clone();
            window.on_mouse_event(move |ev: &LongPressEvent, phase, w, cx| {
                if phase == DispatchPhase::Bubble && (ev.phase != TouchPhase::Started || hit.is_hovered_at(ev.start_position,w)) {
                    ed.update(cx, |ed,cx| ed.touch_selection(ev,w,cx));
                }
            });
            let ed = editor.clone();
            let hit = body.clone();
            window.on_mouse_event(move |ev: &ScrollEvent, phase, w, cx| {
                if ev.source != ScrollSource::Touch { return; }
                let owner = &ed.read(cx).body_pan;
                if phase == DispatchPhase::Capture && ev.touch_phase == TouchPhase::Started {
                    owner.set(hit.is_hovered_at(ev.position,w));
                }
                if !owner.get() { return; }
                if phase == DispatchPhase::Capture && ed.update(cx, |ed,cx| ed.bounce_scroll(ev,cx)) {
                    cx.stop_propagation();
                }
                if phase == DispatchPhase::Bubble { ed.update(cx, |ed,cx| ed.swipe_scroll(ev,w,cx)); }
            });
            let ed = editor.clone();
            window.on_mouse_event(move |ev: &TouchDragEvent, phase, w, cx| {
                if phase == DispatchPhase::Bubble && (ev.phase != TouchPhase::Started || body.is_hovered_at(ev.start_position,w)) {
                    ed.update(cx, |ed,cx| {
                        if ev.phase != TouchPhase::Started && ed.range_drag_move(ev,w,cx) { cx.stop_propagation(); return; }
                        if !ed.pick_touch(ev,w,cx) { ed.grip_touch(ev,w,cx); }
                    });
                }
            });
        }).absolute().inset_0()
    }

    fn pick_row_at(&self, y: Pixels) -> Option<BlockId> {
        let rows = self.row_ids();
        self.block_bounds.borrow().iter().filter(|(id,_)| rows.contains(id))
            .min_by_key(|(_,b)| if y < b.top() { b.top()-y } else { (y-b.bottom()).max(px(0.)) })
            .map(|(id,_)| *id)
    }

    fn pick_touch(&mut self, ev: &TouchDragEvent, w: &mut Window, cx: &mut Context<Self>) -> bool {
        if ev.phase == TouchPhase::Started {
            self.pick_drag = None;
            let view = self.list.viewport_bounds();
            if !self.block_bar_on() || !view.contains(&ev.start_position) || ev.start_position.x < view.right()-px(44.) { return false; }
            let Some(id) = self.pick_row_at(ev.position.y) else { return false };
            let Some(bounds) = self.block_rect(id) else { return false };
            if ev.position.y < bounds.top()-px(4.) || ev.position.y > bounds.bottom()+px(4.) { return false; }
            self.pick_drag = Some(PickDrag { base:self.core.block_sel_ids(), anchor:id,end:None,moved:false,y:ev.position.y,tick:std::time::Instant::now() });
            w.prevent_default();
            return true;
        }
        let Some(drag) = self.pick_drag.as_mut() else { return false };
        drag.y = ev.position.y;
        drag.moved |= (drag.y-ev.start_position.y).abs() >= px(6.);
        if ev.phase == TouchPhase::Cancelled {
            let drag = self.pick_drag.take().unwrap();
            if drag.moved { self.core.extend_block_picks(&drag.base,None); }
        } else if drag.moved {
            self.update_pick_span();
            if ev.phase == TouchPhase::Ended { self.pick_drag = None; }
        } else if ev.phase == TouchPhase::Ended {
            let id = self.pick_drag.take().unwrap().anchor;
            self.toggle_block(id,w,cx);
        }
        cx.notify();
        true
    }

    fn pick_edges(&self) -> (Pixels,Pixels) {
        let view = self.list.viewport_bounds();
        (view.top()+px(crate::ios::chrome_top()), view.bottom().min(crate::ios::sheet_point().y-px(70.)))
    }

    fn update_pick_span(&mut self) {
        let Some(drag) = self.pick_drag.as_ref() else { return };
        let (top,bottom) = self.pick_edges();
        let Some(id) = self.pick_row_at(drag.y.max(top).min(bottom)) else { return };
        let drag = self.pick_drag.as_mut().unwrap();
        if drag.end != Some(id) {
            drag.end = Some(id);
            self.core.extend_block_picks(&drag.base,Some((drag.anchor,id)));
        }
    }

    pub(super) fn pick_autoscroll(&mut self, window: &mut Window) {
        if !self.block_bar_on() || self.phone_panel_open() || crate::ios::overlay() { self.pick_drag = None; return; }
        let (top,bottom) = self.pick_edges();
        let Some(drag) = self.pick_drag.as_mut().filter(|d| d.moved) else { return };
        let now = std::time::Instant::now();
        let dt = (now-drag.tick).as_secs_f32().min(0.05);
        drag.tick = now;
        let height = f32::from(bottom-top);
        if height <= 0. { return; }
        let band = (height * 0.15).min(96.);
        let depth = |distance: Pixels| (1. - f32::from(distance)/band).clamp(0.,1.);
        let edge = depth(bottom-drag.y) - depth(drag.y-top);
        if edge == 0. { return; }
        let speed = edge.signum() * (180. + height * 3.5 * edge.abs().powf(1.5));
        self.list.scroll_by(px(speed*dt)); self.update_pick_span(); window.request_animation_frame();
    }

    pub(super) fn phone_panel_open(&self) -> bool {
        self.compact() && (self.menu.is_some() || self.slash.is_some() || self.expanded_panel() || self.core.cell_sel.is_some())
    }
    pub(super) fn dismiss_phone_panel(&mut self, at: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(Menu::Block { sub: sub @ Some(BlockSub::Indent | BlockSub::Color), .. }) = &mut self.menu {
            *sub = None;
        } else if matches!(self.menu, Some(Menu::Link { .. })) {
            self.close_link_panel(window,cx); window.blur(cx); crate::ios::editor_surface::hide_keyboard();
        } else if matches!(self.menu, Some(Menu::Equation { .. })) {
            self.close_equation_editor(true,window,cx); window.blur(cx);
        } else if self.menu.is_some() { self.menu = None; }
        else if self.slash.is_some() { self.slash = None; }
        else if self.find.is_some() { self.close_find(window, cx); window.blur(cx); }
        else if self.cat_pinned { self.cat_pinned = false; }
        else if self.core.cell_sel.is_some() && !self.cell_tap(at, cx) {
            self.core.cell_sel = None; self.table_page = 0;
        }
        cx.notify();
    }

    /// 编辑时光标所在的块：右侧画拖动标记。
    fn grip_block(&self, window: &Window) -> Option<BlockId> {
        if !self.compact() || self.read_only || !self.focus.is_focused(window) || self.menu.is_some() || self.slash.is_some() { return None; }
        if self.core.in_block_mode() || self.core.cell_sel.is_some() || self.dragging || self.block_drag.is_some() { return None; }
        let id = self.core.sel.head.block;
        (id != super::core::TITLE && !matches!(self.core.kind_of(id), None | Some(BlockKind::Grid | BlockKind::Column { .. }))).then_some(id)
    }

    /// 拖动标记：三道很淡的短横，在块右边的页边里、对着块的竖直中间。只是画出来并记下位置，触摸在 `grip_touch` 里接。
    pub(super) fn touch_grip_mark(&self, window: &Window) -> Option<AnyElement> {
        self.touch_grip.set(None);
        let id = self.grip_block(window)?;
        let view = self.list.viewport_bounds();
        let origin = point(view.right() - px(MARK_X), self.mark_y(id)?);
        // 能点的范围比画出来的大一圈：左边到正文的边，右边到屏幕的边，上下各多 4pt。
        self.touch_grip.set(Some((id, Bounds::new(point(origin.x - px(4.), origin.y - px(4.)), size(px(MARK_X + 4.), px(GRIP + 8.))))));
        let bar = || div().w(px(14.)).h(px(1.5)).rounded_full().bg(Theme::text_disabled().opacity(0.6));
        let mark = div().size(px(GRIP)).flex().flex_col().items_center().justify_center().gap(px(3.)).child(bar()).child(bar()).child(bar());
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
        // 手指点一下总会挪几个点：离落点不到 10pt 就算点（`end_block_drag` 也是这个数，这么近不会挪动块）。
        let tapped = matches!(ev.phase, TouchPhase::Ended) && self.block_drag.as_ref().is_some_and(|d| { let m = ev.position - d.from; f32::from(m.x).hypot(f32::from(m.y)) <= 10. });
        let block = self.block_drag.as_ref().and_then(|d| d.ids.first().copied());
        self.lift_move(ev.phase, ev.position, window, cx);
        if tapped && let Some(block) = block { self.open_block_menu(block, false, window, cx); }
    }

    /// 长按：把手指下面的块提起来。只读、按在标题上、块提不起来时返回假，长按照旧选词。
    pub(super) fn lift_block(&mut self, at: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.compact() || self.read_only { return false; }
        let editing = self.focus.is_focused(window);
        // 手指下面最里层的块（引用、高亮块里的那一段，而不是整个容器）。
        let hit = self.block_bounds.borrow().iter().filter(|(id, b)| **id != super::core::TITLE && b.contains(&at))
            .min_by(|a, b| f32::from(a.1.size.height).total_cmp(&f32::from(b.1.size.height))).map(|(id, _)| *id);
        // 表格格子里的块不提起来（格子有自己的选择），长按照旧选词。
        let Some(id) = hit.filter(|id| self.core.cell_of(*id).is_none()) else { return false };
        self.press_handle(id, at);
        if self.block_drag.is_none() { return false; }
        window.prevent_default();
        crate::ios::take_long_press();
        self.touch_lift = Some(Lift::LongPress { editing, caret: editing && self.core.sel.head.block == id });
        // 提起来就是拖动的开始，不用等手指再挪 2pt。
        self.block_drag_move(point(at.x, at.y + px(3.)), true, window, cx);
        cx.notify();
        true
    }

    /// 拖着块的手指动了或抬起了。
    pub(super) fn lift_move(&mut self, phase: TouchPhase, at: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let near = |d: &super::drag::BlockDrag| { let m = at - d.from; f32::from(m.x).hypot(f32::from(m.y)) <= 10. };
        match phase {
            TouchPhase::Moved => {
                // 编辑中长按提起的块一拖动，编辑就结束（照 Craft）：键盘收起，松手后停在阅读状态。
                if let Some(Lift::LongPress { editing: true, .. }) = self.touch_lift && self.block_drag.as_ref().is_some_and(|d| !near(d)) {
                    self.touch_lift = Some(Lift::LongPress { editing: false, caret: false });
                    window.blur(cx);
                }
                self.block_drag_move(at, true, window, cx)
            }
            TouchPhase::Ended => {
                // 长按提起后没挪就松手：没在编辑时选中这一块（或原来选中的那几块），升起块动作栏，不抢焦点、键盘不出来；
                // 编辑时按的是光标所在的块，同样选中它并收起键盘；按的是别的块就什么都不变，接着编辑。
                let lift = self.touch_lift.take();
                let held = matches!(lift, Some(Lift::LongPress { .. })) && self.block_drag.as_ref().is_some_and(near);
                let one = self.block_drag.as_ref().filter(|d| d.sel.is_none()).and_then(|d| d.ids.first().copied());
                self.end_block_drag(Some(at), window, cx);
                match lift {
                    Some(Lift::LongPress { editing: true, caret: true }) if held => if let Some(id) = one { self.open_block_menu(id, false, window, cx); },
                    Some(Lift::LongPress { editing: false, .. }) if held => { if let Some(id) = one { self.core.select_block(id); } self.menu = None; cx.notify(); }
                    _ => {}
                }
            }
            TouchPhase::Cancelled => { self.touch_lift = None; self.block_drag = None; self.toolbar_hidden = false; cx.notify(); }
            _ => {}
        }
    }
}

/// 底部面板顶上的抓手（照 Craft）：按住它往下拖，面板跟着手走，松手时拖过 `PULL_COMMIT` 就收起这一层，不够就弹回。
/// 面板收起后的退场照旧由 `ios::shot` 做（从拖到的位置滑出屏幕）。同一时间只有一块在拖。
#[derive(Clone, Copy)]
struct Pull { key: &'static str, from: f32, dy: f32, back: Option<(std::time::Instant, f32)> }
thread_local! { static PULL: std::cell::Cell<Option<Pull>> = const { std::cell::Cell::new(None) }; }
/// 面板上沿往下这么高的一条接这个手势（就是抓手让出的那一条，不压第一行）；拖过多远松手算收起；弹回用时。
const PULL_ZONE: f32 = crate::ios::GRAB_ROOM;
const PULL_COMMIT: f32 = 80.;
const PULL_BACK: f32 = 0.25;

/// 这块面板现在被拖下去多少（弹回的路上按缓动算）。
fn pull_offset(key: &str) -> f32 {
    let Some(p) = PULL.with(|c| c.get()).filter(|p| p.key == key) else { return 0. };
    let Some((at, from)) = p.back else { return p.dy };
    let t = at.elapsed().as_secs_f32() / PULL_BACK;
    if t >= 1. { PULL.with(|c| c.set(None)); return 0. }
    from * (1. - t).powi(3)
}

impl Editor {
    /// 带抓手、能往下拖着收的底部面板。和 `rise_at` 一样按调用的那一处认面板（`track_caller` 一路传下去）。
    #[track_caller]
    pub(super) fn pull_sheet(&self, key: &'static str, rel: Point<Pixels>, el: impl IntoElement, cx: &mut Context<Self>) -> AnyElement {
        let dy = pull_offset(key);
        // 块菜单上面叠着子面板时，拖的只能是子面板：下面这层不接。
        let covered = key == "sheet-block" && matches!(self.menu, Some(Menu::Block { sub: Some(BlockSub::Indent | BlockSub::Color), .. }));
        let editor = cx.entity().downgrade();
        let grab = div().absolute().top(px(8.)).left_0().right_0().flex().justify_center()
            .child(div().w(px(36.)).h(px(5.)).rounded_full().bg(Theme::text_disabled().opacity(0.5)));
        let watch = canvas(|_, _, _| {}, move |bounds, _, window, _| {
            // 弹回的路上一直出帧。
            if PULL.with(|c| c.get()).is_some_and(|p| p.key == key && p.back.is_some()) { window.request_animation_frame(); }
            if covered { return; }
            let editor = editor.clone();
            window.on_mouse_event(move |ev: &TouchDragEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble { return; }
                let cur = PULL.with(|c| c.get());
                match ev.phase {
                    TouchPhase::Started => {
                        if cur.is_some_and(|p| p.back.is_none()) || !bounds.contains(&ev.start_position) || ev.start_position.y > bounds.top() + px(PULL_ZONE) { return; }
                        window.prevent_default();
                        PULL.with(|c| c.set(Some(Pull { key, from: f32::from(ev.start_position.y), dy: 0., back: None })));
                    }
                    _ if !cur.is_some_and(|p| p.key == key && p.back.is_none()) => {}
                    TouchPhase::Moved => {
                        let Some(mut p) = cur else { return };
                        // 往上拖只给一点阻力感，不让面板离开底边。
                        let raw = f32::from(ev.position.y) - p.from;
                        p.dy = if raw >= 0. { raw } else { -rubber(-raw, 40.) };
                        PULL.with(|c| c.set(Some(p)));
                        let _ = editor.update(cx, |_, cx| cx.notify());
                    }
                    _ => {
                        let Some(mut p) = cur else { return };
                        if ev.phase == TouchPhase::Ended && p.dy > PULL_COMMIT {
                            let _ = editor.update(cx, |e, cx| e.dismiss_sheet(key, cx));
                            // 收起后这一帧不再画它；留着位移到下一次拖。
                            PULL.with(|c| c.set(None));
                        } else {
                            p.back = Some((std::time::Instant::now(), p.dy));
                            PULL.with(|c| c.set(Some(p)));
                            let _ = editor.update(cx, |_, cx| cx.notify());
                        }
                    }
                }
            });
        }).absolute().inset_0();
        rise_at(key, rel, div().relative().top(px(dy)).child(el).child(grab).child(watch))
    }

    /// 往下拖收起一块面板：和点面板外面一样只收这一层——叠着的子面板回到块菜单，块菜单回到动作栏，别的直接收起。
    fn dismiss_sheet(&mut self, key: &str, cx: &mut Context<Self>) {
        match &mut self.menu {
            Some(Menu::Block { sub, .. }) if key == "sheet-block-sub" => *sub = None,
            _ => self.menu = None,
        }
        crate::ios::glass::haptic();
        cx.notify();
    }
}

// ---- 格式面板（`crate::ios::format`）和键盘上方那一条（`crate::ios::keybar`）要的几样。
impl Editor {
    /// 格式面板改的是哪一块：选中的第一块。
    pub fn format_block(&self) -> Option<BlockId> { self.core.block_sel_ids().first().copied() }
    /// 点「格式」：选中光标所在的块（标题行不算；已经选着块就照旧），收起「/」菜单，把它滚进看得见的地方。
    pub fn format_begin(&mut self) -> Option<BlockId> {
        if !self.core.in_block_mode() {
            let block = self.core.sel.head.block;
            if block == super::core::TITLE { return None; }
            self.core.select_block(block);
        }
        (self.slash, self.menu, self.touch_menu) = (None, None, false);
        let block = self.format_block()?;
        self.reveal = Some((super::render::Reveal::Block(block), 0));
        Some(block)
    }
    /// 「/」菜单里对这一块的动作：先删掉打的「/」和后面的字，再改光标所在的块。
    pub(super) fn slash_act(&mut self, act: &str, window: &mut Window, cx: &mut Context<Self>) {
        use fdl_core::model::{Align, ColorName};
        if let Some(s) = self.slash.take() {
            let end = self.core.sel.head.offset.max(s.start);
            self.core.sel = super::core::Selection { anchor: super::core::Pos::new(s.block, s.trigger), head: super::core::Pos::new(s.block, end) };
            self.core.delete_selection();
        }
        self.slash_reset();
        let block = self.core.sel.head.block;
        match act {
            "indent" => self.core.indent(),
            "outdent" => self.core.outdent(),
            "align-l" | "align-c" | "align-r" => self.core.set_align(&[block], match act { "align-c" => Align::Center, "align-r" => Align::Right, _ => Align::Left }),
            "tint" => {
                let has = self.core.block(block).is_some_and(|b| b.bg.is_some());
                self.core.set_block_bg(&[block], (!has).then(|| ColorName("light-gray".into())));
            }
            _ if act.starts_with("font-") => if !self.markdown { self.core.set_font(&[block], act.strip_prefix("font-").filter(|f| *f != "system").map(str::to_string)) },
            // 清除格式：整块回到正文的样子（粗体这些、颜色、对齐、底色、字体都去掉）。
            "clear" => self.core.apply_format(&[block], &super::core::BlockFormat { kind: None, align: Align::Left, bg: None, font: None, marks: Default::default() }),
            "delete" => { self.core.select_block(block); self.core.delete_selected_blocks(); }
            // 颜色：打开格式面板的颜色页（和「格式」里的彩虹圈同一页）。
            "color" => { self.changed(window, cx); return crate::ios::menu::dispatch(|w, window, cx| crate::ios::format::open_at(crate::ios::format::Page::Color, w, window, cx)); }
            _ => {}
        }
        self.changed(window, cx);
    }

    /// 「/」按钮：在光标处打一个「/」弹出菜单，和自己打的一样。前面紧挨着字时先空一格（「/」只在词首才算数）。
    pub fn type_slash(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let h = self.core.sel.head;
        if self.read_only || h.block == super::core::TITLE { return; }
        let glued = self.core.text_of(h.block).is_some_and(|t| t.text[..h.offset.min(t.text.len())].chars().last().is_some_and(|c| !c.is_whitespace()));
        // 分两下打：菜单只认刚打的正好是一个「/」。
        if glued { EntityInputHandler::replace_text_in_range(self, None, " ", window, cx); }
        EntityInputHandler::replace_text_in_range(self, None, "/", window, cx);
        window.notify_text_input_changed(&self.focus);
    }
}

/// 块选中。桌面靠手柄菜单和 Shift 点击；手机上选中整块后底部升起一条矮的动作栏，改类型再点「格式」展开块菜单，
/// 选中块右边有一颗圆点，点它取消选择，左右横滑切换选择（照 Craft，可以隔着选：`Core::toggle_pick`）。
impl Editor {
    pub(super) fn block_bar_on(&self) -> bool {
        self.compact() && self.core.in_block_mode() && self.slash.is_none() && !self.block_drag.as_ref().is_some_and(|d| d.started) && !crate::ios::format::is_open()
    }

    /// 块动作栏：格式 / 添加 / 剪切 / 复制 / 删除。选了多块时只留对多块有意义的后三项。
    pub(super) fn block_bar(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.block_bar_on() || self.menu.is_some() { return None; }
        let ids = self.core.block_sel_ids();
        let one = (ids.len() == 1).then(|| ids[0]);
        let tr = crate::settings::tr;
        let glass = crate::ios::glass::supported();
        let item = |id: &'static str, ic: &'static str, label: &'static str, tint: Hsla| {
            div().id(id).flex_1().min_w_0().h(px(58.)).flex().flex_col().items_center().justify_center().gap(px(3.)).rounded(px(16.)).relative()
                .when(!glass, |d| d.bg(tint.opacity(0.06)))
                .active(|s| s.bg(Theme::fill_pressed()))
                .when(glass, |d| d.child(crate::ios::glass::tile(id, (ic, 22., tint), label.into())))
                .when(!glass, |d| d.child(icon(ic, 22., tint)).child(div().text_size(px(11.)).line_height(px(14.)).text_color(tint).whitespace_nowrap().child(label)))
                .on_mouse_down(MouseButton::Left, move |_, window, cx| { cx.stop_propagation(); window.prevent_default(); if glass { crate::ios::glass::pulse(id); } })
        };
        let open = |sub: Option<BlockSub>| cx.listener(move |e: &mut Editor, _: &ClickEvent, _, cx| {
            let Some(block) = e.core.block_sel_ids().first().copied() else { return };
            e.menu = Some(Menu::Block { block, sub });
            e.size_pick = (0, 0);
            e.slash_scroll.set_offset(Point::default());
            cx.notify();
        });
        let act = |cut: bool, copy: bool| cx.listener(move |e: &mut Editor, _: &ClickEvent, window, cx| {
            if copy { e.copy_selection(cx); }
            if copy && !cut { return cx.notify(); }
            let n = e.core.block_sel_ids().len();
            e.core.delete_selected_blocks();
            e.changed(window, cx);
            crate::ios::blocks_removed(n, copy);
        });
        let kind = one.and_then(|id| self.core.kind_of(id));
        let crop = one.filter(|&id| self.can_phone_crop(id));
        let bar = div().when(!glass, |d| d.bg(crate::theme::raised(Theme::bg_body())).shadow(crate::theme::shadow_menu())).rounded(px(24.)).relative().when(glass, |d| d.child(crate::ios::glass::panel("block-bar-glass", 24.))).id("block-bar").occlude().w(crate::ios::menu_w(0.)).p(px(6.)).flex().items_center().gap(px(4.))
            .on_mouse_down(MouseButton::Left, |_, window, cx| { cx.stop_propagation(); window.prevent_default(); })
            .when_some(crop, |d,id| d.child(
                item("bb-crop","crop",tr("裁剪","Crop"),Theme::text()).on_click(cx.listener(move |ed: &mut Editor,_,w,cx| {
                    ed.start_crop(id,w,cx); if ed.cropping() { ed.core.clear_block_sel(); ed.set_caret(super::core::Pos::new(id,0),false); }
                }))))
            .when_some(one.filter(|_| matches!(kind, Some(BlockKind::Code { .. }))), |d,id| d.child(
                item("bb-code","code",tr("编辑代码","Edit code"),Theme::text()).on_click(cx.listener(move |ed: &mut Editor,_,w,cx| {
                    ed.core.clear_block_sel(); ed.set_caret(super::core::Pos::new(id,0),false); ed.open_lang_menu(id,w,cx);
                }))))
            .when_some(one.filter(|_| matches!(kind, Some(BlockKind::Table(_)))), |d,id| d.child(
                item("bb-table","table",tr("编辑表格","Edit table"),Theme::text()).on_click(cx.listener(move |ed: &mut Editor,_,_,cx| {
                    ed.core.select_cells(CellSel { table:id, anchor:(0,0), head:(0,0), kind:CellSelKind::Range });
                    ed.table_page=0; ed.table_panel_target.set(None); cx.notify();
                }))))
            .when(one.is_some() && crop.is_none() && !matches!(kind,Some(BlockKind::Table(_) | BlockKind::Code { .. })), |d| d
                // 「格式」：和键盘上方的「格式」是同一块面板（`crate::ios::format`）。
                .child(item("bb-format", "type", tr("格式", "Format"), Theme::text()).on_click(|_, _, _| crate::ios::menu::dispatch(|w, window, cx| crate::ios::format::open(w, window, cx)))))
            .when(one.is_some(), |d| d.child(item("bb-add", "square-plus", tr("添加", "Add"), Theme::text()).on_click(open(Some(BlockSub::Add(usize::MAX))))))
            .child(item("bb-cut", "scissors", tr("剪切", "Cut"), Theme::text()).on_click(act(true, true)))
            .child(item("bb-copy", "copy", tr("复制", "Copy"), Theme::text()).on_click(act(false, true)))
            .child(item("bb-del", "trash-2", tr("删除", "Delete"), Theme::danger()).on_click(act(true, false)));
        Some(rise_at("sheet-block-bar", self.rel(crate::ios::sheet_point()), bar))
    }

    /// 多选时每行右侧都有选择圈；实心表示已选，空心表示可以加选。
    pub(super) fn block_circles(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        if !self.block_bar_on() || self.menu.is_some() { return vec![]; }
        let picked: std::collections::HashSet<BlockId> = self.core.block_sel_ids().into_iter().collect();
        // 选中的容器里的块也算选中（圈是实心的）。
        let on = |id: BlockId| { let mut up = Some(id); while let Some(b) = up { if picked.contains(&b) { return true; } up = self.core.parent_of(b).map(|p| p.id); } false };
        let rows = self.row_ids();
        let vis: Vec<_> = self.block_bounds.borrow().keys().copied().filter(|id| rows.contains(id)).collect();
        let view = self.list.viewport_bounds();
        let mut out = vec![];
        // 圈在右边那条道里，对着块的竖直中间（`mark_y`）：表格格子里的块不给圈（选格子走表格自己的选择），
        // 和已有的圈挤在一起的也不给（引用、高亮块里的段落和容器自己的那一个，留容器的）。
        let mut ys = vec![];
        for id in vis.iter().copied() {
            let Some(y) = self.mark_y(id) else { continue };
            let selected = on(id);
            // 横着滑的那一行的圈跟着它一起挪。
            let origin = point(view.right() - px(MARK_X) + px(self.swipe_offset(id).unwrap_or(0.)), y);
            if ys.iter().any(|o: &Pixels| (*o - y).abs() < px(22.)) { continue; }
            ys.push(y);
            let dot = div().size(px(14.)).rounded_full().map(|d| if selected { d.bg(Theme::primary()) } else { d.border_1().border_color(Theme::text_disabled().opacity(0.55)) });
            let hit = div().id(("block-circle", id.0)).w(px(44.)).h(px(44.)).flex().items_center().justify_center().child(dot)
                .on_mouse_down(MouseButton::Left, cx.listener(move |ed: &mut Editor, _: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    window.prevent_default();
                    ed.toggle_block(id, window, cx);
                }));
            out.push(at(self.rel(origin - point(px(4.), px(4.))), Anchor::TopLeft, hit.with_animation(SharedString::from(format!("pick-dot-{}", id.0)), Animation::new(std::time::Duration::from_millis(180)), |d, t| d.opacity(0.5 + 0.5 * t))));
        }
        out
    }
}

impl Editor {
    /// 块右边的标记（拖动标记、选择圈，36pt 见方）的上边：对着块的竖直中间。块只露出一部分时对着露出来的那一段的中间，
    /// 露出来的不够放就没有。
    fn mark_y(&self, id: BlockId) -> Option<Pixels> {
        let rect = self.block_rect(id)?;
        let view = self.list.viewport_bounds();
        let (top, bottom) = (rect.top().max(view.top() + px(crate::ios::doc_top())), rect.bottom().min(view.bottom()));
        // 一行的块比标记矮：上下各让出几个点。
        let room = px(GRIP).min(rect.size.height);
        (bottom - top >= room).then(|| (top + bottom - px(GRIP)) / 2.)
    }

    /// 点圈、横着滑：切换这一块选没选中。没有选区时选中它（收起键盘，升起动作栏）；有选区时把它加进来或拿出去，
    /// 中间的块不动（照 Craft 的任意多选）；拿掉最后一块时退出块选中。
    pub(super) fn toggle_block(&mut self, id: BlockId, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        window.refresh();
        if !self.core.in_block_mode() { return self.open_block_menu(id, false, window, cx) }
        self.core.toggle_pick(id);
        cx.notify();
    }

    pub fn finish_block_selection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.core.clear_block_sel(); self.menu = None; self.swipe = None; self.touch_menu = false;
        window.refresh(); cx.notify();
    }

    /// 这一行现在横向挪开了多远（没在滑、已经弹回去了是 None）。
    pub(super) fn swipe_offset(&self, id: BlockId) -> Option<f32> {
        let w = self.swipe.as_ref().filter(|w| w.id == id)?;
        let Some((at, from)) = w.released else { return Some(w.dx.signum() * SWIPE_MAX * (1. - (-w.dx.abs() / SWIPE_MAX).exp())) };
        let t = at.elapsed().as_secs_f32() / SWIPE_BACK;
        (t < 1.).then(|| from * (1. - t).powi(3))
    }
    /// 弹回去的这一小会儿一直出帧（横滑的行和拉过头的正文都是）。
    pub(super) fn swipe_tick(&self) -> Option<AnyElement> {
        let swipe = self.swipe.as_ref().and_then(|w| w.released).is_some_and(|(at, _)| at.elapsed().as_secs_f32() < SWIPE_BACK);
        let bounce = self.bounce.anim.is_some_and(|(at, ..)| at.elapsed().as_secs_f32() < BOUNCE_END);
        if !swipe && !bounce { return None; }
        Some(canvas(|_, window, _| window.request_animation_frame(), |_, _, _, _| {}).absolute().size_0().into_any_element())
    }

    /// 正文的标题（连同它下面那行信息）是不是已经滚到悬浮的顶栏底下了：是的话顶栏中间显示小字标题。
    pub fn title_scrolled(&self) -> bool {
        if !self.compact() || !crate::ios::floating() { return false; }
        if self.list.logical_scroll_top().item_ix > 0 { return true; }
        self.list.bounds_for_item(0).is_some_and(|b| f32::from(b.bottom()) < f32::from(self.list.viewport_bounds().top()) + crate::ios::doc_top())
    }

    /// 底部的铅笔出不出：能编辑、没选着块或单元格、没开着菜单、目录、查找和大图（铅笔是原生的玻璃，会盖在这些上面）。
    pub fn can_append(&self) -> bool {
        self.compact() && !self.read_only && !self.core.in_block_mode() && self.core.cell_sel.is_none() && self.menu.is_none() && self.slash.is_none() && self.block_drag.is_none()
            && !self.cat_pinned && self.find.is_none() && !self.lightbox_open()
    }

    /// 铅笔：光标放到文末的空行（没有就新起一行），开始编辑，并滚到那里。
    pub fn append_line(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only { return; }
        window.focus(&self.focus, cx);
        (self.menu, self.slash, self.touch_menu) = (None, None, false);
        self.core.clear_block_sel();
        let id = self.core.trailing_line();
        self.set_caret(super::core::Pos::new(id, 0), false);
        // 滚到光标那一行（文末后面还有引用面板和留白，滚到列表末尾会把光标顶出去）。
        self.autoscroll = true;
        self.changed(window, cx);
    }

    /// 底部的撤销 / 重做：不用先进编辑。
    pub fn history_step(&mut self, redo: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only { return; }
        (self.menu, self.slash, self.touch_menu) = (None, None, false);
        if redo { self.core.redo(); } else { self.core.undo(); }
        self.changed(window, cx);
    }

    /// 底部的粘贴：没在编辑时贴到文末（新起一行）。
    pub fn paste_at_end(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only { return; }
        if !self.focus.is_focused(window) {
            let id = self.core.trailing_line();
            self.set_caret(super::core::Pos::new(id, 0), false);
            self.autoscroll = true;
        }
        self.paste(false, window, cx);
    }

    /// 底部的「+」：在文末新起一行，再插入文件、图片，或者打开插入菜单。
    pub fn append_with(&mut self, what: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_only { return; }
        self.append_line(window, cx);
        match what {
            "doc-add-file" => self.run_slash(super::menus::SlashKind::File, window, cx),
            "doc-add-image" => self.run_slash(super::menus::SlashKind::Image, window, cx),
            _ => { let block = self.core.sel.head.block; self.open_block_menu(block, true, window, cx); }
        }
    }

    /// 编辑时点正文下面的空白：退出编辑（收起键盘），不再把光标放到文末。没在编辑时照旧是从文末开始写。
    pub(super) fn tap_below(&mut self, at: Option<Point<Pixels>>, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.compact() || self.read_only || !(self.focus.is_focused(window) || self.core.in_block_mode()) { return false; }
        // `at`：点在列表最后一项（文末的点击区）之下才算；点击区自己接到的不用再判断。
        if let Some(at) = at && !self.list.bounds_for_item(self.items.len().saturating_sub(1)).is_some_and(|b| at.y > b.bottom()) { return false; }
        if self.core.in_block_mode() { self.finish_block_selection(window, cx); }
        (self.menu, self.slash, self.touch_menu) = (None, None, false);
        // 可聚焦的外层在按下时会自己把焦点拿回来：拦掉。
        window.prevent_default();
        cx.stop_propagation();
        window.blur(cx);
        cx.notify();
        true
    }

    /// 有背景时纸这张卡片的上沿（相对正文区的上沿）：跟着正文滚，也跟着回弹。
    pub fn paper_top(&self) -> f32 { crate::ios::doc_top() - f32::from(self.list.scroll_px_offset_for_scrollbar().y.abs()) + self.bounce_offset() }
    /// 正文整体挪开多少（拉过头时跟手，松手后弹回）。
    pub(super) fn bounce_offset(&self) -> f32 {
        match self.bounce.anim {
            Some((at, x0, v0)) => {
                let t = at.elapsed().as_secs_f32();
                if t >= BOUNCE_END { 0. } else { (x0 + (v0 + BOUNCE_W * x0) * t) * (-BOUNCE_W * t).exp() }
            }
            None if self.bounce.raw == 0. => 0.,
            None => rubber(self.bounce.raw, self.bounce_dim()),
        }
    }
    fn bounce_dim(&self) -> f32 { f32::from(self.list.viewport_bounds().size.height).max(1.) }

    /// 竖着的平移：到了头还往外拖的那部分记成回弹，不交给列表（返回 true 时这次事件到此为止）。
    /// 手指往回走时先把拉出去的收回来，收完了列表才接着滚，不然会两头一起动。
    /// 惯性撞到边缘只弹一小下，之后的惯性不再管；带着回弹松手后跟来的惯性也一样。
    pub(super) fn bounce_scroll(&mut self, ev: &ScrollEvent, cx: &mut Context<Self>) -> bool {
        if ev.source != ScrollSource::Touch { return false; }
        let ScrollDelta::Pixels(delta) = ev.delta else { return false };
        let (dy, now, dim) = (f32::from(delta.y), std::time::Instant::now(), self.bounce_dim());
        if ev.touch_phase == TouchPhase::Started {
            // 弹到一半又按住：从眼前的位置接着拖。
            let x = self.bounce_offset();
            let ok = self.compact() && !crate::ios::overlay() && self.menu.is_none() && self.slash.is_none() && self.block_drag.is_none() && !self.dragging
                && self.list.viewport_bounds().contains(&ev.position);
            self.bounce = Bounce { ok, raw: if ok { unrubber(x, dim) } else { 0. }, finger: ok, ..Default::default() };
        }
        // 这次手势不是落在正文上的（菜单、面板里的滚动）：连同它的惯性都不管。
        if !self.bounce.ok { return false; }
        let top = self.list.logical_scroll_top();
        let at_top = top.item_ix == 0 && top.offset_in_item <= px(0.);
        // 不够一屏的文档两头都算到了头，往哪边拖都弹。
        let at_bottom = self.list.is_scrolled_to_end().unwrap_or_else(|| self.list.max_offset_for_scrollbar().y <= px(0.5));
        let out = dy != 0. && ((dy > 0. && at_top) || (dy < 0. && at_bottom));
                let b = &mut self.bounce;
        let dt = b.last.replace(now).map_or(1. / 60., |t| now.duration_since(t).as_secs_f32()).clamp(1. / 120., 1. / 30.);
        if b.finger {
            let mut used = false;
            if b.raw != 0. && dy != 0. {
                let next = b.raw + dy;
                b.raw = if next * b.raw <= 0. { 0. } else { next };
                used = true;
            } else if out {
                b.raw = dy;
                used = true;
            }
            if matches!(ev.touch_phase, TouchPhase::Ended | TouchPhase::Cancelled) {
                b.finger = false;
                if b.raw != 0. { (b.anim, b.raw, b.spent, used) = (Some((now, rubber(b.raw, dim), 0.)), 0., true, true); }
            }
            if used { cx.notify(); }
            return used;
        }
        if b.spent { return true; }
        if out && b.anim.is_none() {
            (b.anim, b.spent) = (Some((now, 0., (dy / dt).clamp(-BOUNCE_V, BOUNCE_V))), true);
            cx.notify();
            return true;
        }
        false
    }

    /// 横着的平移开始了：落在能滑的一行上就认下这次手势。
    fn swipe_begin(&mut self, at: Point<Pixels>, dx: f32, cx: &mut Context<Self>) {
        if !self.compact() || self.read_only || self.selection_bar() || crate::ios::overlay() || self.slash.is_some() || self.block_drag.is_some() || self.dragging || self.core.cell_sel.is_some() { return; }
        if self.menu.is_some() || !self.list.viewport_bounds().contains(&at) { return; }
        // 手指下面的那一行：文本、标题、列表项各一行，引用、高亮块、分栏连同里面的算一行（`render::Item`）。
        let rows = self.row_ids();
        let hit = self.block_bounds.borrow().iter().filter(|(id, b)| rows.contains(id) && b.contains(&at))
            .min_by(|a, b| f32::from(a.1.size.height).total_cmp(&f32::from(b.1.size.height))).map(|(id, _)| *id);
        let Some(id) = hit else { return };
        if matches!(self.core.kind_of(id), None | Some(BlockKind::Table(_) | BlockKind::Code { .. })) || self.core.cell_of(id).is_some() { return; }
        self.swipe = Some(Swipe { id, dx, released: None });
        cx.notify();
    }

    /// 手指的平移（GPUI 把它变成滚动事件，方向一开始就锁死）。横着的、落在一行上的：这一行跟着手指，
    /// 滑够了松手就切换它选没选中。表格、代码块自己要横着滚，不接；松手后惯性带来的事件也不管。
    pub(super) fn swipe_scroll(&mut self, ev: &ScrollEvent, window: &mut Window, cx: &mut Context<Self>) {
        if ev.source != ScrollSource::Touch { return; }
        let ScrollDelta::Pixels(delta) = ev.delta else { return };
        let dx = f32::from(delta.x);
        // 上一次滑动松手后还带着惯性时，紧接着的触摸是「接住惯性」：开始时位移是 0，方向等第一次移动才定。
        thread_local! { static UNDECIDED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
        match ev.touch_phase {
            TouchPhase::Started => {
                self.swipe = None;
                UNDECIDED.with(|u| u.set(delta == Point::default()));
                if dx != 0. && delta.y == px(0.) { self.swipe_begin(ev.position, dx, cx); }
            }
            TouchPhase::Moved if UNDECIDED.with(|u| u.get()) && self.swipe.is_none() => {
                if delta == Point::default() { return; }
                UNDECIDED.with(|u| u.set(false));
                if dx != 0. && delta.y == px(0.) { self.swipe_begin(ev.position, dx, cx); }
            }
            TouchPhase::Moved => if let Some(w) = self.swipe.as_mut().filter(|w| w.released.is_none()) { w.dx += dx; cx.notify(); },
            TouchPhase::Ended | TouchPhase::Cancelled => {
                UNDECIDED.with(|u| u.set(false));
                let Some(w) = self.swipe.as_mut().filter(|w| w.released.is_none()) else { return };
                w.dx += dx;
                let (id, dx) = (w.id, w.dx);
                let from = self.swipe_offset(id).unwrap_or(0.);
                if let Some(w) = self.swipe.as_mut() { w.released = Some((std::time::Instant::now(), from)); }
                if ev.touch_phase == TouchPhase::Ended && dx.abs() >= SWIPE_COMMIT { crate::ios::glass::haptic(); self.toggle_block(id, window, cx); }
                cx.notify();
            }
        }
    }
}

/// 表格的触屏选择。桌面靠拖过单元格和表格边上的行 / 列条，手机上手指拖是滚动、边条太小：
/// 光标在表格里时键盘条上有「选择单元格」，进去后选区就是桌面的那个 `cell_sel`，底部升起原有的表格工具栏。
impl Editor {
    /// 光标所在的单元格（没在表格里是 None）。
    pub fn caret_cell(&self) -> Option<(BlockId, usize, usize)> { self.core.cell_of(self.core.sel.head.block) }

    /// 进入单元格选择：先选中光标所在的那一格。
    pub fn cell_select_start(&mut self, cx: &mut Context<Self>) {
        let Some((table, r, c)) = self.caret_cell() else { return };
        (self.menu, self.slash, self.touch_menu, self.toolbar_hidden) = (None, None, false, false);
        self.core.sel = super::core::Selection::caret(self.core.sel.head);
        self.table_page = 0; self.table_panel_target.set(None);
        self.core.select_cells(CellSel { table, anchor: (r, c), head: (r, c), kind: CellSelKind::Range });
        if crate::ios::editor_surface::enabled() { crate::ios::editor_surface::hide_keyboard(); }
        cx.notify();
    }

    /// 选着单元格时点了同一张表的另一格：把选区伸到那里（选行 / 列时就是伸到那一行 / 列）。
    pub(super) fn cell_tap(&mut self, p: Point<Pixels>, cx: &mut Context<Self>) -> bool {
        let Some(sel) = self.core.cell_sel else { return false };
        let Some((r, c)) = self.slot_at(sel.table, p, false) else { return false };
        let Some(head) = self.core.table(sel.table).map(|t| t.origin(r, c)) else { return false };
        self.menu = None;
        self.core.select_cells(CellSel { head, ..sel });
        cx.notify();
        true
    }

    /// 表格工具栏上面的一条：单元格 / 行 / 列，选了整行整列时可以在后面插入，右边是完成。
    pub(super) fn cell_mode_strip(&self, sel: CellSel, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let tr = crate::settings::tr;
        let seg = |id: &'static str, label: &'static str, kind: CellSelKind| {
            let on = sel.kind == kind;
            div().id(id).h(px(32.)).px(px(12.)).flex().items_center().rounded(px(8.)).text_size(px(14.)).whitespace_nowrap()
                .text_color(if on { Theme::primary() } else { Theme::text() }).when(on, |d| d.bg(Theme::fill_turninto_active()).font_weight(FontWeight::MEDIUM))
                .child(label)
                .on_mouse_down(MouseButton::Left, cx.listener(move |e: &mut Editor, _: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    window.prevent_default();
                    e.menu = None;
                    e.core.select_cells(CellSel { kind, ..sel });
                    cx.notify();
                }))
        };
        let plain = |id: &'static str, label: &'static str, tint: Hsla| div().id(id).h(px(32.)).px(px(12.)).flex().items_center().rounded(px(8.)).text_size(px(14.)).whitespace_nowrap()
            .text_color(tint).active(|s| s.bg(Theme::fill_pressed())).child(label);
        let whole = sel.kind != CellSelKind::Range;
        let cols = sel.kind == CellSelKind::Cols;
        div().w_full().h(px(44.)).px(px(8.)).flex().items_center().gap(px(2.))
            .child(seg("cm-cell", tr("单元格", "Cells"), CellSelKind::Range))
            .child(seg("cm-row", tr("行", "Rows"), CellSelKind::Rows))
            .child(seg("cm-col", tr("列", "Columns"), CellSelKind::Cols))
            .child(div().flex_1())
            .when(whole, |d| d.child(plain("cm-insert", if cols { tr("右侧插入", "Insert right") } else { tr("下方插入", "Insert below") }, Theme::text())
                .on_mouse_down(MouseButton::Left, cx.listener(move |e: &mut Editor, _: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    window.prevent_default();
                    let Some((table, r)) = e.core.cell_range() else { return };
                    e.menu = None;
                    e.table_insert_at(table, cols, if cols { r.right + 1 } else { r.bottom + 1 }, window, cx);
                }))))
            .child(plain("cm-done", tr("完成", "Done"), Theme::primary()).font_weight(FontWeight::MEDIUM)
                .on_mouse_down(MouseButton::Left, cx.listener(|e: &mut Editor, _: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    window.prevent_default();
                    e.menu = None;
                    e.core.cell_sel = None;
                    cx.notify();
                })))
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
            _ if id.starts_with(crate::ios::voice::PICK) => { self.touch_menu = false; return crate::ios::voice::start(self, id, window, cx); }
            // 系统收起了菜单。全选之后菜单要留着（换成带剪切、复制的那一条），那一次收起不算。
            _ => { if !self.touch_keep.is_some_and(|t| t.elapsed() < std::time::Duration::from_millis(600)) { self.touch_menu = false; cx.notify(); } return; }
        };
        window.dispatch_action(action, cx);
        self.touch_menu = id == "edit-all";
        self.touch_keep = (id == "edit-all").then(std::time::Instant::now);
        cx.notify();
    }

    /// 手机：选中了正文里的文字，键盘上方那一条换成选区工具栏（`keybar` 让位，`toolbar` 升起）。
    pub fn selection_bar(&self) -> bool {
        self.compact() && !self.core.sel.is_collapsed() && !self.core.in_block_mode() && self.core.cell_sel.is_none() && !self.toolbar_hidden && !self.dragging
            && self.slash.is_none() && self.core.ordered().0.block != super::core::TITLE
            && matches!(self.menu, None | Some(Menu::TurnInto | Menu::Align | Menu::Color | Menu::TbMore))
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
            if edit && !collapsed { items.push(("edit-cut", tr("剪切", "Cut").into(), vec![])); }
            if !collapsed { items.push(("edit-copy", tr("复制", "Copy").into(), vec![])); }
            if edit { items.push(("edit-paste", tr("粘贴", "Paste").into(), vec![])); }
            items.push(("edit-all", tr("全选", "Select All").into(), vec![]));
            if edit && !collapsed && let Some(item) = crate::ios::voice::menu_item() { items.push(item); }
            crate::ios::editmenu::present(rect, items, window);
            return None;
        }
        // 选区在同一行时居中，跨行时对着选区开头那一行的中间。
        let x = if sp.y == ep.y { (sp.x + ep.x) / 2. } else { (sp.x + view.right() - px(20.)) / 2. };
        let above = sp.y - px(H + 10.) >= view.top() + px(crate::ios::doc_top());
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

// ---- 「/」菜单（手机）：光标下面的一张小卡片，先分类，点进一类在同一张卡片里换内容；打了字就变成平铺的筛选结果。
// 从工具条或块菜单打开的添加面板不走这里，还是底部面板。

/// 分类照 Craft 的斜杠菜单（录屏 2026-11-15 19:25）拆细，只放已经做了的：插入用的是编辑器原有的条目（`SlashKind`），
/// 缩进、对齐、字体、颜色、清除格式、删除区块是对光标所在这一块的动作（`slash_act`）。一类只有一项时直接是那一项。
#[derive(Clone, Copy)]
enum Pick { K(super::menus::SlashKind), A(&'static str, &'static str, &'static str, &'static str) }
const SLASH_CATS: &[(&str, &str, &str, &[Pick])] = {
    use super::menus::SlashKind as K;
    use Pick::{A, K as I};
    &[
        ("列表", "Lists", "list", &[I(K::Bullet), I(K::Ordered), I(K::Todo)]),
        ("缩进", "Indent", "indent-increase", &[A("indent", "增加缩进", "Indent", "indent-increase"), A("outdent", "减少缩进", "Outdent", "indent-decrease")]),
        ("文本样式", "Text Styles", "type", &[I(K::Text), I(K::Heading(1)), I(K::Heading(2)), I(K::Heading(3)), I(K::Heading(4)), I(K::Quote), I(K::Code)]),
        ("装饰", "Decorations", "callout", &[I(K::Callout), A("tint", "块背景", "Block Background", "view-card")]),
        ("颜色", "Color", "palette", &[A("color", "颜色", "Color", "palette")]),
        ("对齐", "Alignment", "align-left", &[A("align-l", "左对齐", "Left", "align-left"), A("align-c", "居中", "Center", "align-center"), A("align-r", "右对齐", "Right", "align-right")]),
        ("字体", "Font", "baseline", &[A("font-system", "System", "System", "type"), A("font-serif", "Serif", "Serif", "type"), A("font-mono", "Mono", "Mono", "code"), A("font-rounded", "Round", "Round", "type")]),
        ("提醒我", "Remind Me", "reminder", &[I(K::Reminder)]),
        ("插入文件或媒体", "Insert File or Media", "image", &[I(K::Image), I(K::File)]),
        ("插入表格", "Insert Table", "table", &[I(K::Table)]),
        ("页面", "Page", "file-page", &[I(K::Page), I(K::Card)]),
        ("插入特殊语法片段", "Insert Special Syntax", "braces", &[I(K::Formula), I(K::Link), I(K::Button), I(K::Countdown)]),
        ("插入分隔符", "Insert Divider", "divider", &[I(K::Divider)]),
        ("分栏", "Columns", "columns-2", &[I(K::Grid)]),
        ("清除格式", "Clear Formatting", "swatch-none", &[A("clear", "清除格式", "Clear Formatting", "swatch-none")]),
        ("删除区块", "Delete Block", "trash-2", &[A("delete", "删除区块", "Delete Block", "trash-2")]),
    ]
};
const SLASH_CARD_W: f32 = 285.;
// 点进了哪一类：记着是哪一次「/」（块和触发的位置），换一次就从头来。
thread_local! { static SLASH_CAT: std::cell::Cell<Option<(BlockId, usize, usize)>> = const { std::cell::Cell::new(None) }; }

impl Editor {
    fn slash_cat(&self) -> Option<usize> {
        let s = self.slash.as_ref()?;
        SLASH_CAT.with(|c| c.get()).filter(|(b, t, _)| *b == s.block && *t == s.trigger).map(|(_, _, cat)| cat)
    }
    pub(super) fn slash_reset(&self) { SLASH_CAT.with(|c| c.set(None)); }
    /// 打字触发的「/」菜单这会儿是不是分着级的（还没打筛选的字）。
    pub(super) fn slash_leveled(&self) -> bool { self.compact() && self.slash.is_some() && self.slash_query().is_empty() }
    /// 退格：在某一类里先回到分类那一级（返回真，这一下不删字）。
    pub(super) fn slash_up(&mut self) -> bool {
        if !self.slash_leveled() || self.slash_cat().is_none() { return false; }
        SLASH_CAT.with(|c| c.set(None));
        self.slash_scroll.set_offset(Point::default());
        true
    }

    pub(super) fn phone_slash(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        use super::menus::SLASH_ITEMS;
        let s = self.slash.as_ref()?;
        let (p, lh) = self.point_of(super::core::Pos::new(s.block, s.trigger))?;
        // 任务、高亮块在桌面的两组里各有一份：这里每种只留一份。
        let mut items: Vec<usize> = vec![];
        for i in self.slash_results() { if !items.iter().any(|j| SLASH_ITEMS[*j].kind == SLASH_ITEMS[i].kind) { items.push(i); } }
        let leveled = self.slash_query().is_empty();
        let cat = self.slash_cat().filter(|_| leveled);
        let row = |id: ElementId| div().id(id).flex_none().h(px(44.)).mx(px(8.)).px(px(12.)).flex().items_center().rounded(px(14.)).active(|s| s.bg(Theme::fill_hover()));
        let label = |text: SharedString| div().ml(px(14.)).flex_1().min_w_0().text_size(px(16.)).line_height(px(22.)).text_color(Theme::text()).whitespace_nowrap().overflow_hidden().text_ellipsis().child(text);
        let (block, trigger) = (s.block, s.trigger);
        let mut list = div().id("slash-card-list").py(px(6.)).flex().flex_col().overflow_y_scroll().track_scroll(&self.slash_scroll);
        let mut n = 0usize;
        // 一行：图标、名字，点进一类的行尾一个小箭头。照 Craft：图标都是灰的，删除是红的。
        let pick_row = |id: ElementId, glyph: &'static str, text: SharedString, more: bool, danger: bool| {
            let tint = if danger { crate::feishu_theme::c(0xd83931) } else { Theme::icon() };
            row(id).child(icon(glyph, 19., tint)).child(label(text).when(danger, |d| d.text_color(tint))).when(more, |d| d.child(icon("chevron-right", 14., Theme::text_caption())))
        };
        let has = |k: super::menus::SlashKind| items.iter().any(|i| SLASH_ITEMS[*i].kind == k);
        let usable = |p: &Pick| match p { Pick::K(k) => has(*k), Pick::A(..) => true };
        let run = |p: Pick| cx.listener(move |e: &mut Editor, _: &MouseDownEvent, window, cx| {
            cx.stop_propagation();
            window.prevent_default();
            let composing = e.marked.take().is_some();
            match p { Pick::K(kind) => e.run_slash(kind, window, cx), Pick::A(act, ..) => e.slash_act(act, window, cx) }
            if composing { window.notify_text_input_changed(&e.focus); }
        });
        let pick_el = |ix: usize, p: Pick| match p {
            Pick::K(k) => { let it = SLASH_ITEMS.iter().find(|it| it.kind == k).unwrap(); pick_row(("slash-pick", ix).into(), it.icon, crate::settings::label(it.label).into(), false, false) }
            Pick::A(act, zh, en, glyph) => pick_row(("slash-pick", ix).into(), glyph, crate::settings::tr(zh, en).into(), false, act == "delete"),
        };
        if leveled && cat.is_none() {
            for (c, (zh, en, glyph, picks)) in SLASH_CATS.iter().enumerate() {
                let picks: Vec<Pick> = picks.iter().copied().filter(|p| usable(p)).collect();
                if picks.is_empty() { continue; }
                n += 1;
                // 只有一项的类直接就是那一项。
                if let [only] = picks.as_slice() {
                    let danger = matches!(only, Pick::A("delete", ..));
                    list = list.child(pick_row(("slash-cat", c).into(), glyph, crate::settings::tr(zh, en).into(), false, danger).on_mouse_down(MouseButton::Left, run(*only)));
                    continue;
                }
                list = list.child(pick_row(("slash-cat", c).into(), glyph, crate::settings::tr(zh, en).into(), true, false)
                    .on_mouse_down(MouseButton::Left, cx.listener(move |e: &mut Editor, _: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        window.prevent_default();
                        SLASH_CAT.with(|s| s.set(Some((block, trigger, c))));
                        e.slash_scroll.set_offset(Point::default());
                        cx.notify();
                    })));
            }
        } else if let Some(c) = cat {
            let (zh, en, _, picks) = SLASH_CATS[c];
            list = list.child(row("slash-back".into()).h(px(36.)).child(icon("chevron-left", 14., Theme::text_caption()))
                .child(div().ml(px(6.)).text_size(px(13.)).line_height(px(18.)).text_color(Theme::text_caption()).child(crate::settings::tr(zh, en)))
                .on_mouse_down(MouseButton::Left, cx.listener(|e: &mut Editor, _: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    window.prevent_default();
                    if e.slash_up() { cx.notify(); }
                })));
            for (ix, p) in picks.iter().copied().filter(|p| usable(p)).enumerate() {
                n += 1;
                list = list.child(pick_el(ix, p).on_mouse_down(MouseButton::Left, run(p)));
            }
        } else {
            for (ix, i) in items.iter().enumerate() {
                let it = &SLASH_ITEMS[*i];
                let kind = it.kind;
                n += 1;
                // 打了字时回车选的是第一条，给它一层底。
                list = list.child(pick_row(("slash-item", ix).into(), it.icon, crate::settings::label(it.label).into(), false, false).when(ix == 0, |d| d.bg(Theme::fill_hover()))
                    .on_mouse_down(MouseButton::Left, run(Pick::K(kind))));
            }
        }
        if n == 0 { return None; }
        let view = self.list.viewport_bounds();
        let vh = f32::from(crate::ios::visible_height(window));
        let top_edge = f32::from(view.top()) + 4. + crate::ios::doc_top();
        let (below, above) = (vh - f32::from(p.y + lh) - 14., f32::from(p.y) - top_edge - 6.);
        let down = below >= 180. || below >= above;
        let w = SLASH_CARD_W.min(f32::from(view.size.width) - 32.);
        let x = f32::from(p.x).clamp(f32::from(view.left()) + 16., f32::from(view.right()) - 16. - w);
        let card = menu().id("slash-card").occlude().w(px(w)).rounded(px(22.)).overflow_hidden().shadow(crate::theme::shadow_dialog())
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(list.max_h(px(if down { below } else { above }.clamp(120., 300.))));
        Some(if down { at(self.rel(point(px(x), p.y + lh + px(6.))), Anchor::TopLeft, card) } else { at(self.rel(point(px(x), p.y - px(6.))), Anchor::BottomLeft, card) })
    }
}

