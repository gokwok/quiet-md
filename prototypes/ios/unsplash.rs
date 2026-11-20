//! Unsplash 照片（照 Craft：封面录屏 2026-11-21 00:17、IMG_9366）：封面和背景都从这里挑网上的照片。
//!
//! 系统的半屏卡片（page sheet）：左上 ✕、中间标题，下面两列瀑布流，每张按原图比例、左下角是摄影师名字，
//! 没下完时先铺这张照片的主色；底下浮着一条搜索框，跟着键盘上下（`keyboardLayoutGuide`）。
//! 没输入时是 Unsplash 的编辑精选，滚到底接着加载下一页。选中后收起卡片，按 Unsplash 的规矩先报一次下载
//! （`download_location`），再下 1600 宽的那张交给封面或背景。只发出使用者输入的关键词。
//!
//! Access Key 编进二进制：构建时读环境变量 `UNSPLASH_ACCESS_KEY`（构建脚本从不进仓库的
//! `prototypes/ios/unsplash.key` 取）。没有 Key 时 `available()` 为假，调用方退回自带的在线图库。
use std::cell::RefCell;
use std::hash::{Hash, Hasher};
use std::time::Duration;

use gpui::*;
use objc2::encode::{Encode, Encoding, RefEncode};
use objc2::runtime::{AnyClass, AnyObject, Bool, ClassBuilder, Sel};
use objc2::{msg_send, sel};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use reqwest::{Client, Url};
use serde_json::Value;

use crate::settings::tr;

const KEY: Option<&str> = option_env!("UNSPLASH_ACCESS_KEY");
const API: &str = "https://api.unsplash.com";
const PER_PAGE: u32 = 24;
/// 两边和两列之间的空，按 IMG_9366 量：16pt。
const GAP: f64 = 16.;
/// 标题栏高度（✕ 和标题的中线在 34pt）。
const HEAD: f64 = 68.;
const BAR_H: f64 = 46.;

pub fn available() -> bool { KEY.is_some_and(|k| !k.trim().is_empty()) }

#[derive(Clone, Copy, PartialEq)]
pub enum Target { Cover, Background }

#[derive(Clone)]
struct Photo { w: f64, h: f64, color: String, thumb: String, full: String, ping: String, author: String, link: String }

struct Sheet {
    vc: *mut AnyObject, scroll: *mut AnyObject, field: *mut AnyObject, status: *mut AnyObject,
    target: Target, photos: Vec<Photo>, tiles: Vec<*mut AnyObject>, cols: [f64; 2], width: f64,
    query: String, page: u32, loading: bool, done: bool, revision: u64,
}
thread_local! { static SHEET: RefCell<Option<Sheet>> = const { RefCell::new(None) }; }

/// CGColor 不是 Objective-C 对象，给它一个不透明的类型，objc2 调试构建才认它的返回值。
#[repr(C)] struct CGColor { _private: [u8; 0] }
unsafe impl RefEncode for CGColor { const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("CGColor", &[])); }
/// 选图按钮的 tag 从这里起算，免得和标题（11）、名字（7）撞上。
const TAG: isize = 1000;

#[repr(C)] #[derive(Clone, Copy)] struct Insets { top: f64, left: f64, bottom: f64, right: f64 }
unsafe impl Encode for Insets { const ENCODING: Encoding = Encoding::Struct("UIEdgeInsets", &[const { Encoding::Double }; 4]); }

