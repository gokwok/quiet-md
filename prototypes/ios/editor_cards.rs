//! 手机上编辑器里的设置卡片和小工具条：表格尺寸、按钮、日期提醒、倒计时、编号、表情、@ 文档、图片和附件的工具条。
//! 都是原生玻璃（`EditorSurface.m`、`cmdmenu.rs`），改动仍走编辑器原来的方法；宽屏和没有玻璃的系统照旧是桌面那套面板。
use std::{cell::Cell, path::PathBuf};
use gpui::*;
use serde_json::{Value, json};
use crate::{ios::{cmdmenu::{self, Extra, Kind, Row}, editor_surface::{self, CARD_INSET, Surface}}, model::{Align, BlockId, FileView, InlineReminder}, settings::{self, text, tr}};
use crate::editor::{button, core::Pos, countdown, emoji, emoji_picker::{self, EmojiTarget}, link, menus::{self, SlashKind}, numbering, overlays::at, reminder, touch::command_row, view::{Editor, Menu}};

// 打开卡片时键盘在不在：在的话收起来，关掉卡片再还给正文。
thread_local! { static TYPING: Cell<Option<bool>> = const { Cell::new(None) }; }

fn zone() -> i64 { crate::time::local_offset() }
fn locale() -> &'static str { if settings::english() { "en_US" } else { "zh_CN" } }
/// 一行在卡片里有多宽。
fn inner() -> f32 { f32::from(crate::ios::menu_w(0.)) - CARD_INSET * 2. }

impl Editor {
    /// 卡片第一次出现：记下键盘在不在，正文的键盘先收起（卡片里要打字的，输入框自己再要）。
    fn card_opens(&self, key: &'static str, window: &Window, cx: &Context<Self>) {
        if editor_surface::live(key) { return; }
        TYPING.with(|t| t.set(Some(crate::ios::keyboard_shown())));
        if !self.focus.is_focused(window) { return; }
        let editor = cx.entity().downgrade();
        window.on_next_frame(move |window, cx| { let _ = editor.update(cx, |e, cx| if e.focus.is_focused(window) { window.blur(cx); editor_surface::hide_keyboard(); }); });
    }
    /// 关掉一个面板之后焦点归谁。桌面和没有玻璃时回到正文；卡片关掉时，只有打开前正在打字才回去。
    pub(in crate::editor) fn phone_refocus(&self, window: &mut Window, cx: &mut Context<Self>) {
        if !editor_surface::enabled() { return window.focus(&self.focus, cx); }
        match TYPING.with(|t| t.take()) {
            Some(true) => window.focus(&self.focus, cx),
            Some(false) => { window.blur(cx); editor_surface::hide_keyboard(); }
            None => {}
        }
    }

    /// 插入表格：9 × 9 的格子，划到哪格就是几行几列。
    pub(in crate::editor) fn phone_table_size(&self, window: &Window, cx: &Context<Self>) -> AnyElement {
        let mut p = Surface::new();
        let pick = p.action(|e, v, w, cx| {
            let Some((r, c)) = v.split_once(',').and_then(|(r, c)| Some((r.parse().ok()?, c.parse().ok()?))) else { return };
            e.run_slash_sized(SlashKind::Table, Some((r, c)), w, cx);
        });
        p.control("grid", "size", 48. + inner() + 10., json!({"title":text("插入支持富文本的表格"),"rows":9,"cols":9,"action":pick}));
        self.card_opens("size", window, cx);
        p.card("size", window, cx, |e, _, cx| { e.menu = None; cx.notify(); })
    }

