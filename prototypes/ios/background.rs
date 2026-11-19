//! 切到后台也接着做完的活（现在只有生成配音）：用户点了才开始、主要在等网络。
//!
//! 有活时一直拿着一个 UIKit 后台任务（`beginBackgroundTask`，切走后大约还有 30 秒，各版本都有）。iOS 26 起再叠一个
//! `BGContinuedProcessingTask`：系统在灵动岛、锁屏上显示进度，用户可以停掉，不受 30 秒限制。几件活合用一个任务，
//! 进度加在一起；全做完就交还，之后再有活另提交一个（标识每次换一个后缀：同一个标识注册两次系统会结束应用）。
//! 系统没给（模拟器、旧系统、系统忙）就只靠 UIKit 那个。
//!
//! 系统停掉这个任务（用户点了停止、或系统收回，分不出是哪种）时，还没做完的活都按停止处理，由各自的 `halt` 收尾；
//! 这一批之后不再提交。状态都在主线程上：系统的回调先排回主线程再碰。
use std::{cell::{Cell, RefCell}, ffi::{CStr, c_void}, ptr::null_mut, time::Instant};

use block2::RcBlock;
use objc2::{class, msg_send, runtime::{AnyClass, AnyObject}};

use crate::settings::tr;

#[link(name = "BackgroundTasks", kind = "framework")]
unsafe extern "C" { static BGTaskSchedulerErrorDomain: *const AnyObject; }
#[link(name = "Foundation", kind = "framework")]
unsafe extern "C" { static NSRunLoopCommonModes: *const AnyObject; }
unsafe extern "C" {
    static _dispatch_main_q: c_void;
    fn dispatch_async_f(queue: *const c_void, context: *mut c_void, work: extern "C" fn(*mut c_void));
}

/// 一步的进度格数。一步里只有一个请求、看不到真实进度：按时间慢慢涨，最多到 95，这一步做完才跳满。
const STEP: i64 = 100;

struct Work {
    id: u64,
    /// 角色名：只有一段时写进标题。
    label: String,
    stage: &'static str,
    steps: u32,
    done: u32,
    since: Instant,
    ended: bool,
    failed: bool,
    halt: Option<Box<dyn FnOnce()>>,
}

#[derive(Clone, Copy, PartialEq)]
enum Bg { Idle, Refused, Submitted(u64), Running(u64) }

thread_local! {
    /// 这一批的活（做完的也留着算进度），全做完时清空。
    static WORK: RefCell<Vec<Work>> = const { RefCell::new(Vec::new()) };
    static NEXT: Cell<u64> = const { Cell::new(0) };
    static BG: Cell<Bg> = const { Cell::new(Bg::Idle) };
    /// 系统给的 `BGContinuedProcessingTask`（持有一次引用）。
    static TASK: Cell<*mut AnyObject> = const { Cell::new(null_mut()) };
    /// 上次给系统的标题和副标题：没变就不再报。
    static SHOWN: RefCell<(String, String)> = const { RefCell::new((String::new(), String::new())) };
    static TIMER: Cell<*mut AnyObject> = const { Cell::new(null_mut()) };
    /// UIKit 后台任务的标识，0 是没有。
    static UIKIT: Cell<usize> = const { Cell::new(0) };
    /// 这一批的 UIKit 时间用完了：不再要，免得在后台反复要、反复到期。
    static SPENT: Cell<bool> = const { Cell::new(false) };
}

/// 一件活。丢掉（`drop`）就算做完了；`fail` 过的算失败。
pub struct Ticket(u64);

/// 开始一件活：`steps` 步，现在在做 `stage`。系统停掉后台任务时调 `halt`（主线程上）。
pub fn begin(label: String, stage: &'static str, steps: u32, halt: impl FnOnce() + 'static) -> Ticket {
    let id = NEXT.with(|n| { n.set(n.get() + 1); n.get() });
    WORK.with(|w| w.borrow_mut().push(Work { id, label, stage, steps: steps.max(1), done: 0, since: Instant::now(), ended: false, failed: false, halt: Some(Box::new(halt)) }));
    sync();
    Ticket(id)
}

impl Ticket {
    /// 上一步做完了，接着做 `stage`。
    pub fn advance(&self, stage: &'static str) {
        with_work(self.0, |w| { w.done = (w.done + 1).min(w.steps - 1); w.stage = stage; w.since = Instant::now(); });
        report();
    }
    pub fn fail(&self) { with_work(self.0, |w| w.failed = true) }
}

impl Drop for Ticket {
    fn drop(&mut self) {
        let halt = WORK.with(|w| w.borrow_mut().iter_mut().find(|x| x.id == self.0 && !x.ended).map(|x| { x.ended = true; x.halt.take() }));
        drop(halt);
        sync();
    }
}

