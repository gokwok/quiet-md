//! 手机上文档卡片的瀑布流（照 Craft）：两列，卡片按内容高低不等，每张放进当时较矮的那一列。
//! 高度只用索引里已有的东西估（标题、纯文本正文的前几百字），不为排版去解析每篇文档；
//! 只画屏幕上下各多一屏之内的卡片。有文件夹的那一层还是等高的网格（`home.rs`），文件夹卡片有自己的形状。
use gpui::*;

use crate::app::Workspace;
use crate::store::Entry;

/// 卡片的高最矮、最高是宽的几倍。
const MIN: f32 = 0.72;
const MAX: f32 = 1.5;
/// 缩略图里的字（`home_preview_style::SCALE` 缩过的正文）：一个汉字宽多少、一行多高、段间多少。
const GLYPH: f32 = 4.4;
const LINE: f32 = 7.3;
const PARA: f32 = 3.;
/// 纸的边距（`render_home_card` 的 18）、标题一行连同下面的间距。
const PAD: f32 = 18.;
const TITLE: f32 = 28.;
/// 估高时最多看正文的这么多字：再多卡片也到顶了。
const PEEK: usize = 800;

/// 一张卡片该多高：纸的边距、标题，加上正文按缩略图的字号排下来的行数，夹在 `MIN`–`MAX` 之间。
pub fn card_h(e: &Entry, w: f32) -> f32 {
    let per_line = ((w - PAD * 2.) / GLYPH).max(1.);
    let mut h = PAD * 2. + TITLE;
    for para in e.body.chars().take(PEEK).collect::<String>().split('\n') {
        // 西文字母约半个汉字宽。
        let width: f32 = para.chars().map(|c| if c.is_ascii() { 0.55 } else { 1. }).sum();
        if width == 0. { continue; }
        h += (width / per_line).ceil() * LINE + PARA;
        if h > w * MAX { break; }
    }
    h.clamp(w * MIN, w * MAX).round()
}

/// 瀑布流本身：一个能滚的区域，里面是总高的一块，卡片按算好的位置绝对定位。`lead`（大标题和搜索）在最上面跟着一起滚。
#[allow(clippy::too_many_arguments)]
pub fn grid(w: &Workspace, entries: &[Entry], cols: usize, side: f32, gap: f32, avail: f32, top: f32, bottom: f32, lead: Option<AnyElement>, scroll: &ScrollHandle, window: &Window, cx: &mut Context<Workspace>) -> AnyElement {
    let cols = cols.max(1);
    let cw = (avail - gap * (cols - 1) as f32) / cols as f32;
    let mut ends = vec![top; cols];
    let mut slots = Vec::with_capacity(entries.len());
    for (i, e) in entries.iter().enumerate() {
        let h = card_h(e, cw);
        let c = (0..cols).min_by(|a, b| ends[*a].total_cmp(&ends[*b])).unwrap_or(0);
        slots.push((i, side + c as f32 * (cw + gap), ends[c], h));
        ends[c] += h + gap;
    }
    let total = ends.iter().copied().fold(top, f32::max) + bottom;
    // 滚动的偏移是上一帧的：上下各多画一屏，快速滑动时边上也不缺卡片。
    let off = -f32::from(scroll.offset().y);
    let view = f32::from(window.viewport_size().height);
    let (lo, hi) = (off - view, off + view * 2.);
    let cards: Vec<AnyElement> = slots.into_iter().filter(|(_, _, y, h)| y + h >= lo && *y <= hi)
        .map(|(i, x, y, h)| div().absolute().left(px(x)).top(px(y)).w(px(cw)).h(px(h)).child(w.render_home_card(&entries[i], h, cx)).into_any_element())
        .collect();
    div().id("home-masonry").flex_1().min_h_0().overflow_y_scroll().track_scroll(scroll)
        .child(div().relative().w_full().h(px(total))
            .children(lead.map(|l| div().absolute().top_0().left_0().right_0().child(l)))
            .children(cards))
        .into_any_element()
}
