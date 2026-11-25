//! 手机上文档卡片的瀑布流（照 Craft）：两列，卡片按内容高低不等、下沿错开，每张放进当时较矮的那一列。
//! 高度是把缩略图那一页真的排一遍量出来的（图片按原比例、字按实际折行），每篇量一次记着；只量到屏幕下面一屏为止，
//! 再往下的按索引里的正文估，滚到跟前才去解析文档——量过的都在它们上面，位置不会再变。
//! 只画屏幕上下各多一屏之内的卡片。有文件夹的那一层还是等高的网格（`home.rs`），文件夹卡片有自己的形状。
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use gpui::*;

use crate::app::Workspace;
use crate::store::Entry;

/// 卡片的高最矮、最高是宽的几倍（Craft 实测：几行字的 132/115，满是字的 678/416）。
const MIN: f32 = 1.15;
const MAX: f32 = 1.63;
/// 放不下的长文档不都顶到最高，各自收在这个倍数和最高之间；两列的下沿至少错开这么多。
const TALL: f32 = 1.2;
const APART: f32 = 48.;
/// 为了错开，空着大半的短文档能收到这么矮。
const LOW: f32 = 0.9;
/// 缩略图里的字（`home_preview_style::SCALE` 缩过的正文）：一个汉字宽多少、一行多高、段间多少。
const GLYPH: f32 = 5.9;
const LINE: f32 = 9.6;
const PARA: f32 = 4.1;
/// 纸的边距（Craft 实测 30/921 屏宽）。
const PAD: f32 = 14.;
/// 标题一行连同下面的间距。
const TITLE: f32 = 31.;
/// 估高时最多看正文的这么多字：再多卡片也到顶了。
const PEEK: usize = 800;
/// 有背景的文档，纸外面那一圈（`render_home_card`）。
const FRAME: f32 = 5.;
/// 纸下沿渐隐的一段有多高（Craft 实测 25–40）。
const FADE: f32 = 40.;

// 量过的高度：哪一篇、多宽的纸 → 当时的样子（内容、样式、字体）和高度。
thread_local! { static HEIGHTS: RefCell<HashMap<(String, u32), (u64, f32)>> = RefCell::new(HashMap::new()); }
// 正在画的这张卡片整篇都放下了。
thread_local! { static WHOLE: Cell<bool> = const { Cell::new(false) }; }

/// 卡片里纸的边距。
pub fn pad() -> f32 { if super::is_compact() { PAD } else { 18. } }
/// 缩略图标题的字号：卡片上 16（Craft 实测），文件夹里露出来的那张小纸 15。
pub fn title_size(pad: f32) -> f32 { if super::is_compact() && pad > 12. { 16. } else { 15. } }
/// 这张卡片整篇都放下了，下沿不用渐隐。
pub fn whole() -> bool { WHOLE.with(|w| w.get()) }
/// 纸的下沿一段渐隐（照 Craft）：写不下的字、图慢慢淡进纸色，而不是一刀切断。
pub fn fade(paper: Hsla) -> Div {
    div().absolute().left_0().right_0().bottom_0().h(px(FADE)).bg(linear_gradient(180., linear_color_stop(paper.opacity(0.), 0.), linear_color_stop(paper.opacity(0.95), 1.)))
}

/// 还没量的卡片估多高：纸的边距、标题，加上正文按缩略图的字号排下来的行数。
fn guess(e: &Entry, w: f32) -> f32 {
    let per_line = ((w - PAD * 2.) / GLYPH).max(1.);
    let mut h = PAD * 2. + TITLE;
    for para in e.body.chars().take(PEEK).collect::<String>().split('\n') {
        // 西文字母约半个汉字宽。
        let width: f32 = para.chars().map(|c| if c.is_ascii() { 0.55 } else { 1. }).sum();
        if width == 0. { continue; }
        h += (width / per_line).ceil() * LINE + PARA;
        if h > w * MAX { break; }
    }
    h
}

/// 整篇内容要多高的卡片：`render_home_card` 的那一页按同样的宽排一遍。
fn measure(w: &Workspace, e: &Entry, cw: f32, window: &mut Window, cx: &mut App) -> f32 {
    let mut stamp = rustc_hash::FxHasher::default();
    (e.meta.modified, &e.title, &e.icon, e.meta.starred > 0, format!("{:?}", e.meta.style), super::style::cjk::generation(), crate::editor_theme::body_size().to_bits()).hash(&mut stamp);
    let (key, stamp) = ((e.id.clone(), cw.to_bits()), stamp.finish());
    if let Some((s, h)) = HEIGHTS.with(|m| m.borrow().get(&key).copied()) && s == stamp { return h; }
    let frame = if e.meta.style.as_ref().is_some_and(|s| s.background.is_some()) { FRAME } else { 0. };
    let inner = px(cw - frame * 2.);
    let mut page = div().w(inner).child(w.thumb_page_inked(e, PAD, None)).into_any_element();
    let h = f32::from(page.layout_as_root(size(AvailableSpace::Definite(inner), AvailableSpace::MaxContent), window, cx).height) + frame;
    HEIGHTS.with(|m| m.borrow_mut().insert(key, (stamp, h)));
    h
}

