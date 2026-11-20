//! 样式面板里的笔尖、纸角和文档缩略页。由 UIKit 按这篇文档的实际用色绘制。
use gpui::{Hsla, black, white};
use objc2::msg_send;
use objc2::runtime::{AnyClass, AnyObject};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};

use super::{CGAffineTransform, Card, Fill, MiniDivider, TILE, cg, fill, font, gradient, label, mask_path, plain, rect, ring, torn_edges, ui};
use super::super::{glass::ui_color, style::{FADED_PAPER_OPACITY, contrast}};

fn shade(color: Hsla, lightness: f32) -> Hsla { Hsla { l: (color.l + lightness).clamp(0., 1.), ..color } }

fn paper_label(paper: Hsla, ink: Hsla) -> Hsla {
    if contrast(paper, ink) >= 4.5 { return ink; }
    if contrast(paper, black()) >= 4.5 { black() } else { white() }
}

pub(super) unsafe fn ink_tile(view: *mut AnyObject, width: f64, title: &str, paper: Hsla, ink: Hsla) {
    unsafe {
        let frame = rect(0., 0., width, TILE);
        gradient(view, frame, &[shade(paper, 0.035), shade(paper, -0.04)], (0., 0.), (1., 1.), 0.);
        ring(view, frame, 16., ui_color(paper_label(paper, ink).opacity(0.08)), 0., 0.5);
        let text = label(view, rect(8., 45., width - 16., 26.), title, font(17., 0.29, ""), ui_color(paper_label(paper, ink)), 1);
        let _: () = msg_send![text, setIsAccessibilityElement: false];

        let nib = rect(width / 2. - 6.5, 77., 13., 24.);
        let tip = gradient(view, nib, &[shade(ink, -0.19), shade(ink, 0.16), ink], (0., 0.5), (1., 0.5), 0.);
        let bezier = AnyClass::get(c"UIBezierPath").unwrap();
        let path: *mut AnyObject = msg_send![bezier, bezierPath];
        let _: () = msg_send![path, moveToPoint: CGPoint::new(1., 24.)];
        let _: () = msg_send![path, addCurveToPoint: CGPoint::new(6.5, 0.), controlPoint1: CGPoint::new(-1., 12.), controlPoint2: CGPoint::new(3., 4.)];
        let _: () = msg_send![path, addCurveToPoint: CGPoint::new(12., 24.), controlPoint1: CGPoint::new(10., 4.), controlPoint2: CGPoint::new(14., 13.)];
        let _: () = msg_send![path, closePath];
        mask_path(tip, nib.size, msg_send![path, CGPath]);
        fill(tip, rect(6.3, 3., 0.7, 17.), &Fill::Color(shade(ink, -0.26).opacity(0.6)), 0.35);

        let barrel = rect(width / 2. - 19., 96., 38., 34.);
        let body = gradient(view, barrel, &[shade(ink, -0.19), ink, shade(ink, 0.25), shade(ink, -0.06)], (0., 0.5), (1., 0.5), 0.);
        let path: *mut AnyObject = msg_send![bezier, bezierPath];
        let _: () = msg_send![path, moveToPoint: CGPoint::new(0., 34.)];
        let _: () = msg_send![path, addQuadCurveToPoint: CGPoint::new(9., 0.), controlPoint: CGPoint::new(4., 12.)];
        let _: () = msg_send![path, addLineToPoint: CGPoint::new(29., 0.)];
        let _: () = msg_send![path, addQuadCurveToPoint: CGPoint::new(38., 34.), controlPoint: CGPoint::new(34., 12.)];
        let _: () = msg_send![path, closePath];
        mask_path(body, barrel.size, msg_send![path, CGPath]);
        fill(body, rect(6., 0., 26., 1.), &Fill::Color(white().opacity(0.23)), 0.);
    }
}

