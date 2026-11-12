//! 系统自己的弹出菜单（`UIMenu`）：出现时的变形、按住滑动时跟手的高亮、收起，都是系统做的。
//!
//! 做法是在 GPUI 元素的位置盖一个透明的原生 `UIButton`，把菜单挂在它上面。和 `glass.rs` 的玻璃不同，
//! 这个按钮自己接触摸（系统要靠它弹菜单），那一小块的点击不经过 GPUI；菜单项被点后再回到 GPUI 里处理。
use std::cell::{Cell, RefCell};

use gpui::*;
use objc2::{ClassType, define_class, msg_send};
use objc2::runtime::{AnyClass, AnyObject, NSObject};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::app::Workspace;

/// 菜单里的一项：点了交给 `ios::menu_action` 的标识、标题、SF Symbols 里的图标名（空着就没有图标）、
/// 前面打不打勾、属于第几组（组和组之间系统画分隔线）、是不是删除一类（系统画成红字）、
/// 子菜单（有子项时它自己不能点，系统就地展开）。
#[derive(Clone, PartialEq, Default)]
pub struct Item { pub id: &'static str, pub title: SharedString, pub symbol: &'static str, pub checked: bool, pub group: u8, pub danger: bool, pub kids: Vec<Item> }
impl Item {
    pub fn new(id: &'static str, title: impl Into<SharedString>, symbol: &'static str, group: u8) -> Self { Item { id, title: title.into(), symbol, group, ..Default::default() } }
    pub fn checked(mut self, on: bool) -> Self { self.checked = on; self }
    pub fn danger(mut self) -> Self { self.danger = true; self }
    pub fn kids(mut self, kids: Vec<Item>) -> Self { self.kids = kids; self }
}

struct Button { key: &'static str, view: *mut AnyObject, frame: Bounds<Pixels>, items: Vec<Item>, dark: bool, seen: bool, shown: bool }

thread_local! { static BUTTONS: RefCell<Vec<Button>> = const { RefCell::new(Vec::new()) }; }
thread_local! { static ALLOWED: Cell<bool> = const { Cell::new(false) }; }
thread_local! { static TARGET: RefCell<Option<(AsyncApp, AnyWindowHandle, WeakEntity<Workspace>)>> = const { RefCell::new(None) }; }

/// 每帧开头调用，和 `glass::begin` 一样的规矩：`allowed` 为假时全部藏起，这一帧没被点到的下一帧藏起。
/// 顺便记下回到 GPUI 的路：菜单项的回调来自系统，手里没有 `App`。
pub fn begin(allowed: bool, window: &Window, cx: &mut Context<Workspace>) {
    ALLOWED.with(|a| a.set(allowed));
    REGIONS.with(|r| r.borrow_mut().clear());
    super::files::attach(window);
    follow_appearance(window);
    super::editmenu::responder_actions(window);
    TARGET.with(|t| if t.borrow().is_none() { *t.borrow_mut() = Some((cx.to_async(), window.window_handle(), cx.weak_entity())); });
    BUTTONS.with(|b| for button in b.borrow_mut().iter_mut() { button.seen = false; if !allowed { button.set_shown(false); } });
    window.on_next_frame(|_, _| BUTTONS.with(|b| for button in b.borrow_mut().iter_mut() { if !button.seen { button.set_shown(false); } }));
}

impl Button {
    fn set_shown(&mut self, on: bool) {
        // 藏起时可能还要淡出一小会儿（换页）：这期间不再接触摸，不然紧跟着的一下点按会落到旧页的按钮上。
        if self.shown != on && super::shot::show(self.view, on) { self.shown = on; unsafe { let _: () = msg_send![self.view, setUserInteractionEnabled: on]; } }
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
    let parent = super::shot::overlay(handle.ui_view.as_ptr() as *mut AnyObject);
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
            let _: () = msg_send![button.view, setMenu: menu("", std::ptr::null_mut(), &items, false)];
            button.items = items;
        }
    });
}

