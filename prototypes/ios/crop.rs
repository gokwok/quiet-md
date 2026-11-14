//! 空间头像的裁切（照 Craft，IMG_9116）：选好图片后在方框里拖动、双指缩放，取方框里的那一块。
//!
//! 全是系统控件：`UIScrollView` 管拖动和缩放，方框是整个宽度的正方形、竖直居中，框外压暗；
//! 上面是标题，下面一条是「取消」「选取」。取出来的是不超过 512 像素见方的 PNG，放在临时目录里，
//! 之后和别的图片一样按路径读（`space::read_icon`）。
use std::cell::{Cell, RefCell};
use std::path::PathBuf;

use objc2::encode::{Encode, Encoding, RefEncode};
use objc2::runtime::{AnyClass, AnyObject, ClassBuilder, Sel};
use objc2::{msg_send, sel};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};

use super::files::Reply;
use crate::settings::tr;

/// 取出来的图最大边长（像素）。头像最大画到几十点，512 足够清楚。
const OUT_MAX: f64 = 512.;
/// 底下那条按钮的高度（不含底部安全区）。按 IMG_9116 量：73pt。
const BAR_H: f64 = 73.;
/// 标题中线离面板顶边的距离，同上量得 50pt。
const TITLE_Y: f64 = 50.;

/// 开着的那一个裁切面板。视图都挂在 `vc` 的视图上，由它持有；`vc` 是 `alloc` 来的那一份，收起后放掉。
struct Crop { reply: Reply<Option<PathBuf>>, source: PathBuf, vc: *mut AnyObject, scroll: *mut AnyObject, image: *mut AnyObject, dim: *mut AnyObject, frame: *mut AnyObject, title: *mut AnyObject, bar: *mut AnyObject, cancel: *mut AnyObject, choose: *mut AnyObject, size: Cell<(f64, f64)>, square: Cell<CGRect> }
// 调试构建会核对消息的编码：这几个系统类型要和系统的对上。
#[repr(C)] #[derive(Clone, Copy)] struct Insets { top: f64, left: f64, bottom: f64, right: f64 }
unsafe impl Encode for Insets { const ENCODING: Encoding = Encoding::Struct("UIEdgeInsets", &[const { Encoding::Double }; 4]); }
#[repr(C)] struct CGColor { _private: [u8; 0] }
unsafe impl RefEncode for CGColor { const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("CGColor", &[])); }
#[repr(C)] struct CGPath { _private: [u8; 0] }
unsafe impl RefEncode for CGPath { const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("CGPath", &[])); }

thread_local! { static CROP: RefCell<Option<Crop>> = const { RefCell::new(None) }; }

unsafe fn ns_string(s: &str) -> *mut AnyObject {
    let c = std::ffi::CString::new(s).unwrap_or_default();
    unsafe { msg_send![AnyClass::get(c"NSString").unwrap(), stringWithUTF8String: c.as_ptr()] }
}
unsafe fn white(alpha: f64) -> *mut AnyObject { unsafe { msg_send![AnyClass::get(c"UIColor").unwrap(), colorWithWhite: 1f64, alpha: alpha] } }
unsafe fn black(alpha: f64) -> *mut AnyObject { unsafe { msg_send![AnyClass::get(c"UIColor").unwrap(), colorWithWhite: 0f64, alpha: alpha] } }
unsafe fn new_view(class: &std::ffi::CStr) -> *mut AnyObject {
    unsafe {
        let v: *mut AnyObject = msg_send![AnyClass::get(class).unwrap(), alloc];
        msg_send![v, initWithFrame: CGRect::new(CGPoint::new(0., 0.), CGSize::new(0., 0.))]
    }
}
/// 加进父视图后放掉自己那一份（父视图持有）。
unsafe fn add(parent: *mut AnyObject, view: *mut AnyObject) -> *mut AnyObject {
    unsafe { let _: () = msg_send![parent, addSubview: view]; let _: () = msg_send![view, release]; }
    view
}

/// 把 `path` 的图片裁成方的：弹出裁切面板，选了交回新图的路径，取消交回 `None`。读不出图片时原样交回，
/// 由调用方照常报格式不对。
pub fn square(path: PathBuf) -> Reply<Option<PathBuf>> {
    if CROP.with(|c| c.borrow().is_some()) { return Reply::ready(None); }
    let reply = Reply::pending();
    let built = unsafe { build(&path, reply.handle()) };
    if !built { return Reply::ready(Some(path)); }
    present(0);
    reply
}