/// 内容要 `full` 高的这一篇，卡片画多高：短的按内容，长的各收各的高（同一篇总是一样）；
/// 接在 `y` 下面、下沿快和别的列 `ends` 对齐了，就收短一点（字本来就放不下，或下面空着），收不了就放长，排出来总是错落的。
fn height(id: &str, full: f32, cw: f32, y: f32, ends: impl Iterator<Item = f32>) -> f32 {
    let mut pick = rustc_hash::FxHasher::default();
    id.hash(&mut pick);
    let t = (pick.finish() >> 48) as f32 / 65535.;
    let h = if full > cw * MAX { cw * (TALL + (MAX - TALL) * t) } else { full.max(cw * MIN) };
    let Some(e) = ends.filter(|e| (y + h - e).abs() < APART).min_by(|a, b| (y + h - a).abs().total_cmp(&(y + h - b).abs())) else { return h.round() };
    let (short, long) = (e - APART - y, e + APART - y);
    let floor = if full > h + 0.5 || long > cw * MAX { cw * MIN } else { full.max(cw * LOW) };
    (if short >= floor { short } else { long }).round()
}

/// 瀑布流本身：一个能滚的区域，里面是总高的一块，卡片按算好的位置绝对定位。`lead`（大标题和搜索）在最上面跟着一起滚。
#[allow(clippy::too_many_arguments)]
pub fn grid(entries: Vec<Entry>, cols: usize, side: f32, gap: f32, avail: f32, top: f32, bottom: f32, lead: Option<AnyElement>, scroll: &ScrollHandle, cx: &mut Context<Workspace>) -> AnyElement {
    super::style::cjk::watch(cx);
    let cols = cols.max(1);
    let cw = (avail - gap * (cols - 1) as f32) / cols as f32;
    let off = -f32::from(scroll.offset().y);
    let body = cx.processor(move |w, (), window: &mut Window, cx| {
        // 滚动的偏移是上一帧的：上下各多画一屏，快速滑动时边上也不缺卡片。
        let view = f32::from(window.viewport_size().height);
        let (lo, hi) = (off - view, off + view * 2.);
        let mut ends = vec![top; cols];
        let mut slots = Vec::new();
        for (i, e) in entries.iter().enumerate() {
            let c = (0..cols).min_by(|a, b| ends[*a].total_cmp(&ends[*b])).unwrap_or(0);
            let y = ends[c];
            let full = if y <= hi { measure(w, e, cw, window, cx) } else { guess(e, cw) };
            let h = height(&e.id, full, cw, y, (0..cols).filter(|o| *o != c).map(|o| ends[o]));
            if y + h >= lo && y <= hi { slots.push((i, side + c as f32 * (cw + gap), y, h, full <= h + 0.5)); }
            ends[c] += h + gap;
        }
        let total = ends.iter().copied().fold(top, f32::max) + bottom;
        // 横滑的那张最后画：挪到旁边一列时压在那一列的卡片上面。
        super::pick::on_top(&mut slots, |s| entries[s.0].id.clone());
        let cards: Vec<AnyElement> = slots.into_iter().map(|(i, x, y, h, whole)| {
            WHOLE.with(|f| f.set(whole));
            let card = w.render_home_card(&entries[i], h, cx);
            WHOLE.with(|f| f.set(false));
            div().absolute().left(px(x)).top(px(y)).w(px(cw)).h(px(h)).child(card).into_any_element()
        }).collect();
        div().relative().w_full().h(px(total)).children(cards).into_any_element()
    });
    div().id("home-masonry").flex_1().min_h_0().overflow_y_scroll().track_scroll(scroll)
        .child(div().relative().w_full()
            .child(Late { build: Some(Box::new(move |window, cx| body((), window, cx))), child: None })
            .children(lead.map(|l| div().absolute().top_0().left_0().right_0().child(l))))
        .into_any_element()
}

/// 排版时才造的元素：量高度要 `Window` 能改，画页面的那些函数手里只有只读的。
type Build = Box<dyn FnOnce(&mut Window, &mut App) -> AnyElement>;
struct Late { build: Option<Build>, child: Option<AnyElement> }
impl IntoElement for Late { type Element = Self; fn into_element(self) -> Self { self } }
impl Element for Late {
    type RequestLayoutState = ();
    type PrepaintState = ();
    fn id(&self) -> Option<ElementId> { None }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> { None }
    fn request_layout(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, window: &mut Window, cx: &mut App) -> (LayoutId, ()) {
        let mut child = match self.build.take() { Some(build) => build(window, cx), None => div().into_any_element() };
        let id = child.request_layout(window, cx);
        self.child = Some(child);
        (id, ())
    }
    fn prepaint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, _: Bounds<Pixels>, _: &mut (), window: &mut Window, cx: &mut App) { if let Some(c) = &mut self.child { c.prepaint(window, cx); } }
    fn paint(&mut self, _: Option<&GlobalElementId>, _: Option<&InspectorElementId>, _: Bounds<Pixels>, _: &mut (), _: &mut (), window: &mut Window, cx: &mut App) { if let Some(c) = &mut self.child { c.paint(window, cx); } }
}