pub(super) unsafe fn paper_tile(view: *mut AnyObject, width: f64, title: &str, paper: Hsla, ink: Hsla) {
    unsafe {
        let bounds = rect(0., 0., width, TILE);
        gradient(view, bounds, &[shade(paper, 0.028), paper, shade(paper, -0.055)], (0.2, 0.), (0.8, 1.), 0.);
        let ink = paper_label(paper, ink);
        for y in [29., 42., 55., 68., 81., 94., 107., 120.] {
            fill(view, rect(0., y, width, 0.5), &Fill::Color(ink.opacity(0.10)), 0.);
        }
        let side = (width * 0.71).min(TILE * 0.73);
        let frame = rect(width - side, TILE - side, side, side);
        // 折线沿对角，阴影从折线向纸角逐渐散开。
        let shade_view = gradient(view, frame, &[black().opacity(0.30), black().opacity(0.07), black().opacity(0.)], (0.48, 0.48), (0.87, 0.87), 0.);
        let bezier = AnyClass::get(c"UIBezierPath").unwrap();
        let underside: *mut AnyObject = msg_send![bezier, bezierPath];
        let _: () = msg_send![underside, moveToPoint: CGPoint::new(0., side)];
        let _: () = msg_send![underside, addLineToPoint: CGPoint::new(side, 0.)];
        let _: () = msg_send![underside, addLineToPoint: CGPoint::new(side, side)];
        let _: () = msg_send![underside, closePath];
        mask_path(shade_view, frame.size, msg_send![underside, CGPath]);

        let curl = gradient(view, frame, &[shade(paper, 0.03), shade(paper, 0.105), shade(paper, -0.09)], (0.24, 0.22), (0.92, 0.87), 0.);
        let path: *mut AnyObject = msg_send![bezier, bezierPath];
        let _: () = msg_send![path, moveToPoint: CGPoint::new(0., side)];
        let _: () = msg_send![path, addCurveToPoint: CGPoint::new(side * 0.61, side * 0.25),
            controlPoint1: CGPoint::new(side * 0.29, side * 0.83), controlPoint2: CGPoint::new(side * 0.28, side * 0.29)];
        let _: () = msg_send![path, addQuadCurveToPoint: CGPoint::new(side, 0.), controlPoint: CGPoint::new(side * 0.9, side * 0.20)];
        let edge: *const super::CGPath = msg_send![path, CGPath];
        let lip: *mut AnyObject = msg_send![AnyClass::get(c"CAShapeLayer").unwrap(), layer];
        let _: () = msg_send![lip, setPath: edge];
        let _: () = msg_send![lip, setFillColor: std::ptr::null::<super::CGColor>()];
        let _: () = msg_send![lip, setStrokeColor: cg(ui_color(white().opacity(0.31)))];
        let _: () = msg_send![lip, setLineWidth: 0.65f64];
        let _: () = msg_send![path, closePath];
        mask_path(curl, frame.size, msg_send![path, CGPath]);
        let layer: *mut AnyObject = msg_send![curl, layer];
        let _: () = msg_send![layer, addSublayer: lip];
        ring(view, bounds, 16., ui_color(ink.opacity(0.08)), 0., 0.5);
        let text = label(view, rect(12., 6., width - 24., 23.), title, font(14., 0.23, ""), ui_color(ink), 0);
        let _: () = msg_send![text, setIsAccessibilityElement: false];
    }
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Preview { Gallery, Stack }

unsafe fn shadow(view: *mut AnyObject, bounds: CGRect, radius: f64) {
    unsafe {
        let Some(s) = crate::theme::shadow_card().into_iter().last() else { return; };
        let layer: *mut AnyObject = msg_send![view, layer];
        let bezier = AnyClass::get(c"UIBezierPath").unwrap();
        let path: *mut AnyObject = msg_send![bezier, bezierPathWithRoundedRect: bounds, cornerRadius: radius];
        let path: *const super::CGPath = msg_send![path, CGPath];
        let _: () = msg_send![layer, setShadowPath: path];
        let _: () = msg_send![layer, setShadowColor: cg(ui_color(s.color.into()))];
        let _: () = msg_send![layer, setShadowOpacity: 1f32];
        let _: () = msg_send![layer, setShadowRadius: f64::from(s.blur_radius) * 0.56];
        let _: () = msg_send![layer, setShadowOffset: CGSize::new(f64::from(s.offset.x), f64::from(s.offset.y))];
    }
}

/// 一套样式的样子（照 Craft 样式图库，IMG_9340–9342）：整块铺背景，中间一张窄窄的纸，纸外一圈半透明的玻璃边；
/// 纸上左上角是用这套字体写的名字，有封面时斜着横过一条封面当腰封，分隔符是胶带就斜着贴一条（下端出纸边），
/// 是涂鸦就在纸下部画一笔带小圈的线（照 Gingerbread，IMG_9341）；普通的线不画。主面板「样式」那一格（`Stack`）的纸几乎贴满，下半截出了格子。
pub(super) unsafe fn preview(parent: *mut AnyObject, frame: CGRect, card: &Card, kind: Preview) -> *mut AnyObject {
    unsafe {
        let (w, h) = (frame.size.width, frame.size.height);
        let shell = plain(parent, frame);
        let bounds = rect(0., 0., w, h);
        let gallery = kind == Preview::Gallery;
        let radius = if gallery { 12. } else { 11. };
        if !gallery { shadow(shell, bounds, radius); }
        let fallback = Fill::Gradient(shade(card.paper, -0.07), shade(card.paper, -0.13), 2);
        let background = fill(shell, bounds, card.fill.as_ref().unwrap_or(&fallback), radius);
        if card.faded && card.fill.is_some() { fill(background, bounds, &Fill::Color(card.paper.opacity(FADED_PAPER_OPACITY)), 0.); }
        let (sw, sh, sx, sy) = if gallery { (w * 0.41, h * 0.6, w * 0.295, h * 0.19) } else { (w - 7., h - 7., 3.5, 3.5) };
        if gallery {
            // 纸外那圈玻璃边：浅白的一层加一道细亮边。
            let rim = fill(background, rect(sx - 4., sy - 4., sw + 8., sh + 8.), &Fill::Color(white().opacity(0.26)), 9.);
            ring(rim, rect(0., 0., sw + 8., sh + 8.), 9., ui_color(white().opacity(0.45)), 0., 0.6);
        }
        let page = plain(background, rect(sx, sy, sw, sh));
        let sheet_rect = rect(0., 0., sw, sh);
        shadow(page, sheet_rect, 6.);
        let sheet = gradient(page, sheet_rect, &[shade(card.paper, 0.025), card.paper], (0.18, 0.08), (0.9, 1.), if gallery { 5. } else { 6. });
        let margin = sw * 0.1;
        let size = if gallery { (sw * 0.16).clamp(9., 14.) } else { (sw * 0.12).clamp(9., 12.) };
        let title = label(sheet, rect(margin, sw * 0.07, sw - 2. * margin, size * 2.6), &card.name, font(size, 0.42, card.design), ui_color(paper_label(card.paper, card.ink)), 0);
        let _: () = msg_send![title, setNumberOfLines: 2isize];
        let _: () = msg_send![title, sizeToFit];
        let mut top: CGRect = msg_send![title, frame];
        top.size.width = top.size.width.min(sw - 2. * margin);
        let _: () = msg_send![title, setFrame: top];
        let _: () = msg_send![title, setIsAccessibilityElement: false];
        let below = top.origin.y + top.size.height + 5.;
        if let Some(cover) = &card.cover {
            // 照 Craft（IMG_9345）：封面是斜着横过整张纸的一条「腰封」，两头出纸边，被纸的圆角裁掉。
            let thick = sw * 0.27;
            let band = fill(sheet, rect(-sw * 0.3, sh * 0.52 - thick / 2., sw * 1.6, thick), &Fill::Image(cover.clone()), 0.);
            let (s, c) = (-0.2f64).sin_cos();
            let _: () = msg_send![band, setTransform: CGAffineTransform { a: c, b: s, c: -s, d: c, tx: 0., ty: 0. }];
        }
        match &card.divider {
            MiniDivider::Tape(tape) => {
                let (len, thick) = (sh * 0.62, (sw * 0.2).clamp(8., 18.));
                let centre = (sw * 0.6, (below + sh) / 2. + sh * 0.12);
                let swatch = fill(sheet, rect(centre.0 - len / 2., centre.1 - thick / 2., len, thick), tape, 0.);
                torn_edges(swatch, CGSize::new(len, thick));
                let (s, c) = (-1.15f64).sin_cos();
                let _: () = msg_send![swatch, setTransform: CGAffineTransform { a: c, b: s, c: -s, d: c, tx: 0., ty: 0. }];
            }
            MiniDivider::Doodle(_) => {
                // 照 Craft 的 Gingerbread：纸下部一笔手画的线，左边打个小圈，再缓缓扬上去，右端略出纸边（画在 page 上，不被纸的圆角裁掉）。
                let p = |x: f64, y: f64| CGPoint::new(sw * x, sh * y);
                let path: *mut AnyObject = msg_send![AnyClass::get(c"UIBezierPath").unwrap(), bezierPath];
                let _: () = msg_send![path, moveToPoint: p(0.03, 0.93)];
                let _: () = msg_send![path, addCurveToPoint: p(0.28, 0.80), controlPoint1: p(0.12, 0.93), controlPoint2: p(0.25, 0.88)];
                let _: () = msg_send![path, addCurveToPoint: p(0.19, 0.83), controlPoint1: p(0.31, 0.72), controlPoint2: p(0.16, 0.73)];
                let _: () = msg_send![path, addCurveToPoint: p(0.42, 0.90), controlPoint1: p(0.22, 0.91), controlPoint2: p(0.32, 0.93)];
                let _: () = msg_send![path, addCurveToPoint: p(1.05, 0.69), controlPoint1: p(0.6, 0.86), controlPoint2: p(0.8, 0.66)];
                let stroke: *mut AnyObject = msg_send![AnyClass::get(c"CAShapeLayer").unwrap(), layer];
                let cg_path: *const super::CGPath = msg_send![path, CGPath];
                let _: () = msg_send![stroke, setPath: cg_path];
                let _: () = msg_send![stroke, setFillColor: std::ptr::null::<super::CGColor>()];
                let _: () = msg_send![stroke, setStrokeColor: cg(ui_color(paper_label(card.paper, card.ink).opacity(0.85)))];
                let _: () = msg_send![stroke, setLineWidth: (sw * 0.018).clamp(1.1, 1.6)];
                let cap: *mut AnyObject = msg_send![AnyClass::get(c"NSString").unwrap(), stringWithUTF8String: c"round".as_ptr()];
                let _: () = msg_send![stroke, setLineCap: cap];
                let layer: *mut AnyObject = msg_send![page, layer];
                let _: () = msg_send![layer, addSublayer: stroke];
            }
            MiniDivider::Line => {}
        }
        if gallery { ring(shell, bounds, radius, ui(c"separatorColor"), 0., 0.5); }
        shell
    }
}

pub(super) unsafe fn stack(parent: *mut AnyObject, width: f64, current: &Card, others: &[Card]) {
    unsafe {
        // 照 Craft（IMG_9339、9343）：当前这套居中、下半截出了格子，后面两套往右斜着露出一条边。
        // 尺寸和位置照 Craft（思程DEMO台本 3）：前面那张 80pt 宽、离格子顶 15pt，左边落在格子正中，后面两张各往右错开 17pt；
        // 格子窄时整叠往左让，最后一张右边留 16pt。
        let (w, h) = (80., 118.);
        let left = (width * 0.5).min(width - w - 34. - 16.);
        // 背景是图片时，卡片四周透出一小圈同一张图的晕影（IMG_9359：只是一点点，不是整张）。纯色、渐变背景没有。
        if let Some(Fill::Image(path)) = &current.fill {
            let halo = rect(left - 46., 0., w + 126., TILE);
            let ghost = fill(parent, halo, &Fill::Image(path.clone()), 0.);
            let _: () = msg_send![ghost, setAlpha: 0.3f64];
            let mask: *mut AnyObject = msg_send![AnyClass::get(c"CAGradientLayer").unwrap(), layer];
            let radial: *mut AnyObject = msg_send![AnyClass::get(c"NSString").unwrap(), stringWithUTF8String: c"radial".as_ptr()];
            let _: () = msg_send![mask, setType: radial];
            let stops: *mut AnyObject = msg_send![AnyClass::get(c"NSMutableArray").unwrap(), array];
            for a in [1., 0.8, 0.3, 0.] { let _: () = msg_send![stops, addObject: cg(ui_color(black().opacity(a))) as *mut AnyObject]; }
            let _: () = msg_send![mask, setColors: stops];
            let _: () = msg_send![mask, setStartPoint: CGPoint::new(0.5, 0.6)];
            // 半径在横向正好够到两边、纵向够到底边，四周都淡到没有，不留方边。
            let _: () = msg_send![mask, setEndPoint: CGPoint::new(1., 1.)];
            let _: () = msg_send![mask, setFrame: rect(0., 0., halo.size.width, halo.size.height)];
            let layer: *mut AnyObject = msg_send![ghost, layer];
            let _: () = msg_send![layer, setMask: mask];
        }
        for (i, card) in others.iter().enumerate().rev() {
            let offset = i as f64 + 1.;
            let page = preview(parent, rect(left + offset * 17., 17. + offset * 3., w, h), card, Preview::Stack);
            let (s, c) = (offset * 0.075).sin_cos();
            let transform = CGAffineTransform { a: c, b: s, c: -s, d: c, tx: 0., ty: 0. };
            let _: () = msg_send![page, setTransform: transform];
        }
        preview(parent, rect(left, 15., w, h), current, Preview::Stack);
    }
}
