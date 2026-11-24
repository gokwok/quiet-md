//! 手机上的编辑表单与目录。原生输入框把改动发给编辑器已有的字段订阅。
use gpui::*;
use crate::{ios::editor_surface::Surface, model::BlockId, settings::{text, tr}};
use super::view::Editor;
#[path = "editor_cards.rs"]
mod cards;

impl Editor {
    pub fn expanded_panel(&self) -> bool {
        self.cat_pinned || self.find.is_some() || matches!(self.menu,Some(super::view::Menu::Lang {..} | super::view::Menu::Equation {..}))
            // 「添加」的命令面板停在顶上，盖着顶栏那排按钮。
            || crate::ios::glass::supported() && matches!(self.menu,Some(super::view::Menu::Block { sub: Some(super::view::BlockSub::Add(_)), .. }))
    }
    pub(super) fn phone_link(&self, block: BlockId, start: usize, only_link: bool, window: &Window, cx: &Context<Self>) -> AnyElement {
        let mut panel = Surface::at(block,start);
        if !only_link { panel.field(&self.link_text_field,text("文本"),false,self.link_text_field.read(cx).is_focused(window),cx); }
        panel.field(&self.link_field,text("链接"),false,self.link_field.read(cx).is_focused(window),cx);
        let mut ok = panel.button(text("确认"),"checkmark",|ed,w,cx| ed.submit_link(w,cx));
        ok["disabled"] = serde_json::json!(!self.link_input_ok(cx));
        panel.line(vec![ok]);
        self.native_sheet("link",panel,window,cx)
    }

    pub(super) fn phone_equation(&self, block: BlockId, offset: usize, window: &Window, cx: &Context<Self>) -> AnyElement {
        let mut panel = Surface::at(block,offset);
        let done = panel.button(tr("完成","Done"),"checkmark",|ed,w,cx| ed.close_equation_editor(true,w,cx));
        panel.header(text("代码"),done,true);
        panel.styled("preview",vec![],118.);
        panel.field(&self.eq_field,"LaTeX",true,true,cx);
        let hide = panel.button(tr("收起键盘","Hide keyboard"),"keyboard.chevron.compact.down",|_,_,_| crate::ios::editor_surface::hide_keyboard());
        panel.line(vec![trailing(hide)]);
        panel.page("equation",window,cx,Some(self.eq_field.read(cx).text.clone()))
    }

    pub(super) fn phone_languages(&self, block: BlockId, current: &str, window: &Window, cx: &Context<Self>) -> AnyElement {
        let mut p = Surface::at(block,0);
        let done = p.button(tr("完成","Done"),"checkmark",|e,w,cx| e.close_menu_focus(w,cx));
        p.header(text("代码"),done,true);
        let value = self.core.text_of(block).map_or("",|t| t.text.as_str());
        p.source(tr("代码","Code"),value,current,move |e,v,w,cx| {
            if !e.core.is_code(block) { return; }
            if let Some(old) = e.core.text_of(block) && old.text != v {
                let before=&old.text;
                let start=before.chars().zip(v.chars()).take_while(|(a,b)|a==b).map(|(c,_)|c.len_utf8()).sum::<usize>();
                let tail=before[start..].chars().rev().zip(v[start..].chars().rev()).take_while(|(a,b)|a==b).map(|(c,_)|c.len_utf8()).sum::<usize>();
                let end=before.len()-tail; e.core.replace_in_block(block,start..end,&v[start..v.len()-tail]);
                e.changed(w,cx); e.autoscroll=false; e.reveal=None;
            }
        });
        let mut choices = vec![];
        for lang in super::menus::filter_languages("") {
            let mut choice = p.button(lang,"",move |e,w,cx| { e.set_lang(block,lang,w,cx); });
            choice["on"] = serde_json::json!(current.eq_ignore_ascii_case(lang)); choices.push(choice);
        }
        let language = serde_json::json!({"title":current,"symbol":"chevron.up.chevron.down","children":choices});
        let copy = p.button(text("复制"),"doc.on.doc",move |e,_,cx| { if let Some(t) = e.core.text_of(block) { cx.write_to_clipboard(ClipboardItem::new_string(t.text.clone())); } });
        let hide = p.button(tr("收起键盘","Hide keyboard"),"keyboard.chevron.compact.down",|_,_,_| crate::ios::editor_surface::hide_keyboard());
        let mut copy = trailing(copy); copy["reserve"] = 44.into();
        p.line(vec![language,copy,icon(hide)]);
        p.page("language",window,cx,None)
    }

