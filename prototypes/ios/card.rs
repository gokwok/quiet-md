//! 手机上「新建文件夹」的卡片：一整张原生的玻璃（`UIGlassEffect`），里面的输入框、按钮、菜单都是系统控件。
//!
//! 手机版式（`ios::floating`）本来就要有玻璃，所以没有另一套 GPUI 画的。GPUI 画不到玻璃上面，所以卡片里没有 GPUI 的东西；GPUI 那边只留压暗的遮罩和一块占位（`slot`），卡片跟着占位走
//! （占位贴在键盘上面）。状态：建在哪一层记在 `ios::NEW_FOLDER`；名称和描述就在两个输入框里，图标和颜色记在这里，
//! 点「创建」时由 `take` 一起交出去。按钮走 `ios::menu_action`（`new-folder-create` / `new-folder-cancel`）。
use std::cell::{Cell, RefCell};

use gpui::*;
use objc2::msg_send;
use objc2::runtime::{AnyClass, AnyObject};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::settings::tr;

/// 卡片的高：标题一行、名称、三行设置、「创建」。行高是定死的，占位按这个高度留。
pub const HEIGHT: f32 = (PAD + 28. + 14. + 16. + 6. + 44. + 10. + ROW * 3. + 10. + 48. + PAD) as f32;
const PAD: f64 = 16.;
const ROW: f64 = 46.;

/// 文件夹可选的颜色：文档里用的那几个颜色名（`theme::callout_border_swatch` 给出色值）。
pub const COLORS: [(&str, &str, &str); 6] = [("red", "红色", "Red"), ("orange", "橙色", "Orange"), ("yellow", "黄色", "Yellow"), ("green", "绿色", "Green"), ("blue", "蓝色", "Blue"), ("purple", "紫色", "Purple")];

/// 新文件夹填好的内容。
#[derive(Default)]
pub struct Draft { pub name: String, pub note: String, pub icon: Option<String>, pub color: Option<String> }