/// 拼成系统的菜单。同一组的项收进一个「就地展开」的子菜单，系统在组和组之间画分隔线。
/// `wait`：长按出的上下文菜单，点了的项等菜单收起的动画做完再执行（`ContextDelegate`）。
unsafe fn menu(title: &str, image: *mut AnyObject, items: &[Item], wait: bool) -> *mut AnyObject {
    unsafe {
        let groups: *mut AnyObject = msg_send![AnyClass::get(c"NSMutableArray").unwrap(), array];
        for group in items.chunk_by(|a, b| a.group == b.group) {
            let children: *mut AnyObject = msg_send![AnyClass::get(c"NSMutableArray").unwrap(), array];
            for item in group { let _: () = msg_send![children, addObject: element(item, wait)]; }
            // options 1 = UIMenuOptionsDisplayInline
            let inline: *mut AnyObject = msg_send![AnyClass::get(c"UIMenu").unwrap(), menuWithTitle: ns_string(""), image: std::ptr::null_mut::<AnyObject>(), identifier: std::ptr::null_mut::<AnyObject>(), options: 1usize, children: children];
            let _: () = msg_send![groups, addObject: inline];
        }
        msg_send![AnyClass::get(c"UIMenu").unwrap(), menuWithTitle: ns_string(title), image: image, identifier: std::ptr::null_mut::<AnyObject>(), options: 0usize, children: groups]
    }
}
unsafe fn element(item: &Item, wait: bool) -> *mut AnyObject {
    unsafe {
        let image: *mut AnyObject = if item.symbol.is_empty() { std::ptr::null_mut() } else { msg_send![AnyClass::get(c"UIImage").unwrap(), systemImageNamed: ns_string(item.symbol)] };
        if !item.kids.is_empty() { return menu(&item.title, image, &item.kids, wait); }
        let id = item.id;
        let handler = block2::RcBlock::new(move |_action: *mut AnyObject| if wait && CONTEXT_OPEN.with(|o| o.get()) { CONTEXT_PICK.with(|p| p.set(Some(id))) } else { chosen(id) });
        let action: *mut AnyObject = msg_send![AnyClass::get(c"UIAction").unwrap(), actionWithTitle: ns_string(&item.title), image: image, identifier: std::ptr::null_mut::<AnyObject>(), handler: &*handler];
        // 1 = UIMenuElementStateOn
        if item.checked { let _: () = msg_send![action, setState: 1isize]; }
        // 2 = UIMenuElementAttributesDestructive
        if item.danger { let _: () = msg_send![action, setAttributes: 2usize]; }
        action
    }
}

/// 应用的深浅色压在整个窗口上（`settings::apply_appearance` 在 macOS 上做的同一件事）：系统画的菜单、
/// 上下文菜单、选择器都跟应用走，不跟系统。「跟随系统」时不压。
fn follow_appearance(window: &Window) {
    thread_local! { static STYLE: Cell<Option<isize>> = const { Cell::new(None) }; }
    let style = match crate::settings::prefs().appearance.as_str() { "light" => 1isize, "dark" => 2, _ => 0 };
    if STYLE.with(|s| s.get()) == Some(style) { return; }
    let Ok(handle) = HasWindowHandle::window_handle(window) else { return; };
    let RawWindowHandle::UiKit(handle) = handle.as_raw() else { return; };
    unsafe {
        let ui: *mut AnyObject = msg_send![handle.ui_view.as_ptr() as *mut AnyObject, window];
        if ui.is_null() { return; }
        let _: () = msg_send![ui, setOverrideUserInterfaceStyle: style];
    }
    STYLE.with(|s| s.set(Some(style)));
}

// ---- 长按出的上下文菜单