unsafe fn ns_string(s: &str) -> *mut AnyObject {
    let c = std::ffi::CString::new(s).unwrap_or_default();
    unsafe { msg_send![AnyClass::get(c"NSString").unwrap(), stringWithUTF8String: c.as_ptr()] }
}
unsafe fn ui(name: &std::ffi::CStr) -> *mut AnyObject { unsafe { msg_send![AnyClass::get(c"UIColor").unwrap(), performSelector: Sel::register(name)] } }
unsafe fn new_view(class: &std::ffi::CStr, frame: CGRect) -> *mut AnyObject {
    unsafe { let v: *mut AnyObject = msg_send![AnyClass::get(class).unwrap(), alloc]; msg_send![v, initWithFrame: frame] }
}
/// 加进父视图后放掉自己那一份（父视图持有）。
unsafe fn add(parent: *mut AnyObject, view: *mut AnyObject) -> *mut AnyObject {
    unsafe { let _: () = msg_send![parent, addSubview: view]; let _: () = msg_send![view, release]; }
    view
}
fn zero() -> CGRect { CGRect::new(CGPoint::new(0., 0.), CGSize::new(0., 0.)) }
unsafe fn hex_color(hex: &str) -> *mut AnyObject {
    let v = u32::from_str_radix(hex.trim_start_matches('#'), 16).unwrap_or(0xdddddd);
    let f = |s: u32| f64::from((v >> s) & 0xff) / 255.;
    unsafe { msg_send![AnyClass::get(c"UIColor").unwrap(), colorWithRed: f(16), green: f(8), blue: f(0), alpha: 1f64] }
}

/// 弹出 Unsplash 卡片，选中的照片交给 `target`。
pub fn open(target: Target) {
    if !available() || SHEET.with(|s| s.borrow().is_some()) { return; }
    unsafe {
        let root = super::super::files::root_controller();
        if root.is_null() { return; }
        build(target);
        let vc = SHEET.with(|s| s.borrow().as_ref().map(|s| s.vc)).unwrap_or(std::ptr::null_mut());
        let _: () = msg_send![root, presentViewController: vc, animated: true, completion: std::ptr::null_mut::<AnyObject>()];
        let pc: *mut AnyObject = msg_send![vc, presentationController];
        let _: () = msg_send![pc, setDelegate: vc];
    }
    load(true);
}

