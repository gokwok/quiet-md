//! 改一段长文字（提示词的内容、正则、助手和人设的描述）：系统的页面卡片，顶上导航栏左边取消、右边完成，下面整页是 `UITextView`。
//! 预设里的字要原样留着：弯引号、破折号、自动更正、首字母大写、拼写检查都关掉。
use std::ffi::CStr;

use objc2::{class, msg_send, runtime::AnyObject, Encode, Encoding};

use super::ns_string;

#[repr(C)] #[derive(Clone, Copy)] struct Insets { top: f64, left: f64, bottom: f64, right: f64 }
unsafe impl Encode for Insets { const ENCODING: Encoding = Encoding::Struct("UIEdgeInsets", &[const { Encoding::Double }; 4]); }

/// 弹出来改 `value`；点完成时把改后的字交给 `done`（没改也交）。`mono` 用等宽字（正则）。返回假是没弹出来（上面正弹着别的）。
pub fn edit(title: &str, value: &str, mono: bool, done: impl Fn(String) + 'static) -> bool {
    unsafe {
        let host = super::super::files::root_controller();
        if host.is_null() { return false; }
        let presented: *mut AnyObject = msg_send![host, presentedViewController];
        if !presented.is_null() { return false; }

        let text: *mut AnyObject = msg_send![class!(UITextView), new];
        let font: *mut AnyObject = if mono { msg_send![class!(UIFont), monospacedSystemFontOfSize: 15f64, weight: 0f64] } else { msg_send![class!(UIFont), systemFontOfSize: 17f64] };
        let _: () = msg_send![text, setFont: font];
        let _: () = msg_send![text, setText: ns_string(value)];
        let _: () = msg_send![text, setTextContainerInset: Insets { top: 16., left: 14., bottom: 32., right: 14. }];
        let _: () = msg_send![text, setAlwaysBounceVertical: true];
        // 2 = UIScrollViewKeyboardDismissModeInteractive：往下拖就收键盘。
        let _: () = msg_send![text, setKeyboardDismissMode: 2isize];
        let background: *mut AnyObject = msg_send![class!(UIColor), systemBackgroundColor];
        let _: () = msg_send![text, setBackgroundColor: background];
        // 输入特性在 UITextView 上是转发实现的，objc2 调试构建的方法检查找不到，直接走 objc_msgSend（同 `prompt`）。
        let set: unsafe extern "C" fn(*mut AnyObject, objc2::runtime::Sel, isize) = std::mem::transmute(objc2::ffi::objc_msgSend as *const ());
        set(text, objc2::sel!(setAutocapitalizationType:), 0);
        set(text, objc2::sel!(setAutocorrectionType:), 1);
        set(text, objc2::sel!(setSpellCheckingType:), 1);
        set(text, objc2::sel!(setSmartQuotesType:), 1);
        set(text, objc2::sel!(setSmartDashesType:), 1);
        set(text, objc2::sel!(setSmartInsertDeleteType:), 1);

        let page: *mut AnyObject = msg_send![class!(UIViewController), new];
        let _: () = msg_send![page, setView: text];
        let _: () = msg_send![page, setTitle: ns_string(title)];
        let nav: *mut AnyObject = msg_send![class!(UINavigationController), alloc];
        let nav: *mut AnyObject = msg_send![nav, initWithRootViewController: page];
        // 卡片不能往下一划就关掉：改了一半的字不会悄悄丢。
        let _: () = msg_send![nav, setModalInPresentation: true];
        let _: () = msg_send![nav, setOverrideUserInterfaceStyle: if crate::theme::dark() { 2isize } else { 1isize }];

        let close = move |save: bool| {
            if save {
                let s: *mut AnyObject = msg_send![text, text];
                let chars: *const std::ffi::c_char = if s.is_null() { std::ptr::null() } else { msg_send![s, UTF8String] };
                done(if chars.is_null() { String::new() } else { CStr::from_ptr(chars).to_string_lossy().into_owned() });
            }
            let _: bool = msg_send![text, resignFirstResponder];
            let none: Option<&block2::Block<dyn Fn()>> = None;
            let _: () = msg_send![nav, dismissViewControllerAnimated: true, completion: none];
        };
        let close = std::rc::Rc::new(close);
        let button = |system: isize, save: bool| -> *mut AnyObject {
            let run = close.clone();
            let handler = block2::RcBlock::new(move |_: *mut AnyObject| run(save));
            let action: *mut AnyObject = msg_send![class!(UIAction), actionWithTitle: ns_string(""), image: std::ptr::null_mut::<AnyObject>(), identifier: std::ptr::null_mut::<AnyObject>(), handler: &*handler];
            let item: *mut AnyObject = msg_send![class!(UIBarButtonItem), alloc];
            msg_send![item, initWithBarButtonSystemItem: system, primaryAction: action]
        };
        let item: *mut AnyObject = msg_send![page, navigationItem];
        // UIBarButtonSystemItemCancel = 1，Done = 0（iOS 26 上画成叉和勾）。
        let _: () = msg_send![item, setLeftBarButtonItem: button(1, false)];
        let _: () = msg_send![item, setRightBarButtonItem: button(0, true)];

        // 短的字直接弹键盘；长的（整段提示词）先让人看，点哪里改哪里。
        let short = value.chars().count() < 240;
        let focus = block2::RcBlock::new(move || if short { let _: bool = msg_send![text, becomeFirstResponder]; });
        let _: () = msg_send![host, presentViewController: nav, animated: true, completion: &*focus];
        let _: () = msg_send![text, release];
        let _: () = msg_send![page, release];
        let _: () = msg_send![nav, release];
        true
    }
}

