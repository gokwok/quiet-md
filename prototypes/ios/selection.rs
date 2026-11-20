//! 表格单元格的原生横向工具栏。
use gpui::*;
use serde_json::{Value, json};
use fdl_core::{editor::table_ops::{CellSel, CellSelKind}, model::{Align, BlockKind, ColorName, Marks}};
use crate::{ios::editor_surface::Surface, settings::{text, tr}, feishu_theme as theme};
use super::{core::Selection, menus, view::{Editor, Mark, Menu}};

fn tool(mut item: Value, on: bool) -> Value { item["iconOnly"] = json!(true); item["on"] = json!(on); item }
fn menu(title: &str, symbol: &str, children: Vec<Value>) -> Value {
    json!({"title":title,"symbol":symbol,"children":children})
}

impl Editor {
    pub(super) fn phone_cell_selection(&self, sel: CellSel, window: &Window, cx: &Context<Self>) -> AnyElement {
        let mut p = Surface::new();
        let block = self.core.cell_text_blocks().first().copied();
        let Some((table,range)) = self.core.cell_range() else { return div().into_any_element() };
        let page = self.table_page;
        if page == 0 {
            let mut tabs = vec![];
            for (dest,label,symbol) in [(1,tr("编辑表格","Edit table"),"tablecells"),(2,tr("选择单元格","Select cells"),"rectangle.dashed"),(3,text("格式"),"textformat")] {
                let mut item = p.button(label,symbol,move |e,_,cx| { e.table_page=dest; cx.notify(); });
                item["stack"]=true.into(); item["equal"]=true.into(); tabs.push(item);
            }
            p.styled("tableTabs",tabs,80.);
            p.row(tr("添加内容","Add content"),"plus.circle",|e,w,cx| {
                e.core.cell_sel=None; e.close_menu_focus(w,cx); w.blur(cx); w.focus(&e.focus,cx);
            });
            p.row(tr("插入公式","Insert equation"),"function",|e,w,cx| { e.core.cell_sel=None; e.run_slash(menus::SlashKind::Formula,w,cx); });
            let mut actions = vec![];
            let merged = self.core.merged_selected().is_some();
            let mut merge = p.button(text(if merged {"拆分单元格"} else {"合并单元格"}),"rectangle.split.2x1",|e,w,cx| {
                if e.core.merged_selected().is_some() { e.core.table_unmerge(); } else { e.core.table_merge(); } e.changed(w,cx);
            });
            merge["disabled"] = (!merged && self.core.selected_cells().len()<2).into(); actions.push(merge);
            let action = menu(tr("动作","Actions"),"command",actions);
            p.line(vec![action]);
            if let Some(t) = self.core.table(table) {
                let sortable = t.rows.len().saturating_sub(t.header_rows)>1 && t.cells().all(|(_,_,c)|c.rowspan<=1);
                let mut sorts=vec![];
                for (desc,label) in [(false,tr("升序","Ascending")),(true,tr("降序","Descending"))] {
                    sorts.push(p.button(label,"",move |e,w,cx| { if e.core.table_sort_rows(table,sel.anchor.1,desc) { e.changed(w,cx); } }));
                }
                let mut sort=menu(tr("排序","Sort"),"arrow.up.arrow.down",sorts); sort["disabled"]=(!sortable).into(); p.line(vec![sort]);
            }
            let header = vec![p.button(text("标题行"),"",move |e,w,cx| { e.core.table_toggle_header_row(table); e.changed(w,cx); }),
                p.button(text("标题列"),"",move |e,w,cx| { e.core.table_toggle_header_col(table); e.changed(w,cx); })];
            p.line(vec![menu(text("标题"),"rectangle.topthird.inset.filled",header)]);
            p.row(tr("重置列大小","Reset column widths"),"",move |e,w,cx| { e.core.table_distribute(table); e.changed(w,cx); });
            p.row(tr("清除单元格样式","Clear cell style"),"",|e,w,cx| {
                e.core.table_clear_style(); e.changed(w,cx);
            });
            let mut clear = p.button(tr("清除单元格内容","Clear cell contents"),"",|e,w,cx| { e.core.table_clear_cells(); e.changed(w,cx); });
            clear["danger"]=true.into(); p.line(vec![clear]);
        } else {
            let back = p.button(tr("返回","Back"),"xmark",|e,_,cx| { e.table_page=0; e.menu=None; cx.notify(); });
            p.header(match page { 1=>tr("编辑表格","Edit table"),2=>tr("选择单元格","Select cells"),_=>text("格式") },back,false);
        }
        if page==1 {
            for (cols,label) in [(false,tr("插入行","Insert row")),(true,tr("插入列","Insert column"))] {
                p.row(label,"plus.square",move |e,w,cx| e.table_insert_at(table,cols,if cols {range.right+1} else {range.bottom+1},w,cx));
            }
            for (cols,forward,label,symbol) in [(false,false,tr("向上移动行","Move row up"),"arrow.up"),(false,true,tr("向下移动行","Move row down"),"arrow.down"),
                (true,false,tr("向左移动列","Move column left"),"arrow.left"),(true,true,tr("向右移动列","Move column right"),"arrow.right")] {
                let (lo,hi,limit) = if cols { (range.left,range.right,self.core.table(table).unwrap().ncols()) } else { (range.top,range.bottom,self.core.table(table).unwrap().rows.len()) };
                let mut item = p.button(label,symbol,move |e,w,cx| {
                    e.core.select_cells(CellSel{kind:if cols {CellSelKind::Cols} else {CellSelKind::Rows},..sel});
                    if e.core.table_move_selected(if forward { hi+2 } else { lo.saturating_sub(1) }) { e.changed(w,cx); }
                }); item["disabled"]=(if forward { hi+1>=limit } else { lo==0 }).into(); p.line(vec![item]);
            }
            for (kind,label) in [(CellSelKind::Rows,text("删除行")),(CellSelKind::Cols,text("删除列"))] {
                let mut item=p.button(label,"trash",move |e,w,cx| { e.core.select_cells(CellSel{kind,..sel}); e.core.table_delete_selected(); e.changed(w,cx); });
                item["danger"]=true.into(); p.line(vec![item]);
            }
        }
        if page==2 {
            for (kind,label) in [(CellSelKind::Range,text("单元格")),(CellSelKind::Rows,text("行")),(CellSelKind::Cols,text("列"))] {
                p.row(label,if kind==sel.kind {"checkmark"} else {""},move |e,_,cx| { e.core.select_cells(CellSel{kind,..sel}); cx.notify(); });
            }
            p.row(text("完成"),"checkmark",|e,_,cx| { e.core.cell_sel=None; cx.notify(); });
        }
        if page==3 && self.menu.is_none() {
            let mut marks=vec![];
            for (mark,label,symbol) in [(Mark::Bold,"粗体","bold"),(Mark::Italic,"斜体","italic"),(Mark::Strike,"删除线","strikethrough"),(Mark::Code,"代码","chevron.left.forwardslash.chevron.right"),(Mark::Underline,"下划线","underline")] {
                let active=self.mark_active(match mark { Mark::Bold=>|m|m.bold,Mark::Italic=>|m|m.italic,Mark::Underline=>|m|m.underline,Mark::Strike=>|m|m.strike,Mark::Code=>|m|m.code });
                let mut item=tool(p.button(text(label),symbol,move |e,w,cx| e.toggle_mark(mark,w,cx)),active); item["equal"]=true.into(); marks.push(item);
            }
            p.line(marks);
            let mut align=vec![];
            for (a,label,symbol) in [(Align::Left,"左对齐","text.alignleft"),(Align::Center,"居中对齐","text.aligncenter"),(Align::Right,"右对齐","text.alignright")] {
                let mut item=tool(p.button(text(label),symbol,move |e,w,cx| e.align(a,w,cx)),self.current_align()==a); item["equal"]=true.into(); align.push(item);
            }
            let mut vertical=vec![]; for (v,label) in [(None,"顶部对齐"),(Some("middle"),"垂直居中"),(Some("bottom"),"底部对齐")] {
                vertical.push(p.button(text(label),"",move |e,w,cx| {e.core.table_set_valign(v);e.changed(w,cx);}));
            }
            let mut other=menu(text("垂直居中"),"arrow.up.and.down",vertical); other["equal"]=true.into(); other["iconOnly"]=true.into(); align.push(other);p.line(align);
            let mut fonts=vec![]; for (name,key) in [(tr("默认字体","Default font"),None),("Serif",Some("serif")),("Mono",Some("mono")),("Round",Some("rounded"))] {
                fonts.push(p.button(name,"",move |e,w,cx| { let ids=e.core.cell_text_blocks(); e.core.set_font(&ids,key.map(str::to_string)); e.changed(w,cx); }));
            }
            p.line(vec![menu(tr("字体","Font"),"textformat",fonts)]);
            let mut kinds=vec![]; for it in menus::TURN_ITEMS { let k=(it.kind)(); if matches!(k,BlockKind::Callout{..}) {continue;}
                let mut item=p.button(text(it.label),"",move |e,w,cx| e.turn(k.clone(),false,w,cx));
                item["on"]=block.and_then(|b|self.core.kind_of(b)).is_some_and(|kind|menus::same_kind(kind,&(it.kind)())).into(); kinds.push(item); }
            p.line(vec![menu(tr("文本样式","Text style"),"textformat.size",kinds)]);
            let mut bands=vec![];
            for (name,label) in [(None,tr("无","None")),(Some("light-gray"),tr("灰色","Gray")),(Some("light-blue"),tr("蓝色","Blue")),(Some("light-green"),tr("绿色","Green")),(Some("light-yellow"),tr("黄色","Yellow"))] {
                bands.push(p.button(label,"",move |e,w,cx| { e.core.table_band_rows(table,name.map(|n|ColorName(n.into()))); e.changed(w,cx); }));
            }
            p.line(vec![menu(tr("隔行填色","Alternate row fill"),"circle.lefthalf.filled",bands)]);
            p.row(tr("单元格填充颜色","Cell fill color"),"paintpalette",|e,_,cx| {e.menu=Some(Menu::CellBg);cx.notify();});
            p.row(text("文本颜色"),"textformat",|e,_,cx| {e.menu=Some(Menu::Color);cx.notify();});
        }
        if matches!(self.menu,Some(Menu::Color | Menu::CellBg)) {
            let cell_bg = matches!(self.menu,Some(Menu::CellBg));
            let (ink,bg) = self.current_colors();
            if !cell_bg {
                p.caption(text("字体颜色"));
                let mut dots = vec![];
                for &(name,_) in &theme::FONT_COLORS {
                    let on = ink.as_ref().map(|n|n.0.as_str()) == name;
                    let item = p.button(name.unwrap_or(text("默认")),"",move |e,w,cx| e.apply_color(Some(name.filter(|_| !on).map(|n|ColorName(n.into()))),None,w,cx));
                    dots.push(dot(item,Some(name.and_then(theme::text_color).unwrap_or_else(theme::Theme::text)),on));
                }
                p.line(dots);
            }
            p.caption(text(if cell_bg { "单元格背景颜色" } else { "背景颜色" }));
            let bg = if cell_bg { self.core.first_selected_cell().and_then(|c|c.bg.clone()) } else { bg };
            let names: &[Option<&str>] = if cell_bg { &theme::CELL_BG_COLORS } else { &theme::BG_COLORS };
            for group in names.chunks(8) {
                let mut dots = vec![];
                for &name in group {
                    let on = bg.as_ref().map(|c|theme::bg_canon(&c.0)) == name;
                    let item = p.button(name.unwrap_or(text("默认")),"",move |e,w,cx| {
                        let value = name.map(|n|ColorName(n.into()));
                        if cell_bg { e.core.table_set_bg(value); e.changed(w,cx); } else { e.apply_color(None,Some(value.filter(|_| !on)),w,cx); }
                        e.menu = None; cx.notify();
                    });
                    dots.push(dot(item,name.and_then(if cell_bg {theme::bg_swatch} else {theme::bg_color}),on));
                }
                p.line(dots);
            }
            let reset = p.button(text("恢复默认"),"",move |e,w,cx| { if cell_bg { e.core.table_set_bg(None); e.changed(w,cx); } else { e.apply_color(Some(None),Some(None),w,cx); } e.menu = None; cx.notify(); });
            let done = p.button(tr("完成","Done"),"",|e,_,cx| { e.menu = None; cx.notify(); }); p.line(vec![reset,done]);
        }

        // 换面板或选区时把当前格露在面板上方。之后仍能自由滚动正文。
        let top = crate::ios::sheet_point().y - p.height().min(crate::ios::visible_height(window)-px(crate::ios::doc_top()+24.));
        if self.table_panel_target.get()!=Some((sel,top)) && let Some((_,r,_))=self.caret_cell()
            && let Some(bounds)=self.tables.geo.borrow().get(&(table,false,r)).copied() {
            self.table_panel_target.set(Some((sel,top)));
            let delta = bounds.bottom()-top+px(10.);
            if delta>px(0.) {
                let ed=cx.weak_entity(); window.on_next_frame(move |_,cx| { let _=ed.update(cx,|e,cx| {
                    if e.core.cell_sel==Some(sel) {e.list.scroll_by(delta);cx.notify();}
                }); });
            }
        }
        self.native_sheet("selection",p,window,cx)
    }
    pub(super) fn native_sheet(&self,key:&'static str,p:Surface,window:&Window,cx:&Context<Self>) -> AnyElement {
        let h = crate::ios::visible_height(window) - px(crate::ios::doc_top()+24.);
        super::overlays::at(self.rel(crate::ios::sheet_point()),Anchor::BottomCenter,p.element(key,crate::ios::menu_w(0.),h,cx.weak_entity()))
    }
}
fn dot(item:Value, color:Option<Hsla>, on:bool) -> Value {
    let mut item = tool(item,on);
    item["swatch"] = color.map(Rgba::from).map_or_else(|| json!([]), |c| json!([c.r,c.g,c.b,c.a])); item
}