    /// 按钮设置：文字、颜色、执行操作、超链接地址；左「取消」右「确定」，填得不对「确定」就点不了。
    pub(in crate::editor) fn phone_button(&self, panel: &button::Panel, window: &Window, cx: &Context<Self>) -> AnyElement {
        let mut p = Surface::at(panel.at.block, panel.at.offset);
        let link = panel.preview.action == crate::model::ButtonAction::OpenLink;
        let cancel = p.button(text("取消"), "", |e, w, cx| e.close_button_panel(w, cx));
        let mut ok = p.button(text("确定"), "", |e, w, cx| e.submit_button(w, cx));
        let bad = button::text_error(&self.btn.text.read(cx).text).is_some() || link && !link::is_link_input(self.btn.url.read(cx).text.trim());
        ok["disabled"] = bad.into();
        p.bar(tr("按钮设置", "Button"), cancel, ok);
        // 先点开的是按钮文字；焦点在哪个框由原生那边自己管，只在刚出现时给一次。
        p.labelled(&self.btn.text, text("按钮文字"), text("请输入按钮"), false, true, true, cx);
        p.mark("select", true.into());
        let colors: Vec<&str> = button::COLORS.iter().flatten().copied().collect();
        let on = colors.iter().position(|c| *c == button::normalize_color(&panel.preview.color));
        let pick = p.action(|e, v, _, cx| {
            let Some(name) = v.parse::<usize>().ok().and_then(|i| button::COLORS.iter().flatten().nth(i)) else { return };
            if let Some(p) = &mut e.btn.panel { p.preview.color = (*name).into(); }
            cx.notify();
        });
        let swatches: Vec<Value> = colors.iter().map(|c| { let c = Rgba::from(button::palette(c).bg); json!([c.r, c.g, c.b, c.a]) }).collect();
        p.push(json!({"swatches":swatches,"on":on,"action":pick,"title":text("按钮颜色"),"rule":true,"h":104}));
        let mut children = vec![];
        for act in button::ACTIONS {
            let mut item = p.button(text(act.label()), "", move |e, w, cx| e.set_button_action(act, w, cx));
            item["on"] = (act == panel.preview.action).into(); children.push(item);
        }
        p.push(json!({"title":text("执行操作"),"value":text(panel.preview.action.label()),"children":children,"rule":link,"h":52}));
        if link { p.labelled(&self.btn.url, text("超链接地址"), text("请输入链接地址"), true, false, false, cx); }
        self.card_opens("button", window, cx);
        p.card("button", window, cx, |e, w, cx| e.close_button_panel(w, cx))
    }

    /// 日期提醒：系统日历，下面是「设置时间」和「提醒」两行开关；改了就生效。
    pub(in crate::editor) fn phone_reminder(&self, r: &InlineReminder, window: &Window, cx: &Context<Self>) -> AnyElement {
        let mut p = match &self.rm.panel { Some(panel) => Surface::at(panel.at.block, panel.at.offset), None => Surface::new() };
        let close = p.button(tr("关闭", "Close"), "xmark", |e, w, cx| e.close_reminder(w, cx));
        p.header(text("日期提醒"), close, false);
        let seconds = |v: &str| v.parse::<i64>().ok().map(|s| s * 1000);
        let day = p.action(move |e, v, w, cx| if let Some(t) = seconds(v) { e.apply_reminder(|r| reminder::on_day(r, reminder::day_of(t)), w, cx) });
        p.control("day", "date", 330., json!({"value":r.expire / 1000,"zone":zone(),"locale":locale(),"action":day,"rule":true}));
        let toggle = p.action(|e, v, w, cx| { let on = v == "1"; e.apply_reminder(|r| reminder::toggle(r, !on, r.should_notify, reminder::now()), w, cx) });
        let clock = p.action(move |e, v, w, cx| if let Some(t) = seconds(v) { e.apply_reminder(|r| reminder::at_time(r, (t - reminder::day_start(t)) / 60_000 * 60_000), w, cx) });
        let mut row = json!({"title":text("设置时间"),"on":!r.whole_day,"toggle":toggle,"action":clock,"zone":zone(),"locale":locale(),"rule":true});
        if !r.whole_day { row["time"] = (r.expire / 1000).into(); }
        p.control("time", "switch", 52., row);
        let toggle = p.action(|e, v, w, cx| { let on = v == "1"; e.apply_reminder(|r| reminder::toggle(r, r.whole_day, on, reminder::now()), w, cx) });
        let mut row = json!({"title":text("提醒"),"on":r.should_notify,"toggle":toggle});
        if r.should_notify {
            let lag = r.notify - r.expire;
            let children: Vec<Value> = reminder::jet_lags(r.whole_day, lag).into_iter().map(|(label, v)| {
                let mut item = p.button(label, "", move |e, w, cx| e.apply_reminder(|r| reminder::with_jet(r, v), w, cx));
                item["on"] = (v == lag).into(); item
            }).collect();
            row["children"] = children.into(); row["value"] = reminder::jet_label(r.whole_day, lag).into();
        }
        p.control("notify", "switch", 52., row);
        self.card_opens("reminder", window, cx);
        p.card("reminder", window, cx, |e, w, cx| e.close_reminder(w, cx))
    }

