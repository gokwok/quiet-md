//! 成形：模型返回的原文 → 写进文档的块。
//! 1. `store`：跑「存下来时」的 AI 输出正则（SillyTavern 存进聊天前就跑），结果就是记录里存的原文；
//! 2. 去掉思考（`<think>`、`<thinking>`）；
//! 3. 跑「显示时」的正则（深度 0）。替换结果是 HTML 的不跑：匹配到的整段收进一个折叠块（脚本名做标题，原文放在里面），
//!    只匹配到行内一小段的（比如注音）留着原文不动；
//! 4. 照 SillyTavern 显示的样子收拾：HTML 注释不显示，标签不显示、里面的字照常显示，`<br>` 是换行；
//! 5. 按 Markdown 一行一块导入（空行不成块，行首的半角空白去掉免得变成缩进），每段文字都带上这次生成的标记。
//!
//! 流式输出时每收到一段就对「已经稳了的前缀」（`Shaper::settled`）成形一次：没写完的思考、注释、标签、
//! 要收进折叠块的段落、刚起头的 Markdown 标记先不动，等它写完。
use std::sync::OnceLock;

use fancy_regex::Regex;
use fdl_core::model::{Block, BlockKind, RichText};
use fdl_core::paste::{Off, markdown_to_blocks};

use super::macros::Macros;
use super::preset::Preset;
use super::regex::{AI_OUTPUT, Compiled, Scope, Script};

pub struct Shaped {
    pub blocks: Vec<Block>,
    /// 显示用的文字（导入前）。
    pub display: String,
    pub notes: Vec<String>,
}

/// 一次生成里要反复成形（流式时每段一次）：预设的输出正则只编译一次。编译不了的跳过。
pub struct Shaper {
    store: Vec<(Script, Compiled)>,
    display: Vec<(Script, Compiled)>,
}

impl Shaper {
    pub fn new(preset: &Preset, macros: &mut Macros) -> Shaper {
        let mut pick = |scope| preset.regex_scripts().into_iter().filter(|s| s.applies(AI_OUTPUT, scope, Some(0)))
            .filter_map(|s| { let c = s.compile(macros).ok()?; Some((s, c)) }).collect();
        Shaper { store: pick(Scope::Stored), display: pick(Scope::Display) }
    }

    /// 「存下来时」的 AI 输出正则（两个开关都没开的）。运行出错的那条跳过。
    pub fn store(&self, raw: &str, macros: &mut Macros) -> String {
        let mut text = raw.to_owned();
        for (s, c) in &self.store { if let Ok(t) = s.run_compiled(c, &text, macros) { text = t; } }
        text
    }

    /// 原文里可以先成形的前缀有多长（字节）。后面的还在写，成形结果会变。
    pub fn settled(&self, raw: &str) -> usize {
        static THINK: OnceLock<Regex> = OnceLock::new();
        let mut cut = raw.len();
        // 没闭合的思考：从开头标签起都不动。
        if let Some(m) = re(&THINK, r"(?i)<(think|thinking)>").find_iter(raw).filter_map(Result::ok).last() {
            let close = format!("</{}>", &raw[m.start() + 1..m.end() - 1]).to_ascii_lowercase();
            if !raw[m.end()..].to_ascii_lowercase().contains(&close) { cut = cut.min(m.start()); }
        }
        // 没闭合的注释。
        if let Some(at) = raw.rfind("<!--") && !raw[at..].contains("-->") { cut = cut.min(at); }
        // 写了一半的标签：最后一个 `<` 后面还没有 `>`，而且看着像标签。
        if let Some(at) = raw.rfind('<') && !raw[at..].contains('>') && raw[at..].len() < 64
            && raw[at + 1..].chars().next().is_none_or(|c| c.is_ascii_alphabetic() || c == '/' || c == '!') { cut = cut.min(at); }
        // 正则要匹配的段落还没写完（会被删掉、换掉或收进折叠块），或者末尾正写着它要找的字。
        for (_, c) in self.store.iter().chain(&self.display) {
            if let Some(at) = c.unfinished(raw) { cut = cut.min(at); }
            if let Some(at) = c.partial_tail(&raw[..cut]) { cut = cut.min(at); }
        }
        // 最后一行刚起头、还看不出是不是 Markdown 标记（`-` 还是 `---`，`#` 还是 `##`）。
        let line = raw[..cut].rfind('\n').map_or(0, |i| i + 1);
        let tail = raw[line..cut].trim_start_matches([' ', '\t']);
        if tail.chars().count() < 4 && tail.chars().next().is_some_and(|c| "#-*+>|`=_~".contains(c) || c.is_ascii_digit()) { cut = line; }
        cut
    }