    fn document_header(&self, panel: &mut Surface, active: usize) {
        let title = crate::settings::display_title(&self.core.doc.title.text);
        let close = panel.button(tr("关闭","Close"),"xmark",|e,w,cx| {
            if e.find.is_some() { e.close_find(w,cx); } e.cat_pinned = false; cx.notify();
        });
        panel.document_title(&title,&crate::localized_format!("在 {} 中","In {}",self.space_label),close,mini_page(&self.core.doc,30.));
        let mut tabs = vec![];
        for (i,(name,symbol)) in [("目录","list.bullet"),("任务","checkmark.circle"),("附件","paperclip"),("查找","doc.text.magnifyingglass")].into_iter().enumerate() {
            let mut tab = panel.button(text(name),symbol,move |e,w,cx| {
                if i == 3 { e.cat_pinned = false; e.open_find(false,w,cx); }
                else { if e.find.is_some() { e.close_find(w,cx); } w.blur(cx); crate::ios::editor_surface::hide_keyboard(); e.cat_pinned = true; e.phone_tab = i; cx.notify(); }
            });
            tab["on"] = (i==active).into(); tabs.push(tab);
        }
        panel.styled("tabs",tabs,48.);
    }

    pub(super) fn phone_find(&self, replace: bool, n: usize, _counter: &str, window: &Window, cx: &Context<Self>) -> AnyElement {
        let mut p = Surface::new();
        self.document_header(&mut p,3);
        let mut modes = vec![];
        for (label,to) in [("查找",false),("查找和替换",true)] {
            let mut item = p.button(text(label),"",move |ed,_,cx| if let Some(f) = &mut ed.find { f.replace = to; cx.notify(); });
            item["on"] = (replace==to).into(); modes.push(item);
        }
        let mode = serde_json::json!({"title": text(if replace { "查找和替换" } else { "查找" }),"symbol":"chevron.down","menuTitle":true,"children":modes});
        // 选项菜单分两段：匹配方式，然后是各个开关。
        let mut options = vec![];
        for regex in [false,true] {
            let mut item=p.button(if regex {tr("正则表达式","Regular expression")} else {text("文本")},"",move |e,w,cx| {
                e.search_options[0]=regex; e.refresh_find(true,cx); e.scroll_to_current(w); cx.notify();
            });
            item["on"]=(self.search_options[0]==regex).into(); item["group"]=0.into(); options.push(item);
        }
        for (i,label) in [(1,tr("区分大小写","Match case")),(3,tr("忽略变音符号","Ignore diacritics")),(2,tr("包括部分匹配","Include partial matches"))] {
            let mut item = p.button(label,"",move |e,w,cx| { e.search_options[i]=!e.search_options[i]; e.refresh_find(true,cx); e.scroll_to_current(w); cx.notify(); });
            item["on"]=(if i==2 {!self.search_options[i]} else {self.search_options[i]}).into();
            item["disabled"]=(i==3 && self.search_options[0]).into(); item["group"]=1.into(); options.push(item);
        }
        p.line(vec![mode,serde_json::json!({"title":tr("查找选项","Search options"),"symbol":"slider.horizontal.3","iconOnly":true,"trailing":true,"muted":true,"children":options})]);
        p.field(&self.find_field,text("在文档内查找"),false,true,cx); p.mark("search",true.into());
        if replace { p.field(&self.replace_field,text("替换为"),false,false,cx); }
        let count = if n == 0 { tr("暂无结果","No results").to_owned() } else { crate::localized_format!("{} 个结果","{} results",n) };
        let mut actions = vec![serde_json::json!({"title":count,"label":true})];
        if replace {
            actions.push(p.button(text("替换"),"",|e,w,cx| e.replace_one(w,cx)));
            actions.push(p.button(text("全部替换"),"",|e,w,cx| e.replace_all(w,cx)));
        }
        for (label,symbol,next) in [("上一处","arrow.up",false),("下一处","arrow.down",true)] {
            let mut item = icon(p.button(text(label),symbol,move |e,w,cx| e.step_find(next,w,cx)));
            item["muted"] = true.into(); item["disabled"] = (n==0).into();
            if !next { item["trailing"] = true.into(); item["reserve"] = 44.into(); } actions.push(item);
        }
        p.line(actions); p.list();
        if let Some(find) = &self.find {
            // 结果按所在的标题分组：标题本身命中时就落在自己那一组。
            let headings: std::collections::HashMap<_,_> = self.cat_entries.iter().map(|e|(e.id,e.text.to_string())).collect();
            let mut under = std::collections::HashMap::new();
            let mut current = crate::settings::display_title(&self.core.doc.title.text).to_string();
            for unit in self.core.units() { if let Some(t) = headings.get(&unit.id) { current = t.clone(); } under.insert(unit.id,current.clone()); }
            let mut group: Option<String> = None;
            for (i,(id,range)) in find.matches.iter().enumerate() {
                let Some(t) = self.core.text_of(*id) else { continue };
                let heading = under.get(id).cloned().or_else(|| group.clone()).unwrap_or_default();
                if group.as_ref() != Some(&heading) { p.push(serde_json::json!({"group":heading,"h":if group.is_none() {34} else {44}})); group = Some(heading); }
                let mut start = t.text[..range.start].char_indices().rev().nth(11).map_or(0,|(i,_)|i);
                // 截在单词中间时退到下一个空格之后，不留半个词。
                if start > 0 && let Some(gap) = t.text[start..range.start].find(' ') { start += gap+1; }
                let end = t.text[range.end..].char_indices().nth(80).map_or(t.len(),|(i,_)|range.end+i);
                let lead = if start > 0 { "…" } else { "" };
                let snippet = format!("{lead}{}",t.text[start..end].replace('\n'," "));
                let mut item = p.button(&snippet,"text.alignleft",move |e,w,cx| {
                    if let Some(f) = &mut e.find { f.current = Some(i); } e.scroll_to_current(w); e.close_find(w,cx);
                });
                item["snippet"] = snippet.clone().into();
                item["on"] = (find.current == Some(i)).into();
                item["match"] = serde_json::json!([lead.encode_utf16().count()+t.text[start..range.start].encode_utf16().count(),t.text[range.clone()].encode_utf16().count()]);
                p.push(item);
            }
        }
        p.page("find",window,cx,None)
    }

