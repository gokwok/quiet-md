//! 设置 › 关于 Margin（手机）：iOS 版叫 Margin（Info.plist 的 `CFBundleDisplayName`，中英文一样），源码是开源的 Quiet.md。照这台 iPhone 上的真实情况写——图标就是主屏上那张，文档在本机或 iCloud 云盘，
//! 没有账号也不收集数据；没有「软件更新」（App 由系统更新）。下一级「开源许可」：本体的 MIT 许可、素材、
//! 以及 iOS 版真正编进去的开源组件（`licences.txt`，由 `licences.sh` 从 Cargo.lock 生成）。
use gpui::{prelude::FluentBuilder, *};

use crate::app::Workspace;
use crate::settings::{tr, Page};
use crate::theme::Theme;

/// 桌面上的「Quiet.md」在手机上叫这个，和主屏幕上图标下的名字一样。
pub fn name() -> &'static str { "Margin" }
const LICENSE: &str = include_str!("../../LICENSE");
const CRATES: &str = include_str!("licences.txt");

pub fn page(_w: &Workspace, cx: &mut Context<Workspace>) -> AnyElement {
    let head = div().w_full().pt(px(12.)).pb(px(28.)).px(px(24.)).flex().flex_col().items_center()
        .child(div().relative().rounded(px(92. * 0.225)).shadow(crate::theme::shadow_button()).child(super::appicon::current_icon(92.))
            // 深色模式下叠一圈细边，图标和黑底之间有个界。
            .when(crate::theme::dark(), |d| d.child(div().absolute().inset_0().rounded(px(92. * 0.225)).border_1().border_color(Theme::line_border()))))
        .child(div().mt(px(16.)).text_size(px(24.)).line_height(px(30.)).font_weight(FontWeight::BOLD).child(name()))
        .child(div().mt(px(2.)).text_size(px(15.)).line_height(px(20.)).text_color(Theme::text_caption()).child(format!("{} {}", tr("版本", "Version"), env!("CARGO_PKG_VERSION"))))
        // 两句各占一行，不让最后一两个字单独掉到第三行。
        .child(div().mt(px(14.)).flex().flex_col().items_center().text_center().text_size(px(15.)).line_height(px(22.)).text_color(Theme::text_caption())
            .child(tr("本地优先的笔记，不用注册账号。", "Local-first notes. No account needed."))
            .child(tr("文档都是普通文件，存在本机或 iCloud 云盘。", "Documents are plain files, on this device or in iCloud Drive.")));
    let privacy = crate::settings::row(tr("隐私", "Privacy"), Some(tr("不需要账号，不收集任何使用数据，也没有广告。",
        "No account, no usage data collected, no ads.").into()), div());
    let licences = div().id("about-licences").w_full().h(px(52.)).px(px(16.)).flex().items_center().gap(px(8.)).rounded(px(10.)).active(|d| d.bg(Theme::fill_pressed()))
        .text_size(px(17.)).text_color(crate::settings::row_ink()).child(div().flex_1().child(tr("开源许可", "Open Source Licences")))
        .child(div().text_size(px(17.)).text_color(Theme::text_caption()).child("MIT"))
        .child(crate::ui::icon("chevron-right", 14., crate::settings::row_chevron()))
        .on_click(cx.listener(|w, _, _, cx| { super::glass::haptic(); w.open_settings(Page::Licences, cx); }));
    div().w_full().flex().flex_col()
        .child(head)
        .child(crate::settings::flat_group(None, vec![privacy.into_any_element(), licences.into_any_element()], 16.))
        .child(div().mt(px(36.)).pb(px(40.)).w_full().text_center().text_size(px(13.)).line_height(px(18.)).text_color(Theme::text_placeholder()).child(copyright()))
        .into_any_element()
}

/// 字标：英文名，全大写。
pub const WORDMARK: &str = "MARGIN";

/// 设置第一级最底下（照 Craft，IMG_9167）：灰色的字标，下面一行版本号。字标只用英文名，
/// GPUI 没有字距，按字拆开用间距排。浅色是 Craft 那一页的实测值（字标 #D6D6D6、版本 #B6B6B5），深色用语义色。
pub fn footer() -> Div {
    let dark = crate::theme::dark();
    let (mark, ver): (Hsla, Hsla) = if dark { (Theme::text_disabled(), Theme::text_placeholder()) } else { (rgb(0xd6d6d6).into(), rgb(0xb6b6b5).into()) };
    let spaced = |s: &str, gap: f32| div().flex().gap(px(gap)).children(s.chars().map(|c| div().child(c.to_string())));
    div().w_full().pt(px(72.)).pb(px(56.)).flex().flex_col().items_center().text_color(mark)
        // 和 Craft 的字标差不多大：约 110 × 20 点。
        .child(spaced(WORDMARK, 3.).text_size(px(22.)).line_height(px(24.)).font_weight(FontWeight::BLACK))
        .child(div().mt(px(10.)).text_size(px(14.)).line_height(px(18.)).text_color(ver).child(format!("{} {}", tr("版本", "Version"), env!("CARGO_PKG_VERSION"))))
}

