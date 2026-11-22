//! 手机上编辑封面（照 Craft 11/21 的录屏）：点封面弹出系统菜单——视图、重新定位或裁剪 / 选择一张图片、拍照、浏览文件 / 移除。
//!
//! - 菜单：封面上平时没有原生按钮（不挡滚动），点了才把一颗透明的 `UIButton` 摆到封面的位置，让它弹出自己的菜单。
//! - 视图：全屏看图，从封面放大出来（iOS 18 的缩放转场）；先是黑底，点一下换成浅底、出关闭和分享，双指或双击放大，下滑收起。
//! - 重新定位或裁剪：底部弹出的深色面板，框外模糊压暗，拖动、双指缩放图片，拖四个角改框；比例菜单 4:3、3:1、2:1、16:9、
//!   自由形式、默认（封面框原本的比例）和重置。裁好的另存一张图作封面，原图和框记在文档库的 `cover-crops.json`，
//!   再打开时从原图、原来的框接着改；不是「默认」比例的，封面的高按裁出来的比例（`aspect`，编辑器的 `cover_h`）。
//! - 选择一张图片：样式面板的封面页（最近用过的封面在那里切换）。拍照、浏览文件：系统的相机和文件选择器。
use std::cell::{Cell, RefCell};

use gpui::*;
use objc2::encode::{Encode, Encoding, RefEncode};
use objc2::msg_send;
use objc2::runtime::{AnyClass, AnyObject, Bool, ClassBuilder, Sel};
use objc2::sel;
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use serde::{Deserialize, Serialize};

use super::menu::{self, Item};
use crate::app::Workspace;
use crate::settings::tr;

#[repr(C)] #[derive(Clone, Copy)] struct Insets { top: f64, left: f64, bottom: f64, right: f64 }
unsafe impl Encode for Insets { const ENCODING: Encoding = Encoding::Struct("UIEdgeInsets", &[const { Encoding::Double }; 4]); }
#[repr(C)] struct CGColor { _private: [u8; 0] }
unsafe impl RefEncode for CGColor { const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("CGColor", &[])); }
#[repr(C)] struct CGPath { _private: [u8; 0] }
unsafe impl RefEncode for CGPath { const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("CGPath", &[])); }

#[link(name = "UIKit", kind = "framework")]
unsafe extern "C" { fn UIImageJPEGRepresentation(image: *mut AnyObject, quality: f64) -> *mut AnyObject; }

fn ns(s: &str) -> *mut AnyObject { unsafe { super::glass::ns_string(s) } }
fn class(name: &std::ffi::CStr) -> &'static AnyClass { AnyClass::get(name).unwrap() }
unsafe fn color(white: f64, alpha: f64) -> *mut AnyObject { unsafe { msg_send![class(c"UIColor"), colorWithWhite: white, alpha: alpha] } }
unsafe fn new_view(name: &std::ffi::CStr) -> *mut AnyObject {
    unsafe { let v: *mut AnyObject = msg_send![class(name), alloc]; msg_send![v, initWithFrame: CGRect::default()] }
}
/// 加进父视图后放掉自己那一份（父视图持有）。
unsafe fn add(parent: *mut AnyObject, view: *mut AnyObject) -> *mut AnyObject {
    unsafe { let _: () = msg_send![parent, addSubview: view]; let _: () = msg_send![view, release]; }
    view
}
unsafe fn symbol(name: &str) -> *mut AnyObject { unsafe { msg_send![class(c"UIImage"), systemImageNamed: ns(name)] } }
fn rect(x: f64, y: f64, w: f64, h: f64) -> CGRect { CGRect::new(CGPoint::new(x, y), CGSize::new(w, h)) }

// ---- 记录：最近用过的封面、裁过的封面