unsafe fn build(target: Target) {
    unsafe {
        let vc: *mut AnyObject = msg_send![controller_class(), new];
        let view: *mut AnyObject = msg_send![vc, view];
        let _: () = msg_send![view, setBackgroundColor: ui(c"systemBackgroundColor")];
        let _: () = msg_send![view, setOverrideUserInterfaceStyle: if crate::theme::dark() { 2isize } else { 1isize }];

        let scroll = add(view, new_view(c"UIScrollView", zero()));
        let _: () = msg_send![scroll, setDelegate: vc];
        let _: () = msg_send![scroll, setAlwaysBounceVertical: true];
        // 1 = 一拖就收键盘。
        let _: () = msg_send![scroll, setKeyboardDismissMode: 1isize];

        let status = add(view, new_view(c"UILabel", zero()));
        let _: () = msg_send![status, setTextAlignment: 1isize];
        let _: () = msg_send![status, setTextColor: ui(c"secondaryLabelColor")];
        let font: *mut AnyObject = msg_send![AnyClass::get(c"UIFont").unwrap(), systemFontOfSize: 15f64];
        let _: () = msg_send![status, setFont: font];

        // 标题栏：左上一颗圆 ✕，中间「Unsplash」。
        let title = add(view, new_view(c"UILabel", CGRect::new(CGPoint::new(0., 14.), CGSize::new(0., 40.))));
        let _: () = msg_send![title, setText: ns_string("Unsplash")];
        let _: () = msg_send![title, setTextAlignment: 1isize];
        let _: () = msg_send![title, setTag: 11isize];
        let font: *mut AnyObject = msg_send![AnyClass::get(c"UIFont").unwrap(), systemFontOfSize: 17f64, weight: 0.3f64];
        let _: () = msg_send![title, setFont: font];
        let close: *mut AnyObject = msg_send![AnyClass::get(c"UIButton").unwrap(), buttonWithType: 1isize];
        let config: *mut AnyObject = msg_send![AnyClass::get(c"UIImageSymbolConfiguration").unwrap(), configurationWithPointSize: 17f64, weight: 5isize];
        let image: *mut AnyObject = msg_send![AnyClass::get(c"UIImage").unwrap(), systemImageNamed: ns_string("xmark"), withConfiguration: config];
        let _: () = msg_send![close, setImage: image, forState: 0usize];
        let _: () = msg_send![close, setTintColor: ui(c"labelColor")];
        let _: () = msg_send![close, setFrame: CGRect::new(CGPoint::new(14., 12.), CGSize::new(44., 44.))];
        let _: () = msg_send![close, setBackgroundColor: ui(c"tertiarySystemFillColor")];
        let layer: *mut AnyObject = msg_send![close, layer];
        let _: () = msg_send![layer, setCornerRadius: 22f64];
        let _: () = msg_send![close, setAccessibilityLabel: ns_string(tr("关闭", "Close"))];
        let _: () = msg_send![close, addTarget: vc, action: sel!(unsplashClose:), forControlEvents: 64usize];
        let _: () = msg_send![view, addSubview: close];

        // 底下浮着的搜索框：玻璃胶囊，贴着键盘（键盘收起时贴着底部安全区）。
        let effect: *mut AnyObject = if let Some(glass) = AnyClass::get(c"UIGlassEffect") { msg_send![glass, effectWithStyle: 0isize] }
            else { msg_send![AnyClass::get(c"UIBlurEffect").unwrap(), effectWithStyle: 8isize] };
        let bar: *mut AnyObject = msg_send![AnyClass::get(c"UIVisualEffectView").unwrap(), alloc];
        let bar: *mut AnyObject = msg_send![bar, initWithEffect: effect];
        let _: () = msg_send![bar, setFrame: CGRect::new(CGPoint::new(0., 0.), CGSize::new(320., BAR_H))];
        let bl: *mut AnyObject = msg_send![bar, layer];
        let _: () = msg_send![bl, setCornerRadius: BAR_H / 2.];
        let _: () = msg_send![bar, setClipsToBounds: true];
        let _: () = msg_send![bar, setTranslatesAutoresizingMaskIntoConstraints: false];
        add(view, bar);
        let inside: *mut AnyObject = msg_send![bar, contentView];
        let glass: *mut AnyObject = msg_send![AnyClass::get(c"UIImage").unwrap(), systemImageNamed: ns_string("magnifyingglass")];
        let lens: *mut AnyObject = msg_send![AnyClass::get(c"UIImageView").unwrap(), alloc];
        let lens: *mut AnyObject = msg_send![lens, initWithImage: glass];
        let _: () = msg_send![lens, setFrame: CGRect::new(CGPoint::new(16., 13.), CGSize::new(20., 20.))];
        let _: () = msg_send![lens, setContentMode: 1isize];
        let _: () = msg_send![lens, setTintColor: ui(c"secondaryLabelColor")];
        add(inside, lens);
        let field = add(inside, new_view(c"UITextField", CGRect::new(CGPoint::new(44., 0.), CGSize::new(320. - 44. - 10., BAR_H))));
        // 2 | 16：跟着胶囊变宽变高。
        let _: () = msg_send![field, setAutoresizingMask: 18usize];
        let _: () = msg_send![field, setPlaceholder: ns_string(tr("搜索照片", "Search photos"))];
        let font: *mut AnyObject = msg_send![AnyClass::get(c"UIFont").unwrap(), systemFontOfSize: 17f64];
        let _: () = msg_send![field, setFont: font];
        // 1 = 编辑时有清除钮。下面两个输入特性在输入框上是转发实现的，objc2 调试构建的方法检查找不到，直接走 objc_msgSend
        // （同 assistant.rs）：6 = UIReturnKeySearch，1 = 不自动更正。
        let _: () = msg_send![field, setClearButtonMode: 1isize];
        let set: unsafe extern "C" fn(*mut AnyObject, Sel, isize) = std::mem::transmute(objc2::ffi::objc_msgSend as *const ());
        set(field, sel!(setReturnKeyType:), 6);
        set(field, sel!(setAutocorrectionType:), 1);
        let _: () = msg_send![field, setDelegate: vc];
        let anchor = |v: *mut AnyObject, name: &std::ffi::CStr| -> *mut AnyObject { msg_send![v, performSelector: Sel::register(name)] };
        let guide: *mut AnyObject = msg_send![view, keyboardLayoutGuide];
        let constraints: [*mut AnyObject; 4] = [
            msg_send![anchor(bar, c"leadingAnchor"), constraintEqualToAnchor: anchor(view, c"leadingAnchor"), constant: GAP],
            msg_send![anchor(bar, c"trailingAnchor"), constraintEqualToAnchor: anchor(view, c"trailingAnchor"), constant: -GAP],
            msg_send![anchor(bar, c"bottomAnchor"), constraintEqualToAnchor: anchor(guide, c"topAnchor"), constant: -10f64],
            msg_send![anchor(bar, c"heightAnchor"), constraintEqualToConstant: BAR_H],
        ];
        for c in constraints { let _: () = msg_send![c, setActive: true]; }

        SHEET.with(|s| *s.borrow_mut() = Some(Sheet { vc, scroll, field, status, target, photos: vec![], tiles: vec![], cols: [0., 0.], width: 0., query: String::new(), page: 0, loading: false, done: false, revision: 0 }));
    }
}

