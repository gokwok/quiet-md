//! 手机上的编辑表单与目录。原生输入框把改动发给编辑器已有的字段订阅。
use gpui::*;
use crate::{ios::editor_surface::Surface, model::BlockId, settings::{text, tr}};
use super::view::Editor;

impl Editor {
    pub fn expanded_panel(&self) -> bool {
        self.cat_pinned || self.find.is_some() || matches!(self.menu,Some(super::view::Menu::Lang {..} | super::view::Menu::Equation {..}))
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
        panel.document_title(&title,&self.space_label,close);
        let mut tabs = vec![];
        for (i,(name,symbol)) in [("目录","list.bullet"),("任务","checkmark.circle"),("附件","paperclip"),("查找","doc.text.magnifyingglass")].into_iter().enumerate() {
            let mut tab = panel.button(text(name),symbol,move |e,w,cx| {
                if i == 3 { e.cat_pinned = false; e.open_find(false,w,cx); }
                else { if e.find.is_some() { e.close_find(w,cx); } w.blur(cx); crate::ios::editor_surface::hide_keyboard(); e.cat_pinned = true; e.phone_tab = i; cx.notify(); }
            });
            tab["on"] = (i==active).into(); tabs.push(tab);
        }
        panel.styled("tabs",tabs,34.);
    }

    pub(super) fn phone_find(&self, replace: bool, n: usize, _counter: &str, window: &Window, cx: &Context<Self>) -> AnyElement {
        let mut p = Surface::new();
        self.document_header(&mut p,3);
        let mut modes = vec![];
        for (label,to) in [("查找",false),("查找和替换",true)] {
            let mut item = p.button(text(label),"",move |ed,_,cx| if let Some(f) = &mut ed.find { f.replace = to; cx.notify(); });
            item["on"] = (replace==to).into(); modes.push(item);
        }
        let mode = serde_json::json!({"title": text(if replace { "查找和替换" } else { "查找" }),"symbol":"chevron.down","children":modes});
        let mut options = vec![];
        for regex in [false,true] {
            let mut item=p.button(if regex {tr("正则表达式","Regular expression")} else {text("文本")},"",move |e,w,cx| {
                e.search_options[0]=regex; e.refresh_find(true,cx); e.scroll_to_current(w); cx.notify();
            });
            item["on"]=(self.search_options[0]==regex).into(); options.push(item);
        }
        for (i,label) in [(1,tr("区分大小写","Match case")),(3,tr("忽略变音符号","Ignore diacritics")),(2,tr("包括部分匹配","Include partial matches"))] {
            let mut item = p.button(label,"",move |e,w,cx| { e.search_options[i]=!e.search_options[i]; e.refresh_find(true,cx); e.scroll_to_current(w); cx.notify(); });
            item["on"]=(if i==2 {!self.search_options[i]} else {self.search_options[i]}).into();
            item["disabled"]=(i==3 && self.search_options[0]).into(); options.push(item);
        }
        p.line(vec![mode,serde_json::json!({"title":tr("查找选项","Search options"),"symbol":"slider.horizontal.3","iconOnly":true,"trailing":true,"children":options})]);
        p.field(&self.find_field,text("在文档内查找"),false,true,cx);
        if replace { p.field(&self.replace_field,text("替换为"),false,false,cx); }
        let mut actions = vec![serde_json::json!({"title":crate::localized_format!("{} 个结果","{} results",n),"label":true})];
        if replace {
            actions.push(p.button(text("替换"),"",|e,w,cx| e.replace_one(w,cx)));
            actions.push(p.button(text("全部替换"),"",|e,w,cx| e.replace_all(w,cx)));
        }
        for (label,symbol,next) in [("上一处","arrow.up",false),("下一处","arrow.down",true)] {
            let mut item = icon(p.button(text(label),symbol,move |e,w,cx| e.step_find(next,w,cx)));
            if !next { item["trailing"] = true.into(); item["reserve"] = 44.into(); } actions.push(item);
        }
        p.line(actions); p.list();
        if let Some(find) = &self.find {
            if !find.matches.is_empty() { p.caption(&crate::settings::display_title(&self.core.doc.title.text)); }
            for (i,(id,range)) in find.matches.iter().enumerate() {
                let Some(t) = self.core.text_of(*id) else { continue };
                let start = t.text[..range.start].char_indices().rev().nth(22).map_or(0,|(i,_)|i);
                let end = t.text[range.end..].char_indices().nth(50).map_or(t.len(),|(i,_)|range.end+i);
                let mut item = p.button(&t.text[start..end].replace('\n'," "),"text.alignleft",move |e,w,cx| {
                    if let Some(f) = &mut e.find { f.current = Some(i); } e.scroll_to_current(w); e.close_find(w,cx);
                });
                item["on"] = (find.current == Some(i)).into();
                item["match"] = serde_json::json!([t.text[start..range.start].encode_utf16().count(),t.text[range.clone()].encode_utf16().count()]); p.line(vec![item]);
            }
        }
        p.page("find",window,cx,None)
    }

    pub(super) fn phone_catalogue(&self, window: &Window, cx: &Context<Self>) -> AnyElement {
        let mut p = Surface::new(); let tab = self.phone_tab;
        self.document_header(&mut p,tab);
        p.caption(text(match tab { 1 => "任务",2 => "附件",_ => "目录" })); p.list();
        if tab == 0 {
            if self.cat_entries.is_empty() { p.caption(tr("文档中没有标题","No headings in this document")); }
            let mut folded = None;
            for (i,entry) in self.cat_entries.iter().enumerate() {
                if folded.is_some_and(|level|entry.level>level) { continue; }
                let id=entry.id;
                let parent = id!=super::core::TITLE && self.cat_entries.get(i+1).is_some_and(|next|next.level>entry.level);
                let closed = parent && self.cat_collapsed.contains(&id);
                folded = closed.then_some(entry.level);
                let mut row = vec![];
                let mut item = p.button(&entry.text,"",move |e,_,cx| e.jump_to_heading(id,cx));
                item["on"]=(self.cat_active==Some(id)).into();
                if parent {
                    item["fill"]=true.into(); item["reserve"]=44.into(); row.push(item);
                    let action=p.button(text(if closed {"展开"} else {"折叠"}),if closed {"chevron.right"} else {"chevron.down"},move |e,_,cx| {
                        if !e.cat_collapsed.remove(&id) { e.cat_collapsed.insert(id); } cx.notify();
                    });
                    row.push(icon(action));
                } else { row.push(item); }
                p.indented_line(row,entry.level);
            }
        } else {
            let mut count = 0;
            for unit in self.core.units() {
                let id = unit.id; let Some(b) = self.core.block(id) else { continue };
                let row = match &b.kind {
                    crate::model::BlockKind::Todo { done } if tab==1 => Some((b.text.text.as_str(),if *done { "checkmark.circle.fill" } else { "circle" })),
                    crate::model::BlockKind::File(f) if tab==2 => Some((f.name.as_str(),"paperclip")),
                    crate::model::BlockKind::Image { caption,.. } if tab==2 => Some((caption.as_deref().unwrap_or(text("图片")),"photo")),
                    _ => None,
                };
                if let Some((name,symbol)) = row { p.row(name,symbol,move |e,_,cx| e.jump_to_heading(id,cx)); count+=1; }
            }
            if count==0 { p.caption(if tab==1 { tr("没有任务","No tasks") } else { tr("没有附件","No attachments") }); }
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