unsafe fn build(path: &std::path::Path, reply: Reply<Option<PathBuf>>) -> bool {
    unsafe {
        let image: *mut AnyObject = msg_send![AnyClass::get(c"UIImage").unwrap(), imageWithContentsOfFile: ns_string(&path.to_string_lossy())];
        if image.is_null() { return false; }
        let vc: *mut AnyObject = msg_send![controller_class(), new];
        let view: *mut AnyObject = msg_send![vc, view];
        let _: () = msg_send![view, setBackgroundColor: black(1.)];
        // 一直是深色（照片要在暗底上看），也不让下滑收起：拖图片时容易误收。
        let _: () = msg_send![vc, setOverrideUserInterfaceStyle: 2isize];
        let _: () = msg_send![vc, setModalInPresentation: true];

        let scroll = add(view, new_view(c"UIScrollView"));
        let _: () = msg_send![scroll, setDelegate: vc];
        let _: () = msg_send![scroll, setShowsHorizontalScrollIndicator: false];
        let _: () = msg_send![scroll, setShowsVerticalScrollIndicator: false];
        // 2：不按安全区自动加内边距，方框的位置由 `layout` 定。
        let _: () = msg_send![scroll, setContentInsetAdjustmentBehavior: 2isize];
        let _: () = msg_send![scroll, setAlwaysBounceVertical: true];
        let _: () = msg_send![scroll, setAlwaysBounceHorizontal: true];
        let iv: *mut AnyObject = msg_send![AnyClass::get(c"UIImageView").unwrap(), alloc];
        let iv: *mut AnyObject = msg_send![iv, initWithImage: image];
        add(scroll, iv);

        // 框外压暗：整块半透明黑，用「偶奇」规则的形状遮罩挖掉方框。
        let dim = add(view, new_view(c"UIView"));
        let _: () = msg_send![dim, setBackgroundColor: black(0.5)];
        let _: () = msg_send![dim, setUserInteractionEnabled: false];
        let mask: *mut AnyObject = msg_send![AnyClass::get(c"CAShapeLayer").unwrap(), layer];
        let _: () = msg_send![mask, setFillRule: ns_string("even-odd")];
        let layer: *mut AnyObject = msg_send![dim, layer];
        let _: () = msg_send![layer, setMask: mask];
        let frame = add(view, new_view(c"UIView"));
        let _: () = msg_send![frame, setUserInteractionEnabled: false];
        let fl: *mut AnyObject = msg_send![frame, layer];
        let _: () = msg_send![fl, setBorderWidth: 1f64];
        let border: *const CGColor = msg_send![white(0.6), CGColor];
        let _: () = msg_send![fl, setBorderColor: border];

        let title = add(view, new_view(c"UILabel"));
        let _: () = msg_send![title, setText: ns_string(tr("选取照片", "Choose Photo"))];
        let _: () = msg_send![title, setTextColor: white(1.)];
        let _: () = msg_send![title, setTextAlignment: 1isize];
        let font: *mut AnyObject = msg_send![AnyClass::get(c"UIFont").unwrap(), systemFontOfSize: 17f64, weight: 0.3f64];
        let _: () = msg_send![title, setFont: font];

        let bar = add(view, new_view(c"UIView"));
        let _: () = msg_send![bar, setBackgroundColor: black(0.55)];
        let button = |label: &str, action: Sel| -> *mut AnyObject {
            // 0：自定义样式，字是白色常规体，和 IMG_9116 一样两边一样重。
            let b: *mut AnyObject = msg_send![AnyClass::get(c"UIButton").unwrap(), buttonWithType: 0isize];
            let _: () = msg_send![b, setTitle: ns_string(label), forState: 0usize];
            let _: () = msg_send![b, setTitleColor: white(1.), forState: 0usize];
            let _: () = msg_send![b, setTitleColor: white(0.4), forState: 1usize];
            let tl: *mut AnyObject = msg_send![b, titleLabel];
            let font: *mut AnyObject = msg_send![AnyClass::get(c"UIFont").unwrap(), systemFontOfSize: 17f64];
            let _: () = msg_send![tl, setFont: font];
            // 64：手指在按钮里抬起。
            let _: () = msg_send![b, addTarget: vc, action: action, forControlEvents: 64usize];
            let _: () = msg_send![bar, addSubview: b];
            b
        };
        let cancel = button(tr("取消", "Cancel"), sel!(cropCancel:));
        let choose = button(tr("选取", "Choose"), sel!(cropChoose:));

        CROP.with(|c| *c.borrow_mut() = Some(Crop { reply, source: path.to_path_buf(), vc, scroll, image: iv, dim, frame, title, bar, cancel, choose, size: Cell::new((0., 0.)), square: Cell::new(CGRect::default()) }));
        true
    }
}

