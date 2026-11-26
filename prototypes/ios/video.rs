//! 文档里的视频（「视频或文件」块的预览视图）：桌面靠 Quick Look（`qlmanage`）出缩略图、点播放弹它的窗口，iOS 上都没有。
//! 缩略图：AVFoundation 截一帧，存进和桌面同一个缩略图目录（`editor::file::make_thumb`）。
//! 播放：点了播放，系统的播放器（`AVPlayerViewController`，自带进度、暂停、全屏）盖在这一块的画面上，就在页面里放；
//! 每帧跟着这一块挪（滚动），这一块不画了（关了文档、删了块、换了页）或放完了就停下拿掉。没在页面上画着的（卡片视图）全屏放。
use std::cell::RefCell;
use std::path::{Path, PathBuf};

use gpui::*;
use objc2::encode::{Encode, Encoding, RefEncode};
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, NSObject};
use objc2::{ClassType, define_class, msg_send};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};

#[repr(C)] #[derive(Clone, Copy, Default)] struct CMTime { value: i64, timescale: i32, flags: u32, epoch: i64 }
unsafe impl Encode for CMTime { const ENCODING: Encoding = Encoding::Struct("?", &[i64::ENCODING, i32::ENCODING, u32::ENCODING, i64::ENCODING]); }
unsafe impl RefEncode for CMTime { const ENCODING_REF: Encoding = Encoding::Pointer(&Self::ENCODING); }
#[repr(C)] struct CGImage { _private: [u8; 0] }
unsafe impl RefEncode for CGImage { const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("CGImage", &[])); }

#[link(name = "AVFoundation", kind = "framework")]
unsafe extern "C" { static AVAudioSessionCategoryPlayback: *const AnyObject; static AVPlayerItemDidPlayToEndTimeNotification: *const AnyObject; }
#[link(name = "AVKit", kind = "framework")]
unsafe extern "C" {}
#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" { fn CGImageRelease(image: *mut CGImage); }
#[link(name = "UIKit", kind = "framework")]
unsafe extern "C" { fn UIImagePNGRepresentation(image: *mut AnyObject) -> *mut AnyObject; }

/// 这一帧画着的视频画面：哪个文件、在哪、露出来的是哪一块、圆角。
#[derive(Clone)]
struct Slot { path: PathBuf, bounds: Bounds<Pixels>, mask: Bounds<Pixels>, radius: f32 }
/// 正在页面里放的那一个：外面一层按露出来的那块剪，里面是播放器的视图。
struct Playing { path: PathBuf, clip: Retained<AnyObject>, controller: Retained<AnyObject> }

thread_local! {
    static SLOTS: RefCell<Vec<Slot>> = const { RefCell::new(Vec::new()) };
    static LAST: RefCell<Vec<Slot>> = const { RefCell::new(Vec::new()) };
    static PLAYING: RefCell<Option<Playing>> = const { RefCell::new(None) };
    static DELEGATE: std::cell::Cell<*mut AnyObject> = const { std::cell::Cell::new(std::ptr::null_mut()) };
    /// 「放完了」的通知：每次放都换一个（页面里的和全屏的都要，循环播放靠它从头再来）。
    static OBSERVER: RefCell<Option<Retained<AnyObject>>> = const { RefCell::new(None) };
    /// 这一帧画着的视频块（长按菜单）：块 → 文件。
    static MENUS: RefCell<Vec<(crate::model::BlockId, PathBuf)>> = const { RefCell::new(Vec::new()) };
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "QuietMdVideoDelegate"]
    struct Delegate;

    impl Delegate {
        /// 存进相册的结果（`save`）。
        #[unsafe(method(video:didFinishSavingWithError:contextInfo:))]
        fn finished_saving(&self, _path: *mut AnyObject, error: *mut AnyObject, _context: *mut std::ffi::c_void) { saved(error.is_null()); }

        /// 页面里那个点了全屏又关上。
        #[unsafe(method(playerViewController:willEndFullScreenPresentationWithAnimationCoordinator:))]
        fn leaving(&self, _vc: *mut AnyObject, coordinator: *mut AnyObject) {
            let done = block2::RcBlock::new(|_: *mut AnyObject| { if at_end() { stop(); } redraw(); });
            unsafe { let _: bool = msg_send![coordinator, animateAlongsideTransition: std::ptr::null_mut::<AnyObject>(), completion: &*done]; }
        }
    }
);