pub(super) struct RangeDrag { original: Selection, offset: Point<Pixels>, keep_keyboard: bool, double_tap_at: Option<std::time::Instant> }

impl Editor {
    pub(super) fn range_drag_move(&mut self, ev: &TouchDragEvent, w: &Window, cx: &mut Context<Self>) -> bool {
        let Some(drag) = &mut self.range_drag else { return false };
        // GPUI iOS 会在分发触摸前把向下拖动当作收键盘。手柄拖动期间保留原输入状态。
        if drag.keep_keyboard && self.focus.is_focused(w) { w.request_virtual_keyboard(); }
        let tuning = GestureTuning::default();
        if (ev.position - ev.start_position).magnitude() > f64::from(tuning.touch_slop) { drag.double_tap_at = None; }
        if ev.phase == TouchPhase::Ended && drag.double_tap_at.is_some_and(|t| t.elapsed() <= tuning.multi_tap_interval) {
            self.range_drag = None;
            self.select_paragraph();
            cx.notify();
            return true;
        }
        let before = self.core.sel;
        if ev.phase == TouchPhase::Cancelled {
            self.core.sel = drag.original;
        } else {
            let endpoint = ev.position + drag.offset;
            self.drag_text_endpoint(endpoint);
        }
        if ev.phase != TouchPhase::Moved {
            self.range_drag = None;
            self.touch_menu = !self.core.sel.is_collapsed();
        }
        if ev.phase != TouchPhase::Moved || self.core.sel != before { cx.notify(); }
        true
    }

