//! 编辑时的「格式」面板（照 Craft，录屏 2026-11-15 19:25）：打字时点键盘上方的「格式」，光标所在的这一块被选中，
//! 键盘收下去，原来键盘的地方升起一块玻璃面板。第一页是块：常用的几种样式、列表和缩进、焦点 / 块两种装饰、颜色；
//! 「更多…」是全部的标题层级，「···」是文字（粗体这些、对齐、字体、复制 / 粘贴格式）。面板开着时右上角仍是 ✓，
//! 点 ✓ 收起；点正文回到键盘。面板用 `panel.rs` 的原生玻璃画，点了的动作以 `fmt-` 开头送回这里（`action`）。
//! 改块都走编辑器已有的方法（`block_menu_action`、`toggle_mark`、`apply_color`、`set_block_bg`），不另写编辑逻辑。
use std::cell::{Cell, RefCell};

use fdl_core::model::{Align, BlockId, BlockKind, ColorName};
use gpui::*;

use super::panel::{Look, Node, Slot as Item};
use crate::app::{View, Workspace};
use crate::editor::view::{BlockAct, Editor, Mark};
use crate::settings::tr;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Page { Block, More, Text, Color }

thread_local! { static PAGE: Cell<Option<Page>> = const { Cell::new(None) }; }
// 「复制格式」记下的样式，「粘贴格式」贴到选中的块上。只在这次打开的应用里记着，不放系统剪贴板。
thread_local! { static COPIED: RefCell<Option<fdl_core::editor::core::BlockFormat>> = const { RefCell::new(None) }; }
// 上一帧面板占了多高（连同下面的留白）：键盘那条在这里让出同样高的地方，选中的块不会被面板挡住。
thread_local! { static HEIGHT: Cell<f32> = const { Cell::new(0.) }; }

pub fn is_open() -> bool { PAGE.with(|p| p.get()).is_some() }
pub fn close() { PAGE.with(|p| p.set(None)); }
/// 面板连同下面的留白有多高（没开时是 0）。
pub fn reserved() -> f32 { if is_open() { HEIGHT.with(|h| h.get()) } else { 0. } }

/// 点了「格式」：选中光标所在的块，收起键盘，升起面板。
pub fn open(w: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) { open_at(Page::Block, w, window, cx) }
/// 同上，直接到某一页（「/」菜单的「颜色」）。
pub fn open_at(page: Page, w: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let Some(editor) = w.editor.clone() else { return };
    if editor.update(cx, |ed, cx| { let block = ed.format_begin(); cx.notify(); block }).is_none() { return; }
    super::stylepanel::close();
    super::glass::haptic();
    PAGE.with(|p| p.set(Some(page)));
    window.focus(&w.focus, cx);
    cx.notify();
}

/// 点 ✓：收起面板，块也不再选着。
pub fn done(w: &mut Workspace, cx: &mut Context<Workspace>) {
    close();
    if let Some(editor) = w.editor.clone() { editor.update(cx, |ed, cx| { ed.core.clear_block_sel(); cx.notify(); }); }
    cx.notify();
}

fn heading(kind: Option<&BlockKind>) -> Option<u8> { match kind { Some(BlockKind::Heading { level, .. }) => Some(*level), _ => None } }
fn turn_act(kind: &str) -> Option<BlockKind> {
    Some(match kind {
        "text" => BlockKind::Text,
        "h1" => BlockKind::Heading { level: 1, seq_auto: false },
        "h2" => BlockKind::Heading { level: 2, seq_auto: false },
        "h3" => BlockKind::Heading { level: 3, seq_auto: false },
        "h4" => BlockKind::Heading { level: 4, seq_auto: false },
        "bullet" => BlockKind::Bullet,
        "ordered" => BlockKind::Ordered { seq: None },
        "todo" => BlockKind::Todo { done: false },
        "quote" => BlockKind::Quote,
        "code" => BlockKind::Code { lang: String::new(), caption: None, wrap: true },
        "callout" => BlockKind::Callout { emoji: String::new(), bg: None, border: None, text_color: None },
        "page" => BlockKind::Page { card: false },
        "card" => BlockKind::Page { card: true },
        _ => return None,
    })
}
/// 同一种再点一下就变回正文（列表、引用这些开关式的）。
fn same(a: &BlockKind, b: &BlockKind) -> bool {
    match (a, b) {
        (BlockKind::Heading { level: x, .. }, BlockKind::Heading { level: y, .. }) => x == y,
        (BlockKind::Page { card: x }, BlockKind::Page { card: y }) => x == y,
        _ => std::mem::discriminant(a) == std::mem::discriminant(b),
    }
}