    pub(super) fn phone_catalogue(&self, window: &Window, cx: &Context<Self>) -> AnyElement {
        let mut p = Surface::new(); let tab = self.phone_tab;
        self.document_header(&mut p,tab);
        p.caption(text(match tab { 1 => "任务",2 => "附件",_ => "目录" })); p.list();
        let empty = |p: &mut Surface, title: &str, symbol: &str| p.push(serde_json::json!({"empty":title,"symbol":symbol}));
        if tab == 0 {
            if self.cat_entries.is_empty() { empty(&mut p,tr("文档中的标题将出现在此处。","Headings in this document will appear here."),"list.bullet"); }
            let mut folded = None;
            for (i,entry) in self.cat_entries.iter().enumerate() {
                if folded.is_some_and(|level|entry.level>level) { continue; }
                let id=entry.id;
                let parent = id!=super::core::TITLE && self.cat_entries.get(i+1).is_some_and(|next|next.level>entry.level);
                let closed = parent && self.cat_collapsed.contains(&id);
                folded = closed.then_some(entry.level);
                let mut row = vec![];
                let mut item = p.button(&entry.text,"",move |e,_,cx| e.jump_to_heading(id,cx));
                item["on"]=(self.cat_active==Some(id)).into(); item["outline"]=true.into();
                if parent {
                    item["fill"]=true.into(); item["reserve"]=44.into(); row.push(item);
                    let mut action=icon(p.button(text(if closed {"展开"} else {"折叠"}),if closed {"chevron.right"} else {"chevron.down"},move |e,_,cx| {
                        if !e.cat_collapsed.remove(&id) { e.cat_collapsed.insert(id); } cx.notify();
                    }));
                    action["minor"]=true.into(); row.push(action);
                } else { row.push(item); }
                p.push(serde_json::json!({"items":row,"indent":entry.level.saturating_sub(1)*16,"h":42}));
            }
        } else {
            let (mut tasks,mut media,mut files) = (vec![],vec![],vec![]);
            for unit in self.core.units() {
                let id = unit.id; let Some(b) = self.core.block(id) else { continue };
                match &b.kind {
                    crate::model::BlockKind::Todo { done } if tab==1 => {
                        let mut item = p.button(&b.text.text,if *done { "checkmark.circle.fill" } else { "circle" },move |e,_,cx| e.jump_to_heading(id,cx));
                        item["outline"]=true.into(); item["done"]=(*done).into(); tasks.push(item);
                    }
                    crate::model::BlockKind::File(f) if tab==2 => {
                        let mut item = p.button(&f.name,"doc",move |e,_,cx| e.jump_to_heading(id,cx)); item["outline"]=true.into(); files.push(item);
                    }
                    crate::model::BlockKind::Image { src,caption,.. } if tab==2 => {
                        let mut item = p.button(caption.as_deref().unwrap_or(text("图片")),"photo",move |e,_,cx| e.jump_to_heading(id,cx));
                        if let Some(path) = fdl_core::store::image_path(src).filter(|p|p.is_file()) { item["path"] = path.to_string_lossy().into_owned().into(); }
                        media.push(item);
                    }
                    _ => {}
                }
            }
            for item in tasks.drain(..) { p.push(serde_json::json!({"items":[item],"h":42})); }
            if !media.is_empty() {
                p.push(serde_json::json!({"caption":tr("媒体","Media"),"h":30}));
                for pair in media.chunks(2) { p.push(serde_json::json!({"media":pair})); }
            }
            if !files.is_empty() {
                p.push(serde_json::json!({"caption":tr("文件","Files"),"h":30}));
                for item in files { p.push(serde_json::json!({"items":[item],"h":42})); }
            }
            if p.is_empty_list() {
                if tab==1 { empty(&mut p,tr("本文档中的任务将出现在此处。","Tasks in this document will appear here."),"checklist"); }
                else { empty(&mut p,tr("本文档中的图片和文件将出现在此处。","Images and files in this document will appear here."),"paperclip"); }
            }
        }
        p.page("catalogue",window,cx,None)
    }
}

