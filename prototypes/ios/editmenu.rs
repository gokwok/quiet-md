//! 系统的编辑菜单（`UIEditMenuInteraction`）：选区上方那条 剪切 / 复制 / 粘贴 / 全选。
//! 样子、朝上还是朝下、放不下时的翻页、点别处收起，都是系统做的；这里只告诉它菜单对着哪块、里面有哪几项。
//! 项被点了走 `menu::chosen` 回到工作区（`ios::menu_action` 里 `edit-` 开头的那一支），再交给编辑器。
use std::cell::{Cell, RefCell};

use gpui::*;
use objc2::runtime::{AnyClass, AnyObject, NSObject};
use objc2::{ClassType, define_class, msg_send};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

/// 菜单里的一项：交给 `ios::menu_action` 的标识和标题。
pub type Item = (&'static str, SharedString);

thread_local! { static INTERACTION: Cell<*mut AnyObject> = const { Cell::new(std::ptr::null_mut()) }; }
thread_local! { static SHOWN: RefCell<Option<(Bounds<Pixels>, Vec<Item>)>> = const { RefCell::new(None) }; }

pub fn supported() -> bool { AnyClass::get(c"UIEditMenuInteraction").is_some() }

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "QuietMdEditMenuDelegate"]
    struct Delegate;

    impl Delegate {
        #[unsafe(method(editMenuInteraction:menuForConfiguration:suggestedActions:))]
        fn menu(&self, _interaction: *mut AnyObject, _configuration: *mut AnyObject, _suggested: *mut AnyObject) -> *mut AnyObject { build() }

        #[unsafe(method(editMenuInteraction:targetRectForConfiguration:))]
        fn target(&self, _interaction: *mut AnyObject, _configuration: *mut AnyObject) -> CGRect {
            let b = SHOWN.with(|s| s.borrow().as_ref().map(|(b, _)| *b)).unwrap_or_default();
            CGRect::new(CGPoint::new(f64::from(b.origin.x), f64::from(b.origin.y)), CGSize::new(f64::from(b.size.width), f64::from(b.size.height)))
        }

        #[unsafe(method(editMenuInteraction:willDismissMenuForConfiguration:animator:))]
        fn dismissed(&self, _interaction: *mut AnyObject, _configuration: *mut AnyObject, _animator: *mut AnyObject) {
            // 系统自己收起的（点了别处，或点了某一项）：告诉编辑器，免得下一帧又把它弹出来。
            if SHOWN.with(|s| s.borrow_mut().take()).is_some() { super::menu::chosen("edit-dismissed"); }
        }
    }
);

/// 现在要显示的那几项，拼成系统的菜单。
fn build() -> *mut AnyObject {
    let items = SHOWN.with(|s| s.borrow().as_ref().map(|(_, items)| items.clone())).unwrap_or_default();
    unsafe {
        let children: *mut AnyObject = msg_send![AnyClass::get(c"NSMutableArray").unwrap(), array];
        for (id, title) in items {
            let handler = block2::RcBlock::new(move |_action: *mut AnyObject| super::menu::chosen(id));
            let action: *mut AnyObject = msg_send![AnyClass::get(c"UIAction").unwrap(), actionWithTitle: ns_string(&title), image: std::ptr::null_mut::<AnyObject>(), identifier: std::ptr::null_mut::<AnyObject>(), handler: &*handler];
            let _: () = msg_send![children, addObject: action];
        }
        msg_send![AnyClass::get(c"UIMenu").unwrap(), menuWithChildren: children]
    }
}