/// 收起。`pick`：选中的照片，收起后去下载。
fn finish(pick: Option<(Photo, Target)>, animated: bool) {
    let Some(sheet) = SHEET.with(|s| s.borrow_mut().take()) else { return };
    unsafe {
        let _: Bool = msg_send![sheet.field, resignFirstResponder];
        if animated { let _: () = msg_send![sheet.vc, dismissViewControllerAnimated: true, completion: std::ptr::null_mut::<AnyObject>()]; }
        let _: () = msg_send![sheet.vc, release];
    }
    if let Some((photo, target)) = pick { apply(photo, target); }
}

fn set_status(text: Option<&str>) {
    SHEET.with(|s| if let Some(s) = s.borrow().as_ref() { unsafe {
        let _: () = msg_send![s.status, setText: ns_string(text.unwrap_or(""))];
        let _: () = msg_send![s.status, setHidden: text.is_none()];
    } });
}

/// 取一页。`reset`：换了关键词，从第一页重来。
fn load(reset: bool) {
    let job = SHEET.with(|s| {
        let mut s = s.borrow_mut();
        let s = s.as_mut()?;
        if reset {
            for t in s.tiles.drain(..) { unsafe { let _: () = msg_send![t, removeFromSuperview]; } }
            s.photos.clear();
            s.cols = [0., 0.];
            s.page = 0;
            s.done = false;
            s.revision = s.revision.wrapping_add(1);
            unsafe { let _: () = msg_send![s.scroll, setContentOffset: CGPoint::new(0., 0.)]; }
        } else if s.loading || s.done { return None; }
        s.loading = true;
        s.page += 1;
        Some((s.query.clone(), s.page, s.revision))
    });
    let Some((query, page, revision)) = job else { return };
    if page == 1 { set_status(Some(tr("正在加载…", "Loading…"))); }
    let work = super::super::assistant::llm::runtime().spawn(search(query, page));
    super::super::menu::dispatch(move |_, _, cx| {
        cx.spawn(async move |_, _| {
            let found = work.await.ok().and_then(Result::ok);
            let current = SHEET.with(|s| s.borrow().as_ref().is_some_and(|s| s.revision == revision));
            if !current { return; }
            let failed = found.is_none();
            // 没取到：页码退回去；后面的页取不到就先停在这儿（换个关键词会从头再来），免得滚一下就重发一次。
            SHEET.with(|s| if let Some(s) = s.borrow_mut().as_mut() { s.loading = false; if failed { s.page -= 1; s.done = page > 1; } });
            let Some((photos, last)) = found else {
                if page == 1 { set_status(Some(tr("无法连接 Unsplash", "Couldn’t reach Unsplash"))); }
                return;
            };
            let empty = photos.is_empty() && page == 1;
            set_status(empty.then(|| tr("没有找到照片", "No photos found")));
            append(photos, last);
        }).detach();
    });
}