/// 面板上的一个动作。
pub fn action(act: &str, w: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let Some(act) = act.strip_prefix("fmt-") else { return };
    if let Some(page) = act.strip_prefix("page-") {
        let page = match page { "more" => Page::More, "text" => Page::Text, "color" => Page::Color, _ => Page::Block };
        PAGE.with(|p| p.set(Some(page)));
        return cx.notify();
    }
    let Some(editor) = w.editor.clone() else { return };
    super::glass::haptic();
    let depth = editor.read(cx).outer.len();
    let back = editor.update(cx, |ed, cx| {
        let Some(block) = ed.format_block() else { return false };
        let ids = ed.core.block_sel_ids();
        let kind = ed.core.kind_of(block).cloned();
        let mut back = false;
        if let Some(k) = act.strip_prefix("kind-") {
            let Some(target) = turn_act(k) else { return false };
            let toggled = kind.as_ref().is_some_and(|now| same(now, &target)) && !matches!(target, BlockKind::Text | BlockKind::Heading { .. });
            // 一行字变成页面：照 Craft 马上打开它（面板在下面收起）。
            let opens = target.is_page() && !kind.as_ref().is_some_and(BlockKind::is_page);
            ed.block_menu_action(block, BlockAct::Turn(if toggled { BlockKind::Text } else { target }), window, cx);
            if opens && ed.core.kind_of(block).is_some_and(BlockKind::is_page) {
                ed.core.clear_block_sel();
                ed.open_page(block, window, cx);
                // 照 Craft：变成页面后接着在里面打字。
                window.focus(&ed.focus.clone(), cx);
                return true;
            }
            back = true;
        } else if let Some(m) = act.strip_prefix("mark-") {
            let m = match m { "b" => Mark::Bold, "i" => Mark::Italic, "u" => Mark::Underline, "s" => Mark::Strike, _ => Mark::Code };
            ed.toggle_mark(m, window, cx);
        } else if let Some(a) = act.strip_prefix("align-") {
            ed.core.set_align(&ids, match a { "c" => Align::Center, "r" => Align::Right, _ => Align::Left });
            ed.changed(window, cx);
        } else if let Some(i) = act.strip_prefix("ink:").and_then(|i| i.parse::<usize>().ok()) {
            let name = crate::theme::FONT_COLORS.get(i).and_then(|c| c.0).map(|n| ColorName(n.into()));
            ed.apply_color(Some(name), None, window, cx);
        } else if let Some(i) = act.strip_prefix("bg:").and_then(|i| i.parse::<usize>().ok()) {
            let name = BG.get(i).copied().flatten().map(|n| ColorName(n.into()));
            ed.set_block_bg(block, name, window, cx);
        } else if let Some(f) = act.strip_prefix("font-") {
            if ed.markdown { return false; }
            ed.core.set_font(&ids, (f != "system").then(|| f.to_string()));
            ed.changed(window, cx);
        } else {
            match act {
                "copy" => { let f = ed.core.format_of(block); COPIED.with(|c| *c.borrow_mut() = f); }
                "paste" => if let Some(f) = COPIED.with(|c| c.borrow().clone()) && !ed.markdown {
                    ed.core.apply_format(&ids, &f);
                    ed.changed(window, cx);
                },
                "fold" => { ed.core.toggle_fold(block); ed.changed(window, cx); }
                "indent" => ed.block_menu_action(block, BlockAct::Indent, window, cx),
                "outdent" => ed.block_menu_action(block, BlockAct::Outdent, window, cx),
                // 焦点：左边一道竖线（引用）；再点变回正文。
                "focus" => {
                    let quote = matches!(kind, Some(BlockKind::Quote));
                    ed.block_menu_action(block, BlockAct::Turn(if quote { BlockKind::Text } else { BlockKind::Quote }), window, cx);
                }
                // 块：一层浅灰的底；有底色时去掉。
                "tint" => {
                    let has = ed.core.block(block).is_some_and(|b| b.bg.is_some());
                    ed.set_block_bg(block, (!has).then(|| ColorName("light-gray".into())), window, cx);
                }
                _ => {}
            }
        }
        // 改完这一块（或这几块）仍然选着，面板接着用。
        if !ed.core.in_block_mode() { ed.core.select_block(block); }
        cx.notify();
        back
    });
    // 「更多…」里挑了一种就回到第一页（照 Craft）。
    // 打开了新变成的页面：面板跟着收起。
    if editor.read(cx).outer.len() > depth { close(); return cx.notify(); }
    if back && PAGE.with(|p| p.get()) == Some(Page::More) { PAGE.with(|p| p.set(Some(Page::Block))); }
    cx.notify();
}

