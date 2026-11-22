//! 「优化描述」整理完之后的确认页（长按图的「编辑提示词」也用它）：系统的页面卡片，顶上左边关掉、右边「生成」。
//! 从上往下：原来的描述（灰字，跟画面一样就不显示）、「画面」（能改的字框）、每个出场人物一张标签卡、「排除」（能改的字框）。
//! 人物卡里是一个个标签片，上面一组是提示词、细线下面一组是排除；每组按「新增 → 去掉 → 保留（预设原顺序）→ +」排，改了什么扫一眼就有。
//! 点保留的变成去掉（划掉，留在原位），再点恢复；点新增的出菜单（编辑、删除）；「+」弹输入框，逗号分开就是几个。保留的超过 8 个时折起来。
//! 不能往下一划就关掉（改了一半的不会悄悄丢）；不自动弹键盘，先让人看。关掉时把监听都撤掉。
use std::{cell::{Cell, RefCell}, ffi::CStr, rc::Rc};

use objc2::{class, msg_send, runtime::AnyObject, Encode, Encoding, RefEncode};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};

use crate::settings::tr;
use super::super::glass::ns_string;

#[repr(C)] #[derive(Clone, Copy)] struct Insets { top: f64, left: f64, bottom: f64, right: f64 }
unsafe impl Encode for Insets { const ENCODING: Encoding = Encoding::Struct("UIEdgeInsets", &[const { Encoding::Double }; 4]); }
#[repr(C)] struct CGColor { _private: [u8; 0] }
unsafe impl RefEncode for CGColor { const ENCODING_REF: Encoding = Encoding::Pointer(&Encoding::Struct("CGColor", &[])); }

/// 一组标签：预设（拆开的，原顺序）、其中被去掉的、新加的。
pub struct Chips { pub preset: Vec<String>, pub removed: Vec<String>, pub added: Vec<String> }
/// 一段：只读的描述、能改的字框（画面、排除）、人物的标签卡（提示词、排除各一组）。
pub enum Section { Note(String), Text { title: String, text: String }, Tags { title: String, look: Chips, negative: Chips } }
/// 「生成」时每段（描述除外）交回的：字框的字，或者人物两组各自的（去掉的, 新加的）。
pub enum Answer { Text(String), Tags { look: (Vec<String>, Vec<String>), negative: (Vec<String>, Vec<String>) } }

const MARGIN: f64 = 20.;
const GAP: f64 = 22.;
const PAD: f64 = 12.;
const CHIP: f64 = 30.;
const SPACE: f64 = 6.;
/// 保留的标签多过这么多就折起来。
const FOLD: usize = 8;

fn same(a: &str, b: &str) -> bool { a.trim().eq_ignore_ascii_case(b.trim()) }

struct Group { preset: Vec<String>, order: Vec<usize>, removed: Vec<bool>, added: Vec<String>, open: bool }
impl Group {
    /// 进页时就去掉的排在保留的前面；之后点掉的留在原位，手指下的东西不挪。
    fn new(c: Chips) -> Group {
        let removed: Vec<bool> = c.preset.iter().map(|t| c.removed.iter().any(|r| same(r, t))).collect();
        let n = c.preset.len();
        let order = (0..n).filter(|&i| removed[i]).chain((0..n).filter(|&i| !removed[i])).collect();
        Group { preset: c.preset, order, removed, added: c.added, open: false }
    }
    fn answer(&self) -> (Vec<String>, Vec<String>) {
        (self.preset.iter().zip(&self.removed).filter(|(_, r)| **r).map(|(t, _)| t.clone()).collect(), self.added.clone())
    }
    /// 输入的标签（逗号分开）放进来：`at` 是正在改的那个新增的位置（换掉它），没有就接在新增后面。
    /// 预设里有的就是恢复它，已经新增过的不重复。
    fn put(&mut self, typed: &str, at: Option<usize>) {
        if let Some(i) = at.filter(|&i| i < self.added.len()) { self.added.remove(i); }
        let mut pos = at.unwrap_or(self.added.len()).min(self.added.len());
        for t in fdl_ai::imagine::pieces(typed) {
            if let Some(i) = self.preset.iter().position(|p| same(p, &t)) { self.removed[i] = false; continue; }
            if self.added.iter().any(|a| same(a, &t)) { continue; }
            self.added.insert(pos, t);
            pos += 1;
        }
    }
}