fn icon(mut item:serde_json::Value) -> serde_json::Value { item["iconOnly"] = true.into(); item }
fn trailing(item:serde_json::Value) -> serde_json::Value { let mut item=icon(item); item["trailing"]=true.into(); item }

/// TextKit uses UTF-16; the editor keeps UTF-8 byte ranges. Normalized matches
/// always map back to complete graphemes before highlight or replacement.
pub(super) struct SearchMatcher { pattern: Option<regex::Regex>, fold: bool }
impl SearchMatcher {
    pub(super) fn new(query:&str,options:[bool;4]) -> Option<Self> {
        if options==[false;4] { return None; }
        let [regex,case,whole,accents]=options;
        let fold=accents && !regex;
        let pattern=if regex { query.to_owned() } else if fold { regex::escape(&unaccent(query)) } else { regex::escape(query) };
        let pattern=if whole { format!(r"\b(?:{pattern})\b") } else { pattern };
        Some(Self { pattern:regex::RegexBuilder::new(&pattern).case_insensitive(!case).build().ok(),fold })
    }
    pub(super) fn ranges(&self,t:&crate::model::RichText) -> Vec<std::ops::Range<usize>> {
        use unicode_segmentation::UnicodeSegmentation;
        let Some(re)=&self.pattern else { return vec![] };
        let mut normalized=String::new(); let mut bounds=vec![];
        let hay=if self.fold {
            for (byte,g) in t.text.grapheme_indices(true) {
                let stripped=unaccent(g); if stripped.is_empty() { continue; }
                bounds.push((normalized.len(),byte)); normalized.push_str(&stripped);
            }
            bounds.push((normalized.len(),t.len())); &normalized
        } else { &t.text };
        let objects:Vec<_>=t.spans().filter_map(|(r,m)| m.inline.is_some().then_some(r)).collect();
        re.find_iter(hay).filter(|m|!m.is_empty()).filter_map(|m| {
            let byte=|i| bounds.binary_search_by_key(&i,|b|b.0).ok().map(|n|bounds[n].1);
            let r=if self.fold { byte(m.start())?..byte(m.end())? } else { m.range() };
            (!objects.iter().any(|obj|obj.start<r.end && r.start<obj.end)).then_some(r)
        }).collect()
    }
}
fn unaccent(s:&str) -> String {
    use unicode_normalization::{UnicodeNormalization,char::is_combining_mark};
    s.nfd().filter(|c|!is_combining_mark(*c)).collect()
}