/// 一张裁出来的封面：它自己（`Cover::src`）、从哪张原图裁的、框在原图上的位置（0–1）、比例（`None` 是封面框原本的比例）。
#[derive(Clone, Serialize, Deserialize)]
struct Crop { src: String, original: String, rect: [f64; 4], ratio: Option<f64>, #[serde(default)] aspect: u8 }

fn crops_path() -> std::path::PathBuf { crate::store::Store::root_dir().join("cover-crops.json") }
fn recent_path() -> std::path::PathBuf { crate::store::Store::root_dir().join("recent-covers.json") }
thread_local! { static CROPS: RefCell<Option<Vec<Crop>>> = const { RefCell::new(None) }; }
thread_local! { static QUIET: Cell<bool> = const { Cell::new(false) }; }

fn with_crops<R>(f: impl FnOnce(&mut Vec<Crop>) -> R) -> R {
    CROPS.with(|c| {
        let mut c = c.borrow_mut();
        let list = c.get_or_insert_with(|| std::fs::read(crops_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default());
        f(list)
    })
}
fn crop_of(src: &str) -> Option<Crop> { with_crops(|l| l.iter().find(|c| c.src == src).cloned()) }
fn save_crop(crop: Crop) {
    with_crops(|l| {
        l.retain(|c| c.src != crop.src);
        l.push(crop);
        // 只留最近的一些：老的裁剪记录没人再打开。
        if l.len() > 200 { let n = l.len() - 200; l.drain(..n); }
        if let Ok(b) = serde_json::to_vec(l) { let _ = std::fs::write(crops_path(), b); }
    });
}

/// 裁成了别的比例的封面：宽比高。编辑器每帧问一次（`Editor::cover_h`），读的是内存里的一份。
pub fn aspect(src: &str) -> Option<f32> { with_crops(|l| l.iter().find(|c| c.src == src).and_then(|c| c.ratio).map(|r| r as f32)) }

/// 设了封面（`Editor::set_cover`）：记进最近用过的封面。裁出来的那张记它的原图。
pub fn remember(src: &str) {
    if QUIET.with(|q| q.get()) { return; }
    let src = crop_of(src).map_or_else(|| src.to_string(), |c| c.original);
    let mut list = recent_all();
    list.retain(|s| *s != src);
    list.insert(0, src);
    list.truncate(12);
    if let Ok(b) = serde_json::to_vec(&list) { let _ = std::fs::write(recent_path(), b); }
}
fn recent_all() -> Vec<String> { std::fs::read(recent_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default() }
/// 最近用过的封面（还在文档库里的），新的在前：样式面板封面页的那几格。
pub fn recent() -> Vec<(String, std::path::PathBuf)> {
    recent_all().into_iter().filter_map(|s| { let p = crate::store::image_path(&s).filter(|p| p.is_file())?; Some((s, p)) }).take(6).collect()
}
/// 封面页里哪一格是现在这张：裁过的算它的原图。
pub fn shown_original(src: &str) -> String { crop_of(src).map_or_else(|| src.to_string(), |c| c.original) }

// ---- 菜单

thread_local! { static BUTTON: Cell<*mut AnyObject> = const { Cell::new(std::ptr::null_mut()) }; }
thread_local! { static METAL: Cell<*mut AnyObject> = const { Cell::new(std::ptr::null_mut()) }; }
thread_local! { static FRAME: Cell<Option<Bounds<Pixels>>> = const { Cell::new(None) }; }

fn camera_available() -> bool { unsafe { msg_send![class(c"UIImagePickerController"), isSourceTypeAvailable: 1isize] } }

fn items() -> Vec<Item> {
    let camera = Item::new("covermenu-camera", tr("拍照", "Take Photo"), "camera", 1);
    vec![
        Item::new("covermenu-view", tr("视图", "View"), "arrow.up.left.and.arrow.down.right", 0),
        Item::new("covermenu-crop", tr("重新定位或裁剪", "Reposition or Crop"), "pencil", 0),
        Item::new("covermenu-pick", tr("选择一张图片", "Choose an Image"), "photo", 1),
        if camera_available() { camera } else { camera.disabled() },
        Item::new("covermenu-files", tr("浏览文件", "Browse Files"), "folder", 1),
        Item::new("covermenu-remove", tr("移除", "Remove"), "trash", 2).danger(),
    ]
}

/// 长按封面（`Editor::lift_block`）：马上弹出同一个菜单。菜单弹出时系统取消了这次触摸，抬手后不会再补点按。
pub fn long_press(frame: Bounds<Pixels>, window: &Window) { show(frame, window); }

/// 点了封面（`Editor::cover_el`）：透明按钮摆到封面上，弹出它的菜单。按钮平时不接触摸，滚动照常从封面上开始。
pub fn tap(frame: Bounds<Pixels>, window: &Window) { show(frame, window); }

fn show(frame: Bounds<Pixels>, window: &Window) {
    let Ok(handle) = HasWindowHandle::window_handle(window) else { return; };
    let RawWindowHandle::UiKit(handle) = handle.as_raw() else { return; };
    FRAME.with(|f| f.set(Some(frame)));
    unsafe {
        let mut button = BUTTON.with(|b| b.get());
        if button.is_null() {
            button = msg_send![class(c"UIButton"), buttonWithType: 0isize];
            let _: () = msg_send![button, setShowsMenuAsPrimaryAction: true];
            let _: () = msg_send![button, setUserInteractionEnabled: false];
            let _: *mut AnyObject = msg_send![button, retain];
            BUTTON.with(|b| b.set(button));
        }
        METAL.with(|m| m.set(handle.ui_view.as_ptr() as *mut AnyObject));
        let parent = super::shot::overlay(handle.ui_view.as_ptr() as *mut AnyObject);
        let sup: *mut AnyObject = msg_send![button, superview];
        if sup != parent { let _: () = msg_send![parent, addSubview: button]; }
        let _: () = msg_send![button, setFrame: rect(f64::from(frame.origin.x), f64::from(frame.origin.y), f64::from(frame.size.width), f64::from(frame.size.height))];
        let _: () = msg_send![button, setOverrideUserInterfaceStyle: if crate::theme::dark() { 2isize } else { 1isize }];
        let _: () = msg_send![button, setMenu: menu::native("", &items())];
        let ok: bool = msg_send![button, respondsToSelector: sel!(performPrimaryAction)];
        // 放到下一轮 run loop 再弹：菜单弹出时 UIKit 会当场取消这次触摸，这时还在 GPUI 的事件分发里，取消送不进去，之后的点按就都收不到了。
        if ok { let _: () = msg_send![button, performSelector: sel!(performPrimaryAction), withObject: std::ptr::null_mut::<AnyObject>(), afterDelay: 0f64]; }
    }
}

/// 菜单里点了一项（`ios::menu_action` 先问这里）。
pub fn menu_action(id: &str, w: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) -> bool {
    if !id.starts_with("covermenu-") { return false; }
    let Some(editor) = w.editor.clone() else { return true; };
    let document = w.current.clone();
    match id {
        "covermenu-view" => if let Some(path) = editor.read(cx).cover_src().and_then(|s| crate::store::image_path(&s)) && let Some(frame) = FRAME.with(|f| f.get()) {
            view(&path, frame, if crate::ios::style::banded() { 10. } else { 0. });
        },
        "covermenu-crop" => if let Some(src) = editor.read(cx).cover_src() {
            // 「默认」是没裁比例时封面本来的形状：宽照现在的框，高照 `cover_height`（不是现在的高，现在可能已经是 3:1）。
            let h = crate::editor::cover::cover_height(window);
            let ratio = editor.read(cx).cover_box().map_or(1.61, |(w, _)| f64::from(w / h.max(1.)));
            // 裁过的从原图接着改；原图不在了（文档库挪过、被删了）就拿现在这张从头裁。
            let found = |s: &str| crate::store::image_path(s).filter(|p| p.is_file());
            let (original, keep) = match crop_of(&src) { Some(c) if found(&c.original).is_some() => (c.original.clone(), Some(c)), _ => (src.clone(), None) };
            if let Some(path) = found(&original) { open_crop(&path, original, ratio, keep); }
        },
        "covermenu-pick" => { super::stylepanel::open_cover(); cx.notify(); }
        "covermenu-camera" => { camera(); }
        "covermenu-files" => {
            let rx = super::files::open_image_file();
            cx.spawn_in(window, async move |this, cx| {
                let Ok(Ok(Some(paths))) = rx.await else { return; };
                let Some(from) = paths.into_iter().next() else { return; };
                let ext = from.extension().and_then(|s| s.to_str()).unwrap_or("jpg").to_ascii_lowercase();
                let Ok(bytes) = std::fs::read(&from) else { return; };
                let _ = this.update(cx, |w, cx| set_image(w, &document, &bytes, &ext, cx));
            }).detach();
        }
        "covermenu-remove" => { super::glass::delete_haptic(); editor.update(cx, |e, cx| { e.set_cover_image(None, cx); }); cx.notify(); }
        _ => {}
    }
    true
}

fn set_image(w: &mut Workspace, document: &Option<String>, bytes: &[u8], ext: &str, cx: &mut Context<Workspace>) {
    if w.current != *document { return; }
    if let Some(editor) = w.editor.clone() { super::glass::selection_haptic(); editor.update(cx, |e, cx| { e.set_cover_image(Some((bytes, ext)), cx); }); }
    cx.notify();
}

// ---- 拍照

fn camera() {
    if !camera_available() { return; }
    unsafe {
        let root = super::files::root_controller();
        if root.is_null() { return; }
        let picker: *mut AnyObject = msg_send![class(c"UIImagePickerController"), new];
        let _: () = msg_send![picker, setSourceType: 1isize];
        let delegate = CAMERA.with(|c| { if c.get().is_null() { let d: *mut AnyObject = msg_send![camera_delegate(), new]; c.set(d); } c.get() });
        let _: () = msg_send![picker, setDelegate: delegate];
        let _: () = msg_send![root, presentViewController: picker, animated: true, completion: std::ptr::null_mut::<AnyObject>()];
        let _: () = msg_send![picker, release];
    }
}
thread_local! { static CAMERA: Cell<*mut AnyObject> = const { Cell::new(std::ptr::null_mut()) }; }

/// 相机拍的图方向只记在 EXIF 里，解码时不认：照 `size`（已转好方向）重画一遍再存 JPEG，长边不超过 `OUT_MAX`。
unsafe fn upright_jpeg(image: *mut AnyObject) -> *mut AnyObject {
    unsafe {
        let size: CGSize = msg_send![image, size];
        if size.width <= 0. || size.height <= 0. { return UIImageJPEGRepresentation(image, 0.9); }
        let k = (OUT_MAX / size.width.max(size.height)).min(1.);
        let (w, h) = ((size.width * k).round(), (size.height * k).round());
        let format: *mut AnyObject = msg_send![class(c"UIGraphicsImageRendererFormat"), preferredFormat];
        let _: () = msg_send![format, setScale: 1f64];
        let _: () = msg_send![format, setOpaque: true];
        let renderer: *mut AnyObject = msg_send![class(c"UIGraphicsImageRenderer"), alloc];
        let renderer: *mut AnyObject = msg_send![renderer, initWithSize: CGSize::new(w, h), format: format];
        let actions = block2::RcBlock::new(move |_ctx: *mut AnyObject| { let _: () = msg_send![image, drawInRect: rect(0., 0., w, h)]; });
        let data: *mut AnyObject = msg_send![renderer, JPEGDataWithCompressionQuality: 0.9f64, actions: &*actions];
        let _: () = msg_send![renderer, release];
        data
    }
}

fn camera_delegate() -> &'static AnyClass {
    static CLASS: std::sync::OnceLock<&'static AnyClass> = std::sync::OnceLock::new();
    CLASS.get_or_init(|| {
        extern "C-unwind" fn picked(_this: &AnyObject, _: Sel, picker: *mut AnyObject, info: *mut AnyObject) {
            let bytes = unsafe {
                let image: *mut AnyObject = msg_send![info, objectForKey: ns("UIImagePickerControllerOriginalImage")];
                let data = if image.is_null() { std::ptr::null_mut() } else { upright_jpeg(image) };
                let _: () = msg_send![picker, dismissViewControllerAnimated: true, completion: std::ptr::null_mut::<AnyObject>()];
                restack();
                if data.is_null() { return; }
                let ptr: *const std::ffi::c_void = msg_send![data, bytes];
                let len: usize = msg_send![data, length];
                std::slice::from_raw_parts(ptr as *const u8, len).to_vec()
            };
            menu::dispatch(move |w, _, cx| { let doc = w.current.clone(); set_image(w, &doc, &bytes, "jpg", cx); });
        }
        extern "C-unwind" fn cancelled(_this: &AnyObject, _: Sel, picker: *mut AnyObject) {
            unsafe { let _: () = msg_send![picker, dismissViewControllerAnimated: true, completion: std::ptr::null_mut::<AnyObject>()]; }
            restack();
        }
        let mut b = ClassBuilder::new(c"QuietMdCameraDelegate", class(c"NSObject")).expect("register QuietMdCameraDelegate");
        unsafe {
            b.add_method(sel!(imagePickerController:didFinishPickingMediaWithInfo:), picked as extern "C-unwind" fn(_, _, _, _));
            b.add_method(sel!(imagePickerControllerDidCancel:), cancelled as extern "C-unwind" fn(_, _, _));
        }
        b.register()
    })
}

// ---- 视图：全屏看图

struct Viewer { vc: *mut AnyObject, scroll: *mut AnyObject, image: *mut AnyObject, close: *mut AnyObject, share: *mut AnyObject, source: *mut AnyObject, path: std::path::PathBuf, chrome: Cell<bool>, size: Cell<(f64, f64)> }
thread_local! { static VIEWER: RefCell<Option<Viewer>> = const { RefCell::new(None) }; }

/// 全屏看一张图：从 `frame`（窗口里的位置，圆角 `radius`）放大出来。封面的「视图」和生成的图（`ios::imagine`）的「视图」都用它。
pub(super) fn view(path: &std::path::Path, frame: Bounds<Pixels>, radius: f64) {
    if VIEWER.with(|v| v.borrow().is_some()) { return; }
    unsafe {
        let root = super::files::root_controller();
        let image: *mut AnyObject = msg_send![class(c"UIImage"), imageWithContentsOfFile: ns(&path.to_string_lossy())];
        if root.is_null() || image.is_null() { return; }
        // 放大的起点：和封面一样摆着的一张图，盖在 Metal 上面（先 flush，免得新的一帧先出来）；收起后拿掉。
        let source: *mut AnyObject = msg_send![class(c"UIImageView"), alloc];
        let source: *mut AnyObject = msg_send![source, initWithImage: image];
        let _: () = msg_send![source, setContentMode: 2isize];
        let _: () = msg_send![source, setClipsToBounds: true];
        if radius > 0. { let l: *mut AnyObject = msg_send![source, layer]; let _: () = msg_send![l, setCornerRadius: radius]; }
        let _: () = msg_send![source, setFrame: rect(f64::from(frame.origin.x), f64::from(frame.origin.y), f64::from(frame.size.width), f64::from(frame.size.height))];
        let host: *mut AnyObject = msg_send![root, view];
        let _: () = msg_send![host, addSubview: source];
        let _: () = msg_send![class(c"CATransaction"), flush];

        let vc: *mut AnyObject = msg_send![viewer_class(), new];
        let view: *mut AnyObject = msg_send![vc, view];
        let _: () = msg_send![view, setBackgroundColor: color(0., 1.)];
        let _: () = msg_send![vc, setModalPresentationStyle: 0isize];
        let _: () = msg_send![vc, setOverrideUserInterfaceStyle: 2isize];
        let scroll = add(view, new_view(c"UIScrollView"));
        let _: () = msg_send![scroll, setDelegate: vc];
        let _: () = msg_send![scroll, setShowsHorizontalScrollIndicator: false];
        let _: () = msg_send![scroll, setShowsVerticalScrollIndicator: false];
        let _: () = msg_send![scroll, setContentInsetAdjustmentBehavior: 2isize];
        let _: () = msg_send![scroll, setAlwaysBounceVertical: true];
        let iv: *mut AnyObject = msg_send![class(c"UIImageView"), alloc];
        let iv: *mut AnyObject = msg_send![iv, initWithImage: image];
        add(scroll, iv);
        // 点一下换底色、出按钮；双击放大或复原。
        let double: *mut AnyObject = msg_send![class(c"UITapGestureRecognizer"), alloc];
        let double: *mut AnyObject = msg_send![double, initWithTarget: vc, action: sel!(viewerDouble:)];
        let _: () = msg_send![double, setNumberOfTapsRequired: 2usize];
        let _: () = msg_send![scroll, addGestureRecognizer: double];
        let single: *mut AnyObject = msg_send![class(c"UITapGestureRecognizer"), alloc];
        let single: *mut AnyObject = msg_send![single, initWithTarget: vc, action: sel!(viewerTap:)];
        let _: () = msg_send![single, requireGestureRecognizerToFail: double];
        let _: () = msg_send![scroll, addGestureRecognizer: single];
        let _: () = msg_send![single, release];
        let _: () = msg_send![double, release];
        let round = |name: &str, action: Sel| -> *mut AnyObject {
            let glass: bool = msg_send![class(c"UIButtonConfiguration"), respondsToSelector: sel!(glassButtonConfiguration)];
            let config: *mut AnyObject = if glass { msg_send![class(c"UIButtonConfiguration"), glassButtonConfiguration] } else { msg_send![class(c"UIButtonConfiguration"), grayButtonConfiguration] };
            let _: () = msg_send![config, setImage: symbol(name)];
            let _: () = msg_send![config, setCornerStyle: 4isize];
            let b: *mut AnyObject = msg_send![class(c"UIButton"), buttonWithConfiguration: config, primaryAction: std::ptr::null_mut::<AnyObject>()];
            let _: () = msg_send![b, addTarget: vc, action: action, forControlEvents: 64usize];
            let _: () = msg_send![b, setAlpha: 0f64];
            let _: () = msg_send![view, addSubview: b];
            b
        };
        let close = round("xmark", sel!(viewerClose:));
        let share = round("square.and.arrow.up", sel!(viewerShare:));
        // iOS 18 起：从封面放大出来，收起时缩回去（下滑也能收）。更早的系统淡入淡出。
        let zoom: bool = AnyClass::get(c"UIViewControllerTransition").is_some_and(|t| msg_send![t, respondsToSelector: sel!(zoomWithOptions:sourceViewProvider:)]);
        if zoom {
            let provider = block2::RcBlock::new(move |_ctx: *mut AnyObject| -> *mut AnyObject { VIEWER.with(|v| v.borrow().as_ref().map_or(std::ptr::null_mut(), |v| v.source)) });
            let transition: *mut AnyObject = msg_send![class(c"UIViewControllerTransition"), zoomWithOptions: std::ptr::null_mut::<AnyObject>(), sourceViewProvider: &*provider];
            let _: () = msg_send![vc, setPreferredTransition: transition];
        } else { let _: () = msg_send![vc, setModalTransitionStyle: 2isize]; }
        VIEWER.with(|v| *v.borrow_mut() = Some(Viewer { vc, scroll, image: iv, close, share, source, path: path.to_path_buf(), chrome: Cell::new(false), size: Cell::new((0., 0.)) }));
        let _: () = msg_send![root, presentViewController: vc, animated: true, completion: std::ptr::null_mut::<AnyObject>()];
    }
}

fn viewer_layout(view: *mut AnyObject) {
    VIEWER.with(|v| {
        let v = v.borrow();
        let Some(v) = v.as_ref() else { return };
        unsafe {
            let bounds: CGRect = msg_send![view, bounds];
            let (w, h) = (bounds.size.width, bounds.size.height);
            if w <= 0. || v.size.get() == (w, h) { return; }
            v.size.set((w, h));
            let safe: Insets = msg_send![view, safeAreaInsets];
            let _: () = msg_send![v.close, setFrame: rect(16., safe.top + 4., 44., 44.)];
            let _: () = msg_send![v.share, setFrame: rect(w - 60., safe.top + 4., 44., 44.)];
            let _: () = msg_send![v.scroll, setFrame: bounds];
            let image: *mut AnyObject = msg_send![v.image, image];
            let size: CGSize = msg_send![image, size];
            if size.width <= 0. || size.height <= 0. { return; }
            let _: () = msg_send![v.scroll, setZoomScale: 1f64];
            let _: () = msg_send![v.image, setFrame: rect(0., 0., size.width, size.height)];
            let _: () = msg_send![v.scroll, setContentSize: size];
            let fit = (w / size.width).min(h / size.height);
            let _: () = msg_send![v.scroll, setMinimumZoomScale: fit];
            let _: () = msg_send![v.scroll, setMaximumZoomScale: fit * 5.];
            let _: () = msg_send![v.scroll, setZoomScale: fit];
            centre(v.scroll, v.image);
        }
    });
}

/// 比屏幕小的那一边居中（缩放后也是）。
unsafe fn centre(scroll: *mut AnyObject, image: *mut AnyObject) {
    unsafe {
        let b: CGRect = msg_send![scroll, bounds];
        let f: CGRect = msg_send![image, frame];
        let x = ((b.size.width - f.size.width) / 2.).max(0.);
        let y = ((b.size.height - f.size.height) / 2.).max(0.);
        let _: () = msg_send![scroll, setContentInset: Insets { top: y, left: x, bottom: y, right: x }];
    }
}

fn viewer_class() -> &'static AnyClass {
    static CLASS: std::sync::OnceLock<&'static AnyClass> = std::sync::OnceLock::new();
    CLASS.get_or_init(|| {
        extern "C-unwind" fn laid_out(this: &AnyObject, _: Sel) {
            let _: () = unsafe { msg_send![super(this, class(c"UIViewController")), viewDidLayoutSubviews] };
            let view: *mut AnyObject = unsafe { msg_send![this, view] };
            viewer_layout(view);
        }
        extern "C-unwind" fn zoomed(_this: &AnyObject, _: Sel, _scroll: *mut AnyObject) -> *mut AnyObject { VIEWER.with(|v| v.borrow().as_ref().map_or(std::ptr::null_mut(), |v| v.image)) }
        extern "C-unwind" fn did_zoom(_this: &AnyObject, _: Sel, scroll: *mut AnyObject) { VIEWER.with(|v| if let Some(v) = v.borrow().as_ref() { unsafe { centre(scroll, v.image) } }); }
        extern "C-unwind" fn dragged(_this: &AnyObject, _: Sel, scroll: *mut AnyObject, _decelerate: Bool) {
            // 没放大时往下拉一段就收起。
            unsafe {
                let zoom: f64 = msg_send![scroll, zoomScale];
                let min: f64 = msg_send![scroll, minimumZoomScale];
                let off: CGPoint = msg_send![scroll, contentOffset];
                let inset: Insets = msg_send![scroll, contentInset];
                if zoom <= min * 1.01 && off.y + inset.top < -90. { close_viewer(); }
            }
        }
        extern "C-unwind" fn tapped(_this: &AnyObject, _: Sel, _g: *mut AnyObject) {
            VIEWER.with(|v| if let Some(v) = v.borrow().as_ref() {
                let on = !v.chrome.get();
                v.chrome.set(on);
                unsafe {
                    let view: *mut AnyObject = msg_send![v.vc, view];
                    let (close, share) = (v.close, v.share);
                    // 照录屏：出按钮时是白底（按钮跟着换成浅色的玻璃），收起按钮回到黑底。
                    let bg: *mut AnyObject = color(if on { 1. } else { 0. }, 1.);
                    let _: () = msg_send![v.vc, setOverrideUserInterfaceStyle: if on { 1isize } else { 2isize }];
                    let _: *mut AnyObject = msg_send![bg, retain];
                    let anim = block2::RcBlock::new(move || {
                        let _: () = msg_send![view, setBackgroundColor: bg];
                        let _: () = msg_send![close, setAlpha: if on { 1f64 } else { 0f64 }];
                        let _: () = msg_send![share, setAlpha: if on { 1f64 } else { 0f64 }];
                    });
                    let _: () = msg_send![class(c"UIView"), animateWithDuration: 0.25f64, animations: &*anim];
                    let _: () = msg_send![bg, release];
                }
            });
        }
        extern "C-unwind" fn double(_this: &AnyObject, _: Sel, g: *mut AnyObject) {
            VIEWER.with(|v| if let Some(v) = v.borrow().as_ref() { unsafe {
                let zoom: f64 = msg_send![v.scroll, zoomScale];
                let min: f64 = msg_send![v.scroll, minimumZoomScale];
                if zoom > min * 1.01 { let _: () = msg_send![v.scroll, setZoomScale: min, animated: true]; return; }
                let at: CGPoint = msg_send![g, locationInView: v.image];
                let k = min * 2.5;
                let b: CGRect = msg_send![v.scroll, bounds];
                let (w, h) = (b.size.width / k, b.size.height / k);
                let _: () = msg_send![v.scroll, zoomToRect: rect(at.x - w / 2., at.y - h / 2., w, h), animated: true];
            } });
        }
        extern "C-unwind" fn close(_this: &AnyObject, _: Sel, _s: *mut AnyObject) { close_viewer(); }
        extern "C-unwind" fn share(_this: &AnyObject, _: Sel, sender: *mut AnyObject) {
            VIEWER.with(|v| if let Some(v) = v.borrow().as_ref() { unsafe {
                let url: *mut AnyObject = msg_send![class(c"NSURL"), fileURLWithPath: ns(&v.path.to_string_lossy())];
                let items: *mut AnyObject = msg_send![class(c"NSArray"), arrayWithObject: url];
                let sheet: *mut AnyObject = msg_send![class(c"UIActivityViewController"), alloc];
                let sheet: *mut AnyObject = msg_send![sheet, initWithActivityItems: items, applicationActivities: std::ptr::null_mut::<AnyObject>()];
                let pop: *mut AnyObject = msg_send![sheet, popoverPresentationController];
                if !pop.is_null() { let _: () = msg_send![pop, setSourceView: sender]; }
                let _: () = msg_send![v.vc, presentViewController: sheet, animated: true, completion: std::ptr::null_mut::<AnyObject>()];
                let _: () = msg_send![sheet, release];
            } });
        }
        extern "C-unwind" fn gone(this: &AnyObject, _: Sel, animated: Bool) {
            let _: () = unsafe { msg_send![super(this, class(c"UIViewController")), viewDidDisappear: animated] };
            // 正在分享时这一页也会「消失」一下：只在它真的被收起时收拾。
            let dismissed: bool = unsafe { msg_send![this, isBeingDismissed] };
            let gone: bool = unsafe { let p: *mut AnyObject = msg_send![this, presentingViewController]; p.is_null() };
            if !(dismissed || gone) { return; }
            if let Some(v) = VIEWER.with(|v| v.borrow_mut().take()) { unsafe {
                let _: () = msg_send![v.source, removeFromSuperview];
                let _: () = msg_send![v.source, release];
                let _: () = msg_send![v.vc, release];
            } }
            restack();
        }
        let mut b = ClassBuilder::new(c"QuietMdCoverViewer", class(c"UIViewController")).expect("register QuietMdCoverViewer");
        unsafe {
            b.add_method(sel!(viewDidLayoutSubviews), laid_out as extern "C-unwind" fn(_, _));
            b.add_method(sel!(viewForZoomingInScrollView:), zoomed as extern "C-unwind" fn(_, _, _) -> _);
            b.add_method(sel!(scrollViewDidZoom:), did_zoom as extern "C-unwind" fn(_, _, _));
            b.add_method(sel!(scrollViewDidEndDragging:willDecelerate:), dragged as extern "C-unwind" fn(_, _, _, _));
            b.add_method(sel!(viewerTap:), tapped as extern "C-unwind" fn(_, _, _));
            b.add_method(sel!(viewerDouble:), double as extern "C-unwind" fn(_, _, _));
            b.add_method(sel!(viewerClose:), close as extern "C-unwind" fn(_, _, _));
            b.add_method(sel!(viewerShare:), share as extern "C-unwind" fn(_, _, _));
            b.add_method(sel!(viewDidDisappear:), gone as extern "C-unwind" fn(_, _, _));
        }
        b.register()
    })
}

/// 全屏弹出的页面收起后，系统把根视图重新放回窗口时会压在玻璃那一层上面（玻璃和菜单按钮是 Metal 视图的兄弟）：
/// 放回 Metal 视图上面，再让 GPUI 画一帧，玻璃照常露面。
fn restack() {
    let metal = METAL.with(|m| m.get());
    if !metal.is_null() { unsafe {
        let sup: *mut AnyObject = msg_send![metal, superview];
        let overlay = super::shot::overlay(metal);
        if !sup.is_null() && overlay != metal { let _: () = msg_send![sup, insertSubview: overlay, aboveSubview: metal]; }
    } }
    menu::dispatch(|_, _, cx| cx.notify());
}

fn close_viewer() {
    let vc = VIEWER.with(|v| v.borrow().as_ref().map(|v| v.vc));
    if let Some(vc) = vc { unsafe { let _: () = msg_send![vc, dismissViewControllerAnimated: true, completion: std::ptr::null_mut::<AnyObject>()]; } }
}

// ---- 重新定位或裁剪

/// 比例菜单：标题、图标、宽比高（自由形式和默认另算）。
const ASPECTS: [(&str, &str, &str, f64); 6] = [
    ("4:3", "4:3", "rectangle.ratio.4.to.3", 4. / 3.), ("3:1", "3:1", "rectangle", 3.), ("2:1", "2:1", "rectangle", 2.),
    ("16:9", "16:9", "rectangle.ratio.16.to.9", 16. / 9.), ("自由形式", "Freeform", "rectangle.dashed", 0.), ("默认", "Default", "rectangle.split.2x2", 0.),
];
const FREE: u8 = 4;
const DEFAULT: u8 = 5;
/// 底下那条的高（不含安全区），框离四周的空。
const BAR: f64 = 64.;
const SIDE: f64 = 10.;
/// 裁出来的图最长边（像素）。
const OUT_MAX: f64 = 2400.;

struct Cropper {
    vc: *mut AnyObject, scroll: *mut AnyObject, image: *mut AnyObject, blur: *mut AnyObject, hole: *mut AnyObject, frame: *mut AnyObject, corners: *mut AnyObject,
    handles: [*mut AnyObject; 4], bar: *mut AnyObject, cancel: *mut AnyObject, ratio_button: *mut AnyObject, done: *mut AnyObject,
    original: String, default_ratio: f64, keep: Option<Crop>,
    aspect: Cell<u8>, rect: Cell<CGRect>, area: Cell<CGRect>, size: Cell<(f64, f64)>, grab: Cell<CGRect>,
}
thread_local! { static CROPPER: RefCell<Option<Cropper>> = const { RefCell::new(None) }; }

fn open_crop(path: &std::path::Path, original: String, default_ratio: f64, keep: Option<Crop>) {
    if CROPPER.with(|c| c.borrow().is_some()) { return; }
    unsafe {
        let root = super::files::root_controller();
        let image: *mut AnyObject = msg_send![class(c"UIImage"), imageWithContentsOfFile: ns(&path.to_string_lossy())];
        if root.is_null() || image.is_null() { return; }
        let vc: *mut AnyObject = msg_send![crop_class(), new];
        let view: *mut AnyObject = msg_send![vc, view];
        let _: () = msg_send![view, setBackgroundColor: color(0.08, 1.)];
        // 照片在暗底上看；拖图片时容易误收，不让下滑收起。1 = 页面式的面板。
        let _: () = msg_send![vc, setOverrideUserInterfaceStyle: 2isize];
        let _: () = msg_send![vc, setModalInPresentation: true];
        let _: () = msg_send![vc, setModalPresentationStyle: 1isize];

        let scroll = add(view, new_view(c"UIScrollView"));
        let _: () = msg_send![scroll, setDelegate: vc];
        let _: () = msg_send![scroll, setShowsHorizontalScrollIndicator: false];
        let _: () = msg_send![scroll, setShowsVerticalScrollIndicator: false];
        let _: () = msg_send![scroll, setContentInsetAdjustmentBehavior: 2isize];
        let _: () = msg_send![scroll, setAlwaysBounceVertical: true];
        let _: () = msg_send![scroll, setAlwaysBounceHorizontal: true];
        let iv: *mut AnyObject = msg_send![class(c"UIImageView"), alloc];
        let iv: *mut AnyObject = msg_send![iv, initWithImage: image];
        add(scroll, iv);

        // 框外：压暗的薄毛玻璃（16 = 超薄深色材质：照录屏，框外的图还认得出），用一块挖了洞的遮罩只盖框外。
        let effect: *mut AnyObject = msg_send![class(c"UIBlurEffect"), effectWithStyle: 16isize];
        let blur: *mut AnyObject = msg_send![class(c"UIVisualEffectView"), alloc];
        let blur: *mut AnyObject = msg_send![blur, initWithEffect: effect];
        add(view, blur);
        let _: () = msg_send![blur, setUserInteractionEnabled: false];
        let mask_view = new_view(c"UIView");
        let hole: *mut AnyObject = msg_send![class(c"CAShapeLayer"), layer];
        let _: () = msg_send![hole, setFillRule: ns("even-odd")];
        let black: *const CGColor = msg_send![color(0., 1.), CGColor];
        let _: () = msg_send![hole, setFillColor: black];
        let ml: *mut AnyObject = msg_send![mask_view, layer];
        let _: () = msg_send![ml, addSublayer: hole];
        let _: () = msg_send![blur, setMaskView: mask_view];
        let _: () = msg_send![mask_view, release];

        let frame = add(view, new_view(c"UIView"));
        let _: () = msg_send![frame, setUserInteractionEnabled: false];
        let fl: *mut AnyObject = msg_send![frame, layer];
        let _: () = msg_send![fl, setBorderWidth: 1f64];
        let border: *const CGColor = msg_send![color(1., 0.55), CGColor];
        let _: () = msg_send![fl, setBorderColor: border];
        // 四个角：粗一点的直角线。
        let corners: *mut AnyObject = msg_send![class(c"CAShapeLayer"), layer];
        let white: *const CGColor = msg_send![color(1., 1.), CGColor];
        let clear: *const CGColor = msg_send![color(0., 0.), CGColor];
        let _: () = msg_send![corners, setStrokeColor: white];
        let _: () = msg_send![corners, setFillColor: clear];
        let _: () = msg_send![corners, setLineWidth: 3f64];
        let vl: *mut AnyObject = msg_send![view, layer];
        let _: () = msg_send![vl, addSublayer: corners];
        // 角上各一块看不见的把手：拖它改框。
        let handles = [0isize, 1, 2, 3].map(|i| {
            let h = add(view, new_view(c"UIView"));
            let _: () = msg_send![h, setTag: i];
            let pan: *mut AnyObject = msg_send![class(c"UIPanGestureRecognizer"), alloc];
            let pan: *mut AnyObject = msg_send![pan, initWithTarget: vc, action: sel!(cropHandle:)];
            let _: () = msg_send![h, addGestureRecognizer: pan];
            let _: () = msg_send![pan, release];
            h
        });

        let bar = add(view, new_view(c"UIView"));
        let _: () = msg_send![bar, setBackgroundColor: color(0.06, 1.)];
        let cancel: *mut AnyObject = msg_send![class(c"UIButton"), buttonWithType: 1isize];
        let _: () = msg_send![cancel, setTitle: ns(tr("取消", "Cancel")), forState: 0usize];
        let _: () = msg_send![cancel, setTintColor: color(1., 1.)];
        let _: () = msg_send![cancel, addTarget: vc, action: sel!(cropCancel:), forControlEvents: 64usize];
        let _: () = msg_send![bar, addSubview: cancel];
        let gray: *mut AnyObject = msg_send![class(c"UIButtonConfiguration"), grayButtonConfiguration];
        let _: () = msg_send![gray, setImage: symbol("rectangle.split.2x2")];
        // 2 = 弹出菜单的上下小箭头（UIButtonConfigurationIndicatorPopup）
        let _: () = msg_send![gray, setIndicator: 2isize];
        let ratio_button: *mut AnyObject = msg_send![class(c"UIButton"), buttonWithConfiguration: gray, primaryAction: std::ptr::null_mut::<AnyObject>()];
        let _: () = msg_send![ratio_button, setShowsMenuAsPrimaryAction: true];
        // 照录屏：往上弹出时也是 4:3 在上、「重置」在最下（2 = 按给的顺序，不因方向倒过来）。
        let _: () = msg_send![ratio_button, setPreferredMenuElementOrder: 2isize];
        let _: () = msg_send![bar, addSubview: ratio_button];
        let filled: *mut AnyObject = msg_send![class(c"UIButtonConfiguration"), filledButtonConfiguration];
        let _: () = msg_send![filled, setTitle: ns(tr("完成", "Done"))];
        let _: () = msg_send![filled, setCornerStyle: 4isize];
        let done: *mut AnyObject = msg_send![class(c"UIButton"), buttonWithConfiguration: filled, primaryAction: std::ptr::null_mut::<AnyObject>()];
        let _: () = msg_send![done, addTarget: vc, action: sel!(cropDone:), forControlEvents: 64usize];
        let _: () = msg_send![bar, addSubview: done];

        let aspect = keep.as_ref().map_or(DEFAULT, |k| k.aspect.min(DEFAULT));
        CROPPER.with(|c| *c.borrow_mut() = Some(Cropper {
            vc, scroll, image: iv, blur, hole, frame, corners, handles, bar, cancel, ratio_button, done, original, default_ratio, keep,
            aspect: Cell::new(aspect), rect: Cell::new(CGRect::default()), area: Cell::new(CGRect::default()), size: Cell::new((0., 0.)), grab: Cell::new(CGRect::default()),
        }));
        ratio_menu();
        let _: () = msg_send![root, presentViewController: vc, animated: true, completion: std::ptr::null_mut::<AnyObject>()];
    }
}

/// 比例菜单（打勾的是现在的）：点了换框；「重置」回到默认比例、整张图。
fn ratio_menu() {
    CROPPER.with(|c| {
        let c = c.borrow();
        let Some(c) = c.as_ref() else { return };
        unsafe {
            let list: *mut AnyObject = msg_send![class(c"NSMutableArray"), array];
            for (i, (zh, en, icon, _)) in ASPECTS.iter().enumerate() {
                let handler = block2::RcBlock::new(move |_a: *mut AnyObject| { set_aspect(i as u8, false); });
                let action: *mut AnyObject = msg_send![class(c"UIAction"), actionWithTitle: ns(tr(zh, en)), image: symbol(icon), identifier: std::ptr::null_mut::<AnyObject>(), handler: &*handler];
                if c.aspect.get() == i as u8 { let _: () = msg_send![action, setState: 1isize]; }
                let _: () = msg_send![list, addObject: action];
            }
            let group: *mut AnyObject = msg_send![class(c"UIMenu"), menuWithTitle: ns(""), image: std::ptr::null_mut::<AnyObject>(), identifier: std::ptr::null_mut::<AnyObject>(), options: 1usize, children: list];
            let reset = block2::RcBlock::new(|_a: *mut AnyObject| { set_aspect(DEFAULT, true); });
            let reset: *mut AnyObject = msg_send![class(c"UIAction"), actionWithTitle: ns(tr("重置", "Reset")), image: symbol("arrow.counterclockwise"), identifier: std::ptr::null_mut::<AnyObject>(), handler: &*reset];
            let tail: *mut AnyObject = msg_send![class(c"NSArray"), arrayWithObject: reset];
            let tail: *mut AnyObject = msg_send![class(c"UIMenu"), menuWithTitle: ns(""), image: std::ptr::null_mut::<AnyObject>(), identifier: std::ptr::null_mut::<AnyObject>(), options: 1usize, children: tail];
            let both: *mut AnyObject = msg_send![class(c"NSMutableArray"), arrayWithObject: group];
            let _: () = msg_send![both, addObject: tail];
            let menu: *mut AnyObject = msg_send![class(c"UIMenu"), menuWithTitle: ns(""), image: std::ptr::null_mut::<AnyObject>(), identifier: std::ptr::null_mut::<AnyObject>(), options: 0usize, children: both];
            let _: () = msg_send![c.ratio_button, setMenu: menu];
        }
    });
}

impl Cropper {
    fn ratio(&self, aspect: u8) -> f64 {
        match aspect { DEFAULT => self.default_ratio, FREE => { let r = self.rect.get(); if r.size.height > 0. { r.size.width / r.size.height } else { self.default_ratio } } a => ASPECTS[a as usize].3 }
    }
    /// 这个比例在可用区域里最大的框，居中。
    fn fit(&self, ratio: f64) -> CGRect {
        let a = self.area.get();
        let (mut w, mut h) = (a.size.width, a.size.width / ratio);
        if h > a.size.height { h = a.size.height; w = h * ratio; }
        rect(a.origin.x + (a.size.width - w) / 2., a.origin.y + (a.size.height - h) / 2., w, h)
    }
    fn image_size(&self) -> CGSize { unsafe { let i: *mut AnyObject = msg_send![self.image, image]; msg_send![i, size] } }
    /// 框里正中的那一点在图上（图片的点）。
    fn focus(&self) -> CGPoint {
        unsafe {
            let r = self.rect.get();
            let zoom: f64 = msg_send![self.scroll, zoomScale];
            let off: CGPoint = msg_send![self.scroll, contentOffset];
            CGPoint::new((off.x + r.origin.x + r.size.width / 2.) / zoom, (off.y + r.origin.y + r.size.height / 2.) / zoom)
        }
    }
    /// 换框：遮罩、边框、角、把手、滚动的内边距和最小缩放都跟着换；`focus` 是要放在框正中的图上那一点。
    fn set_rect(&self, r: CGRect, focus: Option<CGPoint>) {
        self.rect.set(r);
        unsafe {
            let bounds: CGRect = msg_send![self.vc_view(), bounds];
            let path: *mut AnyObject = msg_send![class(c"UIBezierPath"), bezierPathWithRect: bounds];
            let inner: *mut AnyObject = msg_send![class(c"UIBezierPath"), bezierPathWithRect: r];
            let _: () = msg_send![path, appendPath: inner];
            let cg: *mut CGPath = msg_send![path, CGPath];
            let _: () = msg_send![class(c"CATransaction"), begin];
            let _: () = msg_send![class(c"CATransaction"), setDisableActions: true];
            let _: () = msg_send![self.hole, setPath: cg];
            let _: () = msg_send![self.frame, setFrame: r];
            let (x0, y0, x1, y1, l) = (r.origin.x - 1.5, r.origin.y - 1.5, r.origin.x + r.size.width + 1.5, r.origin.y + r.size.height + 1.5, 18f64);
            let p: *mut AnyObject = msg_send![class(c"UIBezierPath"), bezierPath];
            for (cx, cy, dx, dy) in [(x0, y0, 1., 1.), (x1, y0, -1., 1.), (x0, y1, 1., -1.), (x1, y1, -1., -1.)] {
                let _: () = msg_send![p, moveToPoint: CGPoint::new(cx + dx * l, cy)];
                let _: () = msg_send![p, addLineToPoint: CGPoint::new(cx, cy)];
                let _: () = msg_send![p, addLineToPoint: CGPoint::new(cx, cy + dy * l)];
            }
            let cg: *mut CGPath = msg_send![p, CGPath];
            let _: () = msg_send![self.corners, setPath: cg];
            let _: () = msg_send![class(c"CATransaction"), commit];
            for (i, h) in self.handles.iter().enumerate() {
                let (hx, hy) = (if i % 2 == 0 { r.origin.x } else { r.origin.x + r.size.width }, if i < 2 { r.origin.y } else { r.origin.y + r.size.height });
                let _: () = msg_send![*h, setFrame: rect(hx - 22., hy - 22., 44., 44.)];
            }
            let (w, h) = (bounds.size.width, bounds.size.height);
            let _: () = msg_send![self.scroll, setContentInset: Insets { top: r.origin.y, left: r.origin.x, bottom: h - r.origin.y - r.size.height, right: w - r.origin.x - r.size.width }];
            let size = self.image_size();
            if size.width <= 0. || size.height <= 0. { return; }
            let min = (r.size.width / size.width).max(r.size.height / size.height);
            let _: () = msg_send![self.scroll, setMinimumZoomScale: min];
            let _: () = msg_send![self.scroll, setMaximumZoomScale: min * 8.];
            let zoom: f64 = msg_send![self.scroll, zoomScale];
            if zoom < min { let _: () = msg_send![self.scroll, setZoomScale: min]; }
            if let Some(f) = focus { self.centre_on(f); } else { self.clamp(); }
        }
    }
    fn vc_view(&self) -> *mut AnyObject { unsafe { msg_send![self.vc, view] } }
    fn centre_on(&self, f: CGPoint) {
        unsafe {
            let r = self.rect.get();
            let zoom: f64 = msg_send![self.scroll, zoomScale];
            let _: () = msg_send![self.scroll, setContentOffset: CGPoint::new(f.x * zoom - r.origin.x - r.size.width / 2., f.y * zoom - r.origin.y - r.size.height / 2.)];
            self.clamp();
        }
    }
    /// 图片不能拖得露出框：偏移限在内边距允许的范围里。
    fn clamp(&self) {
        unsafe {
            let r = self.rect.get();
            let content: CGSize = msg_send![self.scroll, contentSize];
            let off: CGPoint = msg_send![self.scroll, contentOffset];
            let x = off.x.max(-r.origin.x).min(content.width - r.origin.x - r.size.width);
            let y = off.y.max(-r.origin.y).min(content.height - r.origin.y - r.size.height);
            let _: () = msg_send![self.scroll, setContentOffset: CGPoint::new(x, y)];
        }
    }
}

fn set_aspect(aspect: u8, reset: bool) {
    CROPPER.with(|c| {
        let c = c.borrow();
        let Some(c) = c.as_ref() else { return };
        super::glass::selection_haptic();
        let focus = if reset { let s = c.image_size(); CGPoint::new(s.width / 2., s.height / 2.) } else { c.focus() };
        let ratio = c.ratio(aspect);
        c.aspect.set(aspect);
        let r = c.fit(ratio);
        let animate = block2::RcBlock::new(move || {
            CROPPER.with(|c| if let Some(c) = c.borrow().as_ref() {
                // 换比例：图先缩到刚好盖满新框，框正中还是原来那一点。
                unsafe { let _: () = msg_send![c.scroll, setZoomScale: 0.0001f64]; }
                c.set_rect(r, Some(focus));
            });
        });
        unsafe { let _: () = msg_send![class(c"UIView"), animateWithDuration: 0.3f64, animations: &*animate]; }
    });
    ratio_menu();
}

fn crop_layout(view: *mut AnyObject) {
    CROPPER.with(|c| {
        let c = c.borrow();
        let Some(c) = c.as_ref() else { return };
        unsafe {
            let bounds: CGRect = msg_send![view, bounds];
            let (w, h) = (bounds.size.width, bounds.size.height);
            if w <= 0. || c.size.get() == (w, h) { return; }
            c.size.set((w, h));
            let safe: Insets = msg_send![view, safeAreaInsets];
            let bar_h = BAR + safe.bottom;
            let _: () = msg_send![c.bar, setFrame: rect(0., h - bar_h, w, bar_h)];
            let _: () = msg_send![c.cancel, setFrame: rect(8., 0., 80., BAR)];
            let _: () = msg_send![c.ratio_button, setFrame: rect(w / 2. - 38., (BAR - 34.) / 2., 76., 34.)];
            let _: () = msg_send![c.done, sizeToFit];
            let f: CGRect = msg_send![c.done, frame];
            let dw = f.size.width.max(64.);
            let _: () = msg_send![c.done, setFrame: rect(w - 16. - dw, (BAR - 34.) / 2., dw, 34.)];
            let _: () = msg_send![c.blur, setFrame: bounds];
            let mask: *mut AnyObject = msg_send![c.blur, maskView];
            let _: () = msg_send![mask, setFrame: bounds];
            let _: () = msg_send![c.scroll, setFrame: bounds];
            c.area.set(rect(SIDE, safe.top + 56., w - SIDE * 2., h - bar_h - safe.top - 56. - 56.));

            let size = c.image_size();
            if size.width <= 0. || size.height <= 0. { return; }
            let _: () = msg_send![c.scroll, setZoomScale: 1f64];
            let _: () = msg_send![c.image, setFrame: rect(0., 0., size.width, size.height)];
            let _: () = msg_send![c.scroll, setContentSize: size];
            // 接着上一次的框：比例、缩放和位置都按原图上的那块摆回来。
            let (ratio, prior) = match &c.keep {
                Some(k) if k.rect[2] > 0. && k.rect[3] > 0. => (match c.aspect.get() { FREE => (k.rect[2] * size.width) / (k.rect[3] * size.height), a => c.ratio(a) }, Some(k.rect)),
                _ => (c.ratio(c.aspect.get()), None),
            };
            let r = c.fit(ratio);
            let _: () = msg_send![c.scroll, setZoomScale: 0.0001f64];
            c.set_rect(r, None);
            match prior {
                // 形状对不上（旧记录、「默认」的形状变了）就铺满框、对准原来那块的正中，不跑到别处。
                Some([x, y, pw, ph]) => {
                    let zoom = (r.size.width / (pw * size.width)).max(r.size.height / (ph * size.height)).max(msg_send![c.scroll, minimumZoomScale]);
                    let _: () = msg_send![c.scroll, setZoomScale: zoom];
                    c.centre_on(CGPoint::new((x + pw / 2.) * size.width, (y + ph / 2.) * size.height));
                }
                None => c.centre_on(CGPoint::new(size.width / 2., size.height / 2.)),
            }
        }
    });
}

/// 拖角改框：固定比例时按横向拖的量保比例，自由形式各管各的；不出可用区域，不小于 60 点。
fn drag_handle(g: *mut AnyObject) {
    CROPPER.with(|c| {
        let c = c.borrow();
        let Some(c) = c.as_ref() else { return };
        unsafe {
            let state: isize = msg_send![g, state];
            let view: *mut AnyObject = msg_send![g, view];
            let corner: isize = msg_send![view, tag];
            if state == 1 { c.grab.set(c.rect.get()); return; }
            if state != 2 { return; }
            let t: CGPoint = msg_send![g, translationInView: c.vc_view()];
            let (s, a) = (c.grab.get(), c.area.get());
            let (mut x0, mut y0, mut x1, mut y1) = (s.origin.x, s.origin.y, s.origin.x + s.size.width, s.origin.y + s.size.height);
            let (left, top) = (corner % 2 == 0, corner < 2);
            if left { x0 += t.x } else { x1 += t.x }
            if top { y0 += t.y } else { y1 += t.y }
            x0 = x0.max(a.origin.x); y0 = y0.max(a.origin.y);
            x1 = x1.min(a.origin.x + a.size.width); y1 = y1.min(a.origin.y + a.size.height);
            if x1 - x0 < 60. { if left { x0 = x1 - 60. } else { x1 = x0 + 60. } }
            if y1 - y0 < 60. { if top { y0 = y1 - 60. } else { y1 = y0 + 60. } }
            if c.aspect.get() != FREE {
                let ratio = s.size.width / s.size.height.max(1.);
                let h = (x1 - x0) / ratio;
                if top { y0 = y1 - h } else { y1 = y0 + h }
                if y0 < a.origin.y || y1 > a.origin.y + a.size.height { return; }
            }
            c.set_rect(rect(x0, y0, x1 - x0, y1 - y0), None);
        }
    });
}

/// 框里的那块画成新图（JPEG），连同它在原图上的位置交回。
fn render() -> Option<(Vec<u8>, [f64; 4], Option<f64>, u8)> {
    CROPPER.with(|c| {
        let c = c.borrow();
        let c = c.as_ref()?;
        unsafe {
            let image: *mut AnyObject = msg_send![c.image, image];
            let size: CGSize = msg_send![image, size];
            let scale: f64 = msg_send![image, scale];
            let zoom: f64 = msg_send![c.scroll, zoomScale];
            let off: CGPoint = msg_send![c.scroll, contentOffset];
            let r = c.rect.get();
            if zoom <= 0. || r.size.width <= 0. { return None; }
            let (x, y, w, h) = ((off.x + r.origin.x) / zoom, (off.y + r.origin.y) / zoom, r.size.width / zoom, r.size.height / zoom);
            let k = ((w.max(h) * scale).min(OUT_MAX) / w.max(h)).max(0.01);
            let (ow, oh) = ((w * k).round().max(1.), (h * k).round().max(1.));
            let format: *mut AnyObject = msg_send![class(c"UIGraphicsImageRendererFormat"), preferredFormat];
            let _: () = msg_send![format, setScale: 1f64];
            let _: () = msg_send![format, setOpaque: true];
            let renderer: *mut AnyObject = msg_send![class(c"UIGraphicsImageRenderer"), alloc];
            let renderer: *mut AnyObject = msg_send![renderer, initWithSize: CGSize::new(ow, oh), format: format];
            let draw = rect(-x * k, -y * k, size.width * k, size.height * k);
            let actions = block2::RcBlock::new(move |_ctx: *mut AnyObject| { let _: () = msg_send![image, drawInRect: draw]; });
            let data: *mut AnyObject = msg_send![renderer, JPEGDataWithCompressionQuality: 0.9f64, actions: &*actions];
            let _: () = msg_send![renderer, release];
            if data.is_null() { return None; }
            let ptr: *const std::ffi::c_void = msg_send![data, bytes];
            let len: usize = msg_send![data, length];
            let aspect = c.aspect.get();
            let ratio = (aspect != DEFAULT).then(|| w / h);
            Some((std::slice::from_raw_parts(ptr as *const u8, len).to_vec(), [x / size.width, y / size.height, w / size.width, h / size.height], ratio, aspect))
        }
    })
}

fn finish_crop(done: bool) {
    let result = if done { render() } else { None };
    let Some(c) = CROPPER.with(|c| c.borrow_mut().take()) else { return };
    unsafe {
        let _: () = msg_send![c.vc, dismissViewControllerAnimated: true, completion: std::ptr::null_mut::<AnyObject>()];
        let _: () = msg_send![c.vc, release];
    }
    let Some((bytes, rect, ratio, aspect)) = result else { return };
    let original = c.original;
    menu::dispatch(move |w, _, cx| {
        let Some(editor) = w.editor.clone() else { return };
        super::glass::selection_haptic();
        // 裁出来的这张不进「最近用过的封面」：那里记的是原图。
        QUIET.with(|q| q.set(true));
        let ok = editor.update(cx, |e, cx| e.set_cover_image(Some((&bytes, "jpg")), cx));
        QUIET.with(|q| q.set(false));
        if ok && let Some(src) = editor.read(cx).cover_src() { save_crop(Crop { src, original, rect, ratio, aspect }); }
        cx.notify();
    });
}

fn crop_class() -> &'static AnyClass {
    static CLASS: std::sync::OnceLock<&'static AnyClass> = std::sync::OnceLock::new();
    CLASS.get_or_init(|| {
        extern "C-unwind" fn laid_out(this: &AnyObject, _: Sel) {
            let _: () = unsafe { msg_send![super(this, class(c"UIViewController")), viewDidLayoutSubviews] };
            let view: *mut AnyObject = unsafe { msg_send![this, view] };
            crop_layout(view);
        }
        extern "C-unwind" fn zoomed(_this: &AnyObject, _: Sel, _scroll: *mut AnyObject) -> *mut AnyObject { CROPPER.with(|c| c.borrow().as_ref().map_or(std::ptr::null_mut(), |c| c.image)) }
        extern "C-unwind" fn handle(_this: &AnyObject, _: Sel, g: *mut AnyObject) { drag_handle(g); }
        extern "C-unwind" fn cancel(_this: &AnyObject, _: Sel, _s: *mut AnyObject) { finish_crop(false); }
        extern "C-unwind" fn done(_this: &AnyObject, _: Sel, _s: *mut AnyObject) { finish_crop(true); }
        let mut b = ClassBuilder::new(c"QuietMdCoverCrop", class(c"UIViewController")).expect("register QuietMdCoverCrop");
        unsafe {
            b.add_method(sel!(viewDidLayoutSubviews), laid_out as extern "C-unwind" fn(_, _));
            b.add_method(sel!(viewForZoomingInScrollView:), zoomed as extern "C-unwind" fn(_, _, _) -> _);
            b.add_method(sel!(cropHandle:), handle as extern "C-unwind" fn(_, _, _));
            b.add_method(sel!(cropCancel:), cancel as extern "C-unwind" fn(_, _, _));
            b.add_method(sel!(cropDone:), done as extern "C-unwind" fn(_, _, _));
        }
        b.register()
    })
}