fn append(photos: Vec<Photo>, last: bool) {
    let added: Vec<(usize, Photo)> = SHEET.with(|s| {
        let mut s = s.borrow_mut();
        let Some(s) = s.as_mut() else { return vec![] };
        s.done = last;
        let start = s.photos.len();
        s.photos.extend(photos.iter().cloned());
        unsafe {
            for (i, p) in photos.iter().enumerate() {
                let ix = start + i;
                let tile = add(s.scroll, new_view(c"UIImageView", zero()));
                let _: () = msg_send![tile, setBackgroundColor: hex_color(&p.color)];
                // 2 = 等比铺满，裁掉多的。
                let _: () = msg_send![tile, setContentMode: 2isize];
                let _: () = msg_send![tile, setClipsToBounds: true];
                let _: () = msg_send![tile, setUserInteractionEnabled: true];
                let layer: *mut AnyObject = msg_send![tile, layer];
                let _: () = msg_send![layer, setCornerRadius: 4f64];
                // 左下角摄影师名字，压一层往上淡掉的暗色好认字。
                let shade: *mut AnyObject = msg_send![AnyClass::get(c"CAGradientLayer").unwrap(), layer];
                let stops: *mut AnyObject = msg_send![AnyClass::get(c"NSMutableArray").unwrap(), array];
                for a in [0f64, 0.35] {
                    let c: *mut AnyObject = msg_send![AnyClass::get(c"UIColor").unwrap(), colorWithWhite: 0f64, alpha: a];
                    let cg: *const CGColor = msg_send![c, CGColor];
                    let _: () = msg_send![stops, addObject: cg as *mut AnyObject];
                }
                let _: () = msg_send![shade, setColors: stops];
                let _: () = msg_send![shade, setName: ns_string("shade")];
                let _: () = msg_send![layer, addSublayer: shade];
                let name = add(tile, new_view(c"UILabel", zero()));
                let _: () = msg_send![name, setText: ns_string(&p.author)];
                let _: () = msg_send![name, setTextColor: ui(c"whiteColor")];
                let font: *mut AnyObject = msg_send![AnyClass::get(c"UIFont").unwrap(), systemFontOfSize: 11f64, weight: 0.23f64];
                let _: () = msg_send![name, setFont: font];
                let _: () = msg_send![name, setTag: 7isize];
                let button = add(tile, new_view(c"UIButton", zero()));
                let _: () = msg_send![button, setTag: TAG + ix as isize];
                let _: () = msg_send![button, setAccessibilityLabel: ns_string(&format!("{} · Unsplash", p.author))];
                let _: () = msg_send![button, addTarget: s.vc, action: sel!(unsplashPick:), forControlEvents: 64usize];
                let _: () = msg_send![button, setAutoresizingMask: 18usize];
                s.tiles.push(tile);
                place(s, ix);
            }
            let _: () = msg_send![s.scroll, setContentSize: CGSize::new(s.width, s.cols[0].max(s.cols[1]) + GAP)];
        }
        photos.into_iter().enumerate().map(|(i, p)| (start + i, p)).collect()
    });
    let revision = SHEET.with(|s| s.borrow().as_ref().map_or(0, |s| s.revision));
    for (ix, photo) in added {
        let work = super::super::assistant::llm::runtime().spawn(thumbnail(photo.thumb));
        super::super::menu::dispatch(move |_, _, cx| {
            cx.spawn(async move |_, _| {
                let Ok(Ok(path)) = work.await else { return };
                SHEET.with(|s| if let Some(s) = s.borrow().as_ref().filter(|s| s.revision == revision) && let Some(tile) = s.tiles.get(ix) { unsafe {
                    let image: *mut AnyObject = msg_send![AnyClass::get(c"UIImage").unwrap(), imageWithContentsOfFile: ns_string(&path.to_string_lossy())];
                    if image.is_null() { return; }
                    let _: () = msg_send![*tile, setAlpha: 0f64];
                    let _: () = msg_send![*tile, setImage: image];
                    let tile = *tile as usize;
                    let body = block2::RcBlock::new(move || { let _: () = msg_send![tile as *mut AnyObject, setAlpha: 1f64]; });
                    let _: () = msg_send![AnyClass::get(c"UIView").unwrap(), animateWithDuration: 0.2f64, animations: &*body];
                } });
            }).detach();
        });
    }
}