fn with_work(id: u64, f: impl FnOnce(&mut Work)) { WORK.with(|w| if let Some(x) = w.borrow_mut().iter_mut().find(|x| x.id == id && !x.ended) { f(x) }) }

/// 应用在前台（不是切走了、也不是在后台接着做活）。
pub fn active() -> bool {
    let Some(app) = AnyClass::get(c"UIApplication") else { return false };
    unsafe { let app: *mut AnyObject = msg_send![app, sharedApplication]; let state: isize = msg_send![app, applicationState]; state == 0 }
}

/// 按还有没有活，要或交还两种后台任务；这一批全做完就清空。
fn sync() {
    let busy = WORK.with(|w| w.borrow().iter().any(|x| !x.ended));
    let held = UIKIT.with(Cell::get);
    if busy && held == 0 && !SPENT.with(Cell::get) { UIKIT.with(|u| u.set(begin_uikit())); }
    if !busy {
        if held != 0 { UIKIT.with(|u| u.set(0)); end_uikit(held); }
        SPENT.with(|s| s.set(false));
        let ok = WORK.with(|w| w.borrow().iter().all(|x| !x.failed));
        if matches!(BG.with(Cell::get), Bg::Running(_)) { finish(ok); }
        BG.with(|b| b.set(Bg::Idle));
        WORK.with(|w| w.borrow_mut().clear());
        return;
    }
    if BG.with(Cell::get) == Bg::Idle { submit(); }
    report();
}

fn begin_uikit() -> usize {
    // 到期回调在主线程上。
    let expired = RcBlock::new(|| {
        let id = UIKIT.with(|u| u.replace(0));
        SPENT.with(|s| s.set(true));
        if id != 0 { end_uikit(id); }
    });
    unsafe {
        let app: *mut AnyObject = msg_send![class!(UIApplication), sharedApplication];
        msg_send![app, beginBackgroundTaskWithName: ns(c"Voice over"), expirationHandler: &*expired]
    }
}

fn end_uikit(id: usize) {
    unsafe { let app: *mut AnyObject = msg_send![class!(UIApplication), sharedApplication]; let _: () = msg_send![app, endBackgroundTask: id]; }
}

fn submit() {
    let refuse = || BG.with(|b| b.set(Bg::Refused));
    let Some(request) = AnyClass::get(c"BGContinuedProcessingTaskRequest") else { return refuse() };
    // 只能替前台的应用提交。
    if !active() { return refuse() }
    let serial = NEXT.with(|n| { n.set(n.get() + 1); n.get() });
    unsafe {
        let bundle: *mut AnyObject = msg_send![class!(NSBundle), mainBundle];
        let bundle: *mut AnyObject = msg_send![bundle, bundleIdentifier];
        if bundle.is_null() { return refuse() }
        let bundle = CStr::from_ptr(msg_send![bundle, UTF8String]).to_string_lossy().into_owned();
        let ident = ns_str(&format!("{bundle}.voice.{serial}"));
        let scheduler: *mut AnyObject = msg_send![class!(BGTaskScheduler), sharedScheduler];
        // 启动回调排在主队列上：这里的状态是线程局部的。
        let launched = RcBlock::new(move |task: *mut AnyObject| launch(serial, task));
        let queue = (&raw const _dispatch_main_q).cast_mut().cast::<AnyObject>();
        let ok: bool = msg_send![scheduler, registerForTaskWithIdentifier: ident, usingQueue: queue, launchHandler: &*launched];
        if !ok { log::warn!("Background task identifier is not permitted"); return refuse() }
        let request: *mut AnyObject = msg_send![request, alloc];
        let (title, subtitle) = texts();
        let request: *mut AnyObject = msg_send![request, initWithIdentifier: ident, title: ns_str(&title), subtitle: ns_str(&subtitle)];
        // 系统现在跑不了就算了（不排队）：排到后面再开始时，活多半已经做完了。
        let _: () = msg_send![request, setStrategy: 0isize];
        // 先记上：启动回调也许在提交返回之前就来了。
        BG.with(|b| b.set(Bg::Submitted(serial)));
        let mut error: *mut AnyObject = null_mut();
        let ok: bool = msg_send![scheduler, submitTaskRequest: request, error: &mut error];
        let _: () = msg_send![request, release];
        if !ok {
            // 1：不可用（模拟器、关了后台刷新）；3：没有许可；4：系统现在跑不了。
            let code: isize = if error.is_null() { -1 } else { msg_send![error, code] };
            let ours: bool = !error.is_null() && { let domain: *mut AnyObject = msg_send![error, domain]; msg_send![domain, isEqualToString: BGTaskSchedulerErrorDomain] };
            log::info!("Continued processing task not submitted ({}{code})", if ours { "" } else { "other domain, " });
            refuse();
        }
    }
}