/// 长按能出菜单的一块：它这一帧在哪、菜单是给谁的、标题、圆角（浮起的预览按它剪）、菜单项。
struct Region { bounds: Bounds<Pixels>, target: SharedString, title: SharedString, radius: f32, items: Vec<Item> }
thread_local! { static REGIONS: RefCell<Vec<Region>> = const { RefCell::new(Vec::new()) }; }
thread_local! { static CONTEXT: Cell<*mut AnyObject> = const { Cell::new(std::ptr::null_mut()) }; }
// 这一次的菜单是给谁的（文档或文件夹的 id），和它浮起时的位置、圆角。
// 上下文菜单开着没有，和开着时点了哪一项：浮起的卡片落回原处之后再执行，不然换了页它还盖在新的一页上。
thread_local! { static CONTEXT_OPEN: Cell<bool> = const { Cell::new(false) }; }
thread_local! { static CONTEXT_PICK: Cell<Option<&'static str>> = const { Cell::new(None) }; }
thread_local! { static CONTEXT_FOR: RefCell<Option<(SharedString, Bounds<Pixels>, f32)>> = const { RefCell::new(None) }; }

/// 放在卡片或行里的占位：长按这一块弹出系统的上下文菜单（卡片浮起、其余压暗），菜单项被点后
/// `ios::menu_action` 用 `context_target` 知道是对谁。每帧（prepaint）报一次位置，被滚到可见范围外的部分不算。
pub fn context(target: impl Into<SharedString>, title: impl Into<SharedString>, radius: f32, items: Vec<Item>) -> impl IntoElement {
    let (target, title) = (target.into(), title.into());
    canvas(move |bounds, window, _| {
        if !ALLOWED.with(|a| a.get()) { return; }
        let bounds = bounds.intersect(&window.content_mask().bounds);
        if bounds.size.width <= px(1.) || bounds.size.height <= px(1.) { return; }
        install(window);
        REGIONS.with(|r| r.borrow_mut().push(Region { bounds, target, title, radius, items }));
    }, |_, _, _, _| {}).absolute().inset_0()
}
/// 长按落在能出上下文菜单的地方：GPUI 自己的长按（当右键）就不要再管了。
pub fn context_at(at: Point<Pixels>) -> bool { REGIONS.with(|r| r.borrow().iter().any(|g| g.bounds.contains(&at))) }
/// 上下文菜单是给谁弹的。
pub fn context_target() -> Option<SharedString> { CONTEXT_FOR.with(|c| c.borrow().as_ref().map(|c| c.0.clone())) }

/// 交互挂在 GPUI 的视图上：长按要从它那里认出来。只挂一次。
fn install(window: &Window) {
    if !CONTEXT.with(|c| c.get()).is_null() { return; }
    let Ok(handle) = HasWindowHandle::window_handle(window) else { return; };
    let RawWindowHandle::UiKit(handle) = handle.as_raw() else { return; };
    unsafe {
        let Some(class) = AnyClass::get(c"UIContextMenuInteraction") else { return; };
        // 交互对代理是弱引用：代理一直留着。
        let delegate: *mut AnyObject = msg_send![ContextDelegate::class(), new];
        let interaction: *mut AnyObject = msg_send![class, alloc];
        let interaction: *mut AnyObject = msg_send![interaction, initWithDelegate: delegate];
        let _: () = msg_send![handle.ui_view.as_ptr() as *mut AnyObject, addInteraction: interaction];
        CONTEXT.with(|c| c.set(interaction));
    }
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "QuietMdContextMenuDelegate"]
    struct ContextDelegate;

    impl ContextDelegate {
        #[unsafe(method(contextMenuInteraction:configurationForMenuAtLocation:))]
        fn configuration(&self, _interaction: *mut AnyObject, at: CGPoint) -> *mut AnyObject {
            let at = point(px(at.x as f32), px(at.y as f32));
            // 后报的画在上面。
            let hit = REGIONS.with(|r| r.borrow().iter().rev().find(|g| g.bounds.contains(&at)).map(|g| (g.target.clone(), g.title.clone(), g.bounds, g.radius, g.items.clone())));
            let Some((target, title, bounds, radius, items)) = hit else { return std::ptr::null_mut(); };
            CONTEXT_FOR.with(|c| *c.borrow_mut() = Some((target, bounds, radius)));
            CONTEXT_OPEN.with(|o| o.set(true));
            CONTEXT_PICK.with(|p| p.set(None));
            let provider = block2::RcBlock::new(move |_suggested: *mut AnyObject| -> *mut AnyObject { unsafe { menu(&title, std::ptr::null_mut(), &items, true) } });
            unsafe {
                let none: Option<&block2::Block<dyn Fn() -> *mut AnyObject>> = None;
                msg_send![AnyClass::get(c"UIContextMenuConfiguration").unwrap(), configurationWithIdentifier: std::ptr::null_mut::<AnyObject>(), previewProvider: none, actionProvider: &*provider]
            }
        }

        #[unsafe(method(contextMenuInteraction:configuration:highlightPreviewForItemWithIdentifier:))]
        fn highlight(&self, _interaction: *mut AnyObject, _configuration: *mut AnyObject, _id: *mut AnyObject) -> *mut AnyObject { preview() }

        #[unsafe(method(contextMenuInteraction:configuration:dismissalPreviewForItemWithIdentifier:))]
        fn dismissal(&self, _interaction: *mut AnyObject, _configuration: *mut AnyObject, _id: *mut AnyObject) -> *mut AnyObject { preview() }

        #[unsafe(method(contextMenuInteraction:willEndForConfiguration:animator:))]
        fn will_end(&self, _interaction: *mut AnyObject, _configuration: *mut AnyObject, animator: *mut AnyObject) {
            let done = block2::RcBlock::new(|| { CONTEXT_OPEN.with(|o| o.set(false)); if let Some(id) = CONTEXT_PICK.with(|p| p.take()) { chosen(id); } });
            if animator.is_null() { done.call(()); } else { unsafe { let _: () = msg_send![animator, addCompletion: &*done]; } }
        }
    }
);