/// 把第 `ix` 张放进当时较矮的那一列。
unsafe fn place(s: &mut Sheet, ix: usize) {
    if s.width <= 0. { return; }
    let cw = ((s.width - GAP * 3.) / 2.).max(40.);
    let p = &s.photos[ix];
    let h = (cw * p.h / p.w.max(1.)).clamp(cw * 0.5, cw * 1.8).round();
    let col = if s.cols[0] <= s.cols[1] { 0 } else { 1 };
    let frame = CGRect::new(CGPoint::new(GAP + (cw + GAP) * col as f64, s.cols[col]), CGSize::new(cw, h));
    s.cols[col] += h + GAP;
    unsafe {
        let tile = s.tiles[ix];
        let _: () = msg_send![tile, setFrame: frame];
        let layer: *mut AnyObject = msg_send![tile, layer];
        let subs: *mut AnyObject = msg_send![layer, sublayers];
        let n: usize = if subs.is_null() { 0 } else { msg_send![subs, count] };
        for i in 0..n {
            let l: *mut AnyObject = msg_send![subs, objectAtIndex: i];
            let name: *mut AnyObject = msg_send![l, name];
            if !name.is_null() { let _: () = msg_send![l, setFrame: CGRect::new(CGPoint::new(0., h - 34.), CGSize::new(cw, 34.))]; }
        }
        let label: *mut AnyObject = msg_send![tile, viewWithTag: 7isize];
        let _: () = msg_send![label, setFrame: CGRect::new(CGPoint::new(8., h - 22.), CGSize::new(cw - 16., 16.))];
        let subviews: *mut AnyObject = msg_send![tile, subviews];
        let n: usize = msg_send![subviews, count];
        for i in 0..n {
            let v: *mut AnyObject = msg_send![subviews, objectAtIndex: i];
            if v != label { let _: () = msg_send![v, setFrame: CGRect::new(CGPoint::new(0., 0.), CGSize::new(cw, h))]; }
        }
    }
}

fn layout(view: *mut AnyObject) {
    SHEET.with(|s| {
        let mut s = s.borrow_mut();
        let Some(s) = s.as_mut() else { return };
        unsafe {
            let bounds: CGRect = msg_send![view, bounds];
            let (w, h) = (bounds.size.width, bounds.size.height);
            let title: *mut AnyObject = msg_send![view, viewWithTag: 11isize];
            let _: () = msg_send![title, setFrame: CGRect::new(CGPoint::new(70., 14.), CGSize::new((w - 140.).max(0.), 40.))];
            let _: () = msg_send![s.scroll, setFrame: CGRect::new(CGPoint::new(0., HEAD), CGSize::new(w, h - HEAD))];
            let _: () = msg_send![s.status, setFrame: CGRect::new(CGPoint::new(20., HEAD + 80.), CGSize::new((w - 40.).max(0.), 24.))];
            let safe: Insets = msg_send![view, safeAreaInsets];
            let insets = Insets { top: 4., left: 0., bottom: BAR_H + 24. + safe.bottom, right: 0. };
            let _: () = msg_send![s.scroll, setContentInset: insets];
            if (s.width - w).abs() < 0.5 { return; }
            s.width = w;
            s.cols = [0., 0.];
            for ix in 0..s.tiles.len() { place(s, ix); }
            let _: () = msg_send![s.scroll, setContentSize: CGSize::new(w, s.cols[0].max(s.cols[1]) + GAP)];
        }
    });
}

fn scrolled(scroll: *mut AnyObject) {
    // 改 contentOffset / contentSize 时 UIKit 会当场回调过来，那时 SHEET 正借着：这一下跳过。
    if SHEET.with(|s| s.try_borrow_mut().is_err()) { return; }
    let near_end = unsafe {
        let offset: CGPoint = msg_send![scroll, contentOffset];
        let size: CGSize = msg_send![scroll, contentSize];
        let frame: CGRect = msg_send![scroll, bounds];
        size.height > 0. && offset.y + frame.size.height > size.height - 900.
    };
    if near_end { load(false); }
}

fn searched(field: *mut AnyObject) {
    let text: String = unsafe {
        let _: Bool = msg_send![field, resignFirstResponder];
        let t: *mut AnyObject = msg_send![field, text];
        if t.is_null() { String::new() } else {
            let p: *const std::ffi::c_char = msg_send![t, UTF8String];
            std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned()
        }
    };
    let text: String = text.trim().chars().take(100).collect();
    SHEET.with(|s| if let Some(s) = s.borrow_mut().as_mut() { s.query = text; });
    load(true);
}