/// 弹出来。选择器自己收起是有动画的，还没收完时弹不出：每 0.1 秒再试，3 秒还不行就不裁，原图交回。
fn present(tries: u32) {
    let done = unsafe {
        let root = super::files::root_controller();
        let busy = root.is_null() || { let p: *mut AnyObject = msg_send![root, presentedViewController]; !p.is_null() };
        if busy && tries < 30 {
            let again = block2::RcBlock::new(move |_timer: *mut AnyObject| present(tries + 1));
            let _: *mut AnyObject = msg_send![AnyClass::get(c"NSTimer").unwrap(), scheduledTimerWithTimeInterval: 0.1f64, repeats: false, block: &*again];
            return;
        }
        if busy { false } else {
            let vc = CROP.with(|c| c.borrow().as_ref().map(|c| c.vc)).unwrap_or(std::ptr::null_mut());
            let _: () = msg_send![root, presentViewController: vc, animated: true, completion: std::ptr::null_mut::<AnyObject>()];
            true
        }
    };
    if !done { finish(None, true); }
}

/// 收起并交回结果。`keep`：没能弹出来，把原图交回去（不裁）。
fn finish(path: Option<PathBuf>, keep: bool) {
    let Some(c) = CROP.with(|c| c.borrow_mut().take()) else { return };
    unsafe {
        let _: () = msg_send![c.vc, dismissViewControllerAnimated: true, completion: std::ptr::null_mut::<AnyObject>()];
        let _: () = msg_send![c.vc, release];
    }
    c.reply.send(if keep { Some(c.source) } else { path });
}

/// 面板量好大小后摆放各个部分；大小没变就不动（不然每次布局都会把拖好的位置复原）。
fn layout(view: *mut AnyObject) {
    CROP.with(|c| {
        let c = c.borrow();
        let Some(c) = c.as_ref() else { return };
        unsafe {
            let bounds: CGRect = msg_send![view, bounds];
            let (w, h) = (bounds.size.width, bounds.size.height);
            if w <= 0. || c.size.get() == (w, h) { return; }
            c.size.set((w, h));
            let safe: Insets = msg_send![view, safeAreaInsets];
            let bar_h = BAR_H + safe.bottom;
            let _: () = msg_send![c.bar, setFrame: CGRect::new(CGPoint::new(0., h - bar_h), CGSize::new(w, bar_h))];
            for (b, right) in [(c.cancel, false), (c.choose, true)] {
                let _: () = msg_send![b, sizeToFit];
                let f: CGRect = msg_send![b, frame];
                let bw = f.size.width + 16.;
                let x = if right { w - 16. - bw + 8. } else { 16. - 8. };
                let _: () = msg_send![b, setFrame: CGRect::new(CGPoint::new(x, 0.), CGSize::new(bw, BAR_H))];
            }
            let _: () = msg_send![c.title, setFrame: CGRect::new(CGPoint::new(0., safe.top + TITLE_Y - 18.), CGSize::new(w, 36.))];

            // 方框：整个宽度（宽屏上不超过标题和按钮之间的高度），竖直居中，碰到底下那条就往上让。
            let side = w.min(h - bar_h - safe.top - TITLE_Y * 2.).max(80.);
            let x0 = (w - side) / 2.;
            let y0 = ((h - side) / 2.).min(h - bar_h - side - 8.).max(safe.top + TITLE_Y + 24.);
            let square = CGRect::new(CGPoint::new(x0, y0), CGSize::new(side, side));
            c.square.set(square);
            let _: () = msg_send![c.dim, setFrame: bounds];
            let _: () = msg_send![c.frame, setFrame: square];
            let path: *mut AnyObject = msg_send![AnyClass::get(c"UIBezierPath").unwrap(), bezierPathWithRect: bounds];
            let hole: *mut AnyObject = msg_send![AnyClass::get(c"UIBezierPath").unwrap(), bezierPathWithRect: square];
            let _: () = msg_send![path, appendPath: hole];
            let cg: *mut CGPath = msg_send![path, CGPath];
            let layer: *mut AnyObject = msg_send![c.dim, layer];
            let mask: *mut AnyObject = msg_send![layer, mask];
            let _: () = msg_send![mask, setPath: cg];

            // 图片可以拖到边缘贴着方框：内边距就是方框四周的空白。最小缩放让图片正好盖满方框，开始时居中。
            let _: () = msg_send![c.scroll, setFrame: bounds];
            let _: () = msg_send![c.scroll, setContentInset: Insets { top: y0, left: x0, bottom: h - y0 - side, right: w - x0 - side }];
            let image: *mut AnyObject = msg_send![c.image, image];
            let size: CGSize = msg_send![image, size];
            if size.width <= 0. || size.height <= 0. { return; }
            let _: () = msg_send![c.scroll, setZoomScale: 1f64];
            let _: () = msg_send![c.image, setFrame: CGRect::new(CGPoint::new(0., 0.), size)];
            let _: () = msg_send![c.scroll, setContentSize: size];
            let min = (side / size.width).max(side / size.height);
            let _: () = msg_send![c.scroll, setMinimumZoomScale: min];
            let _: () = msg_send![c.scroll, setMaximumZoomScale: min * 6.];
            let _: () = msg_send![c.scroll, setZoomScale: min];
            let offset = CGPoint::new(-x0 + (size.width * min - side) / 2., -y0 + (size.height * min - side) / 2.);
            let _: () = msg_send![c.scroll, setContentOffset: offset];
        }
    });
}