fn delegate() -> *mut AnyObject {
    let mut d = DELEGATE.with(|d| d.get());
    if d.is_null() { d = unsafe { msg_send![Delegate::class(), new] }; DELEGATE.with(|c| c.set(d)); }
    d
}
/// 页面里那个停在了最后（全屏里放完的，关了全屏时才拿掉）。
fn at_end() -> bool {
    let secs = |t: CMTime| if t.timescale > 0 && t.flags & 1 != 0 { Some(t.value as f64 / t.timescale as f64) } else { None };
    PLAYING.with(|p| p.borrow().as_ref().is_some_and(|p| unsafe {
        let player: *mut AnyObject = msg_send![&*p.controller, player];
        if player.is_null() { return false; }
        let rate: f32 = msg_send![player, rate];
        let item: *mut AnyObject = msg_send![player, currentItem];
        if item.is_null() || rate > 0. { return false; }
        let (now, length): (CMTime, CMTime) = (msg_send![item, currentTime], msg_send![item, duration]);
        matches!((secs(now), secs(length)), (Some(a), Some(b)) if b > 0. && a >= b - 0.05)
    }))
}
/// 关了全屏：系统把 Metal 视图重新提到了那一层上面（见 `shot::reseat_overlay`），再画一帧把玻璃叫回来。
fn redraw() { super::menu::dispatch(|_, window, _| { super::shot::reseat_overlay(); window.refresh(); }); }

fn class(name: &std::ffi::CStr) -> Option<&'static AnyClass> { AnyClass::get(name) }
unsafe fn url(p: &Path) -> *mut AnyObject { unsafe { msg_send![class(c"NSURL").unwrap(), fileURLWithPath: super::glass::ns_string(&p.to_string_lossy())] } }
pub fn is_video(p: &Path) -> bool { p.extension().and_then(|e| e.to_str()).is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "mp4" | "mov" | "m4v")) }

/// 截一帧当缩略图，写成 `dir/poster.png`（后台线程上调）。不是视频或读不出来返回 `None`。
pub fn thumb(p: &Path, dir: &Path) -> Option<PathBuf> {
    if !is_video(p) { return None; }
    let to = dir.join("poster.png");
    objc2::rc::autoreleasepool(|_| unsafe {
        let asset: *mut AnyObject = msg_send![class(c"AVURLAsset")?, URLAssetWithURL: url(p), options: std::ptr::null_mut::<AnyObject>()];
        if asset.is_null() { return None; }
        let generator: *mut AnyObject = msg_send![class(c"AVAssetImageGenerator")?, assetImageGeneratorWithAsset: asset];
        if generator.is_null() { return None; }
        // 拍的时候手机竖着：按轨道的朝向摆正。
        let _: () = msg_send![generator, setAppliesPreferredTrackTransform: true];
        let _: () = msg_send![generator, setMaximumSize: CGSize::new(1200., 1200.)];
        // 默认会挪到最近的关键帧（常常就是开头那一帧）：要准的那一帧。
        let exact = CMTime { value: 0, timescale: 1, flags: 1, epoch: 0 };
        let _: () = msg_send![generator, setRequestedTimeToleranceBefore: exact];
        let _: () = msg_send![generator, setRequestedTimeToleranceAfter: exact];
        // 第一帧常是黑的：取第 1 秒（太短的取开头）。
        let length: CMTime = msg_send![asset, duration];
        let secs = if length.timescale > 0 { length.value as f64 / length.timescale as f64 } else { 0. };
        let at = CMTime { value: if secs > 2. { 600 } else { 0 }, timescale: 600, flags: 1, epoch: 0 };
        let image: *mut CGImage = msg_send![generator, copyCGImageAtTime: at, actualTime: std::ptr::null_mut::<CMTime>(), error: std::ptr::null_mut::<*mut AnyObject>()];
        if image.is_null() { return None; }
        let ui: *mut AnyObject = msg_send![class(c"UIImage")?, imageWithCGImage: image];
        CGImageRelease(image);
        if ui.is_null() { return None; }
        let data = UIImagePNGRepresentation(ui);
        if data.is_null() { return None; }
        let written: bool = msg_send![data, writeToFile: super::glass::ns_string(&to.to_string_lossy()), atomically: true];
        written.then_some(to)
    })
}