fn picked(ix: usize) {
    let pick = SHEET.with(|s| s.borrow().as_ref().and_then(|s| Some((s.photos.get(ix)?.clone(), s.target))));
    super::super::glass::haptic();
    finish(pick, true);
}

// ---- 网络

fn client() -> anyhow::Result<Client> {
    Ok(Client::builder().https_only(true).timeout(Duration::from_secs(15)).build()?)
}

async fn get(http: &Client, url: Url, limit: usize) -> anyhow::Result<Vec<u8>> {
    let mut response = http.get(url).header("Authorization", format!("Client-ID {}", KEY.unwrap_or_default().trim()))
        .header("Accept-Version", "v1").send().await?.error_for_status()?;
    anyhow::ensure!(response.content_length().is_none_or(|n| n <= limit as u64), "response is too large");
    let mut data = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        anyhow::ensure!(data.len().saturating_add(chunk.len()) <= limit, "response is too large");
        data.extend_from_slice(&chunk);
    }
    Ok(data)
}

/// 照片只从 Unsplash 自己的图床下，统一要 JPEG。
fn image_url(source: &str, width: u32) -> anyhow::Result<Url> {
    let mut url = Url::parse(source)?;
    anyhow::ensure!(url.scheme() == "https" && url.host_str() == Some("images.unsplash.com"), "unexpected image host");
    let keep: Vec<(String, String)> = url.query_pairs().filter(|(k, _)| !matches!(k.as_ref(), "fm" | "w" | "q")).map(|(k, v)| (k.into(), v.into())).collect();
    url.query_pairs_mut().clear().extend_pairs(keep).append_pair("fm", "jpg").append_pair("q", "80").append_pair("w", &width.to_string());
    Ok(url)
}

async fn search(query: String, page: u32) -> anyhow::Result<(Vec<Photo>, bool)> {
    let http = client()?;
    let mut url = Url::parse(API)?;
    if query.is_empty() {
        url.set_path("/photos");
    } else {
        url.set_path("/search/photos");
        url.query_pairs_mut().append_pair("query", &query).append_pair("content_filter", "high");
    }
    url.query_pairs_mut().append_pair("page", &page.to_string()).append_pair("per_page", &PER_PAGE.to_string());
    let body: Value = serde_json::from_slice(&get(&http, url, 3_000_000).await?)?;
    let (list, last) = match &body {
        Value::Array(list) => (list.clone(), list.len() < PER_PAGE as usize),
        _ => (body["results"].as_array().cloned().unwrap_or_default(), u64::from(page) >= body["total_pages"].as_u64().unwrap_or(0)),
    };
    // 只留 images.unsplash.com 上的（Unsplash+ 之类别的图床下不了）。
    let photos = list.iter().filter(|p| p["urls"]["raw"].as_str().is_some_and(|u| image_url(u, 400).is_ok())).filter_map(|p| Some(Photo {
        w: p["width"].as_f64()?, h: p["height"].as_f64()?, color: p["color"].as_str().unwrap_or("#dddddd").into(),
        thumb: p["urls"]["raw"].as_str()?.into(), full: p["urls"]["raw"].as_str()?.into(),
        ping: p["links"]["download_location"].as_str()?.into(), author: p["user"]["name"].as_str()?.trim().into(),
        link: p["links"]["html"].as_str().unwrap_or("https://unsplash.com").into(),
    })).collect();
    Ok((photos, last))
}

fn validate_jpeg(data: &[u8]) -> anyhow::Result<()> {
    use image::{ImageFormat, ImageReader};
    let (w, h) = ImageReader::with_format(std::io::Cursor::new(data), ImageFormat::Jpeg).into_dimensions()?;
    anyhow::ensure!(u64::from(w) * u64::from(h) <= 24_000_000, "image is too large");
    Ok(())
}

async fn thumbnail(source: String) -> anyhow::Result<std::path::PathBuf> {
    let url = image_url(&source, 400)?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    url.hash(&mut hasher);
    let path = std::env::temp_dir().join("quiet-unsplash").join(format!("{:016x}.jpg", hasher.finish()));
    if !path.is_file() {
        let data = get(&client()?, url, 2_000_000).await?;
        validate_jpeg(&data)?;
        if let Some(dir) = path.parent() { std::fs::create_dir_all(dir)?; }
        let tmp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
        std::fs::write(&tmp, &data)?;
        std::fs::rename(&tmp, &path)?;
    }
    Ok(path)
}