#[derive(Clone, Copy)]
enum Kind { Kept(usize), Gone(usize), New(usize), More(usize), Plus }

/// 人物卡：卡底、细线、「排除」两个字，两组标签，现在摆着的标签片（视图, 第几组, 宽）。`dirty` 是状态变了、片要重做。
struct Card { view: usize, line: usize, label: usize, groups: [Group; 2], chips: Vec<(usize, usize, f64)>, dirty: bool }

/// 标签片的回调要用的：重排整页（关掉时拿掉，断开引用环）、弹输入框的页面、深浅色。
struct Ctx { relayout: RefCell<Option<Rc<dyn Fn()>>>, nav: Cell<usize>, style: isize }
impl Ctx {
    /// 状态变了：放到主队列下一轮再重排（重排会换掉标签片，正在回调的那个按钮不能当场拿掉）。
    fn again(&self) {
        let Some(f) = self.relayout.borrow().clone() else { return };
        let block = block2::RcBlock::new(move || f());
        unsafe { let q: *mut AnyObject = msg_send![class!(NSOperationQueue), mainQueue]; let _: () = msg_send![q, addOperationWithBlock: &*block]; }
    }
}

fn text_of(view: *mut AnyObject) -> String {
    unsafe {
        let s: *mut AnyObject = msg_send![view, text];
        let chars: *const std::ffi::c_char = if s.is_null() { std::ptr::null() } else { msg_send![s, UTF8String] };
        if chars.is_null() { String::new() } else { CStr::from_ptr(chars).to_string_lossy().into_owned() }
    }
}

unsafe fn color(name: &str) -> *mut AnyObject {
    unsafe { msg_send![class!(UIColor), performSelector: objc2::runtime::Sel::register(&std::ffi::CString::new(name).unwrap())] }
}

/// 一个标签片：胶囊，14pt 字。保留是浅灰底；去掉是透明底、细边、浅字加删除线；新增是强调色的浅底和字；「+」「…N」同保留的灰底。
unsafe fn chip(text: &str, kind: Kind, style: isize) -> (*mut AnyObject, f64) {
    unsafe {
        let b: *mut AnyObject = msg_send![class!(UIButton), buttonWithType: 0isize];
        let fill = color("tertiarySystemFillColor");
        let (bg, fg) = match kind {
            Kind::Kept(_) => (fill, color("labelColor")),
            Kind::Gone(_) => (color("clearColor"), color("tertiaryLabelColor")),
            Kind::New(_) => { let tint = color("tintColor"); (msg_send![tint, colorWithAlphaComponent: 0.15f64], tint) }
            Kind::More(_) | Kind::Plus => (fill, color("secondaryLabelColor")),
        };
        let _: () = msg_send![b, setBackgroundColor: bg];
        let layer: *mut AnyObject = msg_send![b, layer];
        let _: () = msg_send![layer, setCornerRadius: CHIP / 2.];
        if let Kind::Gone(_) = kind {
            let traits: *mut AnyObject = msg_send![class!(UITraitCollection), traitCollectionWithUserInterfaceStyle: style];
            let edge: *mut AnyObject = msg_send![color("separatorColor"), resolvedColorWithTraitCollection: traits];
            let cg: *const CGColor = msg_send![edge, CGColor];
            let _: () = msg_send![layer, setBorderWidth: 1f64];
            let _: () = msg_send![layer, setBorderColor: cg];
        }
        if let Kind::Plus = kind {
            let image: *mut AnyObject = msg_send![class!(UIImage), systemImageNamed: ns_string("plus")];
            let _: () = msg_send![b, setImage: image, forState: 0usize];
            let _: () = msg_send![b, setTintColor: fg];
            return (b, CHIP);
        }
        let font: *mut AnyObject = msg_send![class!(UIFont), systemFontOfSize: 14f64];
        let attrs: *mut AnyObject = msg_send![class!(NSMutableDictionary), dictionary];
        let _: () = msg_send![attrs, setObject: font, forKey: ns_string("NSFont")];
        let _: () = msg_send![attrs, setObject: fg, forKey: ns_string("NSColor")];
        if let Kind::Gone(_) = kind {
            let one: *mut AnyObject = msg_send![class!(NSNumber), numberWithInteger: 1isize];
            let _: () = msg_send![attrs, setObject: one, forKey: ns_string("NSStrikethrough")];
        }
        let title = if let Kind::More(n) = kind { format!("…{n}") } else { text.to_owned() };
        let s: *mut AnyObject = msg_send![class!(NSAttributedString), alloc];
        let s: *mut AnyObject = msg_send![s, initWithString: ns_string(&title), attributes: attrs];
        let _: () = msg_send![b, setAttributedTitle: s, forState: 0usize];
        let size: CGSize = msg_send![s, size];
        let _: () = msg_send![s, release];
        let label: *mut AnyObject = msg_send![b, titleLabel];
        // 4 = NSLineBreakByTruncatingTail：太长的标签尾巴截掉，不折行。
        let _: () = msg_send![label, setLineBreakMode: 4isize];
        (b, size.width.ceil() + 20.)
    }
}

