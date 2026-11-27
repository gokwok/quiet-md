//! 手机上的确认框：系统的 `UIAlertController`（iOS 26 上是一张玻璃卡片，两颗胶囊按钮）。
//!
//! 状态只有 GPUI 那一份（`Workspace::confirm`）：它挂着的每一帧调 `show`，第一次调时弹出来；
//! 按钮点了走 `ios::menu_action`（`confirm-ok` / `confirm-cancel`）；`confirm` 被别处清掉时，下一帧没人调 `show`，自己收起。
use std::cell::{Cell, RefCell};

use gpui::Window;
use objc2::msg_send;
use objc2::runtime::{AnyClass, AnyObject};

// 弹着的那一个（标题和说明拼成的键，换了内容就换一个）。
thread_local! { static SHOWN: RefCell<Option<(String, *mut AnyObject)>> = const { RefCell::new(None) }; }
// 按钮已经点了、GPUI 还没轮到处理的那几帧：`confirm` 还挂着，不能再弹一次。
thread_local! { static ANSWERED: Cell<bool> = const { Cell::new(false) }; }
thread_local! { static SEEN: Cell<bool> = const { Cell::new(false) }; }

/// 用不用系统的提示框：和玻璃同一个开关（旧系统上的版式照旧由 GPUI 画）。
pub fn native() -> bool { super::glass::supported() }

/// 每帧开头调用：这一帧没人要提示框，就把弹着的收起。
pub fn begin(window: &Window) {
    SEEN.with(|s| s.set(false));
    window.on_next_frame(|_, _| if !SEEN.with(|s| s.get()) {
        ANSWERED.with(|a| a.set(false));
        if let Some((_, alert)) = SHOWN.with(|s| s.borrow_mut().take()) { unsafe {
            let none: Option<&block2::Block<dyn Fn()>> = None;
            let _: () = msg_send![alert, dismissViewControllerAnimated: true, completion: none];
            let _: () = msg_send![alert, release];
        } }
    });
}

/// 这一帧要这个提示框。`danger` 时确定按钮是红的。返回假是没弹出来（还没有窗口，或者上面正弹着别的）：
/// 调用的地方这一帧自己画一张（`ios::alert`），不然只剩一层点不动的遮罩。
pub fn show(title: &str, body: &str, action: &str, danger: bool, refs_hide: Option<bool>) -> bool { ask(title, body, action, danger, refs_hide, ("confirm-ok", "confirm-cancel")) }