/// 每帧调用：`rect` 是菜单对着的那块（窗口坐标，一般是选区）。已经按同样的位置和内容显示着就不动。
pub fn present(rect: Bounds<Pixels>, items: Vec<Item>, window: &Window) {
    if SHOWN.with(|s| s.borrow().as_ref().is_some_and(|(r, i)| *r == rect && *i == items)) { return; }
    let Ok(handle) = HasWindowHandle::window_handle(window) else { return; };
    let RawWindowHandle::UiKit(handle) = handle.as_raw() else { return; };
    let view = handle.ui_view.as_ptr() as *mut AnyObject;
    unsafe {
        let mut interaction = INTERACTION.with(|i| i.get());
        if interaction.is_null() {
            let Some(class) = AnyClass::get(c"UIEditMenuInteraction") else { return; };
            // 交互对代理是弱引用：代理一直留着。
            let delegate: *mut AnyObject = msg_send![Delegate::class(), new];
            interaction = msg_send![class, alloc];
            interaction = msg_send![interaction, initWithDelegate: delegate];
            let _: () = msg_send![view, addInteraction: interaction];
            INTERACTION.with(|i| i.set(interaction));
        }
        // 换了位置或内容：先收起旧的。这一次收起是自己发起的，不用告诉编辑器。
        if SHOWN.with(|s| s.borrow_mut().take()).is_some() { let _: () = msg_send![interaction, dismissMenu]; }
        SHOWN.with(|s| *s.borrow_mut() = Some((rect, items)));
        let at = CGPoint::new(f64::from(rect.center().x), f64::from(rect.origin.y));
        let configuration: *mut AnyObject = msg_send![AnyClass::get(c"UIEditMenuConfiguration").unwrap(), configurationWithIdentifier: std::ptr::null_mut::<AnyObject>(), sourcePoint: at];
        let _: () = msg_send![interaction, presentEditMenuWithConfiguration: configuration];
    }
}

/// 系统键盘自己的「粘贴」、三指的剪切 / 复制 / 粘贴手势，发的是响应链上的 `cut:` `copy:` `paste:`。GPUI 的视图
/// 没有实现它们：补上，走和编辑菜单同一条路。每帧调用，只装一次。
pub fn responder_actions(window: &Window) {
    thread_local! { static DONE: Cell<bool> = const { Cell::new(false) }; }
    if DONE.with(|d| d.get()) { return; }
    let Ok(handle) = HasWindowHandle::window_handle(window) else { return; };
    let RawWindowHandle::UiKit(handle) = handle.as_raw() else { return; };
    let view = handle.ui_view.as_ptr() as *mut AnyObject;
    extern "C-unwind" fn cut(_: *mut AnyObject, _: objc2::runtime::Sel, _: *mut AnyObject) { super::menu::chosen("edit-cut") }
    extern "C-unwind" fn copy(_: *mut AnyObject, _: objc2::runtime::Sel, _: *mut AnyObject) { super::menu::chosen("edit-copy") }
    extern "C-unwind" fn paste(_: *mut AnyObject, _: objc2::runtime::Sel, _: *mut AnyObject) { super::menu::chosen("edit-paste") }
    type Act = extern "C-unwind" fn(*mut AnyObject, objc2::runtime::Sel, *mut AnyObject);
    unsafe {
        let class = objc2::ffi::object_getClass(view) as *mut AnyClass;
        for (sel, imp) in [(objc2::sel!(cut:), cut as Act), (objc2::sel!(copy:), copy as Act), (objc2::sel!(paste:), paste as Act)] {
            objc2::ffi::class_addMethod(class, sel, std::mem::transmute::<Act, objc2::runtime::Imp>(imp), c"v@:@".as_ptr());
        }
    }
    DONE.with(|d| d.set(true));
}

/// 不该再显示了（选区没了、开了别的菜单）。
pub fn dismiss() {
    if SHOWN.with(|s| s.borrow_mut().take()).is_none() { return; }
    let interaction = INTERACTION.with(|i| i.get());
    if !interaction.is_null() { unsafe { let _: () = msg_send![interaction, dismissMenu]; } }
}

unsafe fn ns_string(s: &str) -> *mut AnyObject {
    let c = std::ffi::CString::new(s).unwrap_or_default();
    unsafe { msg_send![AnyClass::get(c"NSString").unwrap(), stringWithUTF8String: c.as_ptr()] }
}