/// 字体四格：样张、名字、系统字体的设计、存进块里的名字（`system` 是跟着文档）。
const FONTS: [(&str, &str, &str, &str); 4] = [("Aa", "System", "", "system"), ("Ss", "Serif", "serif", "serif"), ("00", "Mono", "mono", "mono"), ("Rr", "Round", "rounded", "rounded")];

/// 块背景只放浅的一排（和「块」装饰同一种浅色）。
const BG: [Option<&str>; 8] = [None, Some("light-gray"), Some("light-red"), Some("light-orange"), Some("light-yellow"), Some("light-green"), Some("light-blue"), Some("light-purple")];

fn text(label: &str, size: f64, weight: f64) -> Look { Look::Text(label.into(), None, size, weight) }

fn nodes(page: Page, ed: &Editor, block: BlockId) -> Vec<Node> {
    let core = &ed.core;
    let b = core.block(block);
    let kind = core.kind_of(block);
    let level = heading(kind);
    let is = |k: &str| match k {
        "text" => matches!(kind, Some(BlockKind::Text)),
        "bullet" => matches!(kind, Some(BlockKind::Bullet)),
        "ordered" => matches!(kind, Some(BlockKind::Ordered { .. })),
        "todo" => matches!(kind, Some(BlockKind::Todo { .. })),
        "quote" => matches!(kind, Some(BlockKind::Quote)),
        "code" => matches!(kind, Some(BlockKind::Code { .. })),
        "callout" => matches!(kind, Some(BlockKind::Callout { .. })),
        "page" => matches!(kind, Some(BlockKind::Page { card: false })),
        "card" => matches!(kind, Some(BlockKind::Page { card: true })),
        _ => false,
    };
    let pageable = !ed.markdown && kind.is_some_and(|k| k.has_text() && !matches!(k, BlockKind::Code { .. }));
    let header = |title: &str| Node::Header { title: title.into(), warn: false, reset: None, close: "fmt-page-block".into() };
    match page {
        Page::Block => vec![
            Node::Grab,
            Node::Cells { h: 60., rule: false, cells: vec![
                // 照 Craft（录屏 2026-11-15 20:44）：中标题、正文、页面、更多…；标题在「更多…」里。
                Item::new(text(tr("中标题", "Heading"), 16., 0.4), "fmt-kind-h3").on(level == Some(3)),
                Item::new(text(tr("正文", "Body"), 16., 0.), "fmt-kind-text").on(is("text")),
                Item::new(Look::Text(tr("页面", "Page").into(), Some("doc.text"), 15., 0.), "fmt-kind-page").on(is("page")).enabled(pageable),
                Item::new(Look::Text(tr("更多…", "More…").into(), None, 15., 0.), "fmt-page-more"),
            ] },
            Node::Cells { h: 58., rule: false, cells: vec![
                Item::new(Look::Symbol("checkmark.square"), "fmt-kind-todo").tile().on(is("todo")),
                Item::new(Look::Symbol("play.fill"), "fmt-fold").tile().on(b.is_some_and(|b| b.folded)).enabled(core.can_fold(block)),
                Item::new(Look::Symbol("list.bullet"), "fmt-kind-bullet").tile().on(is("bullet")),
                Item::new(Look::Symbol("list.number"), "fmt-kind-ordered").tile().on(is("ordered")),
                Item::new(Look::Symbol("decrease.indent"), "fmt-outdent").tile().enabled(core.can_outdent(block)),
                Item::new(Look::Symbol("increase.indent"), "fmt-indent").tile().enabled(core.can_indent(block)),
            ] },
            Node::Cells { h: 62., rule: false, cells: vec![
                Item::new(Look::Bar(tr("焦点", "Focus").into()), "fmt-focus").tile().span(2.2).on(is("quote")),
                Item::new(text(tr("块", "Block"), 16., 0.), "fmt-tint").tile().span(2.2).on(b.is_some_and(|b| b.bg.is_some())).enabled(kind.is_some_and(|k| k.takes_bg())),
                Item::new(Look::Wheel(current_color(ed, block)), "fmt-page-color").span(1.),
                Item::new(Look::Symbol("ellipsis"), "fmt-page-text").span(1.),
            ] },
        ],
        // 全部的标题层级（照 Craft 的名字：标题 / 副标题 / 中标题 / 小标题），再加几种整块的样式。
        Page::More => vec![
            header(tr("样式", "Styles")),
            Node::Cells { h: 64., rule: true, cells: vec![
                Item::new(text(tr("标题", "Title"), 22., 0.4), "fmt-kind-h1").on(level == Some(1)),
                Item::new(text(tr("副标题", "Subtitle"), 19., 0.3), "fmt-kind-h2").on(level == Some(2)),
                Item::new(text(tr("中标题", "Heading"), 17., 0.3), "fmt-kind-h3").on(level == Some(3)),
            ] },
            Node::Cells { h: 64., rule: true, cells: vec![
                Item::new(text(tr("小标题", "Strong"), 15., 0.4), "fmt-kind-h4").on(level == Some(4)),
                Item::new(text(tr("正文", "Body"), 16., 0.), "fmt-kind-text").on(is("text")),
                Item::new(text(tr("引用", "Quote"), 16., 0.), "fmt-kind-quote").on(is("quote")),
            ] },
            Node::Cells { h: 64., rule: true, cells: vec![
                Item::new(Look::Text(tr("代码", "Code").into(), Some("chevron.left.forwardslash.chevron.right"), 16., 0.), "fmt-kind-code").on(is("code")),
                Item::new(Look::Text(tr("高亮块", "Callout").into(), Some("text.bubble"), 16., 0.), "fmt-kind-callout").on(is("callout")),
            ] },
            // 页面 / 卡片：这一行字成为一页（字是它的标题），再点一下变回正文、内容回到正文里；是页面时换成另一种样子。Markdown 文档存不下，灰着。
            Node::Cells { h: 64., rule: false, cells: vec![
                Item::new(Look::Text(tr("页面", "Page").into(), Some("doc.text"), 16., 0.), "fmt-kind-page").on(is("page")).enabled(pageable),
                Item::new(Look::Text(tr("卡片", "Card").into(), Some("rectangle.portrait.on.rectangle.portrait"), 16., 0.), "fmt-kind-card").on(is("card")).enabled(pageable),
            ] },
        ],
        Page::Text => {
            let on = |get: fn(&fdl_core::model::Marks) -> bool| ed.mark_active(get);
            let align = b.map(|b| b.align).unwrap_or_default();
            let code = is("code");
            let styled = !ed.markdown && kind.is_some_and(|k| k.takes_bg());
            vec![
                header(tr("文字", "Text")),
                Node::Cells { h: 52., rule: false, cells: vec![
                    Item::new(Look::Symbol("bold"), "fmt-mark-b").on(on(|m| m.bold)).enabled(!code),
                    Item::new(Look::Symbol("italic"), "fmt-mark-i").on(on(|m| m.italic)).enabled(!code),
                    Item::new(Look::Symbol("underline"), "fmt-mark-u").on(on(|m| m.underline)).enabled(!code),
                    Item::new(Look::Symbol("strikethrough"), "fmt-mark-s").on(on(|m| m.strike)).enabled(!code),
                    Item::new(Look::Symbol("chevron.left.forwardslash.chevron.right"), "fmt-mark-c").on(on(|m| m.code)).enabled(!code),
                    Item::new(Look::Symbol("text.alignleft"), "fmt-align-l").on(align == Align::Left),
                    Item::new(Look::Symbol("text.aligncenter"), "fmt-align-c").on(align == Align::Center),
                    Item::new(Look::Symbol("text.alignright"), "fmt-align-r").on(align == Align::Right),
                ] },
                // 这一块的字体（Markdown 文档存不下，灰着）。
                Node::Cells { h: 84., rule: false, cells: FONTS.iter().map(|(sample, name, design, key)| {
                    let now = b.and_then(|b| b.font.as_deref()).unwrap_or("system");
                    Item::new(Look::Font(sample.to_string(), name.to_string(), design), format!("fmt-font-{key}")).tile().on(now == *key).enabled(styled)
                }).collect() },
                Node::Cells { h: 58., rule: false, cells: vec![
                    Item::new(Look::Text(tr("复制格式", "Copy Format").into(), Some("doc.on.doc"), 15., 0.), "fmt-copy").tile().enabled(styled),
                    Item::new(Look::Text(tr("粘贴格式", "Paste Format").into(), Some("doc.on.clipboard"), 15., 0.), "fmt-paste").tile().enabled(styled && COPIED.with(|c| c.borrow().is_some())),
                ] },
            ]
        }
        Page::Color => {
            let now = b.and_then(|b| b.bg.as_ref()).map(|c| crate::theme::bg_canon(&c.0));
            let ink = core.text_of(block).filter(|t| t.len() > 0).map(|t| t.marks_at(0).text_color.map(|c| c.0));
            vec![
                header(tr("颜色", "Color")),
                Node::Caption(tr("文字", "Text").into()),
                Node::Dots {
                    fills: crate::theme::FONT_COLORS.iter().map(|(name, _)| Some(name.and_then(crate::theme::text_color).unwrap_or_else(crate::theme::Theme::text))).collect(),
                    picked: ink.and_then(|n| crate::theme::FONT_COLORS.iter().position(|(c, _)| c.map(str::to_string) == n)),
                    act: "fmt-ink:".into(),
                },
                Node::Caption(tr("块背景", "Block background").into()),
                Node::Dots {
                    fills: BG.iter().map(|name| name.and_then(crate::theme::bg_swatch)).collect(),
                    picked: BG.iter().position(|c| *c == now.as_deref()),
                    act: "fmt-bg:".into(),
                },
            ]
        }
    }
}