struct Card { view: *mut AnyObject, name: *mut AnyObject, note: *mut AnyObject, icon: *mut AnyObject, color: *mut AnyObject, create: *mut AnyObject, frame: Bounds<Pixels>, dark: Option<bool>, shown: bool }
thread_local! { static CARD: RefCell<Option<Card>> = const { RefCell::new(None) }; }
thread_local! { static PICKED: RefCell<(Option<String>, Option<&'static str>)> = const { RefCell::new((None, None)) }; }
thread_local! { static SEEN: Cell<bool> = const { Cell::new(false) }; }
// 编辑一个已有的文件夹（照 Craft 的「编辑」，IMG_9145）：卡片一露面就填上它现在的样子，标题「编辑」、按钮「更新」。
thread_local! { static PREFILL: RefCell<Option<Draft>> = const { RefCell::new(None) }; }

/// 下一次露面是编辑这个样子的文件夹（`None` 是新建）。
pub fn prefill(draft: Option<Draft>) { PREFILL.with(|p| *p.borrow_mut() = draft); }

/// 每帧开头调用：这一帧没有占位，卡片就收起（键盘跟着下去），填的内容清掉。
pub fn begin(window: &Window) {
    // 助手的卡片同一个规矩。
    super::assist::card::begin(window);
    SEEN.with(|s| s.set(false));
    window.on_next_frame(|_, _| if !SEEN.with(|s| s.get()) { hide(); });
}

fn hide() {
    CARD.with(|c| if let Some(card) = c.borrow_mut().as_mut().filter(|card| card.shown) { unsafe {
        card.shown = false;
        let _: bool = msg_send![card.view, endEditing: true];
        let _: () = msg_send![card.view, setHidden: true];
        let _: () = msg_send![card.name, setText: ns_string("")];
        let _: () = msg_send![card.note, setText: ns_string("")];
    } });
}

/// 点了「创建」：把填的交出去。
pub fn take() -> Draft {
    let (icon, color) = PICKED.with(|p| p.borrow().clone());
    let (name, note) = CARD.with(|c| c.borrow().as_ref().map(|card| (text_of(card.name), text_of(card.note))).unwrap_or_default());
    Draft { name: name.trim().to_string(), note: note.trim().to_string(), icon, color: color.map(str::to_string) }
}

/// 放在 GPUI 元素里的占位：卡片就摆在这个元素的位置上。
pub fn slot() -> impl IntoElement {
    canvas(move |bounds, window, _| place(bounds, window), |_, _, _, _| {}).absolute().inset_0()
}

fn place(frame: Bounds<Pixels>, window: &Window) {
    let Ok(handle) = HasWindowHandle::window_handle(window) else { return; };
    let RawWindowHandle::UiKit(handle) = handle.as_raw() else { return; };
    let parent = super::shot::overlay(handle.ui_view.as_ptr() as *mut AnyObject);
    SEEN.with(|s| s.set(true));
    let dark = crate::theme::dark();
    CARD.with(|c| unsafe {
        let mut slot = c.borrow_mut();
        if slot.is_none() { *slot = build(parent); }
        let Some(card) = slot.as_mut() else { return; };
        if card.dark != Some(dark) {
            // 深浅色跟应用的设置，不跟系统。
            let _: () = msg_send![card.view, setOverrideUserInterfaceStyle: if dark { 2isize } else { 1isize }];
            card.dark = Some(dark);
        }
        if card.frame != frame {
            let rect = CGRect::new(CGPoint::new(f64::from(frame.origin.x), f64::from(frame.origin.y)), CGSize::new(f64::from(frame.size.width), f64::from(frame.size.height)));
            // 已经露面的：跟着键盘升上来，位置用动画接上。
            if card.shown {
                let (addr, to) = (card.view as usize, rect);
                let body = block2::RcBlock::new(move || { let _: () = msg_send![addr as *mut AnyObject, setFrame: to]; });
                let _: () = msg_send![AnyClass::get(c"UIView").unwrap(), animateWithDuration: 0.25f64, animations: &*body];
            } else { let _: () = msg_send![card.view, setFrame: rect]; }
            layout(card, f64::from(frame.size.width));
            card.frame = frame;
        }
        if !card.shown {
            card.shown = true;
            let edit = PREFILL.with(|p| p.borrow_mut().take());
            let content: *mut AnyObject = msg_send![card.view, contentView];
            let title: *mut AnyObject = msg_send![content, viewWithTag: 1isize];
            let _: () = msg_send![title, setText: ns_string(if edit.is_some() { tr("编辑", "Edit") } else { tr("新建文件夹", "New folder") })];
            let config: *mut AnyObject = msg_send![card.create, configuration];
            let _: () = msg_send![config, setTitle: ns_string(if edit.is_some() { tr("更新", "Update") } else { tr("创建", "Create") })];
            let _: () = msg_send![card.create, setConfiguration: config];
            match &edit {
                Some(d) => {
                    let _: () = msg_send![card.name, setText: ns_string(&d.name)];
                    let _: () = msg_send![card.note, setText: ns_string(&d.note)];
                    let color = d.color.as_deref().and_then(|c| COLORS.iter().find(|(n, _, _)| *n == c).map(|(n, _, _)| *n));
                    PICKED.with(|p| *p.borrow_mut() = (d.icon.clone(), color));
                }
                None => PICKED.with(|p| *p.borrow_mut() = (None, None)),
            }
            // 新建时名字是空的，「创建」先灰掉；编辑时名字已经有了。
            let _: () = msg_send![card.create, setEnabled: edit.as_ref().is_some_and(|d| !d.name.trim().is_empty())];
            refresh(card);
            let _: () = msg_send![parent, bringSubviewToFront: card.view];
            let _: () = msg_send![card.view, setHidden: false];
            let anim: *mut AnyObject = msg_send![AnyClass::get(c"CABasicAnimation").unwrap(), animationWithKeyPath: ns_string("opacity")];
            let from: *mut AnyObject = msg_send![AnyClass::get(c"NSNumber").unwrap(), numberWithDouble: 0f64];
            let _: () = msg_send![anim, setFromValue: from];
            let _: () = msg_send![anim, setDuration: 0.2f64];
            let layer: *mut AnyObject = msg_send![card.view, layer];
            let _: () = msg_send![layer, addAnimation: anim, forKey: ns_string("appear")];
            let _: bool = msg_send![card.name, becomeFirstResponder];
        }
    });
}

unsafe fn build(parent: *mut AnyObject) -> Option<Card> {
    unsafe {
        let view: *mut AnyObject = msg_send![AnyClass::get(c"UIVisualEffectView")?, alloc];
        let effect: *mut AnyObject = msg_send![AnyClass::get(c"UIGlassEffect")?, effectWithStyle: 0isize];
        let view: *mut AnyObject = msg_send![view, initWithEffect: effect];
        if let (Some(corners), Some(corner)) = (AnyClass::get(c"UICornerConfiguration"), AnyClass::get(c"UICornerRadius")) {
            let fixed: *mut AnyObject = msg_send![corner, fixedRadius: 30f64];
            let config: *mut AnyObject = msg_send![corners, configurationWithUniformRadius: fixed];
            let _: () = msg_send![view, setCornerConfiguration: config];
        }
        let _: () = msg_send![view, setHidden: true];
        let content: *mut AnyObject = msg_send![view, contentView];
        let color = |name: &std::ffi::CStr| -> *mut AnyObject { msg_send![AnyClass::get(c"UIColor").unwrap(), performSelector: objc2::runtime::Sel::register(name)] };
        let label = |tag: isize, text: &str, size: f64, weight: f64, tint: *mut AnyObject| {
            let l: *mut AnyObject = msg_send![AnyClass::get(c"UILabel").unwrap(), new];
            let font: *mut AnyObject = msg_send![AnyClass::get(c"UIFont").unwrap(), systemFontOfSize: size, weight: weight];
            let _: () = msg_send![l, setFont: font];
            let _: () = msg_send![l, setText: ns_string(text)];
            let _: () = msg_send![l, setTextColor: tint];
            let _: () = msg_send![l, setTag: tag];
            let _: () = msg_send![content, addSubview: l];
        };
        // 0.3 = UIFontWeightSemibold，0 = Regular
        label(1, tr("新建文件夹", "New folder"), 17., 0.3, color(c"labelColor"));
        label(2, tr("名称", "Name"), 12., 0., color(c"secondaryLabelColor"));
        label(3, tr("描述", "Description"), 16., 0., color(c"labelColor"));
        label(4, tr("图标", "Icon"), 16., 0., color(c"labelColor"));
        label(5, tr("颜色", "Color"), 16., 0., color(c"labelColor"));
        // 7 = UIButtonTypeClose：系统的圆形关闭钮。
        let close: *mut AnyObject = msg_send![AnyClass::get(c"UIButton").unwrap(), buttonWithType: 7isize];
        let _: () = msg_send![close, setTag: 6isize];
        on(close, 64, || super::menu::chosen("new-folder-cancel"));
        let _: () = msg_send![content, addSubview: close];
        // 名称垫一块浅底；描述在自己那一行的右半边，靠右排。
        let well: *mut AnyObject = msg_send![AnyClass::get(c"UIView").unwrap(), new];
        let _: () = msg_send![well, setBackgroundColor: color(c"tertiarySystemFillColor")];
        let layer: *mut AnyObject = msg_send![well, layer];
        let _: () = msg_send![layer, setCornerRadius: 12f64];
        let _: () = msg_send![well, setTag: 7isize];
        let _: () = msg_send![content, addSubview: well];
        let field = |placeholder: &str, align: isize| {
            let f: *mut AnyObject = msg_send![AnyClass::get(c"UITextField").unwrap(), new];
            let font: *mut AnyObject = msg_send![AnyClass::get(c"UIFont").unwrap(), systemFontOfSize: 16f64];
            let _: () = msg_send![f, setFont: font];
            let _: () = msg_send![f, setPlaceholder: ns_string(placeholder)];
            let _: () = msg_send![f, setTextAlignment: align];
            let _: () = msg_send![content, addSubview: f];
            f
        };
        let name = field(tr("文件夹名称", "Folder name"), 0);
        // 2 = NSTextAlignmentRight
        let note = field(tr("添加描述", "Add a description"), 2);
        let menu_button = || {
            // 灰底的小按钮，点了出系统菜单。
            let config: *mut AnyObject = msg_send![AnyClass::get(c"UIButtonConfiguration").unwrap(), grayButtonConfiguration];
            let _: () = msg_send![config, setCornerStyle: 4isize];
            let b: *mut AnyObject = msg_send![AnyClass::get(c"UIButton").unwrap(), buttonWithConfiguration: config, primaryAction: std::ptr::null_mut::<AnyObject>()];
            let _: () = msg_send![b, setShowsMenuAsPrimaryAction: true];
            let _: () = msg_send![content, addSubview: b];
            b
        };
        let (icon, tint) = (menu_button(), menu_button());
        let config: *mut AnyObject = msg_send![AnyClass::get(c"UIButtonConfiguration").unwrap(), filledButtonConfiguration];
        let _: () = msg_send![config, setCornerStyle: 4isize];
        let _: () = msg_send![config, setTitle: ns_string(tr("创建", "Create"))];
        let _: () = msg_send![config, setBaseBackgroundColor: super::glass::ui_color(crate::theme::Theme::primary())];
        let create: *mut AnyObject = msg_send![AnyClass::get(c"UIButton").unwrap(), buttonWithConfiguration: config, primaryAction: std::ptr::null_mut::<AnyObject>()];
        let _: () = msg_send![create, setEnabled: false];
        let _: () = msg_send![content, addSubview: create];
        // 没填名字时「创建」是灰的；回车和点「创建」一样（没名字时不建也不关）。
        let (field_addr, create_addr) = (name as usize, create as usize);
        let ready = move || !text_of(field_addr as *mut AnyObject).trim().is_empty();
        // 1 << 17 = UIControlEventEditingChanged
        on(name, 1 << 17, move || { let _: () = msg_send![create_addr as *mut AnyObject, setEnabled: ready()]; });
        let submit = move || if ready() { super::menu::chosen("new-folder-create") };
        // 1 << 13 = UIControlEventPrimaryActionTriggered（回车）。不接 EditingDidEndOnExit：接了它，回车会顺带收起键盘。
        on(name, 1 << 13, submit);
        on(note, 1 << 13, submit);
        on(create, 64, submit);
        let _: () = msg_send![parent, addSubview: view];
        Some(Card { view, name, note, icon, color: tint, create, frame: Bounds::default(), dark: None, shown: false })
    }
}

/// 控件事件接一段代码（系统在主线程上回调）。
unsafe fn on(control: *mut AnyObject, events: usize, run: impl Fn() + 'static) {
    unsafe {
        let handler = block2::RcBlock::new(move |_action: *mut AnyObject| run());
        let action: *mut AnyObject = msg_send![AnyClass::get(c"UIAction").unwrap(), actionWithTitle: ns_string(""), image: std::ptr::null_mut::<AnyObject>(), identifier: std::ptr::null_mut::<AnyObject>(), handler: &*handler];
        let _: () = msg_send![control, addAction: action, forControlEvents: events];
    }
}

unsafe fn layout(card: &Card, w: f64) {
    unsafe {
        let content: *mut AnyObject = msg_send![card.view, contentView];
        let set = |v: *mut AnyObject, x: f64, y: f64, w: f64, h: f64| { let _: () = msg_send![v, setFrame: CGRect::new(CGPoint::new(x, y), CGSize::new(w, h))]; };
        let tagged = |tag: isize| -> *mut AnyObject { msg_send![content, viewWithTag: tag] };
        let inner = w - PAD * 2.;
        let mut y = PAD;
        set(tagged(1), PAD + 4., y, inner - 40., 28.);
        set(tagged(6), w - PAD - 28., y, 28., 28.);
        y += 28. + 14.;
        set(tagged(2), PAD + 4., y, inner, 16.);
        y += 16. + 6.;
        set(tagged(7), PAD, y, inner, 44.);
        set(card.name, PAD + 12., y, inner - 24., 44.);
        y += 44. + 10.;
        let half = inner / 2.;
        set(tagged(3), PAD + 4., y, 80., ROW);
        set(card.note, PAD + 88., y, inner - 92., ROW);
        y += ROW;
        set(tagged(4), PAD + 4., y, half, ROW);
        set(card.icon, w - PAD - 64., y + 5., 64., ROW - 10.);
        y += ROW;
        set(tagged(5), PAD + 4., y, half, ROW);
        set(card.color, w - PAD - 64., y + 5., 64., ROW - 10.);
        y += ROW + 10.;
        set(card.create, PAD, y, inner, 48.);
    }
}

/// 图标和颜色两颗按钮照当前选的重画，菜单里当前那一项打勾。
unsafe fn refresh(card: &Card) {
    unsafe {
        let (icon, color) = PICKED.with(|p| p.borrow().clone());
        let image = |symbol: &str| -> *mut AnyObject { msg_send![AnyClass::get(c"UIImage").unwrap(), systemImageNamed: ns_string(symbol)] };
        let tinted = |symbol: &str, name: &str| -> *mut AnyObject {
            let Some(c) = crate::theme::callout_border_swatch(name) else { return image(symbol); };
            // 1 = UIImageRenderingModeAlwaysOriginal
            msg_send![image(symbol), imageWithTintColor: super::glass::ui_color(c), renderingMode: 1isize]
        };
        let face = |button: *mut AnyObject, title: Option<&str>, picture: *mut AnyObject| {
            let config: *mut AnyObject = msg_send![button, configuration];
            let _: () = msg_send![config, setTitle: title.map_or(std::ptr::null_mut(), |t| ns_string(t))];
            let _: () = msg_send![config, setImage: picture];
            let _: () = msg_send![button, setConfiguration: config];
        };
        // 和首页共用同一份 SVG，颜色只染描边，不切换为实心或彩色 emoji。
        face(card.icon, None, super::glass::icon_image(super::folder_icons::name(icon.as_deref()), 20.));
        let config: *mut AnyObject = msg_send![card.icon, configuration];
        let ink = color.and_then(crate::theme::callout_border_swatch).unwrap_or_else(crate::theme::Theme::icon);
        let _: () = msg_send![config, setBaseForegroundColor: super::glass::ui_color(ink)];
        let _: () = msg_send![card.icon, setConfiguration: config];
        match color { Some(c) => face(card.color, None, tinted("circle.fill", c)), None => face(card.color, None, image("plus")) }
        let actions = || -> *mut AnyObject { msg_send![AnyClass::get(c"NSMutableArray").unwrap(), array] };
        let pick = |title: &str, picture: *mut AnyObject, checked: bool, run: Box<dyn Fn()>| -> *mut AnyObject {
            let handler = block2::RcBlock::new(move |_action: *mut AnyObject| { run(); CARD.with(|c| if let Some(card) = c.borrow().as_ref() { refresh(card); }); });
            let action: *mut AnyObject = msg_send![AnyClass::get(c"UIAction").unwrap(), actionWithTitle: ns_string(title), image: picture, identifier: std::ptr::null_mut::<AnyObject>(), handler: &*handler];
            // 1 = UIMenuElementStateOn
            if checked { let _: () = msg_send![action, setState: 1isize]; }
            action
        };
        let group = |options: usize, children: *mut AnyObject| -> *mut AnyObject { msg_send![AnyClass::get(c"UIMenu").unwrap(), menuWithTitle: ns_string(""), image: std::ptr::null_mut::<AnyObject>(), identifier: std::ptr::null_mut::<AnyObject>(), options: options, children: children] };
        // 图标：统一的线性图标排成几排（1 | 128 = 就地展开的一排「调色板」），最下面一项换回默认。
        let root = actions();
        for row in super::folder_icons::CHOICES.chunks(6) {
            let cells = actions();
            for choice in row {
                let value = choice.value;
                let picture = super::glass::icon_image(choice.glyph, 20.);
                let _: () = msg_send![cells, addObject: pick(tr(choice.zh, choice.en), picture, icon.as_deref() == Some(value), Box::new(move || PICKED.with(|p| p.borrow_mut().0 = Some(value.to_string()))))];
            }
            let _: () = msg_send![root, addObject: group(1 | 128, cells)];
        }
        let _: () = msg_send![root, addObject: pick(tr("使用默认图标", "Use the default icon"), super::glass::icon_image("folder", 20.), icon.is_none(), Box::new(|| PICKED.with(|p| p.borrow_mut().0 = None)))];
        let _: () = msg_send![card.icon, setMenu: group(0, root)];
        // 颜色：一排色点，下面一项去掉颜色。
        let (root, cells) = (actions(), actions());
        for (name, zh, en) in COLORS {
            let _: () = msg_send![cells, addObject: pick(tr(zh, en), tinted("circle.fill", name), color == Some(name), Box::new(move || PICKED.with(|p| p.borrow_mut().1 = Some(name))))];
        }
        let _: () = msg_send![root, addObject: group(1 | 128, cells)];
        let _: () = msg_send![root, addObject: pick(tr("无颜色", "No color"), image("circle.slash"), color.is_none(), Box::new(|| PICKED.with(|p| p.borrow_mut().1 = None)))];
        let _: () = msg_send![card.color, setMenu: group(0, root)];
    }
}

fn text_of(field: *mut AnyObject) -> String {
    unsafe {
        let text: *mut AnyObject = msg_send![field, text];
        if text.is_null() { return String::new(); }
        let utf8: *const std::ffi::c_char = msg_send![text, UTF8String];
        if utf8.is_null() { String::new() } else { std::ffi::CStr::from_ptr(utf8).to_string_lossy().into_owned() }
    }
}

unsafe fn ns_string(s: &str) -> *mut AnyObject {
    let c = std::ffi::CString::new(s).unwrap_or_default();
    unsafe { msg_send![AnyClass::get(c"NSString").unwrap(), stringWithUTF8String: c.as_ptr()] }
}
