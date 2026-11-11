//! 系统自己的弹出菜单（`UIMenu`）：出现时的变形、按住滑动时跟手的高亮、收起，都是系统做的。
//!
//! 做法是在 GPUI 元素的位置盖一个透明的原生 `UIButton`，把菜单挂在它上面。和 `glass.rs` 的玻璃不同，
//! 这个按钮自己接触摸（系统要靠它弹菜单），那一小块的点击不经过 GPUI；菜单项被点后再回到 GPUI 里处理。
use std::cell::{Cell, RefCell};

use gpui::*;
use objc2::msg_send;
use objc2::runtime::{AnyClass, AnyObject};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::app::Workspace;

/// 菜单里的一项：点了交给 `ios::menu_action` 的标识、标题、SF Symbols 里的图标名（空着就没有图标）、
/// 前面打不打勾、属于第几组（组和组之间系统画分隔线）。
#[derive(Clone, PartialEq)]
pub struct Item { pub id: &'static str, pub title: SharedString, pub symbol: &'static str, pub checked: bool, pub group: u8 }

struct Button { key: &'static str, view: *mut AnyObject, frame: Bounds<Pixels>, items: Vec<Item>, dark: bool, seen: bool, shown: bool }

thread_local! { static BUTTONS: RefCell<Vec<Button>> = const { RefCell::new(Vec::new()) }; }
thread_local! { static ALLOWED: Cell<bool> = const { Cell::new(false) }; }
thread_local! { static TARGET: RefCell<Option<(AsyncApp, AnyWindowHandle, WeakEntity<Workspace>)>> = const { RefCell::new(None) }; }

/// 每帧开头调用，和 `glass::begin` 一样的规矩：`allowed` 为假时全部藏起，这一帧没被点到的下一帧藏起。
/// 顺便记下回到 GPUI 的路：菜单项的回调来自系统，手里没有 `App`。
pub fn begin(allowed: bool, window: &Window, cx: &mut Context<Workspace>) {
    ALLOWED.with(|a| a.set(allowed));
    super::files::attach(window);
    TARGET.with(|t| if t.borrow().is_none() { *t.borrow_mut() = Some((cx.to_async(), window.window_handle(), cx.weak_entity())); });
    BUTTONS.with(|b| for button in b.borrow_mut().iter_mut() { button.seen = false; if !allowed { button.set_shown(false); } });
    window.on_next_frame(|_, _| BUTTONS.with(|b| for button in b.borrow_mut().iter_mut() { if !button.seen { button.set_shown(false); } }));
}

impl Button {
    fn set_shown(&mut self, on: bool) {
        if self.shown != on { self.shown = on; unsafe { let _: () = msg_send![self.view, setHidden: !on]; } }
    }
}

/// 放在 GPUI 元素里的占位：这个元素的位置上盖一个透明的原生按钮，点它弹出系统菜单。
/// 按钮本身不画东西，图标仍由下面的 GPUI 元素画。
pub fn button(key: &'static str, items: Vec<Item>) -> impl IntoElement {
    canvas(move |bounds, window, _| place(key, bounds, None, items.clone(), window), |_, _, _, _| {}).absolute().inset_0()
}
/// 同上，但按钮自己就是一颗系统的玻璃圆钮（图标是 SF Symbols 里的 `symbol`）：菜单从这颗玻璃里变出来。
pub fn glass_button(key: &'static str, symbol: &'static str, color: Hsla, items: Vec<Item>) -> impl IntoElement {
    canvas(move |bounds, window, _| place(key, bounds, Some((symbol, color)), items.clone(), window), |_, _, _, _| {}).absolute().inset_0()
}

fn place(key: &'static str, frame: Bounds<Pixels>, glass: Option<(&'static str, Hsla)>, items: Vec<Item>, window: &Window) {
    let Ok(handle) = HasWindowHandle::window_handle(window) else { return; };
    let RawWindowHandle::UiKit(handle) = handle.as_raw() else { return; };
    let parent = handle.ui_view.as_ptr() as *mut AnyObject;
    let dark = crate::theme::dark();
    BUTTONS.with(|b| unsafe {
        let mut buttons = b.borrow_mut();
        let ix = match buttons.iter().position(|button| button.key == key) {
            Some(ix) => ix,
            None => {
                let Some(class) = AnyClass::get(c"UIButton") else { return; };
                let view: *mut AnyObject = match glass.zip(AnyClass::get(c"UIButtonConfiguration")) {
                    Some(((symbol, color), config)) => {
                        let config: *mut AnyObject = msg_send![config, glassButtonConfiguration];
                        let image: *mut AnyObject = msg_send![AnyClass::get(c"UIImage").unwrap(), systemImageNamed: ns_string(symbol)];
                        let _: () = msg_send![config, setImage: image];
                        let _: () = msg_send![config, setBaseForegroundColor: super::glass::ui_color(color)];
                        // 4 = UIButtonConfigurationCornerStyleCapsule
                        let _: () = msg_send![config, setCornerStyle: 4isize];
                        msg_send![class, buttonWithConfiguration: config, primaryAction: std::ptr::null_mut::<AnyObject>()]
                    }
                    // 0 = UIButtonTypeCustom
                    None => msg_send![class, buttonWithType: 0isize],
                };
                let _: () = msg_send![view, setShowsMenuAsPrimaryAction: true];
                let _: () = msg_send![view, setHidden: true];
                let _: () = msg_send![parent, addSubview: view];
                buttons.push(Button { key, view, frame: Bounds::default(), items: vec![], dark: !dark, seen: false, shown: false });
                buttons.len() - 1
            }
        };
        let button = &mut buttons[ix];
        button.seen = true;
        button.set_shown(ALLOWED.with(|a| a.get()));
        if button.frame != frame {
            let rect = CGRect::new(CGPoint::new(f64::from(frame.origin.x), f64::from(frame.origin.y)), CGSize::new(f64::from(frame.size.width), f64::from(frame.size.height)));
            let _: () = msg_send![button.view, setFrame: rect];
            button.frame = frame;
        }
        if button.dark != dark {
            // 深浅色跟应用的设置，不跟系统。
            let _: () = msg_send![button.view, setOverrideUserInterfaceStyle: if dark { 2isize } else { 1isize }];
            button.dark = dark;
        }
        if button.items != items {
            // 同一组的项收进一个「就地展开」的子菜单，系统在组和组之间画分隔线。
            let groups: *mut AnyObject = msg_send![AnyClass::get(c"NSMutableArray").unwrap(), array];
            for group in items.chunk_by(|a, b| a.group == b.group) {
                let children: *mut AnyObject = msg_send![AnyClass::get(c"NSMutableArray").unwrap(), array];
                for item in group {
                    let id = item.id;
                    let handler = block2::RcBlock::new(move |_action: *mut AnyObject| chosen(id));
                    let image: *mut AnyObject = if item.symbol.is_empty() { std::ptr::null_mut() } else { msg_send![AnyClass::get(c"UIImage").unwrap(), systemImageNamed: ns_string(item.symbol)] };
                    let action: *mut AnyObject = msg_send![AnyClass::get(c"UIAction").unwrap(), actionWithTitle: ns_string(&item.title), image: image, identifier: std::ptr::null_mut::<AnyObject>(), handler: &*handler];
                    // 1 = UIMenuElementStateOn
                    if item.checked { let _: () = msg_send![action, setState: 1isize]; }
                    let _: () = msg_send![children, addObject: action];
                }
                // options 1 = UIMenuOptionsDisplayInline
                let inline: *mut AnyObject = msg_send![AnyClass::get(c"UIMenu").unwrap(), menuWithTitle: ns_string(""), image: std::ptr::null_mut::<AnyObject>(), identifier: std::ptr::null_mut::<AnyObject>(), options: 1usize, children: children];
                let _: () = msg_send![groups, addObject: inline];
            }
            let menu: *mut AnyObject = msg_send![AnyClass::get(c"UIMenu").unwrap(), menuWithChildren: groups];
            let _: () = msg_send![button.view, setMenu: menu];
            button.items = items;
        }
    });
}

/// 菜单项被点了。这是系统的回调，GPUI 可能正忙着：排到它的主线程队列里，轮到时再进工作区。
pub(super) fn chosen(id: &'static str) {
    let Some((app, window, workspace)) = TARGET.with(|t| t.borrow().clone()) else { return; };
    let executor = app.foreground_executor().clone();
    executor.spawn(async move {
        app.update(|cx| {
            let _ = window.update(cx, |_, window, cx| {
                if let Some(workspace) = workspace.upgrade() { workspace.update(cx, |w, cx| super::menu_action(id, w, window, cx)); }
            });
        });
    }).detach();
}

unsafe fn ns_string(s: &str) -> *mut AnyObject {
    let c = std::ffi::CString::new(s).unwrap_or_default();
    unsafe { msg_send![AnyClass::get(c"NSString").unwrap(), stringWithUTF8String: c.as_ptr()] }
}