/// 主页卡片要画一个还没截过的视频（这篇从没打开过）：后台截好，回主线程再画一帧。同一个文件只截一次。
pub fn poster_later(video: PathBuf, make: fn(&Path) -> Option<PathBuf>) {
    static ASKED: std::sync::Mutex<Option<std::collections::HashSet<PathBuf>>> = std::sync::Mutex::new(None);
    if !ASKED.lock().map(|mut a| a.get_or_insert_default().insert(video.clone())).unwrap_or(false) { return; }
    std::thread::spawn(move || {
        if make(&video).is_none() { return; }
        unsafe extern "C" { static _dispatch_main_q: std::ffi::c_void; fn dispatch_async_f(queue: *const std::ffi::c_void, context: *mut std::ffi::c_void, work: extern "C" fn(*mut std::ffi::c_void)); }
        extern "C" fn back(_: *mut std::ffi::c_void) { super::menu::dispatch(|_, window, _| window.refresh()); }
        unsafe { dispatch_async_f((&raw const _dispatch_main_q).cast(), std::ptr::null_mut(), back); }
    });
}

/// 放在预览视图的画面上：点播放时播放器就盖在这一块。
pub fn slot(path: PathBuf, radius: f32) -> impl IntoElement {
    canvas(move |bounds, window, _| {
        let mut mask = bounds.intersect(&window.content_mask().bounds);
        // 键盘弹出时，键盘上面的工具条是 GPUI 画的：播放器不能盖住它。
        if super::keyboard_up(window) { let floor = super::visible_height(window); if mask.bottom() > floor { mask.size.height = (floor - mask.origin.y).max(px(0.)); } }
        SLOTS.with(|s| s.borrow_mut().push(Slot { path, bounds, mask, radius }));
    }, |_, _, _, _| {}).absolute().inset_0()
}

/// 每帧开头（`menu::begin`）。
pub fn begin() { SLOTS.with(|s| s.borrow_mut().clear()); MENUS.with(|m| m.borrow_mut().clear()); }

/// 这一帧画好了：跟着这一块挪；它不在了就停下拿掉。
pub fn finish_frame() {
    let slots = SLOTS.with(|s| s.borrow().clone());
    let at = PLAYING.with(|p| p.borrow().as_ref().map(|p| slots.iter().find(|s| s.path == p.path).cloned()));
    match at {
        Some(Some(s)) => PLAYING.with(|p| if let Some(p) = p.borrow().as_ref() { unsafe { place(p, &s) } }),
        Some(None) => stop(),
        None => {}
    }
    LAST.with(|l| *l.borrow_mut() = slots);
}

/// 打开一个视频（预览视图的播放、卡片视图的打开）：页面上画着它就在那里放，不然全屏放。不是视频返回假。
pub fn open(p: &Path) -> bool { play(p, false) }
/// 直接全屏放（附件工具条的「全屏」）。不是视频返回假。
pub fn full(p: &Path) -> bool { play(p, true) }