/// 浮起来的那张：长按的那一块的快照，按它的圆角剪，放回原处（系统负责浮起和落回的动画）。
fn preview() -> *mut AnyObject {
    let Some((_, bounds, radius)) = CONTEXT_FOR.with(|c| c.borrow().clone()) else { return std::ptr::null_mut(); };
    let Some(clip) = super::shot::snapshot(bounds) else { return std::ptr::null_mut(); };
    let Some(container) = super::shot::container() else { return std::ptr::null_mut(); };
    unsafe {
        let size = CGSize::new(f64::from(bounds.size.width), f64::from(bounds.size.height));
        let params: *mut AnyObject = msg_send![AnyClass::get(c"UIPreviewParameters").unwrap(), new];
        let path: *mut AnyObject = msg_send![AnyClass::get(c"UIBezierPath").unwrap(), bezierPathWithRoundedRect: CGRect::new(CGPoint::new(0., 0.), size), cornerRadius: f64::from(radius)];
        let _: () = msg_send![params, setVisiblePath: path];
        let clear: *mut AnyObject = msg_send![AnyClass::get(c"UIColor").unwrap(), clearColor];
        let _: () = msg_send![params, setBackgroundColor: clear];
        let center = bounds.center();
        let target: *mut AnyObject = msg_send![AnyClass::get(c"UIPreviewTarget").unwrap(), alloc];
        let target: *mut AnyObject = msg_send![target, initWithContainer: container, center: CGPoint::new(f64::from(center.x), f64::from(center.y))];
        let preview: *mut AnyObject = msg_send![AnyClass::get(c"UITargetedPreview").unwrap(), alloc];
        let preview: *mut AnyObject = msg_send![preview, initWithView: &*clip, parameters: params, target: target];
        let _: () = msg_send![params, release];
        let _: () = msg_send![target, release];
        msg_send![preview, autorelease]
    }
}

/// 菜单项被点了。这是系统的回调，GPUI 可能正忙着：排到它的主线程队列里，轮到时再进工作区。
pub(super) fn chosen(id: &'static str) { dispatch(move |w, window, cx| super::menu_action(id, w, window, cx)); }
/// 同上，带着任意一段要在工作区里做的事（原生面板的控件用）。
pub(super) fn dispatch(run: impl FnOnce(&mut Workspace, &mut Window, &mut Context<Workspace>) + 'static) {
    let Some((app, window, workspace)) = TARGET.with(|t| t.borrow().clone()) else { return; };
    let executor = app.foreground_executor().clone();
    executor.spawn(async move {
        app.update(|cx| {
            let _ = window.update(cx, |_, window, cx| {
                if let Some(workspace) = workspace.upgrade() { workspace.update(cx, |w, cx| run(w, window, cx)); }
            });
        });
    }).detach();
}

unsafe fn ns_string(s: &str) -> *mut AnyObject {
    let c = std::ffi::CString::new(s).unwrap_or_default();
    unsafe { msg_send![AnyClass::get(c"NSString").unwrap(), stringWithUTF8String: c.as_ptr()] }
}