    pub fn shape(&self, stored: &str, gen_id: &str, macros: &mut Macros) -> Shaped {
        let mut notes = vec![];
        let mut text = strip_thinking(stored);
        let mut folds: Vec<(String, String)> = vec![];
        for (s, c) in &self.display {
            if !s.emits_html() {
                match s.run_compiled(c, &text, macros) { Ok(t) => text = t, Err(_) => notes.push(format!("正则「{}」运行出错，没有执行", s.name)) }
                continue;
            }
            // 产出 HTML 的：整段匹配换成占位，行内的不动。
            let mut out = String::new();
            let (mut last, mut inline) = (0, 0);
            for m in c.re.find_iter(&text).take(if c.global { usize::MAX } else { 1 }) {
                let Ok(m) = m else { notes.push(format!("正则「{}」运行出错，没有执行", s.name)); break };
                if m.range().is_empty() { continue; }
                if !whole_lines(&text, &m.range()) { inline += 1; continue; }
                out.push_str(&text[last..m.start()]);
                out.push_str(&format!("\n\n{OPEN}{}{CLOSE}\n\n", folds.len()));
                folds.push((s.name.clone(), m.as_str().to_owned()));
                last = m.end();
            }
            out.push_str(&text[last..]);
            text = out;
            if inline > 0 { notes.push(format!("正则「{}」产出 HTML，行内的 {inline} 处保留原文", s.name)); }
        }
        let display = strip_markup(&text);
        // 行首的半角空白去掉（全角空格是正文的缩进，留着）。
        let lines: Vec<&str> = display.split('\n').map(|l| l.trim_start_matches([' ', '\t'])).collect();
        let mut blocks: Vec<Block> = markdown_to_blocks(&lines.join("\n"), Off::default()).into_iter().filter(|b| !(b.kind == BlockKind::Text && b.text.text.trim().is_empty() && b.children.is_empty())).collect();
        for b in &mut blocks {
            let t = b.text.text.trim();
            if b.kind == BlockKind::Text && t.starts_with(OPEN) && t.ends_with(CLOSE) && let Some((name, raw)) = t[OPEN.len_utf8()..t.len() - CLOSE.len_utf8()].parse::<usize>().ok().and_then(|i| folds.get(i)) {
                *b = fold(name, raw);
            }
        }
        for b in &mut blocks { mark(b, gen_id); }
        Shaped { blocks, display, notes }
    }
}

/// 只跑一次时用：「存下来时」的正则。
pub fn store(preset: &Preset, raw: &str, macros: &mut Macros) -> String { Shaper::new(preset, macros).store(raw, macros) }

/// 折叠块在显示文字里的占位：私用区字符，正则和 Markdown 都不会碰。
const OPEN: char = '\u{E000}';
const CLOSE: char = '\u{E001}';

