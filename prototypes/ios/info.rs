//! 文档信息（手机，照 Craft 的「统计」，截图 2026-11-24）：底部一张原生玻璃卡片（`panel.rs`），
//! 完整文档 / 这一页两段，下面一行一个数；整页连同顶栏的玻璃一起压暗，点卡片外面、✕、往下拖都收起。
//! 宽屏和不支持玻璃的系统照旧是 `docinfo.rs` 的对话框。

use super::panel::{Material, Node};
use crate::app::{View, Workspace};
use crate::docinfo;
use crate::model::Document;
use crate::settings::tr;
use gpui::*;
use objc2::runtime::{AnyClass, AnyObject};
use objc2::msg_send;
use objc2_core_foundation::CGRect;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::cell::{Cell, RefCell};

/// 一段的数：区块、字数、字符数（不含空格）、字符数（含空格）。
type Counts = [usize; 4];
thread_local! {
    static TAB: Cell<usize> = const { Cell::new(0) };
    // 打开时数一遍（长文档不逐帧重数），收起时清掉。
    static COUNTS: RefCell<Option<[Counts; 2]>> = const { RefCell::new(None) };
    static DIM: Cell<*mut AnyObject> = const { Cell::new(std::ptr::null_mut()) };
    static SHOWN: Cell<bool> = const { Cell::new(false) };
    static SEEN: Cell<bool> = const { Cell::new(false) };
}

/// 用不用这张卡片（不用时是桌面的对话框）。
pub fn native() -> bool { super::floating() }

fn counts(doc: &Document, pages: bool) -> Counts {
    let mut n = super::spaces::tally(&doc.blocks, pages);
    for (n, t) in n.iter_mut().zip(super::spaces::tally_text(&doc.title.text)) { *n += t; }
    n
}
/// 阅读时间，精确到秒（照 Craft，每分钟 160 字，和空间统计同一个速度）。
fn reading(words: usize) -> String {
    let s = words * 60 / 160;
    match (s / 60, s % 60, crate::settings::english()) {
        (0, s, false) => format!("{s}秒"), (m, s, false) => format!("{m}分钟{s}秒"),
        (0, s, true) => format!("{s} s"), (m, s, true) => format!("{m} min {s} s"),
    }
}

fn nodes(w: &Workspace, cx: &App) -> Option<Vec<Node>> {
    let info = w.doc_info.as_ref()?;
    let ed = w.editor.as_ref()?.read(cx);
    let [all, page] = COUNTS.with(|c| *c.borrow_mut().get_or_insert_with(|| [counts(&ed.root().doc, true), counts(&ed.core.doc, false)]));
    let tab = TAB.with(|t| t.get());
    let [blocks, words, chars, spaced] = if tab == 0 { all } else { page };
    let n = super::spaces::grouped;
    let rows = [
        (tr("字数", "Words"), n(words)),
        (tr("字符数", "Characters"), n(spaced)),
        (tr("不包括空格", "Without spaces"), n(chars)),
        (tr("区块", "Blocks"), n(blocks)),
        (tr("阅读时间", "Reading time"), reading(words)),
        (tr("大小", "Size"), docinfo::format_size(info.size)),
        (crate::settings::text("创建时间"), crate::ui::format_date(info.created)),
        (crate::settings::text("最近修改"), crate::ui::format_relative(info.modified)),
    ];
    let last = rows.len() - 1;
    Some([
        Node::Header { title: tr("文档信息", "Document information").into(), warn: false, reset: None, close: "info-close".into() },
        Node::Segments { symbols: vec![], titles: vec![tr("完整文档", "Whole document").into(), tr("这一页", "This page").into()], picked: tab, act: "info-tab:".into() },
    ].into_iter().chain(rows.into_iter().enumerate().map(|(i, (label, value))| Node::Stat { label: label.into(), value, rule: i < last })).collect())
}