fn play(p: &Path, full: bool) -> bool {
    if !is_video(p) { return false; }
    stop();
    session();
    let inline = LAST.with(|l| l.borrow().iter().find(|s| s.path == p).cloned()).filter(|_| !full);
    unsafe {
        let (Some(av), Some(vc)) = (class(c"AVPlayer"), class(c"AVPlayerViewController")) else { return true };
        let player: Retained<AnyObject> = msg_send![av, playerWithURL: url(p)];
        let controller: Retained<AnyObject> = msg_send![vc, new];
        let _: () = msg_send![&*controller, setPlayer: &*player];
        // 画中画要后台播放的权限：不放。
        let _: () = msg_send![&*controller, setAllowsPictureInPicturePlayback: false];
        let _: () = msg_send![&*controller, setDelegate: delegate()];
        let root = super::files::root_controller();
        if root.is_null() { return true; }
        // 放完了：循环播放的从头再来；不然页面里的拿掉播放器，露出封面和中间的播放按钮（全屏里放完的等关了全屏）。
        // 这条通知可能从别的线程来：交给主队列。
        let item: *mut AnyObject = msg_send![&*player, currentItem];
        let center: *mut AnyObject = msg_send![class(c"NSNotificationCenter").unwrap(), defaultCenter];
        let main: *mut AnyObject = msg_send![class(c"NSOperationQueue").unwrap(), mainQueue];
        let (file, again) = (p.to_path_buf(), player.clone());
        let ended = block2::RcBlock::new(move |_: *mut AnyObject| {
            if looping(&file) { let _: () = msg_send![&*again, seekToTime: CMTime { value: 0, timescale: 1, flags: 1, epoch: 0 }]; let _: () = msg_send![&*again, play]; }
            else if PLAYING.with(|p| p.borrow().as_ref().is_some_and(|p| p.path == file)) { stop(); redraw(); }
        });
        let observer: Retained<AnyObject> = msg_send![center, addObserverForName: AVPlayerItemDidPlayToEndTimeNotification, object: item, queue: main, usingBlock: &*ended];
        if let Some(old) = OBSERVER.with(|o| o.borrow_mut().replace(observer)) { let _: () = msg_send![center, removeObserver: &*old]; }
        match (inline, super::shot::metal()) {
            (Some(s), Some(metal)) => {
                let clip: Retained<AnyObject> = msg_send![class(c"UIView").unwrap(), new];
                let _: () = msg_send![&*clip, setClipsToBounds: true];
                let view: *mut AnyObject = msg_send![&*controller, view];
                let layer: *mut AnyObject = msg_send![view, layer];
                let _: () = msg_send![layer, setCornerRadius: f64::from(s.radius)];
                let _: () = msg_send![layer, setMasksToBounds: true];
                let _: () = msg_send![root, addChildViewController: &*controller];
                let _: () = msg_send![&*clip, addSubview: view];
                let _: () = msg_send![metal, addSubview: &*clip];
                let _: () = msg_send![&*controller, didMoveToParentViewController: root];
                let playing = Playing { path: p.to_path_buf(), clip, controller };
                place(&playing, &s);
                PLAYING.with(|p| *p.borrow_mut() = Some(playing));
            }
            _ => {
                let _: () = msg_send![&*controller, setModalPresentationStyle: 0isize];
                let _: () = msg_send![root, presentViewController: &*controller, animated: true, completion: std::ptr::null_mut::<AnyObject>()];
            }
        }
        let _: () = msg_send![&*player, play];
    }
    true
}

/// 摆到这一块上：外层是露出来的那块，播放器按整块的位置放在里面。
unsafe fn place(p: &Playing, s: &Slot) {
    let rect = |b: Bounds<Pixels>, o: Point<Pixels>| CGRect::new(CGPoint::new(f64::from(b.origin.x - o.x), f64::from(b.origin.y - o.y)), CGSize::new(f64::from(b.size.width), f64::from(b.size.height)));
    unsafe {
        let _: () = msg_send![&*p.clip, setFrame: rect(s.mask, Point::default())];
        let view: *mut AnyObject = msg_send![&*p.controller, view];
        let _: () = msg_send![view, setFrame: rect(s.bounds, s.mask.origin)];
        let hidden = s.mask.size.width <= px(1.) || s.mask.size.height <= px(1.);
        let was: bool = msg_send![&*p.clip, isHidden];
        let _: () = msg_send![&*p.clip, setHidden: hidden];
        // 滚出屏幕就暂停（滚回来要自己再点播放）。
        if hidden && !was { let player: *mut AnyObject = msg_send![&*p.controller, player]; if !player.is_null() { let _: () = msg_send![player, pause]; } }
    }
}

