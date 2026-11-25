//! 出帧的快慢。GPUI 靠一个 `CADisplayLink` 出帧，建好后没有声明帧率范围，ProMotion 的屏上系统只给它 60Hz
//! （`Info.plist` 的 `CADisableMinimumFrameDurationOnPhone` 只是允许更高，还得自己要）。滚动、打字、跟手都是 GPUI 一帧帧画的，
//! 所以换掉它场景代理的 `renderFrame:`：连着出帧时向这个 link 要 80–120Hz，闲下来还给系统。不改 GPUI 的源码。

use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
use objc2::{msg_send, sel};
use std::cell::Cell;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// 两帧隔得比这短，算是连着画（一闪一闪的光标不算）。
const RUN: Duration = Duration::from_millis(60);
/// 最后一帧之后还留多久：手指停一下、惯性收尾时不来回切。
const LINGER: Duration = Duration::from_millis(400);

static ORIGINAL: OnceLock<Imp> = OnceLock::new();
thread_local! {
    static LAST: Cell<Option<Instant>> = const { Cell::new(None) };
    static BUSY: Cell<Option<Instant>> = const { Cell::new(None) };
    // 上一次设过的 link 和快慢：没变就不再设。
    static SET: Cell<(*const AnyObject, bool)> = const { Cell::new((std::ptr::null(), false)) };
}

/// 要在场景连上（GPUI 建 link）之前换好：`ios::run` 的启动回调里调用。
pub fn install() {
    let Some(method) = AnyClass::get(c"GPUIIosSceneDelegate").and_then(|c| c.instance_method(sel!(renderFrame:))) else { return };
    if ORIGINAL.get().is_some() { return; }
    // 签名和原来的一样：`-(void)renderFrame:(CADisplayLink *)link`。
    let _ = ORIGINAL.set(method.implementation());
    unsafe { method.set_implementation(std::mem::transmute::<extern "C-unwind" fn(&AnyObject, Sel, &AnyObject), Imp>(render)); }
}
extern "C-unwind" fn render(this: &AnyObject, cmd: Sel, link: &AnyObject) {
    let busy = BUSY.with(|b| b.get()).is_some_and(|t| t.elapsed() < LINGER);
    if SET.with(|s| s.replace((link, busy))) != (link as *const AnyObject, busy) {
        // 三个 0 是系统的默认（`CAFrameRateRangeDefault`）。
        let rate = if busy { super::shot::FAST } else { super::shot::FrameRate { minimum: 0., maximum: 0., preferred: 0. } };
        unsafe { let _: () = msg_send![link, setPreferredFrameRateRange: rate]; }
    }
    if let Some(original) = ORIGINAL.get() {
        let original = unsafe { std::mem::transmute::<Imp, unsafe extern "C-unwind" fn(&AnyObject, Sel, &AnyObject)>(*original) };
        unsafe { original(this, cmd, link) }
    }
}
/// 画了一帧（`shot::attach` 每帧调用）。
pub fn drew() {
    let now = Instant::now();
    if LAST.with(|l| l.replace(Some(now))).is_some_and(|t| now - t < RUN) { BUSY.with(|b| b.set(Some(now))); }
}