fn re(cell: &'static OnceLock<Regex>, p: &str) -> &'static Regex { cell.get_or_init(|| Regex::new(p).unwrap()) }

/// 去掉思考：成对的 `<think>…</think>`、`<thinking>…</thinking>`，连同后面的空白。没闭合的不动。
pub fn strip_thinking(s: &str) -> String {
    static R: OnceLock<Regex> = OnceLock::new();
    re(&R, r"(?is)<(think|thinking)>.*?</\1>\s*").replace_all(s, "").into_owned()
}

/// SillyTavern 显示时看不见的：HTML 注释、标签本身（里面的字照常显示）；`<br>` 是换行。
fn strip_markup(s: &str) -> String {
    static COMMENT: OnceLock<Regex> = OnceLock::new();
    static BR: OnceLock<Regex> = OnceLock::new();
    static TAG: OnceLock<Regex> = OnceLock::new();
    let s = re(&COMMENT, r"(?s)<!--.*?-->").replace_all(s, "");
    let s = re(&BR, r"(?i)<br\s*/?>").replace_all(&s, "\n");
    re(&TAG, r"</?[A-Za-z][^\s<>/]*(?:\s[^<>]*)?/?>").replace_all(&s, "").into_owned()
}

/// 这段匹配占满了它所在的行（前后只有空白），或者跨了行：算「整段」，收进折叠块。
fn whole_lines(text: &str, r: &std::ops::Range<usize>) -> bool {
    let before = &text[..r.start];
    let after = &text[r.end..];
    text[r.clone()].contains('\n') || (before.rsplit('\n').next().unwrap_or("").trim().is_empty() && after.split('\n').next().unwrap_or("").trim().is_empty())
}

/// 只跑一次时用：成形。
pub fn shape(preset: &Preset, stored: &str, gen_id: &str, macros: &mut Macros) -> Shaped { Shaper::new(preset, macros).shape(stored, gen_id, macros) }

/// 收起来的一段：高亮块里一个折叠的段落，标题是脚本名，原文一行一块放在下面。
fn fold(name: &str, raw: &str) -> Block {
    let mut title = Block::new(BlockKind::Text, RichText::plain(name));
    title.folded = true;
    title.children = raw.trim().split('\n').map(|l| Block::new(BlockKind::Text, RichText::plain(l.trim_end()))).collect();
    let mut callout = Block::new(BlockKind::Callout { emoji: "📦".into(), bg: None, border: None, text_color: None }, RichText::default());
    callout.children = vec![title];
    callout
}

fn mark(b: &mut Block, id: &str) {
    let n = b.text.len();
    b.text.update_marks(0..n, |m| m.generated = Some(id.to_owned()));
    for c in &mut b.children { mark(c, id); }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tavern::macros::Env;
    use serde_json::json;

    fn preset(scripts: serde_json::Value) -> Preset { Preset::parse(json!({ "extensions": { "regex_scripts": scripts } }).to_string().as_bytes()).unwrap() }

    fn xml(blocks: Vec<Block>) -> String {
        let doc = fdl_core::model::Document { title: RichText::default(), icon: None, cover: None, blocks };
        fdl_core::docxml::serialize(&doc).split_once('\n').map_or(String::new(), |(_, rest)| rest.to_owned())
    }

    #[test]
    fn prose_with_tags_and_thinking() {
        let p = preset(json!([{ "scriptName": "标题", "findRegex": "/### 正文\\s*/g", "replaceString": "", "placement": [2], "markdownOnly": true }]));
        let mut m = Macros::new(Env::default());
        let s = shape(&p, "<thinking>想</thinking>\n### 正文\n\n<content>\n<!-- 注释 -->\n　　她推开门。\n  缩进<br>换行\n\n---\n![](http://x/a.png)\n</content>", "g1", &mut m);
        assert_eq!(xml(s.blocks), "<p><span gen=\"g1\">　　她推开门。</span></p>\n<p><span gen=\"g1\">缩进</span></p>\n<p><span gen=\"g1\">换行</span></p>\n<hr/>\n<p><span gen=\"g1\">!<a href=\"http://x/a.png\">http://x/a.png</a></span></p>\n");
    }

    #[test]
    fn html_scripts_fold_whole_sections_only() {
        let p = preset(json!([
            { "scriptName": "摘要卡片", "findRegex": "/<meow_FM>[\\s\\S]*?<\\/meow_FM>/g", "replaceString": "<div class=\"card\">$0</div>", "placement": [2], "markdownOnly": true },
            { "scriptName": "注音", "findRegex": "/“([^”]+)”『([^』]+)』/g", "replaceString": "<ruby>$1<rt>$2</rt></ruby>", "placement": [2], "markdownOnly": true },
            { "scriptName": "只发送", "findRegex": "/正文/g", "replaceString": "X", "placement": [2], "promptOnly": true },
        ]));
        let mut m = Macros::new(Env::default());
        let s = shape(&p, "正文“春”『はる』。\n<meow_FM>\ntime: 夜\n</meow_FM>\n后文", "g", &mut m);
        assert_eq!(xml(s.blocks), concat!(
            "<p><span gen=\"g\">正文“春”『はる』。</span></p>\n",
            "<callout emoji=\"📦\">\n  <p folded=\"true\"><span gen=\"g\">摘要卡片</span>\n",
            "    <p><span gen=\"g\">&lt;meow_FM&gt;</span></p>\n    <p><span gen=\"g\">time: 夜</span></p>\n    <p><span gen=\"g\">&lt;/meow_FM&gt;</span></p>\n  </p>\n</callout>\n",
            "<p><span gen=\"g\">后文</span></p>\n"));
        assert_eq!(s.notes, ["正则「注音」产出 HTML，行内的 1 处保留原文"]);
    }

    #[test]
    fn stored_scripts_only_run_in_store() {
        let p = preset(json!([{ "scriptName": "存", "findRegex": "/a/g", "replaceString": "b", "placement": [2] }]));
        let mut m = Macros::new(Env::default());
        assert_eq!(store(&p, "aa", &mut m), "bb");
        assert_eq!(shape(&p, "aa", "g", &mut m).display, "aa");
    }

    #[test]
    fn settles_only_finished_parts() {
        let p = preset(json!([{ "scriptName": "卡片", "findRegex": "/<FM>[\\s\\S]*?<\\/FM>/g", "replaceString": "<div>$0</div>", "placement": [2], "markdownOnly": true }]));
        let mut m = Macros::new(Env::default());
        let s = Shaper::new(&p, &mut m);
        let cut = |t: &'static str| &t[..s.settled(t)];
        assert_eq!(cut("正文。\n<thinking>想"), "正文。\n");
        assert_eq!(cut("<think>想</think>正文"), "<think>想</think>正文");
        assert_eq!(cut("甲<!-- 注"), "甲");
        assert_eq!(cut("甲<conte"), "甲");
        assert_eq!(cut("甲 a < b"), "甲 a < b");
        assert_eq!(cut("甲\n<FM>\ntime"), "甲\n");
        assert_eq!(cut("甲\n<FM>\ntime\n</FM>\n乙"), "甲\n<FM>\ntime\n</FM>\n乙");
        assert_eq!(cut("甲\n--"), "甲\n");
        assert_eq!(cut("甲\n- 一项"), "甲\n- 一项");
    }
}