/// 给按钮挂上点按的回调（UIControlEventTouchUpInside = 1 << 6）。
unsafe fn on_tap(b: *mut AnyObject, f: impl Fn() + 'static) {
    unsafe {
        let block = block2::RcBlock::new(move |_: *mut AnyObject| f());
        let none = std::ptr::null_mut::<AnyObject>();
        let action: *mut AnyObject = msg_send![class!(UIAction), actionWithTitle: ns_string(""), image: none, identifier: none, handler: &*block];
        let _: () = msg_send![b, addAction: action, forControlEvents: 64usize];
    }
}

/// 输入框：标题、原来的字，点「完成」交出输入的字。
unsafe fn ask(ctx: &Ctx, title: &str, value: &str, done: impl Fn(String) + 'static) {
    unsafe {
        let nav = ctx.nav.get() as *mut AnyObject;
        if nav.is_null() { return; }
        let none = std::ptr::null_mut::<AnyObject>();
        let alert: *mut AnyObject = msg_send![class!(UIAlertController), alertControllerWithTitle: ns_string(title), message: none, preferredStyle: 1isize];
        let value = value.to_owned();
        let configure = block2::RcBlock::new(move |field: *mut AnyObject| {
            let _: () = msg_send![field, setText: ns_string(&value)];
            let set: unsafe extern "C" fn(*mut AnyObject, objc2::runtime::Sel, isize) = std::mem::transmute(objc2::ffi::objc_msgSend as *const ());
            set(field, objc2::sel!(setAutocapitalizationType:), 0);
            set(field, objc2::sel!(setAutocorrectionType:), 1);
            set(field, objc2::sel!(setSpellCheckingType:), 1);
        });
        let _: () = msg_send![alert, addTextFieldWithConfigurationHandler: &*configure];
        let fields: *mut AnyObject = msg_send![alert, textFields];
        let field: *mut AnyObject = msg_send![fields, firstObject];
        let cancel: *mut AnyObject = msg_send![class!(UIAlertAction), actionWithTitle: ns_string(tr("取消", "Cancel")), style: 1isize, handler: Option::<&block2::Block<dyn Fn(*mut AnyObject)>>::None];
        let _: () = msg_send![alert, addAction: cancel];
        let save = block2::RcBlock::new(move |_: *mut AnyObject| { let t = text_of(field); if !t.trim().is_empty() { done(t) } });
        let ok: *mut AnyObject = msg_send![class!(UIAlertAction), actionWithTitle: ns_string(tr("完成", "Done")), style: 0isize, handler: &*save];
        let _: () = msg_send![alert, addAction: ok];
        let _: () = msg_send![alert, setPreferredAction: ok];
        let _: () = msg_send![alert, setOverrideUserInterfaceStyle: ctx.style];
        let none: Option<&block2::Block<dyn Fn()>> = None;
        let _: () = msg_send![nav, presentViewController: alert, animated: true, completion: none];
    }
}