/// 彩虹圈中间的那颗：这一块的文字颜色，没有就看底色。
fn current_color(ed: &Editor, block: BlockId) -> Option<Hsla> {
    let ink = ed.core.text_of(block).filter(|t| t.len() > 0).and_then(|t| t.marks_at(0).text_color).and_then(|c| crate::theme::text_color(&c.0));
    ink.or_else(|| ed.core.block(block).and_then(|b| b.bg.as_ref()).and_then(|c| crate::theme::bg_swatch(&crate::theme::bg_canon(&c.0))))
}

/// 面板的占位：贴在底边上面，两侧各留 13pt（和样式面板一样）。又点回正文（键盘回来了）或块不再选着时收起。
pub fn element(w: &Workspace, window: &Window, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    let page = PAGE.with(|p| p.get())?;
    let editor = w.editor.as_ref().filter(|_| w.view == View::Doc);
    let Some(ed) = editor.map(|e| e.read(cx)).filter(|ed| !ed.focus.is_focused(window) && ed.core.in_block_mode()) else { close(); return None };
    let block = ed.format_block()?;
    let nodes = nodes(page, ed, block);
    let (_, _, viewport) = super::layout();
    let width = viewport.width - px(26.);
    let height = px(super::panel::height(&nodes, f32::from(width)));
    let bottom = super::insets().bottom.max(px(34.)) + px(1.);
    HEIGHT.with(|h| h.set(f32::from(height + bottom) - f32::from(super::insets().bottom) + 8.));
    Some(div().id("format-panel").absolute().left(px(13.)).bottom(bottom).w(width).h(height).occlude()
        .child(super::panel::slot(nodes)).into_any_element())
}