/// LICENSE 里那一行「Copyright (c) …」，换成 ©。
fn copyright() -> String {
    LICENSE.lines().find_map(|l| l.trim().strip_prefix("Copyright (c)")).map_or_else(String::new, |r| format!("© {}", r.trim()))
}

/// 许可的写法不统一（`MIT/Apache-2.0`、`Apache-2.0 OR MIT`…）：只有「任选其一」的拆开排好再合起来，同一种归到一组。
fn normal(lic: &str) -> String {
    if lic.contains("AND") || lic.contains('(') { return lic.to_string() }
    let mut parts: Vec<&str> = lic.split(" OR ").flat_map(|p| p.split('/')).map(str::trim).filter(|p| !p.is_empty()).collect();
    parts.sort_by_key(|p| match *p { "MIT" => 0, "Apache-2.0" => 1, _ => 2 });
    parts.dedup();
    parts.join(" / ")
}

fn para(text: impl Into<SharedString>) -> Div {
    div().px(px(16.)).text_size(px(13.)).line_height(px(19.)).text_color(Theme::text_caption()).child(text.into())
}

/// 开源许可：本体 → 素材 → 组件（按许可分组，组里是包名）。
pub fn licences(_w: &Workspace, _cx: &mut Context<Workspace>) -> AnyElement {
    let mut groups: Vec<(String, Vec<&str>)> = Vec::new();
    for line in CRATES.lines() {
        let mut f = line.split('\t');
        let (Some(name), _, Some(lic)) = (f.next(), f.next(), f.next()) else { continue };
        let key = normal(lic);
        match groups.iter_mut().find(|g| g.0 == key) {
            Some(g) => { if g.1.last() != Some(&name) { g.1.push(name) } }
            None => groups.push((key, vec![name])),
        }
    }
    groups.sort_by(|a, b| b.1.len().cmp(&a.1.len()).then(a.0.cmp(&b.0)));
    let total: usize = groups.iter().map(|g| g.1.len()).sum();
    // MIT 许可正文：标题行和版权行上面已经说了，这里从「Permission…」起。
    let body = LICENSE.split_once("Permission").map_or(LICENSE.to_string(), |(_, r)| format!("Permission{r}"));
    let assets = [
        (tr("KaTeX 字体", "KaTeX fonts"), "SIL OFL 1.1 / MIT"),
        (tr("Lucide 图标", "Lucide icons"), "ISC"),
        (tr("Nucleo 图标", "Nucleo icons"), tr("商业授权", "Commercial licence")),
    ];
    let packages: Vec<AnyElement> = groups.into_iter().map(|(lic, names)| div().w_full().py(px(12.)).flex().flex_col().gap(px(4.))
        .child(div().px(px(16.)).flex().items_end().gap(px(6.)).text_size(px(15.)).line_height(px(20.)).font_weight(FontWeight::SEMIBOLD)
            .child(lic).child(div().text_size(px(13.)).font_weight(FontWeight::NORMAL).text_color(Theme::text_placeholder()).child(names.len().to_string())))
        // 一个包名一块、不在中间断开（`core-foundation-sys` 不会断在连字符上）。
        .child(div().px(px(16.)).flex().flex_wrap().gap_x(px(5.)).text_size(px(13.)).line_height(px(19.)).text_color(Theme::text_caption())
            .children(names.iter().enumerate().map(|(i, n)| div().whitespace_nowrap().child(if i + 1 < names.len() { format!("{n} ·") } else { n.to_string() }))))
        .into_any_element()).collect();
    div().w_full().flex().flex_col().gap(px(28.)).pb(px(48.))
        .child(crate::settings::flat_group(Some(name()), vec![
            div().w_full().flex().flex_col().gap(px(8.)).pt(px(4.))
                .child(para(tr("Margin 由开源项目 Quiet.md 构建，以 MIT 许可发布。", "Margin is built from the open-source Quiet.md project and released under the MIT License.")))
                .child(div().px(px(16.)).text_size(px(15.)).line_height(px(20.)).child(format!("MIT License · {}", copyright())))
                .children(body.split("\n\n").map(|p| para(p.split_whitespace().collect::<Vec<_>>().join(" "))))
                .into_any_element()], 16.))
        .child(crate::settings::flat_group(Some(tr("素材", "Assets")), assets.into_iter().map(|(a, l)| crate::settings::row(a, None,
            div().text_size(px(15.)).text_color(Theme::text_caption()).child(l)).into_any_element()).collect(), 16.))
        .child(div().w_full().flex().flex_col()
            .child(crate::settings::flat_group(Some(tr("开源组件", "Open Source Components")), Vec::new(), 16.))
            .child(para(crate::localized_format!("这个 App 用到了 {} 个开源 Rust 组件，感谢它们的作者。", "This app is built with {} open-source Rust packages. Thanks to their authors.", total)).pb(px(4.)))
            .child(crate::settings::flat_group(None, packages, 16.)))
        .into_any_element()
}