/// 一组要摆的片：新增 → 去掉 → 保留（折起来时只摆前 8 个，后面一个「…N」）→「+」。
fn kinds(g: &Group) -> Vec<(String, Kind)> {
    let mut out: Vec<(String, Kind)> = g.added.iter().enumerate().map(|(i, t)| (t.clone(), Kind::New(i))).collect();
    let kept = g.removed.iter().filter(|r| !**r).count();
    let mut shown = 0;
    for &i in &g.order {
        if g.removed[i] { out.push((g.preset[i].clone(), Kind::Gone(i))); }
        else if g.open || kept <= FOLD || shown < FOLD { out.push((g.preset[i].clone(), Kind::Kept(i))); shown += 1; }
    }
    if !g.open && kept > FOLD { out.push((String::new(), Kind::More(kept - FOLD))); }
    out.push((String::new(), Kind::Plus));
    out
}

/// 重做一张卡的标签片（状态变了才做），按宽度流式摆好，返回卡的高。
unsafe fn arrange(card: &Rc<RefCell<Card>>, ctx: &Rc<Ctx>, width: f64) -> f64 {
    unsafe {
        let mut c = card.borrow_mut();
        if c.dirty {
            for &(v, _, _) in &c.chips { let _: () = msg_send![v as *mut AnyObject, removeFromSuperview]; }
            let mut chips = vec![];
            for g in 0..2 {
                for (text, kind) in kinds(&c.groups[g]) {
                    let (b, w) = chip(&text, kind, ctx.style);
                    let (card2, ctx2) = (card.clone(), ctx.clone());
                    let change = move |f: &dyn Fn(&mut Group)| { { let mut c = card2.borrow_mut(); f(&mut c.groups[g]); c.dirty = true; } ctx2.again(); };
                    match kind {
                        Kind::Kept(i) | Kind::Gone(i) => on_tap(b, move || { super::super::glass::haptic(); change(&|g| g.removed[i] = !g.removed[i]) }),
                        Kind::More(_) => on_tap(b, move || change(&|g| g.open = true)),
                        Kind::Plus => {
                            let ctx3 = ctx.clone();
                            let change = Rc::new(change);
                            on_tap(b, move || { let change = change.clone(); ask(&ctx3, tr("添加标签", "Add Tags"), "", move |t| change(&|g| g.put(&t, None))) });
                        }
                        Kind::New(i) => {
                            let change = Rc::new(change);
                            let (edit, del) = (change.clone(), change);
                            let (ctx3, old) = (ctx.clone(), text.clone());
                            let edit = block2::RcBlock::new(move |_: *mut AnyObject| {
                                let edit = edit.clone();
                                ask(&ctx3, tr("编辑标签", "Edit Tag"), &old, move |t| edit(&|g| g.put(&t, Some(i))));
                            });
                            let del = block2::RcBlock::new(move |_: *mut AnyObject| del(&|g| { if i < g.added.len() { g.added.remove(i); } }));
                            let none = std::ptr::null_mut::<AnyObject>();
                            let pencil: *mut AnyObject = msg_send![class!(UIImage), systemImageNamed: ns_string("pencil")];
                            let trash: *mut AnyObject = msg_send![class!(UIImage), systemImageNamed: ns_string("trash")];
                            let a: *mut AnyObject = msg_send![class!(UIAction), actionWithTitle: ns_string(tr("编辑", "Edit")), image: pencil, identifier: none, handler: &*edit];
                            let d: *mut AnyObject = msg_send![class!(UIAction), actionWithTitle: ns_string(tr("删除", "Delete")), image: trash, identifier: none, handler: &*del];
                            // 2 = UIMenuElementAttributesDestructive
                            let _: () = msg_send![d, setAttributes: 2usize];
                            let items: *mut AnyObject = msg_send![class!(NSMutableArray), array];
                            let _: () = msg_send![items, addObject: a];
                            let _: () = msg_send![items, addObject: d];
                            let menu: *mut AnyObject = msg_send![class!(UIMenu), menuWithTitle: ns_string(""), children: items];
                            let _: () = msg_send![b, setMenu: menu];
                            let _: () = msg_send![b, setShowsMenuAsPrimaryAction: true];
                        }
                    }
                    let _: () = msg_send![c.view as *mut AnyObject, addSubview: b];
                    chips.push((b as usize, g, w));
                }
            }
            c.chips = chips;
            c.dirty = false;
        }
        let inner = width - PAD * 2.;
        let mut y = PAD;
        for g in 0..2 {
            if g == 1 {
                y += CHIP + PAD;
                let _: () = msg_send![c.line as *mut AnyObject, setFrame: CGRect::new(CGPoint::new(PAD, y), CGSize::new(inner, 0.5))];
                y += PAD;
                let _: () = msg_send![c.label as *mut AnyObject, setFrame: CGRect::new(CGPoint::new(PAD, y), CGSize::new(inner, 16.))];
                y += 22.;
            }
            let mut x = PAD;
            for &(v, _, w) in c.chips.iter().filter(|(_, cg, _)| *cg == g) {
                let w = w.min(inner);
                if x > PAD && x + w > PAD + inner { x = PAD; y += CHIP + SPACE; }
                let _: () = msg_send![v as *mut AnyObject, setFrame: CGRect::new(CGPoint::new(x, y), CGSize::new(w, CHIP))];
                x += w + SPACE;
            }
        }
        y + CHIP + PAD
    }
}