/// 按 Unsplash 的规矩先报一次下载，再下大图。
async fn full(photo: Photo) -> anyhow::Result<Vec<u8>> {
    let http = client()?;
    let ping = Url::parse(&photo.ping)?;
    anyhow::ensure!(ping.host_str() == Some("api.unsplash.com"), "unexpected download host");
    let _ = get(&http, ping, 100_000).await;
    let data = get(&http, image_url(&photo.full, 1600)?, 12_000_000).await?;
    validate_jpeg(&data)?;
    Ok(data)
}

fn apply(photo: Photo, target: Target) {
    let credit = (photo.author.clone(), photo.link.clone());
    let work = super::super::assistant::llm::runtime().spawn(full(photo));
    super::super::menu::dispatch(move |w, window, cx| {
        let document = w.current.clone();
        cx.spawn_in(window, async move |this, cx| {
            let bytes = work.await.ok().and_then(Result::ok);
            let _ = this.update_in(cx, |w, _, cx| {
                if w.current != document { return; }
                let Some(bytes) = bytes else {
                    return w.show_toast_kind(crate::ui::ToastKind::Error, tr("没能下载这张照片", "Couldn’t download the photo"), cx);
                };
                match target {
                    Target::Cover => if let Some(editor) = w.editor.clone() { super::super::glass::selection_haptic(); editor.update(cx, |e, cx| { e.set_cover_image(Some((&bytes, "jpg")), cx); }); },
                    Target::Background => super::unsplash_background(&bytes, &credit.0, &credit.1, w, cx),
                }
            });
        }).detach();
    });
}

fn controller_class() -> &'static AnyClass {
    static CLASS: std::sync::OnceLock<&'static AnyClass> = std::sync::OnceLock::new();
    CLASS.get_or_init(|| {
        extern "C-unwind" fn laid_out(this: &AnyObject, _: Sel) {
            let _: () = unsafe { msg_send![super(this, AnyClass::get(c"UIViewController").unwrap()), viewDidLayoutSubviews] };
            let view: *mut AnyObject = unsafe { msg_send![this, view] };
            layout(view);
        }
        extern "C-unwind" fn close(_this: &AnyObject, _: Sel, _sender: *mut AnyObject) { finish(None, true); }
        extern "C-unwind" fn pick(_this: &AnyObject, _: Sel, sender: *mut AnyObject) { let tag: isize = unsafe { msg_send![sender, tag] }; picked((tag - TAG).max(0) as usize); }
        extern "C-unwind" fn scroll(_this: &AnyObject, _: Sel, view: *mut AnyObject) { scrolled(view); }
        extern "C-unwind" fn ret(_this: &AnyObject, _: Sel, field: *mut AnyObject) -> Bool { searched(field); Bool::YES }
        // 往下拖收起了（没经过 ✕）。
        extern "C-unwind" fn dismissed(_this: &AnyObject, _: Sel, _pc: *mut AnyObject) { finish(None, false); }
        let mut b = ClassBuilder::new(c"QuietMdUnsplashController", AnyClass::get(c"UIViewController").unwrap()).expect("register QuietMdUnsplashController");
        unsafe {
            b.add_method(sel!(viewDidLayoutSubviews), laid_out as extern "C-unwind" fn(_, _));
            b.add_method(sel!(unsplashClose:), close as extern "C-unwind" fn(_, _, _));
            b.add_method(sel!(unsplashPick:), pick as extern "C-unwind" fn(_, _, _));
            b.add_method(sel!(scrollViewDidScroll:), scroll as extern "C-unwind" fn(_, _, _));
            b.add_method(sel!(textFieldShouldReturn:), ret as extern "C-unwind" fn(_, _, _) -> _);
            b.add_method(sel!(presentationControllerDidDismiss:), dismissed as extern "C-unwind" fn(_, _, _));
        }
        b.register()
    })
}