/// 停下，拿掉页面里的播放器。
pub fn stop() {
    // 进了全屏还在放：不打断，等它自己关了。
    let full = PLAYING.with(|p| p.borrow().as_ref().is_some_and(|p| unsafe { let f: *mut AnyObject = msg_send![&*p.controller, presentedViewController]; !f.is_null() }));
    if full { return; }
    let Some(p) = PLAYING.with(|p| p.borrow_mut().take()) else { return };
    unsafe {
        let player: *mut AnyObject = msg_send![&*p.controller, player];
        if !player.is_null() { let _: () = msg_send![player, pause]; }
        let _: () = msg_send![&*p.controller, willMoveToParentViewController: std::ptr::null_mut::<AnyObject>()];
        let _: () = msg_send![&*p.clip, removeFromSuperview];
        let _: () = msg_send![&*p.controller, removeFromParentViewController];
    }
}

/// 静音开关打着也有声音（和配音、试听一样）。
fn session() {
    unsafe {
        let Some(s) = class(c"AVAudioSession") else { return };
        let s: *mut AnyObject = msg_send![s, sharedInstance];
        let _: bool = msg_send![s, setCategory: AVAudioSessionCategoryPlayback, error: std::ptr::null_mut::<*mut AnyObject>()];
    }
}

// ---- 长按菜单

const KEY: &str = "video:";
const VIEW: &str = "video-view";
const SAVE: &str = "video-save";
const REPLACE: &str = "video-replace";
const SWITCH: &str = "video-switch";
const LOOP: &str = "video-loop";
const REMOVE: &str = "video-remove";

/// 视频块（预览视图的画面、卡片视图的卡片）上的系统上下文菜单，和生成的插图同一套（`menu::context`）：
/// 视图（全屏放）、存储到照片、替换视频、换成另一种视图、循环播放、移除。长按它不再提起来拖，拖要先选中再拖右边的手柄。
pub fn context(path: PathBuf, id: crate::model::BlockId, radius: f32, preview: bool) -> AnyElement {
    let tr = crate::settings::tr;
    let on = looping(&path);
    MENUS.with(|m| m.borrow_mut().push((id, path)));
    let switch = if preview { super::menu::Item::new(SWITCH, tr("卡片视图", "Card View"), "rectangle", 1) } else { super::menu::Item::new(SWITCH, tr("预览视图", "Preview"), "doc.richtext", 1) };
    let items = vec![
        super::menu::Item::new(VIEW, tr("视图", "View"), "arrow.up.left.and.arrow.down.right", 0),
        super::menu::Item::new(SAVE, tr("存储到照片", "Save to Photos"), "square.and.arrow.down", 0),
        super::menu::Item::new(REPLACE, tr("替换视频", "Replace Video"), "arrow.triangle.2.circlepath", 1),
        switch,
        super::menu::Item::new(LOOP, tr("循环播放", "Loop"), "repeat", 1).checked(on),
        super::menu::Item::new(REMOVE, tr("移除", "Remove"), "trash", 2).danger(),
    ];
    div().absolute().inset_0().child(super::menu::context(SharedString::from(format!("{KEY}{}", id.0)), "", radius, items)).into_any_element()
}

/// 长按落在视频块的菜单上（`lift_block` 先问这里）：菜单由系统弹，块不提起来。
pub fn press(id: crate::model::BlockId, at: Point<Pixels>) -> bool { MENUS.with(|m| m.borrow().iter().any(|(b, _)| *b == id)) && super::menu::context_at(at) }

/// 菜单是对哪一块弹的，和它的文件。
fn pressed() -> Option<(crate::model::BlockId, PathBuf)> {
    let n: u64 = super::menu::context_target()?.strip_prefix(KEY)?.parse().ok()?;
    MENUS.with(|m| m.borrow().iter().find(|(b, _)| b.0 == n).cloned())
}

