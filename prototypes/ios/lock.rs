//! 设置 › 安全（照 Craft，IMG_9171）：用面容 ID 锁住 Margin。开着时，App 一进后台就盖上一层（多任务切换里也看不到文档），
//! 刚打开和回到前台时要验证才揭开；验证不过就停在锁屏上，点「解锁」再来。验证交给系统的 LocalAuthentication
//! （策略 2：生物识别，认不出时可以用设备密码），Margin 自己不碰也不存生物信息。盖的那层是系统视图，不经过 GPUI。
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU8, Ordering};
use std::sync::Arc;

use objc2::runtime::{AnyClass, AnyObject, Sel};
use objc2::{msg_send, sel};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};

#[link(name = "LocalAuthentication", kind = "framework")]
unsafe extern "C" {}

/// LAPolicyDeviceOwnerAuthentication：生物识别，失败时可退到设备密码。
const POLICY: isize = 2;

/// 盖着的那层（只在主线程加；验证成功的回调在别的线程，取走指针后交给主线程移除）。
static COVER: AtomicPtr<AnyObject> = AtomicPtr::new(null_mut());
/// 这次进前台后验证过了。进后台清掉。
static UNLOCKED: AtomicBool = AtomicBool::new(false);
/// 刚打开或刚从后台回来：变成活跃时自动问一次。验证面板本身也会让 App 失活再活跃，不能每次都问，不然取消了会一直弹。
static ASK_ON_ACTIVE: AtomicBool = AtomicBool::new(true);
/// 正在验证。
static ASKING: AtomicBool = AtomicBool::new(false);

/// 这台设备能用什么解锁。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind { FaceId, TouchId, OpticId, Passcode, Unavailable }

pub fn kind() -> Kind {
    unsafe {
        let Some(class) = AnyClass::get(c"LAContext") else { return Kind::Unavailable };
        let ctx: *mut AnyObject = msg_send![class, new];
        let mut err: *mut AnyObject = null_mut();
        let ok: bool = msg_send![ctx, canEvaluatePolicy: POLICY, error: &mut err];
        let bio: isize = msg_send![ctx, biometryType];
        let _: () = msg_send![ctx, release];
        if !ok { return Kind::Unavailable }
        match bio { 2 => Kind::FaceId, 1 => Kind::TouchId, 4 => Kind::OpticId, _ => Kind::Passcode }
    }
}

/// 「面容 ID」这类名字，放进设置的文案里。
pub fn method(k: Kind) -> &'static str {
    use crate::settings::tr;
    match k { Kind::FaceId => tr("面容 ID", "Face ID"), Kind::TouchId => tr("触控 ID", "Touch ID"), Kind::OpticId => tr("视控 ID", "Optic ID"), Kind::Passcode | Kind::Unavailable => tr("设备密码", "Passcode") }
}

fn enabled() -> bool { crate::settings::prefs().lock }

/// 让系统验证一次。结果放进返回值：0 还在等，1 通过，2 没通过（取消、失败、设备不支持）。回调在系统的线程上。
pub fn evaluate(reason: &str) -> Arc<AtomicU8> {
    let result = Arc::new(AtomicU8::new(0));
    unsafe {
        let Some(class) = AnyClass::get(c"LAContext") else { result.store(2, Ordering::SeqCst); return result };
        let ctx: *mut AnyObject = msg_send![class, new];
        let done = result.clone();
        let reply = block2::RcBlock::new(move |ok: objc2::runtime::Bool, _err: *mut AnyObject| { done.store(if ok.as_bool() { 1 } else { 2 }, Ordering::SeqCst); });
        let _: () = msg_send![ctx, evaluatePolicy: POLICY, localizedReason: ns_string(reason), reply: &*reply];
        // 系统在验证结束前一直持有它。
        let _: () = msg_send![ctx, release];
    }
    result
}

/// 启动时调一次：跟着 App 进后台、回前台盖上和揭开。
pub fn install() {
    unsafe {
        let center: *mut AnyObject = msg_send![AnyClass::get(c"NSNotificationCenter").unwrap(), defaultCenter];
        let queue: *mut AnyObject = msg_send![AnyClass::get(c"NSOperationQueue").unwrap(), mainQueue];
        let background = block2::RcBlock::new(|_: *mut AnyObject| {
            UNLOCKED.store(false, Ordering::SeqCst);
            if enabled() { cover(); ASK_ON_ACTIVE.store(true, Ordering::SeqCst); }
        });
        let active = block2::RcBlock::new(|_: *mut AnyObject| {
            if !enabled() || UNLOCKED.load(Ordering::SeqCst) { return }
            cover();
            if ASK_ON_ACTIVE.swap(false, Ordering::SeqCst) { ask(); }
        });
        let null: *mut AnyObject = null_mut();
        let _: *mut AnyObject = msg_send![center, addObserverForName: ns_string("UIApplicationDidEnterBackgroundNotification"), object: null, queue: queue, usingBlock: &*background];
        let _: *mut AnyObject = msg_send![center, addObserverForName: ns_string("UIApplicationDidBecomeActiveNotification"), object: null, queue: queue, usingBlock: &*active];
    }
}

/// 刚在设置里打开锁：这一次已经验证过，不用马上锁。
pub fn mark_unlocked() { UNLOCKED.store(true, Ordering::SeqCst); }