fn launch(serial: u64, task: *mut AnyObject) {
    unsafe {
        // 这一批在系统开始之前就做完了（或已经换了一批）：直接交还。
        if BG.with(Cell::get) != Bg::Submitted(serial) { let _: () = msg_send![task, setTaskCompletedWithSuccess: true]; return; }
        let _: *mut AnyObject = msg_send![task, retain];
        TASK.with(|t| t.set(task));
        // 到期回调在哪个队列上没说：排回主线程。
        let expired = RcBlock::new(move || dispatch_async_f((&raw const _dispatch_main_q).cast(), serial as *mut c_void, expire));
        let _: () = msg_send![task, setExpirationHandler: &*expired];
    }
    BG.with(|b| b.set(Bg::Running(serial)));
    SHOWN.with(|s| *s.borrow_mut() = Default::default());
    tick(true);
    report();
}

extern "C" fn expire(serial: *mut c_void) {
    if BG.with(Cell::get) != Bg::Running(serial as u64) { return }
    finish(false);
    BG.with(|b| b.set(Bg::Refused));
    let halts: Vec<_> = WORK.with(|w| w.borrow_mut().iter_mut().filter(|x| !x.ended).filter_map(|x| { x.ended = true; x.failed = true; x.halt.take() }).collect());
    log::info!("Continued processing task expired; stopping {} voice over(s)", halts.len());
    for halt in halts { halt() }
    sync();
}

/// 交还系统的任务。
fn finish(ok: bool) {
    tick(false);
    let task = TASK.with(|t| t.replace(null_mut()));
    if task.is_null() { return }
    unsafe {
        let _: () = msg_send![task, setExpirationHandler: None::<&block2::Block<dyn Fn()>>];
        let _: () = msg_send![task, setTaskCompletedWithSuccess: ok];
        let _: () = msg_send![task, release];
    }
}

/// 系统的任务在跑时，每半秒报一次进度（等网络时没有别的事会报，太久不动会被当成卡住）。
fn tick(on: bool) {
    let old = TIMER.with(|t| t.replace(null_mut()));
    if !old.is_null() { unsafe { let _: () = msg_send![old, invalidate]; let _: () = msg_send![old, release]; } }
    if !on { return }
    let each = RcBlock::new(|_: *mut AnyObject| report());
    unsafe {
        let timer: *mut AnyObject = msg_send![class!(NSTimer), timerWithTimeInterval: 0.5f64, repeats: true, block: &*each];
        let run_loop: *mut AnyObject = msg_send![class!(NSRunLoop), mainRunLoop];
        let _: () = msg_send![run_loop, addTimer: timer, forMode: NSRunLoopCommonModes];
        let _: *mut AnyObject = msg_send![timer, retain];
        TIMER.with(|t| t.set(timer));
    }
}

fn report() {
    let task = TASK.with(Cell::get);
    if task.is_null() { return }
    let (total, done) = WORK.with(|w| w.borrow().iter().fold((0, 0), |(total, done), x| {
        let steps = x.steps as i64 * STEP;
        let creep = (95. * (1. - (-x.since.elapsed().as_secs_f64() / 8.).exp())) as i64;
        (total + steps, done + if x.ended { steps } else { x.done as i64 * STEP + creep })
    }));
    let texts = texts();
    unsafe {
        let progress: *mut AnyObject = msg_send![task, progress];
        // 先设总数再设完成数。
        let _: () = msg_send![progress, setTotalUnitCount: total.max(1)];
        let _: () = msg_send![progress, setCompletedUnitCount: done.min(total)];
        if SHOWN.with(|s| *s.borrow() != texts) {
            let _: () = msg_send![task, updateTitle: ns_str(&texts.0), subtitle: ns_str(&texts.1)];
            SHOWN.with(|s| *s.borrow_mut() = texts);
        }
    }
}

/// 标题说是谁、几段，副标题说做到哪了。这一批有过几段就一直按几段写（做到只剩一段也不换回去）。
fn texts() -> (String, String) {
    WORK.with(|w| {
        let w = w.borrow();
        let open: Vec<_> = w.iter().filter(|x| !x.ended).collect();
        match (w.len(), open.as_slice()) {
            (1, [one]) if one.label.is_empty() => (tr("配音", "Voice Over").into(), one.stage.into()),
            (1, [one]) => (crate::localized_format!("配音 · {}", "Voice Over · {}", one.label), one.stage.into()),
            (n, _) if n > 1 => (crate::localized_format!("配音 · {} 段", "Voice Over · {} clips", n),
                crate::localized_format!("已完成 {} / {}", "{} of {} done", n - open.len(), n)),
            _ => (tr("配音", "Voice Over").into(), String::new()),
        }
    })
}

unsafe fn ns(s: &CStr) -> *mut AnyObject { unsafe { msg_send![class!(NSString), stringWithUTF8String: s.as_ptr()] } }
unsafe fn ns_str(s: &str) -> *mut AnyObject { let c = std::ffi::CString::new(s).unwrap_or_default(); unsafe { ns(&c) } }