/// 卡片的占位：左右各 12，离底边 34（Craft 量的）；底下一层压暗。
pub fn element(w: &Workspace, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    let nodes = (native() && w.view == View::Doc).then(|| nodes(w, cx)).flatten();
    let Some(nodes) = nodes else {
        COUNTS.with(|c| c.borrow_mut().take());
        TAB.with(|t| t.set(0));
        return None;
    };
    let (_, _, viewport) = super::layout();
    let width = viewport.width - px(24.);
    let height = px(super::panel::height(&nodes, f32::from(width)));
    // 底下先压暗过，纸色厚一点才和 Craft 的一样是一块浅的磨砂。
    let paper = crate::editor_theme::Theme::bg_body();
    let material = Material { tint: None, wash: Some(paper.opacity(if crate::theme::dark() { 0.32 } else { 0.38 })) };
    Some(div().id("doc-info-card").absolute().inset_0().occlude()
        .child(canvas(|_, window, _| dim(window), |_, _, _, _| {}).absolute().inset_0())
        .child(div().absolute().left(px(12.)).bottom(super::insets().bottom.max(px(34.))).w(width).h(height).child(super::panel::slot(nodes, material)))
        .into_any_element())
}

pub fn action(act: &str, w: &mut Workspace, cx: &mut Context<Workspace>) {
    match act.split_once(':') {
        Some((_, tab)) => TAB.with(|t| t.set(tab.parse().unwrap_or(0))),
        None => w.doc_info = None,
    }
    cx.notify();
}

/// 每帧开头调用：这一帧没有卡片，压暗就淡掉。
pub fn begin(window: &Window) {
    SEEN.with(|s| s.set(false));
    window.on_next_frame(|_, _| if !SEEN.with(|s| s.get()) && SHOWN.with(|s| s.replace(false)) { unsafe {
        let view = DIM.with(|d| d.get()) as usize;
        let body = block2::RcBlock::new(move || { let _: () = msg_send![view as *mut AnyObject, setAlpha: 0f64]; });
        let done = block2::RcBlock::new(move |_finished: objc2::runtime::Bool| if !SHOWN.with(|s| s.get()) { let _: () = msg_send![view as *mut AnyObject, setHidden: true]; });
        let _: () = msg_send![AnyClass::get(c"UIView").unwrap(), animateWithDuration: 0.18f64, animations: &*body, completion: &*done];
    } });
}
fn dim(window: &Window) {
    let Ok(handle) = HasWindowHandle::window_handle(window) else { return };
    let RawWindowHandle::UiKit(handle) = handle.as_raw() else { return };
    let parent = super::shot::overlay(handle.ui_view.as_ptr() as *mut AnyObject);
    SEEN.with(|s| s.set(true));
    if SHOWN.with(|s| s.replace(true)) { return; }
    unsafe {
        let mut view = DIM.with(|d| d.get());
        if view.is_null() {
            let bounds: CGRect = msg_send![parent, bounds];
            view = msg_send![AnyClass::get(c"UIControl").unwrap(), alloc];
            view = msg_send![view, initWithFrame: bounds];
            // 2 | 16：宽高跟着父视图。
            let _: () = msg_send![view, setAutoresizingMask: 18usize];
            let _: () = msg_send![view, setAlpha: 0f64];
            let _: () = msg_send![view, setAccessibilityLabel: super::glass::ns_string(tr("关闭", "Close"))];
            // 1 = UIControlEventTouchDown
            super::assist::card::on(view, 1, || super::menu::dispatch(|w, _, cx| action("info-close", w, cx)));
            let _: () = msg_send![parent, addSubview: view];
            DIM.with(|d| d.set(view));
        }
        // Craft 截图：白纸压成 #D0D0D0 一档；深色下再重一点。
        let _: () = msg_send![view, setBackgroundColor: super::glass::ui_color(crate::theme::rgbaf(0, 0, 0, if crate::theme::dark() { 0.4 } else { 0.18 }))];
        let _: () = msg_send![view, setHidden: false];
        let _: () = msg_send![parent, bringSubviewToFront: view];
        let view = view as usize;
        let body = block2::RcBlock::new(move || { let _: () = msg_send![view as *mut AnyObject, setAlpha: 1f64]; });
        let _: () = msg_send![AnyClass::get(c"UIView").unwrap(), animateWithDuration: 0.25f64, animations: &*body];
    }
}
