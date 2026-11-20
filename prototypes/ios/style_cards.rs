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
pub(super) enum Preview { Gallery, Stack, Badge }

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

pub(super) unsafe fn preview(parent: *mut AnyObject, frame: CGRect, card: &Card, kind: Preview) -> *mut AnyObject {
    unsafe {
        let (w, h) = (frame.size.width, frame.size.height);
        let shell = plain(parent, frame);
        let bounds = rect(0., 0., w, h);
        let radius = if kind == Preview::Gallery { 14. } else { 8. };
        if kind == Preview::Stack { shadow(shell, bounds, radius); }
        let fallback = Fill::Gradient(shade(card.paper, -0.07), shade(card.paper, -0.13), 2);
        let background = fill(shell, bounds, card.fill.as_ref().unwrap_or(&fallback), radius);
        if card.faded && card.fill.is_some() { fill(background, bounds, &Fill::Color(card.paper.opacity(FADED_PAPER_OPACITY)), 0.); }
        let (inset, top, bottom) = match kind {
            Preview::Gallery => (w * 0.23, h * 0.13, h * 0.13),
            Preview::Stack => (5., 5., 5.),
            Preview::Badge => (w * 0.20, 5., 5.),
        };
        let (sw, sh) = (w - 2. * inset, h - top - bottom);
        let sheet_frame = rect(inset, top, sw, sh);
        let page = plain(background, sheet_frame);
        let sheet_rect = rect(0., 0., sw, sh);
        if kind == Preview::Gallery { shadow(page, sheet_rect, 6.); }
        let sheet = gradient(page, sheet_rect, &[shade(card.paper, 0.025), card.paper], (0.18, 0.08), (0.9, 1.), 6.);
        ring(page, sheet_rect, 6., ui_color(paper_label(card.paper, card.ink).opacity(0.13)), 0., 0.55);
        let badge = kind == Preview::Badge;
        let margin = if badge { 4. } else { sw * 0.12 };
        let body = sw - 2. * margin;
        if !badge {
            let title = label(sheet, rect(margin, sh * 0.095, body, sh * 0.29), &card.name,
                font((sh * 0.125).clamp(9., 17.), 0.42, card.design), ui_color(card.ink), 0);
            let lines = if card.name.chars().count() <= 6 { 1isize } else { 2isize };
            let _: () = msg_send![title, setNumberOfLines: lines];
            let _: () = msg_send![title, setIsAccessibilityElement: false];
        }
        let rule_h = if badge { 6. } else { (sh * 0.15).clamp(9., 18.) };
        let rule = rect(margin, sh * 0.58, body, rule_h);
        match &card.divider {
            MiniDivider::Tape(path) => {
                let swatch = fill(sheet, rule, &Fill::Image(path.clone()), 0.);
                torn_edges(swatch, rule.size);
                if !badge {
                    let (s, c) = (-0.12f64).sin_cos();
                    let transform = CGAffineTransform { a: c, b: s, c: -s, d: c, tx: 0., ty: 0. };
                    let _: () = msg_send![swatch, setTransform: transform];
                }
            }
            MiniDivider::Doodle(path) => {
                let image = fill(sheet, rule, &Fill::Image(path.clone()), 0.);
                let _: () = msg_send![image, setContentMode: 1isize];
            }
            MiniDivider::Line => { fill(sheet, rect(margin, rule.origin.y + rule_h * 0.52, body, 1.), &Fill::Color(card.ink.opacity(0.3)), 0.5); }
        }
        fill(sheet, rect(margin, if badge { sh * 0.27 } else { sh * 0.43 }, body * 0.82, 1.2), &Fill::Color(card.ink.opacity(0.47)), 0.6);
        if !badge {
            for (dy, fraction) in [(0.82, 0.97), (0.88, 0.63)] {
                fill(sheet, rect(margin, sh * dy, body * fraction, 1.), &Fill::Color(card.ink.opacity(0.31)), 0.5);
            }
        }
        ring(shell, bounds, radius, ui(c"separatorColor"), 0., if kind == Preview::Stack { 0.65 } else { 0.5 });
        shell
    }
}

pub(super) unsafe fn stack(parent: *mut AnyObject, width: f64, current: &Card, others: &[Card]) {
    unsafe {
        let (w, h, left) = (90., 118., width - 135.);
        for (i, card) in others.iter().enumerate().rev() {
            let offset = i as f64 + 1.;
            let page = preview(parent, rect(left + offset * 13., 17. + offset * 5., w, h), card, Preview::Stack);
            let (s, c) = (offset * 0.083).sin_cos();
            let transform = CGAffineTransform { a: c, b: s, c: -s, d: c, tx: 0., ty: 0. };
            let _: () = msg_send![page, setTransform: transform];
        }
        preview(parent, rect(left, 15., w, h), current, Preview::Stack);
    }
}