fn ask() {
    if ASKING.swap(true, Ordering::SeqCst) { return }
    let reason = crate::settings::tr("解锁 Margin", "Unlock Margin");
    unsafe {
        let Some(class) = AnyClass::get(c"LAContext") else { ASKING.store(false, Ordering::SeqCst); return };
        let ctx: *mut AnyObject = msg_send![class, new];
        let reply = block2::RcBlock::new(|ok: objc2::runtime::Bool, _err: *mut AnyObject| {
            ASKING.store(false, Ordering::SeqCst);
            if !ok.as_bool() { return }
            UNLOCKED.store(true, Ordering::SeqCst);
            // 不在主线程：把那层交给主线程拿掉（父视图还持有它，到移除时才释放）。
            let view = COVER.swap(null_mut(), Ordering::SeqCst);
            if !view.is_null() {
                let null: *mut AnyObject = null_mut();
                let remove: Sel = sel!(removeFromSuperview);
                let _: () = msg_send![view, performSelectorOnMainThread: remove, withObject: null, waitUntilDone: false];
            }
        });
        let _: () = msg_send![ctx, evaluatePolicy: POLICY, localizedReason: ns_string(reason), reply: &*reply];
        let _: () = msg_send![ctx, release];
    }
}

/// 盖上：整窗一层厚磨砂，正中一把锁、「Margin 已锁定」和「解锁」。已经盖着就不再盖。
fn cover() {
    if !COVER.load(Ordering::SeqCst).is_null() { return }
    unsafe {
        let root = super::files::root_controller();
        if root.is_null() { return }
        let view: *mut AnyObject = msg_send![root, view];
        let window: *mut AnyObject = msg_send![view, window];
        if window.is_null() { return }
        let bounds: CGRect = msg_send![window, bounds];
        // UIBlurEffectStyleSystemThickMaterial = 9：看不出底下的字。
        let effect: *mut AnyObject = msg_send![AnyClass::get(c"UIBlurEffect").unwrap(), effectWithStyle: 9isize];
        let blur: *mut AnyObject = msg_send![AnyClass::get(c"UIVisualEffectView").unwrap(), alloc];
        let blur: *mut AnyObject = msg_send![blur, initWithEffect: effect];
        let _: () = msg_send![blur, setFrame: bounds];
        let _: () = msg_send![blur, setAutoresizingMask: 18usize];
        let content: *mut AnyObject = msg_send![blur, contentView];
        let (w, h) = (bounds.size.width, bounds.size.height);
        let mid = h * 0.42;
        // 锁。
        let config: *mut AnyObject = msg_send![AnyClass::get(c"UIImageSymbolConfiguration").unwrap(), configurationWithPointSize: 44f64, weight: 4isize];
        let symbol: *mut AnyObject = msg_send![AnyClass::get(c"UIImage").unwrap(), systemImageNamed: ns_string("lock.fill"), withConfiguration: config];
        let icon: *mut AnyObject = msg_send![AnyClass::get(c"UIImageView").unwrap(), alloc];
        let icon: *mut AnyObject = msg_send![icon, initWithImage: symbol];
        let secondary: *mut AnyObject = msg_send![AnyClass::get(c"UIColor").unwrap(), secondaryLabelColor];
        let _: () = msg_send![icon, setTintColor: secondary];
        let _: () = msg_send![icon, setContentMode: 1isize];
        let _: () = msg_send![icon, setFrame: CGRect::new(CGPoint::new(w / 2. - 30., mid - 76.), CGSize::new(60., 60.))];
        // 文字。
        let label: *mut AnyObject = msg_send![AnyClass::get(c"UILabel").unwrap(), new];
        let _: () = msg_send![label, setText: ns_string(crate::settings::tr("Margin 已锁定", "Margin is locked"))];
        let font: *mut AnyObject = msg_send![AnyClass::get(c"UIFont").unwrap(), systemFontOfSize: 17f64, weight: 0.3f64];
        let _: () = msg_send![label, setFont: font];
        let _: () = msg_send![label, setTextAlignment: 1isize];
        let _: () = msg_send![label, setFrame: CGRect::new(CGPoint::new(0., mid), CGSize::new(w, 24.))];
        // 解锁。
        let button: *mut AnyObject = msg_send![AnyClass::get(c"UIButton").unwrap(), buttonWithType: 1isize];
        let _: () = msg_send![button, setTitle: ns_string(crate::settings::tr("解锁", "Unlock")), forState: 0usize];
        let bold: *mut AnyObject = msg_send![AnyClass::get(c"UIFont").unwrap(), systemFontOfSize: 17f64, weight: 0.23f64];
        let title: *mut AnyObject = msg_send![button, titleLabel];
        let _: () = msg_send![title, setFont: bold];
        let _: () = msg_send![button, setFrame: CGRect::new(CGPoint::new(w / 2. - 80., mid + 44.), CGSize::new(160., 44.))];
        let handler = block2::RcBlock::new(|_: *mut AnyObject| ask());
        let action: *mut AnyObject = msg_send![AnyClass::get(c"UIAction").unwrap(), actionWithHandler: &*handler];
        let _: () = msg_send![button, addAction: action, forControlEvents: 64usize];
        // 转屏、分屏时跟着居中。
        for v in [icon, label, button] {
            let _: () = msg_send![v, setAutoresizingMask: if v == label { 2usize | 8 | 32 } else { 1usize | 4 | 8 | 32 }];
            let _: () = msg_send![content, addSubview: v];
        }
        let _: () = msg_send![icon, release];
        let _: () = msg_send![label, release];
        let _: () = msg_send![window, addSubview: blur];
        let _: () = msg_send![blur, release];
        COVER.store(blur, Ordering::SeqCst);
    }
}

unsafe fn ns_string(s: &str) -> *mut AnyObject {
    let c = std::ffi::CString::new(s).unwrap_or_default();
    unsafe { msg_send![AnyClass::get(c"NSString").unwrap(), stringWithUTF8String: c.as_ptr()] }
}