/// 工作区收到的菜单动作。
pub fn menu_action(action: &str, w: &mut crate::app::Workspace, window: &mut Window, cx: &mut Context<crate::app::Workspace>) -> bool {
    if !action.starts_with("video-") { return false; }
    let Some((id, path)) = pressed() else { return true };
    let Some(editor) = w.editor.clone() else { return true };
    match action {
        VIEW => { full(&path); }
        SAVE => save(&path),
        LOOP => set_looping(&path, !looping(&path)),
        SWITCH => editor.update(cx, |e, cx| {
            let Some(crate::model::BlockKind::File(f)) = e.core.kind_of(id).cloned() else { return };
            let to = if f.view == crate::model::FileView::Preview { crate::model::FileView::Card } else { crate::model::FileView::Preview };
            e.core.set_file_view(id, to); e.changed(window, cx);
        }),
        REMOVE => { super::glass::delete_haptic(); editor.update(cx, |e, cx| { e.core.delete_block(id); e.changed(window, cx); }); }
        REPLACE => {
            let reply = super::files::open_video();
            let editor = editor.downgrade();
            window.spawn(cx, async move |cx| {
                let Some(p) = reply.await.ok().and_then(Result::ok).flatten().and_then(|v| v.into_iter().next()) else { return };
                let _ = editor.update_in(cx, |e, window, cx| e.replace_file_from(id, p, window, cx));
            }).detach();
        }
        _ => {}
    }
    true
}

// ---- 循环播放：按文件记在 `NSUserDefaults`（文档库里的路径换了设备会变，记最后两段：`<uuid>/<名字>`）。

const LOOPS: &str = "QuietMdVideoLoop";
fn loop_key(p: &Path) -> String { p.components().rev().take(2).collect::<Vec<_>>().into_iter().rev().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/") }
fn loops() -> Vec<String> {
    unsafe {
        let d: *mut AnyObject = msg_send![class(c"NSUserDefaults").unwrap(), standardUserDefaults];
        let a: *mut AnyObject = msg_send![d, stringArrayForKey: super::glass::ns_string(LOOPS)];
        if a.is_null() { return Vec::new(); }
        let n: usize = msg_send![a, count];
        (0..n).filter_map(|i| { let s: *mut AnyObject = msg_send![a, objectAtIndex: i]; let c: *const std::ffi::c_char = msg_send![s, UTF8String]; (!c.is_null()).then(|| std::ffi::CStr::from_ptr(c).to_string_lossy().into_owned()) }).collect()
    }
}
fn looping(p: &Path) -> bool { loops().contains(&loop_key(p)) }
fn set_looping(p: &Path, on: bool) {
    let key = loop_key(p);
    let mut all = loops();
    all.retain(|k| *k != key);
    if on { all.push(key); }
    unsafe {
        let a: *mut AnyObject = msg_send![class(c"NSMutableArray").unwrap(), array];
        for k in &all { let _: () = msg_send![a, addObject: super::glass::ns_string(k)]; }
        let d: *mut AnyObject = msg_send![class(c"NSUserDefaults").unwrap(), standardUserDefaults];
        let _: () = msg_send![d, setObject: a, forKey: super::glass::ns_string(LOOPS)];
    }
}

// ---- 存储到照片

#[link(name = "UIKit", kind = "framework")]
unsafe extern "C" { fn UISaveVideoAtPathToSavedPhotosAlbum(path: *mut AnyObject, target: *mut AnyObject, selector: objc2::runtime::Sel, context: *mut std::ffi::c_void); fn UIVideoAtPathIsCompatibleWithSavedPhotosAlbum(path: *mut AnyObject) -> bool; }

/// 存进相册（只要「添加」的权限，Info.plist 的 `NSPhotoLibraryAddUsageDescription`）；存好、存不了都说一声。
fn save(p: &Path) {
    unsafe {
        let path = super::glass::ns_string(&p.to_string_lossy());
        if !UIVideoAtPathIsCompatibleWithSavedPhotosAlbum(path) { return saved(false); }
        UISaveVideoAtPathToSavedPhotosAlbum(path, delegate(), objc2::sel!(video:didFinishSavingWithError:contextInfo:), std::ptr::null_mut());
    }
}
fn saved(ok: bool) {
    let tr = crate::settings::tr;
    super::menu::dispatch(move |w, _, cx| if ok { w.show_toast_kind(crate::ui::ToastKind::Success, tr("已存储到照片", "Saved to Photos"), cx) } else { w.show_toast_kind(crate::ui::ToastKind::Error, tr("无法存储到照片", "Could not save to Photos"), cx) });
}