    /// 倒计时设置：时长（四个滚轮）或具体日期（系统日历带时间），结束时要不要气泡提醒。
    pub(in crate::editor) fn phone_countdown(&self, d: &countdown::Dialog, window: &Window, cx: &Context<Self>) -> AnyElement {
        let mut p = Surface::new();
        let cancel = p.button(text("取消"), "", |e, w, cx| e.close_countdown(w, cx));
        let mut ok = p.button(text("确定"), "", |e, w, cx| e.confirm_countdown(w, cx));
        let end = crate::time::local_to_unix(d.date.0, d.date.1, d.date.2, d.time.0, d.time.1);
        ok["disabled"] = (d.by_date && end < crate::time::now() as i64).into();
        p.bar(text("倒计时设置"), cancel, ok);
        let items: Vec<Value> = [(false, text("输入倒计时时长")), (true, text("选择具体日期"))].into_iter().map(|(by, title)| {
            let mut item = p.button(title, "", move |e, _, cx| e.cd_set_mode(by, cx));
            item["on"] = (by == d.by_date).into(); item
        }).collect();
        p.styled("tabs", items, 44.);
        if d.by_date {
            let pick = p.action(|e, v, _, cx| {
                let Ok(t) = v.parse::<i64>() else { return };
                let (y, m, day, h, mi) = crate::time::civil(t.max(0) as u64);
                if let Some(d) = &mut e.cd.dialog { d.date = (y, m, day); d.time = (h, mi); }
                cx.notify();
            });
            p.control("end", "date", 380., json!({"value":end,"clock":true,"zone":zone(),"locale":locale(),"action":pick,"rule":true}));
        } else {
            let pick = p.action(|e, v, _, cx| {
                for (f, v) in e.cd.fields.clone().iter().zip(v.split(',')) { f.update(cx, |f, cx| if f.text != v { f.set_text(v, cx) }); }
                cx.notify();
            });
            let value: Vec<u32> = (0..4).map(|i| self.cd_value(i, cx)).collect();
            let units: Vec<&str> = countdown::UNITS.iter().map(|u| text(u)).collect();
            p.control("length", "wheels", 196., json!({"value":value,"max":countdown::MAX,"units":units,"action":pick,"rule":true}));
        }
        let toggle = p.action(|e, v, _, cx| { if let Some(d) = &mut e.cd.dialog { d.notify = v == "1"; } cx.notify(); });
        p.control("bubble", "switch", 52., json!({"title":text("倒计时结束时显示气泡提醒"),"on":d.notify,"toggle":toggle}));
        self.card_opens("countdown", window, cx);
        p.card("countdown", window, cx, |e, w, cx| e.close_countdown(w, cx))
    }

