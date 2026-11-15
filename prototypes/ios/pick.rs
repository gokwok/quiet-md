//! 浏览页的卡片横着滑一下就选上 / 取消（照文档页横滑选块，`touch.rs` 的 `swipe_scroll`）：卡片跟着手指挪一点，
//! 滑够了松手就切换它选没选中——还没在「选择」里时顺带进去。手指的平移在 GPUI 里是触摸来的滚动事件，方向一开始就锁死，
//! 竖着的照常滚动。哪张卡片在手指下面，看这一帧网格里登记的那几张（`set_cards`）和它们画出来的位置。
use std::cell::RefCell;
use std::time::Instant;

use gpui::*;

use crate::app::Workspace;

/// 卡片最多挪开多远（越拉越沉）；滑过多少算数；松手后弹回去要多久。和文档页的块一样。
const MAX: f32 = 56.;
const COMMIT: f32 = 44.;
const BACK: f32 = 0.28;

struct Swipe { id: String, dx: f32, released: Option<(Instant, f32)> }

thread_local! {
    static SWIPE: RefCell<Option<Swipe>> = const { RefCell::new(None) };
    static CARDS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static UNDECIDED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// 这一帧网格里有哪几张卡片（只有它们能被横滑选中）。
pub fn set_cards(ids: impl IntoIterator<Item = String>) { CARDS.with(|c| *c.borrow_mut() = ids.into_iter().collect()); }

/// 正在横滑（或刚松手、还在弹回去）的那张卡片。它要画在旁边的卡片上面，不然挪过去的那半边会被盖住。
pub fn swiping() -> Option<String> { SWIPE.with(|s| s.borrow().as_ref().map(|w| w.id.clone())).filter(|id| offset(id) != 0.) }
/// 按「横滑的那张最后画」排一下（其余的顺序不变）。
pub fn on_top<T>(items: &mut [T], id_of: impl Fn(&T) -> String) {
    if let Some(id) = swiping() { items.sort_by_key(|t| id_of(t) == id); }
}

/// `id` 这张卡片现在横着挪开了多远。
pub fn offset(id: &str) -> f32 {
    SWIPE.with(|s| {
        let s = s.borrow();
        let Some(w) = s.as_ref().filter(|w| w.id == id) else { return 0. };
        match w.released {
            None => w.dx.signum() * MAX * (1. - (-w.dx.abs() / MAX).exp()),
            Some((at, from)) => { let t = at.elapsed().as_secs_f32() / BACK; if t < 1. { from * (1. - t).powi(3) } else { 0. } }
        }
    })
}

/// 横滑时卡片浮起来（照 Craft）：一道柔和的影子、略放大一点，跟着挪开的距离从 0 到 1。
pub fn lift(id: &str) -> f32 { (offset(id).abs() / (MAX * 0.6)).min(1.) }
/// 浮起的影子：强度按 `lift`。
pub fn lift_shadow(t: f32) -> Vec<BoxShadow> {
    let c = |a: f32| crate::theme::rgbaf(0, 0, 0, a * t);
    vec![
        BoxShadow { color: c(0.08), offset: point(px(0.), px(2.)), blur_radius: px(8.), spread_radius: px(0.), inset: false },
        BoxShadow { color: c(0.18), offset: point(px(0.), px(14.)), blur_radius: px(36.), spread_radius: px(0.), inset: false },
    ]
}

/// 弹回去的那一小会儿要一直出帧。
pub fn tick() -> Option<AnyElement> {
    let moving = SWIPE.with(|s| s.borrow().as_ref().is_some_and(|w| w.released.is_none_or(|(at, _)| at.elapsed().as_secs_f32() < BACK)));
    moving.then(|| canvas(|_, window, _| window.request_animation_frame(), |_, _, _, _| {}).absolute().size_0().into_any_element())
}

fn begin(w: &Workspace, at: Point<Pixels>, dx: f32) {
    if w.row_menu.is_some() || super::menu::context_target().is_some() { return; }
    let bounds = w.home_card_bounds();
    let hit = CARDS.with(|c| c.borrow().iter().find(|id| bounds.get(*id).is_some_and(|b| b.contains(&at))).cloned());
    if let Some(id) = hit { SWIPE.with(|s| *s.borrow_mut() = Some(Swipe { id, dx, released: None })); }
}

/// 网格收到的滚动事件（只管触摸来的、横着的）。
pub fn scroll(w: &mut Workspace, ev: &ScrollEvent, cx: &mut Context<Workspace>) {
    if ev.source != ScrollSource::Touch { return; }
    if ev.touch_phase == TouchPhase::Started { SWIPE.with(|s| { let mut s = s.borrow_mut(); if s.as_ref().is_some_and(|w| w.released.is_none()) { *s = None; } }); }
    let ScrollDelta::Pixels(delta) = ev.delta else { return };
    let dx = f32::from(delta.x);
    let live = SWIPE.with(|s| s.borrow().as_ref().is_some_and(|w| w.released.is_none()));
    match ev.touch_phase {
        TouchPhase::Started => {
            UNDECIDED.with(|u| u.set(delta == Point::default()));
            if dx != 0. && delta.y == px(0.) { begin(w, ev.position, dx); cx.notify(); }
        }
        TouchPhase::Moved if UNDECIDED.with(|u| u.get()) && !live => {
            if delta == Point::default() { return; }
            UNDECIDED.with(|u| u.set(false));
            if dx != 0. && delta.y == px(0.) { begin(w, ev.position, dx); cx.notify(); }
        }
        TouchPhase::Moved => if live { SWIPE.with(|s| if let Some(sw) = s.borrow_mut().as_mut() { sw.dx += dx; }); cx.notify(); },
        TouchPhase::Ended | TouchPhase::Cancelled => {
            UNDECIDED.with(|u| u.set(false));
            if !live { return; }
            let Some((id, total)) = SWIPE.with(|s| s.borrow().as_ref().map(|sw| (sw.id.clone(), sw.dx + dx))) else { return };
            let from = offset(&id);
            SWIPE.with(|s| if let Some(sw) = s.borrow_mut().as_mut() { sw.dx = total; sw.released = Some((Instant::now(), from)); });
            if ev.touch_phase == TouchPhase::Ended && total.abs() >= COMMIT {
                super::glass::haptic();
                w.row_menu = None;
                w.toggle_check(&id);
                cx.notify();
            }
            cx.notify();
        }
    }
}