enum Row { Note(usize), Text(usize, usize), Tags(usize, Rc<RefCell<Card>>) }

/// 弹出来。点「生成」时按顺序交出各段（描述除外）的结果（`Some`），关掉是 `None`。返回假是没弹出来（上面正弹着别的）。
pub fn show(title: &str, sections: Vec<Section>, done: impl Fn(Option<Vec<Answer>>) + 'static) -> bool {
    unsafe {
        let host = super::super::files::root_controller();
        if host.is_null() { return false; }
        let presented: *mut AnyObject = msg_send![host, presentedViewController];
        if !presented.is_null() { return false; }
        let host_view: *mut AnyObject = msg_send![host, view];
        let bounds: CGRect = msg_send![host_view, bounds];
        let width = bounds.size.width;
        let style = if crate::theme::dark() { 2isize } else { 1isize };
        let ctx = Rc::new(Ctx { relayout: RefCell::new(None), nav: Cell::new(0), style });

        let scroll: *mut AnyObject = msg_send![class!(UIScrollView), new];
        let cell = color("secondarySystemGroupedBackgroundColor");
        let grey = color("secondaryLabelColor");
        let _: () = msg_send![scroll, setBackgroundColor: color("systemGroupedBackgroundColor")];
        let _: () = msg_send![scroll, setAlwaysBounceVertical: true];
        // 2 = UIScrollViewKeyboardDismissModeInteractive：往下拖就收键盘。
        let _: () = msg_send![scroll, setKeyboardDismissMode: 2isize];
        let head_font: *mut AnyObject = msg_send![class!(UIFont), systemFontOfSize: 13f64, weight: 0.3f64];
        let body: *mut AnyObject = msg_send![class!(UIFont), systemFontOfSize: 16f64];
        let note_font: *mut AnyObject = msg_send![class!(UIFont), systemFontOfSize: 15f64];
        // 输入特性在 UITextView 上是转发实现的，objc2 调试构建的方法检查找不到，直接走 objc_msgSend（同 `assistant_sheet::edit`）。
        let set: unsafe extern "C" fn(*mut AnyObject, objc2::runtime::Sel, isize) = std::mem::transmute(objc2::ffi::objc_msgSend as *const ());
        let add = |v: *mut AnyObject| { let _: () = msg_send![scroll, addSubview: v]; let _: () = msg_send![v, release]; v as usize };
        let heading = |t: &str| {
            let head: *mut AnyObject = msg_send![class!(UILabel), new];
            let _: () = msg_send![head, setFont: head_font];
            let _: () = msg_send![head, setTextColor: grey];
            let _: () = msg_send![head, setText: ns_string(t)];
            add(head)
        };
        let mut rows: Vec<Row> = vec![];
        let mut fields: Vec<usize> = vec![];
        for s in sections {
            match s {
                Section::Note(text) => {
                    let label: *mut AnyObject = msg_send![class!(UILabel), new];
                    let _: () = msg_send![label, setFont: note_font];
                    let _: () = msg_send![label, setTextColor: grey];
                    let _: () = msg_send![label, setNumberOfLines: 3isize];
                    let _: () = msg_send![label, setText: ns_string(text.trim())];
                    rows.push(Row::Note(add(label)));
                }
                Section::Text { title, text } => {
                    let head = heading(&title);
                    let view: *mut AnyObject = msg_send![class!(UITextView), new];
                    let _: () = msg_send![view, setScrollEnabled: false];
                    let _: () = msg_send![view, setFont: body];
                    let _: () = msg_send![view, setText: ns_string(&text)];
                    let _: () = msg_send![view, setBackgroundColor: cell];
                    let _: () = msg_send![view, setTextContainerInset: Insets { top: 12., left: 12., bottom: 12., right: 12. }];
                    let layer: *mut AnyObject = msg_send![view, layer];
                    let _: () = msg_send![layer, setCornerRadius: 12f64];
                    let _: () = msg_send![view, setClipsToBounds: true];
                    set(view, objc2::sel!(setAutocapitalizationType:), 0);
                    set(view, objc2::sel!(setAutocorrectionType:), 1);
                    set(view, objc2::sel!(setSpellCheckingType:), 1);
                    set(view, objc2::sel!(setSmartQuotesType:), 1);
                    set(view, objc2::sel!(setSmartDashesType:), 1);
                    let view = add(view);
                    fields.push(view);
                    rows.push(Row::Text(head, view));
                }
                Section::Tags { title, look, negative } => {
                    let head = heading(&title);
                    let view: *mut AnyObject = msg_send![class!(UIView), new];
                    let _: () = msg_send![view, setBackgroundColor: cell];
                    let layer: *mut AnyObject = msg_send![view, layer];
                    let _: () = msg_send![layer, setCornerRadius: 12f64];
                    let line: *mut AnyObject = msg_send![class!(UIView), new];
                    let _: () = msg_send![line, setBackgroundColor: color("separatorColor")];
                    let _: () = msg_send![view, addSubview: line];
                    let _: () = msg_send![line, release];
                    let label: *mut AnyObject = msg_send![class!(UILabel), new];
                    let small: *mut AnyObject = msg_send![class!(UIFont), systemFontOfSize: 12f64];
                    let _: () = msg_send![label, setFont: small];
                    let _: () = msg_send![label, setTextColor: color("tertiaryLabelColor")];
                    let _: () = msg_send![label, setText: ns_string(tr("排除", "Exclude"))];
                    let _: () = msg_send![view, addSubview: label];
                    let _: () = msg_send![label, release];
                    let card = Card { view: add(view), line: line as usize, label: label as usize, groups: [Group::new(look), Group::new(negative)], chips: vec![], dirty: true };
                    rows.push(Row::Tags(head, Rc::new(RefCell::new(card))));
                }
            }
        }
        // 从上往下排：小标题、框或卡（按内容高），段与段之间空一截。
        let rows = Rc::new(rows);
        let layout: Rc<dyn Fn()> = {
            let (rows, scroll, ctx) = (rows.clone(), scroll as usize, ctx.clone());
            Rc::new(move || {
                let inner = width - MARGIN * 2.;
                let mut y = 16.;
                let frame = |v: usize, x: f64, y: f64, w: f64, h: f64| { let _: () = msg_send![v as *mut AnyObject, setFrame: CGRect::new(CGPoint::new(x, y), CGSize::new(w, h))]; };
                for row in rows.iter() {
                    match row {
                        Row::Note(label) => {
                            let fit: CGSize = msg_send![*label as *mut AnyObject, sizeThatFits: CGSize::new(width - 48., f64::MAX)];
                            frame(*label, 24., y, width - 48., fit.height.ceil());
                            y += fit.height.ceil() + GAP;
                            continue;
                        }
                        Row::Text(head, _) | Row::Tags(head, _) => { frame(*head, MARGIN + 4., y, inner - 8., 18.); y += 24.; }
                    }
                    let h = match row {
                        Row::Text(_, view) => {
                            let fit: CGSize = msg_send![*view as *mut AnyObject, sizeThatFits: CGSize::new(inner, f64::MAX)];
                            let h = fit.height.max(44.).ceil();
                            frame(*view, MARGIN, y, inner, h);
                            h
                        }
                        Row::Tags(_, card) => {
                            let h = arrange(card, &ctx, inner);
                            frame(card.borrow().view, MARGIN, y, inner, h);
                            h
                        }
                        Row::Note(_) => 0.,
                    };
                    y += h + GAP;
                }
                let _: () = msg_send![scroll as *mut AnyObject, setContentSize: CGSize::new(width, y + 12.)];
            })
        };
        *ctx.relayout.borrow_mut() = Some(layout.clone());
        layout();

        // 字变了重排；键盘起落时底下让出键盘的高度。
        let center: *mut AnyObject = msg_send![class!(NSNotificationCenter), defaultCenter];
        let mut tokens: Vec<usize> = vec![];
        for &text in fields.iter() {
            let relayout = layout.clone();
            let block = block2::RcBlock::new(move |_: *mut AnyObject| relayout());
            let token: *mut AnyObject = msg_send![center, addObserverForName: ns_string("UITextViewTextDidChangeNotification"), object: text as *mut AnyObject, queue: std::ptr::null_mut::<AnyObject>(), usingBlock: &*block];
            let _: *mut AnyObject = msg_send![token, retain];
            tokens.push(token as usize);
        }
        let s = scroll as usize;
        let keyboard = block2::RcBlock::new(move |note: *mut AnyObject| {
            let info: *mut AnyObject = msg_send![note, userInfo];
            let value: *mut AnyObject = msg_send![info, objectForKey: ns_string("UIKeyboardFrameEndUserInfoKey")];
            if value.is_null() { return; }
            let frame: CGRect = msg_send![value, CGRectValue];
            let screen: *mut AnyObject = msg_send![class!(UIScreen), mainScreen];
            let full: CGRect = msg_send![screen, bounds];
            let covered = (full.size.height - frame.origin.y).max(0.);
            let _: () = msg_send![s as *mut AnyObject, setContentInset: Insets { top: 0., left: 0., bottom: covered, right: 0. }];
            let _: () = msg_send![s as *mut AnyObject, setVerticalScrollIndicatorInsets: Insets { top: 0., left: 0., bottom: covered, right: 0. }];
        });
        for name in ["UIKeyboardWillChangeFrameNotification", "UIKeyboardWillHideNotification"] {
            let token: *mut AnyObject = msg_send![center, addObserverForName: ns_string(name), object: std::ptr::null_mut::<AnyObject>(), queue: std::ptr::null_mut::<AnyObject>(), usingBlock: &*keyboard];
            let _: *mut AnyObject = msg_send![token, retain];
            tokens.push(token as usize);
        }

        let page: *mut AnyObject = msg_send![class!(UIViewController), new];
        let _: () = msg_send![page, setView: scroll];
        let _: () = msg_send![page, setTitle: ns_string(title)];
        let nav: *mut AnyObject = msg_send![class!(UINavigationController), alloc];
        let nav: *mut AnyObject = msg_send![nav, initWithRootViewController: page];
        let _: () = msg_send![nav, setModalInPresentation: true];
        let _: () = msg_send![nav, setOverrideUserInterfaceStyle: style];
        ctx.nav.set(nav as usize);

        let close = {
            let ctx = ctx.clone();
            Rc::new(move |go: bool| {
                let answer = go.then(|| rows.iter().filter_map(|r| match r {
                    Row::Note(_) => None,
                    Row::Text(_, view) => Some(Answer::Text(text_of(*view as *mut AnyObject))),
                    Row::Tags(_, card) => { let c = card.borrow(); Some(Answer::Tags { look: c.groups[0].answer(), negative: c.groups[1].answer() }) }
                }).collect::<Vec<_>>());
                let center: *mut AnyObject = msg_send![class!(NSNotificationCenter), defaultCenter];
                for &t in &tokens { let _: () = msg_send![center, removeObserver: t as *mut AnyObject]; let _: () = msg_send![t as *mut AnyObject, release]; }
                let _: bool = msg_send![s as *mut AnyObject, endEditing: true];
                ctx.relayout.borrow_mut().take();
                ctx.nav.set(0);
                let none: Option<&block2::Block<dyn Fn()>> = None;
                let _: () = msg_send![nav, dismissViewControllerAnimated: true, completion: none];
                done(answer);
            })
        };
        // 关掉之后按钮可能还会再送一次（连点）：只认第一次。
        let once = Rc::new(Cell::new(false));
        let button = |go: bool| -> block2::RcBlock<dyn Fn(*mut AnyObject)> {
            let (close, once) = (close.clone(), once.clone());
            block2::RcBlock::new(move |_: *mut AnyObject| if !once.replace(true) { close(go) })
        };
        let none = std::ptr::null_mut::<AnyObject>();
        let (quit, go) = (button(false), button(true));
        let quit: *mut AnyObject = msg_send![class!(UIAction), actionWithTitle: ns_string(""), image: none, identifier: none, handler: &*quit];
        let go: *mut AnyObject = msg_send![class!(UIAction), actionWithTitle: ns_string(tr("生成", "Generate")), image: none, identifier: none, handler: &*go];
        let item: *mut AnyObject = msg_send![page, navigationItem];
        // UIBarButtonSystemItemCancel = 1（iOS 26 上画成叉）；「生成」是文字按钮，Done 样式（2）画成强调的。
        let left: *mut AnyObject = msg_send![class!(UIBarButtonItem), alloc];
        let left: *mut AnyObject = msg_send![left, initWithBarButtonSystemItem: 1isize, primaryAction: quit];
        let right: *mut AnyObject = msg_send![class!(UIBarButtonItem), alloc];
        let right: *mut AnyObject = msg_send![right, initWithPrimaryAction: go];
        let _: () = msg_send![right, setStyle: 2isize];
        let _: () = msg_send![item, setLeftBarButtonItem: left];
        let _: () = msg_send![item, setRightBarButtonItem: right];
        let _: () = msg_send![left, release];
        let _: () = msg_send![right, release];

        let none: Option<&block2::Block<dyn Fn()>> = None;
        let _: () = msg_send![host, presentViewController: nav, animated: true, completion: none];
        let _: () = msg_send![scroll, release];
        let _: () = msg_send![page, release];
        let _: () = msg_send![nav, release];
        true
    }
}