/// 同上，按钮点了走自己的两个标识（确定、取消）：不挂在 `Workspace::confirm` 上的提示框用（比如助手回滚前的提醒）。
pub fn ask(title: &str, body: &str, action: &str, danger: bool, refs_hide: Option<bool>, (ok_id, cancel_id): (&'static str, &'static str)) -> bool {
    SEEN.with(|s| s.set(true));
    if ANSWERED.with(|a| a.get()) { return true; }
    let key = format!("{title}\n{body}\n{action}");
    if SHOWN.with(|s| s.borrow().as_ref().is_some_and(|(k, _)| *k == key)) { return true; }
    unsafe {
        let (Some(class), Some(item)) = (AnyClass::get(c"UIAlertController"), AnyClass::get(c"UIAlertAction")) else { return false; };
        let controller = super::files::root_controller();
        if controller.is_null() { return false; }
        // 上面已经弹着别的（文件选择器之类）：等它收起。
        let busy: *mut AnyObject = msg_send![controller, presentedViewController];
        if let Some((_, old)) = SHOWN.with(|s| s.borrow_mut().take()) {
            let none: Option<&block2::Block<dyn Fn()>> = None;
            let _: () = msg_send![old, dismissViewControllerAnimated: false, completion: none];
            let _: () = msg_send![old, release];
        } else if !busy.is_null() { return false; }
        let message = if body.is_empty() { std::ptr::null_mut() } else { ns_string(body) };
        // 1 = UIAlertControllerStyleAlert
        let alert: *mut AnyObject = msg_send![class, alertControllerWithTitle: ns_string(title), message: message, preferredStyle: 1isize];
        let add = |title: &str, style: isize, id: &'static str| {
            let handler = block2::RcBlock::new(move |_action: *mut AnyObject| {
                ANSWERED.with(|a| a.set(true));
                if let Some((_, alert)) = SHOWN.with(|s| s.borrow_mut().take()) { let _: () = msg_send![alert, release]; }
                super::menu::chosen(id);
            });
            let item: *mut AnyObject = msg_send![item, actionWithTitle: ns_string(title), style: style, handler: &*handler];
            let _: () = msg_send![alert, addAction: item];
            item
        };
        // 1 = Cancel，2 = Destructive，0 = Default
        add(crate::settings::text("取消"), 1, cancel_id);
        if refs_hide == Some(true) {
            add(crate::settings::tr("隐藏且不再提示", "Hide and don’t ask again"), 0, "confirm-hide-refs-always");
        }
        let ok = add(action, if danger { 2 } else { 0 }, ok_id);
        if !danger { let _: () = msg_send![alert, setPreferredAction: ok]; }
        // 深浅色跟应用的设置，不跟系统。
        let _: () = msg_send![alert, setOverrideUserInterfaceStyle: if crate::theme::dark() { 2isize } else { 1isize }];
        let _: *mut AnyObject = msg_send![alert, retain];
        if refs_hide.is_some() {
            let none: Option<&block2::Block<dyn Fn()>> = None;
            let _: () = msg_send![controller, presentViewController: alert, animated: true, completion: none];
            SHOWN.with(|s| *s.borrow_mut() = Some((key, alert)));
            return true;
        }
        let addr = alert as usize;
        // 呈现的容器有时到这一刻才搭好：当场藏一次，转场做完再补一次。
        let done = block2::RcBlock::new(move || undim(addr as *mut AnyObject));
        let _: () = msg_send![controller, presentViewController: alert, animated: true, completion: &*done];
        undim(alert);
        // 压暗是跟着转场淡入的：转场一开始（容器刚搭好）就藏，不然中途会闪一下暗。
        let coordinator: *mut AnyObject = msg_send![alert, transitionCoordinator];
        if !coordinator.is_null() {
            let along = block2::RcBlock::new(move |_context: *mut AnyObject| undim(addr as *mut AnyObject));
            let none: Option<&block2::Block<dyn Fn(*mut AnyObject)>> = None;
            let _: bool = msg_send![coordinator, animateAlongsideTransition: &*along, completion: none];
        }
        SHOWN.with(|s| *s.borrow_mut() = Some((key, alert)));
    }
    true
}

/// 系统提示框自带一层压暗，没有公开的开关：它是呈现容器里一块带底色的普通 `UIView`（提示框自己在另一个私有类的视图里），
/// 找到就藏掉，背后的页面原样留着。找不到（系统换了实现）就算了，只是多一层压暗。
unsafe fn undim(alert: *mut AnyObject) {
    unsafe {
        let presentation: *mut AnyObject = msg_send![alert, presentationController];
        if presentation.is_null() { return; }
        let container: *mut AnyObject = msg_send![presentation, containerView];
        if container.is_null() { return; }
        let plain = AnyClass::get(c"UIView").unwrap();
        let kids: *mut AnyObject = msg_send![container, subviews];
        let n: usize = msg_send![kids, count];
        for i in 0..n {
            let kid: *mut AnyObject = msg_send![kids, objectAtIndex: i];
            let color: *mut AnyObject = msg_send![kid, backgroundColor];
            if std::ptr::eq((*kid).class(), plain) && !color.is_null() { let _: () = msg_send![kid, setHidden: true]; }
        }
    }
}

unsafe fn ns_string(s: &str) -> *mut AnyObject {
    let c = std::ffi::CString::new(s).unwrap_or_default();
    unsafe { msg_send![AnyClass::get(c"NSString").unwrap(), stringWithUTF8String: c.as_ptr()] }
}