/// 只读地看一段分好段的字（生成图的提示词）：同样的页面卡片，左边关掉，右边「拷贝」拷 `copy`。
/// 每段一个灰色小标题，下面是内容（第一段是人写的话，用正文字；后面是标签，用等宽字；都可以选中拷贝一截）；小标题空的那段整段灰着（模型、尺寸这类附注）。
pub fn show(title: &str, sections: &[(String, String)], copy: &str) -> bool {
    unsafe {
        let host = super::super::files::root_controller();
        if host.is_null() { return false; }
        let presented: *mut AnyObject = msg_send![host, presentedViewController];
        if !presented.is_null() { return false; }

        let body: *mut AnyObject = msg_send![class!(UIFont), monospacedSystemFontOfSize: 15f64, weight: 0f64];
        let plain: *mut AnyObject = msg_send![class!(UIFont), systemFontOfSize: 17f64];
        let head: *mut AnyObject = msg_send![class!(UIFont), systemFontOfSize: 13f64, weight: 0.3f64];
        let note: *mut AnyObject = msg_send![class!(UIFont), systemFontOfSize: 13f64];
        let ink: *mut AnyObject = msg_send![class!(UIColor), labelColor];
        let grey: *mut AnyObject = msg_send![class!(UIColor), secondaryLabelColor];
        let all: *mut AnyObject = msg_send![class!(NSMutableAttributedString), new];
        let append = |s: &str, font: *mut AnyObject, color: *mut AnyObject| {
            let attrs: *mut AnyObject = msg_send![class!(NSMutableDictionary), dictionary];
            let _: () = msg_send![attrs, setObject: font, forKey: ns_string("NSFont")];
            let _: () = msg_send![attrs, setObject: color, forKey: ns_string("NSColor")];
            let piece: *mut AnyObject = msg_send![class!(NSAttributedString), alloc];
            let piece: *mut AnyObject = msg_send![piece, initWithString: ns_string(s), attributes: attrs];
            let _: () = msg_send![all, appendAttributedString: piece];
            let _: () = msg_send![piece, release];
        };
        for (i, (h, t)) in sections.iter().enumerate() {
            if i > 0 { append("\n\n", note, grey); }
            if h.is_empty() { append(t.trim(), note, grey); continue; }
            append(&format!("{h}\n"), head, grey);
            append(t.trim(), if i == 0 { plain } else { body }, ink);
        }

        let text: *mut AnyObject = msg_send![class!(UITextView), new];
        let _: () = msg_send![text, setEditable: false];
        let _: () = msg_send![text, setAttributedText: all];
        let _: () = msg_send![all, release];
        let _: () = msg_send![text, setTextContainerInset: Insets { top: 16., left: 14., bottom: 32., right: 14. }];
        let _: () = msg_send![text, setAlwaysBounceVertical: true];
        let background: *mut AnyObject = msg_send![class!(UIColor), systemBackgroundColor];
        let _: () = msg_send![text, setBackgroundColor: background];

        let page: *mut AnyObject = msg_send![class!(UIViewController), new];
        let _: () = msg_send![page, setView: text];
        let _: () = msg_send![page, setTitle: ns_string(title)];
        let nav: *mut AnyObject = msg_send![class!(UINavigationController), alloc];
        let nav: *mut AnyObject = msg_send![nav, initWithRootViewController: page];
        let _: () = msg_send![nav, setOverrideUserInterfaceStyle: if crate::theme::dark() { 2isize } else { 1isize }];

        let close = block2::RcBlock::new(move |_: *mut AnyObject| {
            let none: Option<&block2::Block<dyn Fn()>> = None;
            let _: () = msg_send![nav, dismissViewControllerAnimated: true, completion: none];
        });
        let copy = copy.to_owned();
        let copied = block2::RcBlock::new(move |_: *mut AnyObject| {
            let board: *mut AnyObject = msg_send![class!(UIPasteboard), generalPasteboard];
            let _: () = msg_send![board, setString: ns_string(&copy)];
            super::super::glass::selection_haptic();
        });
        let none = std::ptr::null_mut::<AnyObject>();
        let action = |title: &str, handler: &block2::Block<dyn Fn(*mut AnyObject)>| -> *mut AnyObject {
            msg_send![class!(UIAction), actionWithTitle: ns_string(title), image: none, identifier: none, handler: handler]
        };
        let item: *mut AnyObject = msg_send![page, navigationItem];
        // UIBarButtonSystemItemClose = 24。
        let left: *mut AnyObject = msg_send![class!(UIBarButtonItem), alloc];
        let left: *mut AnyObject = msg_send![left, initWithBarButtonSystemItem: 24isize, primaryAction: action("", &close)];
        let right: *mut AnyObject = msg_send![class!(UIBarButtonItem), alloc];
        let right: *mut AnyObject = msg_send![right, initWithPrimaryAction: action(crate::settings::tr("拷贝", "Copy"), &copied)];
        let _: () = msg_send![item, setLeftBarButtonItem: left];
        let _: () = msg_send![item, setRightBarButtonItem: right];
        let _: () = msg_send![left, release];
        let _: () = msg_send![right, release];

        let none: Option<&block2::Block<dyn Fn()>> = None;
        let _: () = msg_send![host, presentViewController: nav, animated: true, completion: none];
        let _: () = msg_send![text, release];
        let _: () = msg_send![page, release];
        let _: () = msg_send![nav, release];
        true
    }
}