/// 文档面板左上角的小纸：和主页列表行首同一套缩略图（`Workspace::mini_thumb`），照它的排法算成一组矩形交给原生层画。
/// 纸色和字色取这一帧的主题（文档有样式时已盖上）。`w` 是纸宽，高 1.2 倍。
pub(super) fn mini_page(doc: &crate::model::Document, w: f32) -> serde_json::Value {
    use crate::model::BlockKind;
    use serde_json::json;
    let hex = crate::ios::style::to_hex;
    let (paper, ink) = (crate::editor_theme::Theme::bg_body(), crate::editor_theme::Theme::text());
    let h = (w * 1.2).round();
    let (pad_x, pad_top) = (w * 0.11, w * 0.13);
    const PER_LINE: f32 = 40.;
    let unit = (w / 24.).max(0.75);
    let room = h - pad_top;
    let width_of = |t: &str| t.chars().map(|c| if c.is_ascii() { 1. } else { 2. }).sum::<f32>();
    let mut out: Vec<serde_json::Value> = vec![];
    let rect = |x: f32, y: f32, w: f32, h: f32, a: f32, r: f32| json!({"x":x,"y":y,"w":w,"h":h,"a":a,"r":r});
    let bar = |x: f32, y: f32, full: f32, frac: f32, a: f32, t: f32| rect(x, y, full * frac.clamp(0.15, 1.), t, a, t / 2.);
    // 一段字排成几道线，最后一道按余下的字数短一截。
    let lines = |out: &mut Vec<serde_json::Value>, x: f32, y: f32, full: f32, text: &str, max: usize, a: f32, t: f32, gap: f32| -> f32 {
        let n = width_of(text.trim());
        let count = ((n / PER_LINE).ceil() as usize).clamp(1, max);
        let last = if n <= 0. { 0.35 } else { let r = (n / PER_LINE).fract(); if (n / PER_LINE) >= max as f32 || r == 0. { 1. } else { r.max(0.2) } };
        for i in 0..count { out.push(bar(x, y + i as f32 * (t + gap), full, if i + 1 == count { last } else { 1. }, a, t)); }
        count as f32 * t + (count as f32 - 1.) * gap
    };
    let mut used = 0f32;
    // 放不下的最后一块照样放（纸边切掉），放进去以后就停。返回这一块的顶。
    let slot = |used: &mut f32, gap: f32| -> Option<f32> { if *used + gap >= room - 1. { return None; } let y = pad_top + if *used == 0. { 0. } else { *used + gap }; Some(y) };
    let title = crate::settings::display_title(&doc.title.text);
    if !title.trim().is_empty() {
        let frac = (width_of(&title) / (PER_LINE * 0.6)).clamp(0.3, 0.9);
        out.push(bar(pad_x, pad_top, w - pad_x * 2., frac, 0.7, unit * 2.)); used = unit * 2.;
    }
    if let Some(path) = doc.cover.as_ref().and_then(|c| crate::store::image_path(&c.src)) && let Some(y) = slot(&mut used, unit * 1.5) {
        let ph = (w - pad_x * 2.) / 1.75;
        let mut r = rect(pad_x, y, w - pad_x * 2., ph, 1., 1.5); r["img"] = path.to_string_lossy().into_owned().into(); out.push(r); used = y - pad_top + ph;
    }
    fn walk(blocks: &[crate::model::Block], depth: usize, out: &mut Vec<(usize, crate::model::Block)>) {
        for b in blocks { out.push((depth, crate::model::Block { children: vec![], ..b.clone() })); if !b.folded && !matches!(b.kind, BlockKind::Table(_)) { walk(&b.children, depth + 1, out); } }
    }
    let mut flat = vec![]; walk(&doc.blocks, 0, &mut flat);
    let gap = unit * 1.6;
    for (depth, b) in &flat {
        if matches!(b.kind, BlockKind::Grid | BlockKind::Column { .. } | BlockKind::SubPageList) { continue; }
        let Some(y) = slot(&mut used, gap) else { break };
        let indent = (*depth as f32 * unit * 2.).min(w * 0.3);
        let (x, full) = (pad_x + indent, w - pad_x * 2. - indent);
        let text = b.text.text.as_str();
        let tall = match &b.kind {
            BlockKind::Heading { level, .. } => {
                let t = if *level <= 1 { unit * 1.9 } else if *level == 2 { unit * 1.6 } else { unit * 1.3 };
                out.push(bar(x, y, full, (width_of(text) / (PER_LINE * if *level <= 2 { 0.55 } else { 0.8 })).clamp(0.25, 1.), 0.6, t)); t
            }
            BlockKind::Bullet | BlockKind::Ordered { .. } | BlockKind::Todo { .. } => {
                let m = if matches!(b.kind, BlockKind::Todo { .. }) { unit * 1.6 } else { unit * 1.2 };
                let mut mark = rect(x, y + (unit - m) / 2., m, m, 0.45, if m > unit * 1.4 { unit * 0.4 } else { m / 2. });
                if matches!(b.kind, BlockKind::Todo { .. }) { mark["a"] = 0.4.into(); mark["stroke"] = (unit * 0.45).into(); }
                out.push(mark);
                lines(&mut out, x + m + unit, y, full - m - unit, text, 2, 0.28, unit, unit * 0.9)
            }
            BlockKind::Quote => { let t = lines(&mut out, x + unit * 1.6, y, full - unit * 1.6, text, 3, 0.28, unit, unit * 0.9); out.push(rect(x, y, unit * 0.6, t, 0.35, 0.)); t }
            BlockKind::Code { .. } | BlockKind::Callout { .. } => {
                let code = matches!(b.kind, BlockKind::Code { .. });
                let t = if code { unit * 6. } else { unit * 5. };
                let mut bg = rect(x, y, full, t, 0.07, unit);
                if let BlockKind::Callout { bg: Some(c), .. } = &b.kind && let Some(f) = crate::theme::callout_bg(&c.0) { bg["c"] = hex(f).into(); bg["a"] = 1.into(); }
                out.push(bg);
                let inner = full - unit * 2.;
                let (a, b2) = if code { (0.6, 0.8) } else { (0.8, 0.5) };
                out.push(bar(x + unit, y + unit, inner, a, 0.25, unit * 0.8)); out.push(bar(x + unit, y + unit * 2.6, inner, b2, 0.25, unit * 0.8)); t
            }
            BlockKind::Divider { .. } => { out.push(rect(x, y, full, unit * 0.5, 0.15, 0.)); unit * 0.5 }
            BlockKind::Image { src, width, height, .. } => {
                let ratio = match (width, height) { (Some(iw), Some(ih)) if *iw > 0. => (ih / iw).clamp(0.3, 1.2), _ => 0.66 };
                let t = (full * ratio).min(room * 0.6);
                let mut r = rect(x, y, full, t, 0.1, 1.5);
                if let Some(p) = crate::store::image_path(src) { r["img"] = p.to_string_lossy().into_owned().into(); }
                out.push(r); t
            }
            BlockKind::Table(table) => {
                let (rows, cols, rh, line) = (table.rows.len().clamp(1, 4), table.col_widths.len().clamp(1, 4), unit * 2.2, unit * 0.35);
                let t = rows as f32 * rh;
                for r in 0..rows.min(table.header_rows) { out.push(rect(x, y + r as f32 * rh, full, rh, 0.06, 0.)); }
                for r in 1..rows { out.push(rect(x, y + r as f32 * rh, full, line, 0.18, 0.)); }
                for c in 1..cols { out.push(rect(x + full * c as f32 / cols as f32, y, line, t, 0.18, 0.)); }
                let mut frame = rect(x, y, full, t, 0.18, unit * 0.5); frame["stroke"] = line.into(); out.push(frame); t
            }
            _ => if text.trim().is_empty() { unit } else { lines(&mut out, x, y, full, text, 4, 0.25, unit, unit * 0.9) },
        };
        used = y - pad_top + tall;
    }
    // 什么都没写：留一道淡淡的字行。
    if used == 0. { out.push(bar(pad_x, pad_top, w - pad_x * 2., 0.6, 0.2, unit)); }
    json!({"w":w,"h":h,"paper":hex(paper),"ink":hex(ink),"parts":out,"icon":doc.icon})
}