/// 方框里的那一块画成新图，写进临时目录。
fn cropped() -> Option<PathBuf> {
    CROP.with(|c| {
        let c = c.borrow();
        let c = c.as_ref()?;
        unsafe {
            let image: *mut AnyObject = msg_send![c.image, image];
            let size: CGSize = msg_send![image, size];
            let scale: f64 = msg_send![image, scale];
            let zoom: f64 = msg_send![c.scroll, zoomScale];
            let offset: CGPoint = msg_send![c.scroll, contentOffset];
            let sq = c.square.get();
            if zoom <= 0. || sq.size.width <= 0. { return None; }
            // 方框在图片上的位置（图片的点）。
            let (cx, cy, cs) = ((offset.x + sq.origin.x) / zoom, (offset.y + sq.origin.y) / zoom, sq.size.width / zoom);
            let out = (cs * scale).min(OUT_MAX).max(1.).round();
            let k = out / cs;
            let format: *mut AnyObject = msg_send![AnyClass::get(c"UIGraphicsImageRendererFormat").unwrap(), preferredFormat];
            let _: () = msg_send![format, setScale: 1f64];
            let _: () = msg_send![format, setOpaque: false];
            let renderer: *mut AnyObject = msg_send![AnyClass::get(c"UIGraphicsImageRenderer").unwrap(), alloc];
            let renderer: *mut AnyObject = msg_send![renderer, initWithSize: CGSize::new(out, out), format: format];
            let draw = CGRect::new(CGPoint::new(-cx * k, -cy * k), CGSize::new(size.width * k, size.height * k));
            // `drawInRect:` 按图片自己的方向画，拍照时横竖不对的也会摆正。
            let actions = block2::RcBlock::new(move |_ctx: *mut AnyObject| { let _: () = msg_send![image, drawInRect: draw]; });
            let data: *mut AnyObject = msg_send![renderer, PNGDataWithActions: &*actions];
            let _: () = msg_send![renderer, release];
            if data.is_null() { return None; }
            let root = std::env::temp_dir().join("quiet-md-crop");
            // 上一次的已经读走了。
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&root).ok()?;
            let to = root.join(format!("{}.png", uuid::Uuid::new_v4()));
            let written: bool = msg_send![data, writeToFile: ns_string(&to.to_string_lossy()), atomically: true];
            written.then_some(to)
        }
    })
}

fn controller_class() -> &'static AnyClass {
    static CLASS: std::sync::OnceLock<&'static AnyClass> = std::sync::OnceLock::new();
    CLASS.get_or_init(|| {
        extern "C-unwind" fn laid_out(this: &AnyObject, _: Sel) {
            let _: () = unsafe { msg_send![super(this, AnyClass::get(c"UIViewController").unwrap()), viewDidLayoutSubviews] };
            let view: *mut AnyObject = unsafe { msg_send![this, view] };
            layout(view);
        }
        extern "C-unwind" fn zoomed(_this: &AnyObject, _: Sel, _scroll: *mut AnyObject) -> *mut AnyObject {
            CROP.with(|c| c.borrow().as_ref().map(|c| c.image)).unwrap_or(std::ptr::null_mut())
        }
        extern "C-unwind" fn cancel(_this: &AnyObject, _: Sel, _sender: *mut AnyObject) { finish(None, false); }
        extern "C-unwind" fn choose(_this: &AnyObject, _: Sel, _sender: *mut AnyObject) { let p = cropped(); finish(p, false); }
        let mut b = ClassBuilder::new(c"QuietMdCropController", AnyClass::get(c"UIViewController").unwrap()).expect("register QuietMdCropController");
        unsafe {
            b.add_method(sel!(viewDidLayoutSubviews), laid_out as extern "C-unwind" fn(_, _));
            b.add_method(sel!(viewForZoomingInScrollView:), zoomed as extern "C-unwind" fn(_, _, _) -> _);
            b.add_method(sel!(cropCancel:), cancel as extern "C-unwind" fn(_, _, _));
            b.add_method(sel!(cropChoose:), choose as extern "C-unwind" fn(_, _, _));
        }
        b.register()
    })
}
