//! UIKit 画编辑器浮层。GPUI 给出位置和当前状态；系统控件的动作仍回到原来的 Editor。
use std::{cell::{Cell, RefCell}, collections::HashMap, ffi::{CStr, CString, c_char, c_void}, rc::Rc};

use gpui::*;
use gpui::prelude::FluentBuilder;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use serde_json::{Value, json};

use crate::{editor::view::Editor, input::{TextField, TextFieldEvent}};

unsafe extern "C" {
    fn quiet_editor_surface(parent: *mut c_void, key: *const c_char, model: *const c_char, x: f64, y: f64, w: f64, h: f64, dark: bool);
    fn quiet_hide_keyboard();
    fn quiet_hide_editor_surface(key: *const c_char);
}

type Handler = Rc<dyn Fn(&mut Editor, &str, &mut Window, &mut Context<Editor>)>;
struct Mounted { editor: WeakEntity<Editor>, handlers: Vec<Handler>, target: Option<(u64,usize)>, seen: bool, revision: u64 }
thread_local! { static NEXT: Cell<u64> = const { Cell::new(0) }; static LIVE: RefCell<HashMap<&'static str, Mounted>> = RefCell::new(HashMap::new()); }

pub fn begin(window: &Window) {
    LIVE.with(|m| for panel in m.borrow_mut().values_mut() { panel.seen = false; });
    window.on_next_frame(|_, _| LIVE.with(|m| m.borrow_mut().retain(|key, panel| {
        if !panel.seen { unsafe { quiet_hide_editor_surface(CString::new(*key).unwrap().as_ptr()); } }
        panel.seen
    })));
}

#[unsafe(no_mangle)]
unsafe extern "C" fn quiet_editor_action(key: *const c_char, index: usize, value: *const c_char) {
    let key = unsafe { CStr::from_ptr(key) }.to_string_lossy().into_owned();
    let value = unsafe { CStr::from_ptr(value) }.to_string_lossy().into_owned();
    let target = LIVE.with(|m| { let m = m.borrow(); let panel = m.get(key.as_str())?; Some((panel.editor.clone(), panel.handlers.get(index)?.clone(), panel.revision)) });
    if let Some((editor, handle, revision)) = target { super::menu::dispatch(move |w, window, cx| {
        if !enabled() || !LIVE.with(|m| m.borrow().get(key.as_str()).is_some_and(|p| p.revision == revision)) { return; }
        if let Some(ed) = editor.upgrade() && w.editor.as_ref() == Some(&ed) { ed.update(cx, |ed, cx| handle(ed, &value, window, cx)); }
    }); }
}

pub fn hide_keyboard() { unsafe { quiet_hide_keyboard(); } }

pub fn enabled() -> bool { super::floating() && !super::overlay() && !super::stylepanel::is_open() && !super::format::is_open() }

/// 卡片里两边的留白（同 `EditorSurface.m` 的 `inset`）。
pub const CARD_INSET: f32 = 20.;
/// 卡片最高能有多高：顶上让出状态栏，底下让出键盘。
pub fn card_room(window: &Window) -> Pixels {
    let insets = window.insets();
    let bottom = if insets.ime.bottom > px(0.) { insets.ime.bottom + px(8.) } else { insets.safe_area.bottom.max(px(34.)) };
    window.viewport_size().height - insets.safe_area.top - bottom - px(8.)
}
/// 这张卡片上一帧在不在。
pub fn live(key: &str) -> bool { LIVE.with(|m| m.borrow().contains_key(key)) }

pub struct Surface { rows: Vec<Value>, handlers: Vec<Handler>, target: Option<(u64,usize)>, height: f32 }
impl Surface {
    pub fn height(&self) -> Pixels { px(self.height) }
    pub fn new() -> Self { Self { rows: vec![], handlers: vec![], target: None, height: 12. } }
    pub fn at(block: fdl_core::model::BlockId, offset: usize) -> Self { Self { target: Some((block.0,offset)), ..Self::new() } }
    pub fn action(&mut self, f: impl Fn(&mut Editor, &str, &mut Window, &mut Context<Editor>) + 'static) -> usize {
        self.handlers.push(Rc::new(f)); self.handlers.len() - 1
    }
    pub fn button(&mut self, title: &str, symbol: &str, f: impl Fn(&mut Editor, &mut Window, &mut Context<Editor>) + 'static) -> Value {
        let action = self.action(move |e, _, w, cx| f(e, w, cx));
        json!({"title":title,"symbol":symbol,"action":action})
    }
    pub fn row(&mut self, title: &str, symbol: &str, f: impl Fn(&mut Editor, &mut Window, &mut Context<Editor>) + 'static) {
        let item = self.button(title, symbol, f); self.line(vec![item]);
    }
    pub fn line(&mut self, items: Vec<Value>) {
        self.rows.push(json!({"items":items})); self.height += 44.;
    }
    pub fn indented_line(&mut self, items: Vec<Value>, level: usize) {
        self.rows.push(json!({"items":items,"indent":level.saturating_sub(1)*12})); self.height += 44.;
    }
    pub fn caption(&mut self, title: &str) { self.rows.push(json!({"caption":title})); self.height += 28.; }
    pub fn field(&mut self, field: &Entity<TextField>, title: &str, multiline: bool, focus: bool, cx: &App) {
        let target = field.clone();
        let input = self.action(move |_, text, _, cx| target.update(cx, |f, cx| {
            if f.text != text { f.set_text(text, cx); cx.emit(TextFieldEvent::Changed); }
        }));
        let target = field.clone();
        let command = self.action(move |_, key, _, cx| target.update(cx, |_, cx| cx.emit(match key {
            "escape" => TextFieldEvent::Cancel, "up" => TextFieldEvent::Up, "down" => TextFieldEvent::Down,
            "previous" => TextFieldEvent::ShiftSubmit, "leftEdge" => TextFieldEvent::LeftEdge, "rightEdge" => TextFieldEvent::RightEdge,
            _ => TextFieldEvent::Submit,
        })));
        self.rows.push(json!({"field":title,"value":field.read(cx).text,"input":input,"submit":command,"focus":focus,"multi":multiline}));
        self.height += if multiline { 96. } else { 44. };
    }
    /// 卡片里的一行输入：左边名字，右边输入（地址不自动大写、不纠正）。
    pub fn labelled(&mut self, field: &Entity<TextField>, key: &str, hint: &str, url: bool, focus: bool, rule: bool, cx: &App) {
        self.field(field, key, false, focus, cx); self.height += 8.;
        let width = f32::from(super::menu_w(0.)) - CARD_INSET * 2.;
        if let Some(row) = self.rows.last_mut() { for (k, v) in [("label", json!(key)), ("hint", json!(hint)), ("url", json!(url)), ("rule", json!(rule)), ("width", json!(width)), ("h", json!(52))] { row[k] = v; } }
    }
    /// 卡片顶上的一行：左「取消」、中间标题、右边加粗的「确定」。
    pub fn bar(&mut self, title: &str, cancel: Value, mut ok: Value) { ok["primary"] = true.into(); self.push(json!({"bar":title,"items":[cancel,ok],"h":56})); }
    /// 带状态的系统控件，按名字留着（见 `EditorSurface.m` 的 `control:`）。
    pub fn control(&mut self, name: &str, kind: &str, h: f32, mut row: Value) { row["control"] = name.into(); row["kind"] = kind.into(); row["h"] = h.into(); self.push(row); }
    pub fn header(&mut self, title: &str, close: Value, source: bool) {
        self.rows.push(json!({"header":title,"items":[close],"source":source,"h":56})); self.height += 56.;
    }
    pub fn document_title(&mut self, title: &str, space: &str, close: Value, thumb: Value) {
        self.rows.push(json!({"header":title,"space":space,"items":[close],"thumb":thumb,"h":74})); self.height += 74.;
    }
    pub fn styled(&mut self, style: &str, items: Vec<Value>, height: f32) {
        self.rows.push(json!({"style":style,"items":items,"h":height})); self.height += height;
    }
    pub fn source(&mut self, title: &str, value: &str, language: &str, change: impl Fn(&mut Editor,&str,&mut Window,&mut Context<Editor>) + 'static) {
        let input = self.action(change);
        let mut tokens = vec![];
        let (mut byte,mut utf16) = (0,0);
        for (r,color,_) in crate::highlight::spans(language,value) {
            if r.start<byte { continue; }
            utf16 += value[byte..r.start].encode_utf16().count();
            let n = value[r.clone()].encode_utf16().count();
            let c = Rgba::from(color);
            tokens.push(json!([utf16,n,c.r,c.g,c.b])); utf16 += n; byte = r.end;
        }
        self.rows.push(json!({"field":title,"value":value,"multi":true,"tokens":tokens,"input":input,"focus":true,"stretch":true}));
    }
    pub fn list(&mut self) { self.rows.push(json!({"list":true,"h":0})); }
    /// 自定义行（查找结果卡片、分组标题、空状态、附件缩略图），`h` 写在行里。
    pub fn push(&mut self, row: Value) { self.height += row["h"].as_f64().unwrap_or(44.) as f32; self.rows.push(row); }
    /// 列表开始后还没有任何行。
    pub fn is_empty_list(&self) -> bool { self.rows.last().is_some_and(|r| r["list"] == true) }
    /// 标题行 ✕ 左边再加几个钮。
    pub fn mark_items(&mut self, more: Vec<Value>) { if let Some(items) = self.rows.last_mut().and_then(|r| r["items"].as_array_mut()) { items.extend(more); } }
    /// 给最后一行补一个属性（如查找框的放大镜）。
    pub fn mark(&mut self, key: &str, value: Value) { if let Some(row) = self.rows.last_mut() { row[key] = value; } }
    pub fn page(mut self, key: &'static str, window: &Window, cx: &Context<Editor>, preview: Option<String>) -> AnyElement {
        // 轻点页外沿用同一关闭动作，回到阅读态；滑动不触发关闭。
        if let Some(close) = self.rows[0]["items"][0]["action"].as_u64().and_then(|i| self.handlers.get(i as usize)).cloned() {
            let outside = self.action(move |ed,v,w,cx| { close(ed,v,w,cx); w.blur(cx); hide_keyboard(); });
            self.rows[0]["outside"] = outside.into();
        }
        let top = window.insets().safe_area.top;
        let w = window.viewport_size().width;
        let h = (window.viewport_size().height - top - window.insets().ime.bottom).max(px(100.));
        self.height = h.into();
        // 文档面板由原生视图自己升起、收起，底下不垫 GPUI 底色，免得动画时露出一张静止的纸。
        let sheet = matches!(key, "find" | "catalogue");
        let body = div().relative().w(w).h(h).rounded_t(px(24.)).overflow_hidden().when(!sheet, |d| d.bg(crate::theme::Theme::bg_body()))
            .when_some(preview, |d, src| d.child(canvas(move |_,window,_| crate::editor::math::layout(&src,true,20.,crate::theme::Theme::text(),0,window),
                |b,math,window,cx| if let Some(m) = math { m.paint(point(b.center().x-px(m.width/2.),b.center().y+px((m.ascent-m.depth)/2.)),window,cx); })
                .absolute().top(px(62.)).w_full().h(px(118.))))
            .child(self.element(key,w,h,cx.weak_entity()));
        div().absolute().inset_0().child(deferred(anchored().position_mode(AnchoredPositionMode::Window)
            .anchor(Anchor::TopLeft).position(point(px(0.),top)).child(body)).with_priority(4)).into_any_element()
    }
    /// 设置卡片：底部、左右各 12、离底边 34（键盘起来就停在它上面）；压暗和升降都是原生的（`EditorSurface.m` 的 `cardKey`）。
    pub fn card(mut self, key: &'static str, window: &Window, cx: &Context<Editor>, close: impl Fn(&mut Editor, &mut Window, &mut Context<Editor>) + 'static) -> AnyElement {
        let outside = self.action(move |e, _, w, cx| close(e, w, cx));
        if let Some(row) = self.rows.first_mut() { row["outside"] = outside.into(); }
        self.height += 6.;
        let (insets, view) = (window.insets(), window.viewport_size());
        let bottom = if insets.ime.bottom > px(0.) { insets.ime.bottom + px(8.) } else { insets.safe_area.bottom.max(px(34.)) };
        let body = self.element(key, super::menu_w(0.), card_room(window), cx.weak_entity());
        div().absolute().inset_0().child(deferred(anchored().position_mode(AnchoredPositionMode::Window)
            .anchor(Anchor::BottomCenter).position(point(view.width / 2., view.height - bottom)).child(body)).with_priority(4)).into_any_element()
    }
    pub fn element(self, key: &'static str, width: Pixels, max_height: Pixels, editor: WeakEntity<Editor>) -> AnyElement {
        let height = px(self.height).min(max_height.max(px(44.)));
        let model = CString::new(Value::Array(self.rows).to_string()).unwrap();
        let handlers = self.handlers;
        let target = self.target;
        div().relative().w(width).h(height).occlude().on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .child(canvas(|_, _, _| (), move |b, _, window, _| {
                let Ok(handle) = HasWindowHandle::window_handle(window) else { return };
                let RawWindowHandle::UiKit(handle) = handle.as_raw() else { return };
                let parent = super::shot::overlay(handle.ui_view.as_ptr().cast());
                LIVE.with(|m| {
                    let mut m = m.borrow_mut();
                    if m.get(key).is_some_and(|p| p.editor != editor || p.target != target) {
                        unsafe { quiet_hide_editor_surface(CString::new(key).unwrap().as_ptr()); } m.remove(key);
                    }
                    let panel = m.entry(key).or_insert_with(|| Mounted { editor: editor.clone(), handlers: vec![], target, seen: false,
                        revision: NEXT.with(|n| { n.set(n.get()+1); n.get() }) });
                    panel.seen = true; panel.handlers.clone_from(&handlers);
                });
                unsafe { quiet_editor_surface(parent.cast(), CString::new(key).unwrap().as_ptr(), model.as_ptr(), b.origin.x.into(), b.origin.y.into(), b.size.width.into(), b.size.height.into(), crate::theme::dark()); }
            }).absolute().inset_0()).into_any_element()
    }
}