    /// 编号：先是跟着序号的一张小菜单，「修改编号值」再开一张带输入框的卡片。
    pub(in crate::editor) fn phone_number(&self, (number, previous): (u32, Option<u32>), pos: Point<Pixels>, edit: bool, depth: usize, window: &Window, cx: &Context<Self>) -> AnyElement {
        if edit {
            let mut p = Surface::new();
            let cancel = p.button(text("取消"), "", |e, w, cx| e.close_menu_focus(w, cx));
            let mut ok = p.button(text("确定"), "", |e, w, cx| e.submit_ordered_number(w, cx));
            ok["disabled"] = numbering::parse_number(&self.number_field.read(cx).text, depth).is_none().into();
            p.bar(text("修改编号值"), cancel, ok);
            p.labelled(&self.number_field, text("新编号为"), "", false, true, false, cx);
            p.mark("select", true.into()); p.mark("plain", true.into());
            self.card_opens("number", window, cx);
            return p.card("number", window, cx, |e, w, cx| e.close_menu_focus(w, cx));
        }
        let rows = [text("继续之前的编号"), text("开始新列表"), text("修改编号值")].into_iter().enumerate().map(|(i, label)| {
            let enabled = match i { 0 => previous.is_some_and(|n| n != number), 1 => number != 1, _ => true };
            let row = command_row(if enabled { Kind::Pick } else { Kind::Off }, "", label, format!("num:{i}"), false);
            if enabled { row } else { Row { color: crate::editor_theme::Theme::text_disabled(), ..row } }
        }).collect();
        let look = cmdmenu::Look { page: "number".into(), rows, filter: false };
        let h = cmdmenu::height(&look);
        // 底下放不下就翻到序号上面。
        let top = pos.y + self.content_origin.get().y;
        let up = f32::from(top) + h + 8. > f32::from(crate::ios::visible_height(window));
        let card = div().w(px(200.)).h(px(h)).child(cmdmenu::slot(look));
        if up { at(point(pos.x, pos.y - px(24.)), Anchor::BottomLeft, card) } else { at(point(pos.x, pos.y + px(4.)), Anchor::TopLeft, card) }
    }
    pub(in crate::editor) fn phone_number_pick(&mut self, i: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Menu::OrderedNumber { block, pos, depth, .. }) = self.menu.clone() else { return };
        let Some((number, previous)) = self.core.ordered_number_options(block) else { return };
        if i == 2 {
            self.number_field.update(cx, |f, cx| { f.set_text(crate::editor::core::ordered_marker(number, depth).trim_end_matches('.'), cx); f.select_all_text(cx); });
            self.menu = Some(Menu::OrderedNumber { block, pos, edit: true, depth });
        } else {
            self.core.set_ordered_number(block, if i == 0 { previous.unwrap_or(1) } else { 1 }, i == 0);
            self.close_menu_focus(window, cx);
            self.changed(window, cx);
        }
        cx.notify();
    }

    /// 表情：搜索框、分组网格和底下一排分类；文档图标多一个「移除」。
    pub(in crate::editor) fn phone_emoji(&self, target: EmojiTarget, window: &Window, cx: &Context<Self>) -> AnyElement {
        let mut p = Surface::new();
        let close = p.button(tr("关闭", "Close"), "xmark", |e, w, cx| e.close_menu_focus(w, cx));
        p.header(text("表情符号"), close, false);
        let random = p.button(text("随机"), "shuffle", move |e, w, cx| e.pick_emoji(target, emoji::random_pick(), false, w, cx));
        let mut more = vec![random];
        if target == EmojiTarget::DocIcon { more.push(p.button(text("移除"), "", |e, w, cx| { e.close_menu_focus(w, cx); e.pick_doc_icon(None, cx); })); }
        p.mark_items(more);
        p.field(&self.emoji_field, text("搜索"), false, false, cx);
        p.mark("plain", true.into());
        let query = self.emoji_query(cx);
        let sections: Vec<Value> = emoji_picker::sections(&query).iter().map(|s| json!({"title":s.title,"items":s.emojis.iter().map(|e| e.char).collect::<Vec<_>>()})).collect();
        let pick = p.action(move |e, v, w, cx| {
            let Some((s, i)) = v.split_once(',').and_then(|(s, i)| Some((s.parse::<usize>().ok()?, i.parse::<usize>().ok()?))) else { return };
            let found = emoji_picker::sections(&e.emoji_query(cx)).get(s).and_then(|s| s.emojis.get(i).copied());
            if let Some(found) = found { e.pick_emoji(target, found, true, w, cx); }
        });
        let h = (f32::from(editor_surface::card_room(window)) - 56. - 44. - 24.).min(360.);
        let symbols = ["clock", "face.smiling", "leaf", "fork.knife", "soccerball", "car", "lightbulb", "number"];
        p.control("grid", "emoji", h, json!({"sections":sections,"symbols":symbols,"action":pick}));
        self.card_opens("emoji", window, cx);
        p.card("emoji", window, cx, |e, w, cx| e.close_menu_focus(w, cx))
    }

    /// @ 文档：跟着光标的一张列表，同「/」那张（焦点留在正文）。
    pub(in crate::editor) fn phone_mention(&self, selected: usize, trigger: Pos, window: &Window, cx: &Context<Self>) -> Option<AnyElement> {
        let (p, lh) = self.point_of(trigger)?;
        if !self.focus.is_focused(window) {
            let editor = cx.entity().downgrade();
            window.on_next_frame(move |window, cx| { let _ = editor.update(cx, |e, cx| if !e.focus.is_focused(window) && e.mention.take().is_some() { cx.notify(); }); });
            return None;
        }
        let q = self.mention_query().to_lowercase();
        let mut rows: Vec<Row> = self.mention_results().into_iter().enumerate().map(|(i, d)| {
            let title = settings::display_title(&d.title).to_string();
            let lower = title.to_lowercase();
            let mark = (!q.is_empty()).then(|| lower.find(&q)).flatten().filter(|&a| title.is_char_boundary(a) && title.is_char_boundary(a + q.len()) && title.len() == lower.len())
                .map(|a| (title[..a].encode_utf16().count(), title[a..a + q.len()].encode_utf16().count()));
            let extra = Extra { emoji: d.icon.clone(), mark, detail: crate::ui::format_ago(d.modified) };
            Row { hot: i == selected, extra, ..command_row(Kind::Pick, if d.icon.is_some() { "" } else { "doc" }, title, format!("doc:{i}"), false) }
        }).collect();
        if rows.is_empty() { rows.push(command_row(Kind::Note, "", text("没有匹配结果，换个关键词试试吧"), String::new(), false)); }
        let look = cmdmenu::Look { page: "mention".into(), rows, filter: false };
        let view = self.list.viewport_bounds();
        let vh = f32::from(crate::ios::visible_height(window));
        let top = f32::from(view.top()) + 4. + crate::ios::doc_top();
        if f32::from(p.y + lh) < top || f32::from(p.y) > vh { return None; }
        let (below, above) = (vh - f32::from(p.y + lh) - 14., f32::from(p.y) - top - 6.);
        let down = below >= 180. || below >= above;
        let w = 300f32.min(f32::from(view.size.width) - 32.);
        let x = f32::from(p.x).clamp(f32::from(view.left()) + 16., f32::from(view.right()) - 16. - w);
        let (full, max) = (cmdmenu::height(&look), if down { below } else { above }.clamp(120., 300.));
        let h = if full <= max { full } else { ((max - cmdmenu::PAD) / cmdmenu::ROW - 0.5).floor().max(0.) * cmdmenu::ROW + cmdmenu::ROW * 0.5 + cmdmenu::PAD };
        let card = div().w(px(w)).h(px(h)).child(cmdmenu::slot(look));
        Some(if down { at(self.rel(point(px(x), p.y + lh + px(6.))), Anchor::TopLeft, card) } else { at(self.rel(point(px(x), p.y - px(6.))), Anchor::BottomLeft, card) })
    }

    /// 选中文字时工具条上的「颜色」「类型」「更多」：贴在工具条上面的玻璃，键盘留着。
    pub(in crate::editor) fn phone_text_drop(&self, menu: &Menu, kind: &crate::model::BlockKind, window: &Window, cx: &Context<Self>) -> AnyElement {
        let bottom = window.viewport_size().height - crate::ios::safe(window).bottom - px(44. + 8.);
        let width = window.viewport_size().width;
        let room = f32::from(bottom) - f32::from(self.list.viewport_bounds().top()) - crate::ios::doc_top() - 8.;
        let place = |p: Point<Pixels>, anchor: Anchor, el: AnyElement| deferred(anchored().position_mode(AnchoredPositionMode::Window).position(p).anchor(anchor).child(el)).with_priority(3).into_any_element();
        if matches!(menu, Menu::Color) {
            let mut p = Surface::new();
            self.color_rows(&mut p);
            let h = p.height().min(px(room));
            return place(point(width / 2., bottom), Anchor::BottomCenter, p.element("textcolor", crate::ios::menu_w(0.), h, cx.weak_entity()));
        }
        let rows = if matches!(menu, Menu::TbMore) {
            vec![command_row(Kind::Pick, "search", text("文档库内搜索"), "find:0".into(), false), command_row(Kind::Pick, "globe", text("网页搜索"), "find:1".into(), false)]
        } else {
            menus::TURN_ITEMS.iter().enumerate().filter(|(_, it)| !(self.markdown && matches!((it.kind)(), crate::model::BlockKind::Callout { .. })))
                .map(|(i, it)| Row { hot: menus::same_kind(kind, &(it.kind)()), ..command_row(Kind::Pick, it.icon, text(it.label), format!("turn:{i}"), false) }).collect()
        };
        let look = cmdmenu::Look { page: if matches!(menu, Menu::TbMore) { "textmore".into() } else { "textturn".into() }, rows, filter: false };
        let (full, max) = (cmdmenu::height(&look), room.clamp(120., 300.));
        // 露出半行，看得出还能滚。
        let h = if full <= max { full } else { ((max - cmdmenu::PAD) / cmdmenu::ROW - 0.5).floor() * cmdmenu::ROW + cmdmenu::ROW * 0.5 + cmdmenu::PAD };
        place(point(width - px(12.), bottom), Anchor::BottomRight, div().w(px(220.)).h(px(h)).child(cmdmenu::slot(look)).into_any_element())
    }

    /// 图片上面的工具条：裁剪、重置、描述、对齐。
    #[allow(clippy::too_many_arguments)]
    pub(in crate::editor) fn phone_image_bar(&self, id: BlockId, rect: Bounds<Pixels>, ok: bool, reset: bool, small: bool, cropping: bool, captioned: bool, align: Align, window: &Window, cx: &Context<Self>) -> AnyElement {
        let mut p = Surface::at(id, 0);
        let mut items = vec![];
        if ok {
            let mut crop = p.button(if cropping { text("保存裁剪") } else { text("裁剪") }, "crop", move |e, w, cx| if cropping { e.save_crop(true, w, cx) } else { e.start_crop(id, w, cx) });
            crop["on"] = cropping.into(); crop["disabled"] = small.into(); items.push(crop);
            if reset { items.push(p.button(text("重置图片"), "arrow.counterclockwise", move |e, _, cx| e.reset_crop(id, cx))); }
        }
        items.push(p.button(if captioned { text("编辑描述") } else { text("添加描述") }, "text.below.photo", move |e, w, cx| e.edit_caption(id, w, cx)));
        for (symbol, title, a) in [("text.alignleft", text("左对齐"), Align::Left), ("text.aligncenter", text("居中对齐"), Align::Center), ("text.alignright", text("右对齐"), Align::Right)] {
            let mut item = p.button(title, symbol, move |e, w, cx| { e.core.set_align(&[id], a); e.changed(w, cx); });
            item["on"] = (align == a).into(); items.push(item);
        }
        self.floating_bar("imagebar", p, items, rect, window, cx)
    }
    /// 附件上面的工具条：全屏、下载、三种视图。`preview`：能预览的文件，值是能不能全屏看。
    #[allow(clippy::too_many_arguments)]
    pub(in crate::editor) fn phone_file_bar(&self, id: BlockId, rect: Bounds<Pixels>, preview: Option<bool>, view: FileView, path: Option<PathBuf>, window: &Window, cx: &Context<Self>) -> AnyElement {
        let mut p = Surface::at(id, 0);
        let mut items = vec![];
        if preview == Some(true) { items.push(p.button(text("全屏"), "arrow.up.left.and.arrow.down.right", move |e, _, cx| e.open_file(path.as_deref(), cx))); }
        items.push(p.button(text("下载"), "square.and.arrow.down", move |e, _, cx| if let Some(crate::model::BlockKind::File(f)) = e.core.kind_of(id).cloned() { e.download_file(&f, cx); }));
        items.push(p.button(text("文本视图"), "text.alignleft", move |e, w, cx| { e.core.file_to_inline(id); e.changed(w, cx); }));
        for (title, symbol, v) in [(text("卡片视图"), "rectangle", FileView::Card), (text("预览视图"), "doc.richtext", FileView::Preview)] {
            if v == FileView::Preview && preview.is_none() { continue; }
            let mut item = p.button(title, symbol, move |e, w, cx| { e.core.set_file_view(id, v); e.changed(w, cx); });
            item["on"] = (view == v || preview.is_none()).into(); items.push(item);
        }
        self.floating_bar("filebar", p, items, rect, window, cx)
    }
    /// 贴着块上沿的一条玻璃：上面放不下就放到下面；块滚出正文区就不出。
    fn floating_bar(&self, key: &'static str, mut p: Surface, items: Vec<Value>, rect: Bounds<Pixels>, window: &Window, cx: &Context<Self>) -> AnyElement {
        let w = items.len() as f32 * 44. + 20.;
        p.styled("bar", items.into_iter().map(|mut v| { v["iconOnly"] = true.into(); v }).collect(), 36.);
        let h = f32::from(p.height());
        let view = self.list.viewport_bounds();
        let (top, bottom) = (f32::from(view.top()) + crate::ios::doc_top(), f32::from(crate::ios::visible_height(window)) - 8.);
        let above = f32::from(rect.top()) - 8. - h;
        let y = if above >= top { above } else { f32::from(rect.bottom()) + 8. };
        if y < top || y + h > bottom { return Empty.into_any_element(); }
        let x = (f32::from(rect.center().x) - w / 2.).clamp(f32::from(view.left()) + 8., (f32::from(view.right()) - w - 8.).max(f32::from(view.left()) + 8.));
        at(self.rel(point(px(x), px(y))), Anchor::TopLeft, p.element(key, px(w), px(h), cx.weak_entity()))
    }
}