    pub(super) fn phone_selection_handles(&self,window:&Window,cx:&Context<Self>) -> Vec<AnyElement> {
        if !crate::ios::editor_surface::enabled() || !self.selection_bar() || self.menu.is_some() || !self.focus.is_focused(window) { return vec![]; }
        let (start,end)=self.core.ordered();
        let mut handles=vec![];
        for (first,pos,fixed) in [(true,start,end),(false,end,start)] {
            let Some((p,line))=self.point_of(pos) else {continue};
            if p.y < px(crate::ios::doc_top()) || p.y+line>crate::ios::visible_height(window) {continue;}
            let tint=crate::theme::Theme::primary();
            let editor = cx.entity();
            let touch = canvas(|b,w,_| w.insert_hitbox(b, HitboxBehavior::Normal), move |_,hit,w,_| {
                let editor = editor.clone();
                w.on_mouse_event(move |ev: &TouchDragEvent, phase, w, cx| {
                    if phase != DispatchPhase::Bubble || ev.phase != TouchPhase::Started || !hit.is_hovered_at(ev.start_position,w) { return; }
                    editor.update(cx, |e,cx| {
                        if !e.selection_bar() || !e.focus.is_focused(w) { return; }
                        let offset = point(p.x, p.y + line/2.) - ev.start_position;
                        // 短词上的第三下可能落在手柄里。只有在连点范围内抬手，且没有拖动，才选中整段。
                        let tuning = GestureTuning::default();
                        let double_tap_at = e.touch_menu_delay.take().filter(|(t, p)| t.elapsed() <= tuning.multi_tap_interval
                            && (ev.start_position - *p).magnitude() <= f64::from(tuning.multi_tap_slop)).map(|(t, _)| t);
                        e.range_drag = Some(RangeDrag { original:e.core.sel, offset, keep_keyboard:crate::ios::keyboard_up(w), double_tap_at });
                        e.core.sel = Selection { anchor:fixed, head:pos };
                        e.touch_menu = false;
                        w.prevent_default(); cx.stop_propagation(); cx.notify();
                    });
                });
            }).absolute().inset_0();
            let handle=div().id(if first {"range-start"} else {"range-end"}).relative().w(px(24.)).h(line+px(10.))
                .child(div().absolute().left(px(11.)).top(if first {px(8.)} else {px(0.)}).w(px(2.)).h(line).bg(tint))
                .child(div().absolute().left(px(7.)).top(if first {px(0.)} else {line}).size(px(10.)).rounded_full().bg(tint))
                .child(touch)
                .on_mouse_down(MouseButton::Left,cx.listener(move |e,ev:&MouseDownEvent,w,cx| {
                    cx.stop_propagation(); w.prevent_default();
                    // 短词两端的拖柄几乎盖住整个词：双击选词后的第三下多半落在拖柄上，照样是选中整段。
                    if ev.click_count>=3 { e.select_paragraph(); return cx.notify(); }
                    e.core.sel.anchor=fixed; e.core.sel.head=pos;
                    e.touch_menu=false; e.dragging=true; cx.notify();
                }));
            handles.push(super::overlays::at(self.rel(point(p.x-px(12.),p.y-if first {px(8.)} else {px(0.)})),Anchor::TopLeft,handle));
        }
        handles
    }
    pub(super) fn phone_text_tools(&self,window:&Window,cx:&Context<Self>) -> AnyElement {
        use gpui::prelude::FluentBuilder;
        use crate::editor_theme::Theme;
        let width = window.viewport_size().width;
        let button = |id: &'static str,glyph: &'static str,on:bool| div().id(id).flex_none().w(width / 8.).h(px(44.)).flex().items_center().justify_center()
            .active(|s| s.bg(Theme::fill_pressed())).child(crate::editor_ui::icon(glyph,19.,if on { Theme::primary() } else { Theme::text() }));
        let more = |glyph,id,m:Menu| button(id,glyph,false).on_mouse_down(MouseButton::Left,cx.listener(move |ed:&mut Editor,_,w,cx| {
            w.prevent_default(); cx.stop_propagation(); ed.menu = Some(m.clone()); cx.notify();
        }));
        let mut bar = div().id("phone-text-tools").w(width).h(px(44.)).flex().items_center().overflow_x_scroll().restrict_scroll_to_axis().bg(Theme::bg_body());
        bar = bar.when(!self.markdown,|d| d.child(more("pen-line","text-color",Menu::Color)));
        let marks: &[(&str,&str,Mark,fn(&Marks)->bool)] = &[("text-bold","bold",Mark::Bold,|m|m.bold),("text-italic","italic",Mark::Italic,|m|m.italic),
            ("text-strike","strikethrough",Mark::Strike,|m|m.strike),("text-code","code",Mark::Code,|m|m.code)];
        for &(id,glyph,mark,active) in marks {
            bar = bar.child(button(id,glyph,self.mark_active(active)).on_mouse_down(MouseButton::Left,cx.listener(move |ed,_,w,cx| {
                w.prevent_default(); cx.stop_propagation(); ed.toggle_mark(mark,w,cx);
            })));
        }
        bar = bar.child(button("text-link","link",false).on_mouse_down(MouseButton::Left,cx.listener(|ed,_,w,cx| {
            w.prevent_default(); cx.stop_propagation(); ed.link_from_selection(w,cx);
        }))).child(button("text-math","sigma",false).on_mouse_down(MouseButton::Left,cx.listener(|ed,_,w,cx| {
            w.prevent_default(); cx.stop_propagation(); if ed.core.is_code(ed.core.sel.head.block) { return; }
            let source = ed.core.selected_text();
            if let Some(p) = ed.core.insert_equation(&source) { ed.changed(w,cx); ed.open_equation_editor(p,w,cx); }
        }))).child(button("text-hide","keyboard",false).on_mouse_down(MouseButton::Left,cx.listener(|_:&mut Editor,_,w,cx| {
            cx.stop_propagation(); w.blur(cx);
        }))).child(more("type","text-turn",Menu::TurnInto))
            .child(button("text-underline","underline",self.mark_active(|m|m.underline)).on_mouse_down(MouseButton::Left,cx.listener(|ed,_,w,cx| {
                w.prevent_default(); cx.stop_propagation(); ed.toggle_mark(Mark::Underline,w,cx);
            }))).child(more("layout-grid","text-more",Menu::TbMore));
        let bottom = window.viewport_size().height-crate::ios::safe(window).bottom;
        div().absolute().child(deferred(anchored().position_mode(AnchoredPositionMode::Window).position(point(width/2.,bottom)).anchor(Anchor::BottomCenter).child(bar)).with_priority(2)).into_any_element()
    }
}
