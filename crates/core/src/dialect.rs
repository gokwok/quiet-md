//! 助手读写正文用的 Markdown 方言。读（`read`：块 → 方言文本）和写（`write`：方言文本 → 块）都走
//! `markdown` 的写出和 `paste` 的 Markdown 读入，只在方言开关下多几样：
//!
//! - 每块前标短号 `[b12]`（一次 run 里有效，`Refs` 记着短号对应哪篇文档的哪一块）；写入时可以有不带短号的新块。
//! - 视频 `<video src="…">说明</video>`、文件 `<file src="…">文件名</file>`（云端附件是 `token="…"`），
//!   行内提醒 `<reminder at="2026-11-28 09:00" notify="true"/>`（全天只写日期），@文档 `<doc id="…">标题</doc>`，
//!   `#标签`；`<u>` 下划线、`<br>` 换行照 Markdown 导出的写法。
//! - 方言表达不了的块（倒计时、分栏、子页面、卡片、合并过单元格或单元格里不止一段的表格、认不出的元素）
//!   是 `<keep ref="b12"/>`：写回时还在就是原块（内容和属性都不动），不在了就是删掉。
//! - 高亮块读出是头一行写着 emoji 的引用，写回时认出来还是高亮块。
//!
//! 写回时带短号、读出来和原来一字不差的块就是原块本身；改过的块留原来的块标识，
//! 对齐、底色、字体、图片尺寸这类方言里看不见的属性照旧，行内提醒、文件这些对象标签没变的也用回原来的。
//! 配音、生成记录这类标记改写后会丢。认不出的标签按普通文字留着。

use crate::markdown::{dialect_writer, Options};
use crate::model::*;
use std::collections::{HashMap, HashSet};

/// 读入时带短号的块先用这个标识占着（`tagged(12)`），`write` 再换成真的。
const TAG: u64 = 1 << 62;

pub(crate) fn tagged(n: u32) -> BlockId {
    BlockId(TAG | n as u64)
}

fn untag(id: BlockId) -> Option<u32> {
    (id.0 & TAG != 0).then_some((id.0 & !TAG) as u32)
}

/// 行首的 `[b12]`（前面可以有缩进）：短号和它后面的内容（不含缩进）。
pub(crate) fn ref_prefix(line: &str) -> Option<(u32, &str)> {
    let t = line.trim_start_matches(' ');
    let rest = t.strip_prefix("[b")?;
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 || digits > 9 { return None; }
    let n = rest[..digits].parse().ok()?;
    let after = rest[digits..].strip_prefix(']')?;
    if after.is_empty() { return Some((n, after)); }
    after.strip_prefix(' ').map(|r| (n, r))
}

/// 一次 run 里的短号：`b12` ↔ 哪篇文档（宿主给的标识）的哪一块。同一块再读还是同一个号。
/// 块标识只在内存里有效，文档重新载入后旧短号就找不到了（`write` 会说「已经不在了」）。
#[derive(Clone, Debug, Default)]
pub struct Refs {
    next: u32,
    by_block: HashMap<(String, BlockId), u32>,
    entries: HashMap<u32, Entry>,
}

#[derive(Clone, Debug)]
struct Entry {
    doc: String,
    id: BlockId,
    /// 上次读出时这一块写成的样子（不带短号），写回时比对有没有改。
    text: String,
    /// 有序列表这一项的序号。
    seq: u32,
}

impl Refs {
    pub fn new() -> Self {
        Self::default()
    }

    /// `doc` 里这一块的短号（`b12`），没读出过就是 None。
    pub fn short(&self, doc: &str, id: BlockId) -> Option<String> {
        self.by_block.get(&(doc.to_string(), id)).map(|n| format!("b{n}"))
    }

    /// `b12`（或 `[b12]`、`12`）是哪篇文档的哪一块。
    pub fn resolve(&self, short: &str) -> Option<(&str, BlockId)> {
        let s = short.trim().trim_start_matches('[').trim_end_matches(']');
        let n: u32 = s.strip_prefix('b').unwrap_or(s).parse().ok()?;
        self.entries.get(&n).map(|e| (e.doc.as_str(), e.id))
    }

    /// 给 `doc` 里这一块编个号（已有就是原来的），不读出它：列待办、搜索结果这些只报块号的地方用。
    pub fn mark(&mut self, doc: &str, id: BlockId) -> String {
        format!("b{}", self.number(doc, id))
    }

    fn number(&mut self, doc: &str, id: BlockId) -> u32 {
        if let Some(n) = self.by_block.get(&(doc.to_string(), id)) { return *n; }
        self.next += 1;
        let n = self.next;
        self.by_block.insert((doc.to_string(), id), n);
        self.entries.insert(n, Entry { doc: doc.to_string(), id, text: String::new(), seq: 0 });
        n
    }
}

/// `markdown` 写出时的方言状态。
pub(crate) struct Pass<'a> {
    refs: &'a mut Refs,
    doc: &'a str,
    /// 读出：记下每块写成的样子。写回时比对用的那次不记。
    record: bool,
}

impl Pass<'_> {
    fn number(&mut self, id: BlockId) -> u32 {
        untag(id).unwrap_or_else(|| self.refs.number(self.doc, id))
    }

    /// 先占号：父块比子块先编号，读出来照文档顺序。
    pub(crate) fn reserve(&mut self, id: BlockId) {
        self.number(id);
    }

    pub(crate) fn keep(&mut self, id: BlockId) -> String {
        format!("<keep ref=\"b{}\"/>", self.number(id))
    }

    /// 一块写好的 `s` 前面标上短号（`<keep/>` 自己带着，不标）。
    pub(crate) fn mark(&mut self, b: &Block, seq: u32, s: String) -> String {
        let n = self.number(b.id);
        if self.record && let Some(e) = self.refs.entries.get_mut(&n) {
            e.text = s.clone();
            e.seq = seq;
        }
        if is_keep(b) { s } else if s.is_empty() { format!("[b{n}]") } else { format!("[b{n}] {s}") }
    }
}

/// 方言表达不了、读出是 `<keep/>` 的块。
pub(crate) fn is_keep(b: &Block) -> bool {
    match &b.kind {
        BlockKind::Countdown(_) | BlockKind::Grid | BlockKind::Column { .. } | BlockKind::Page { .. } | BlockKind::SubPageList | BlockKind::Raw { .. } => true,
        BlockKind::Table(t) => t.rows.iter().flatten().any(|c| {
            c.colspan != 1 || c.rowspan != 1 || c.blocks.len() > 1 || c.blocks.iter().any(|b| b.kind != BlockKind::Text || !b.children.is_empty() || b.text.text.contains('\n'))
        }),
        _ => false,
    }
}

/// 视频：来源（或名字）是 mp4 / mov / m4v（和 iOS 的 `video::is_video` 一致）。
pub fn is_video(f: &FileBlock) -> bool {
    [&f.src, &f.name].iter().any(|s| {
        let ext = s.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
        matches!(ext.as_str(), "mp4" | "mov" | "m4v")
    })
}

fn attr(s: &str) -> String {
    s.replace('&', "&amp;").replace('"', "&quot;").replace('<', "&lt;").replace('>', "&gt;")
}

fn text(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

fn unescape(s: &str) -> String {
    s.replace("&quot;", "\"").replace("&lt;", "<").replace("&gt;", ">").replace("&#39;", "'").replace("&amp;", "&")
}

pub(crate) fn file_tag(f: &FileBlock) -> String {
    let tag = if is_video(f) { "video" } else { "file" };
    let key = if f.src_kind == "token" { "token" } else { "src" };
    format!("<{tag} {key}=\"{}\">{}</{tag}>", attr(&f.src), text(&f.name))
}

fn reminder_tag(r: &InlineReminder) -> String {
    let (y, m, d, h, mi) = crate::time::civil(r.expire.max(0) as u64 / 1000);
    let at = if r.whole_day { format!("{y}-{m:02}-{d:02}") } else { format!("{y}-{m:02}-{d:02} {h:02}:{mi:02}") };
    format!("<reminder at=\"{at}\" notify=\"{}\"/>", r.should_notify)
}

/// 行内对象在方言里的写法；None：照 Markdown 导出（按钮是链接，别的只剩文字）。
pub(crate) fn inline_text(obj: &Inline) -> Option<String> {
    Some(match &obj.kind {
        InlineKind::Equation(src) => format!("${src}$"),
        InlineKind::File(f) => file_tag(f),
        InlineKind::Reminder(r) => reminder_tag(r),
        InlineKind::DocLink(l) => format!("<doc id=\"{}\">{}</doc>", attr(&l.id), text(&l.title)),
        InlineKind::Tag(name) => format!("#{name}"),
        _ => return None,
    })
}

/// 方言里普通文字多转义的两样：像 `#标签` 的 `#` 和公式的 `$`。
pub(crate) fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '$' || c == '#' && chars.peek().is_some_and(|&n| tag_char(n)) { out.push('\\'); }
        out.push(c);
    }
    out
}

/// 一个方言标签读成的东西。
pub(crate) enum Tag<'a> {
    Obj(Inline),
    /// `<u>…</u>` 里面的 Markdown。
    Under(&'a str),
    Break,
}

/// 2026-11-28 或 2026-11-28 09:00（`T` 隔开也行）。
fn reminder_at(at: &str, notify: bool) -> Option<InlineReminder> {
    const DAY: i64 = 86_400_000;
    let at = at.trim();
    let (date, time) = match at.split_once([' ', 'T']) { Some((d, t)) => (d, Some(t.trim())), None => (at, None) };
    let mut p = date.split('-');
    let (y, m, d): (i64, u32, u32) = (p.next()?.parse().ok()?, p.next()?.parse().ok()?, p.next()?.parse().ok()?);
    if p.next().is_some() || !(1..=12).contains(&m) || d == 0 || d > crate::time::days_in_month(y, m) { return None; }
    Some(match time {
        // 和 `src/editor/reminder.rs` 一致：全天的到期是当天 23:59:59，当天 09:00 提醒；定时的到点提醒。
        None => {
            let day = crate::time::local_to_unix(y, m, d, 0, 0) * 1000;
            InlineReminder { expire: day + DAY - 1000, notify: day + 9 * 3_600_000, should_notify: notify, whole_day: true }
        }
        Some(t) => {
            let (h, mi) = t.split_once(':')?;
            let (h, mi): (u32, u32) = (h.parse().ok()?, mi.get(..2).unwrap_or(mi).parse().ok()?);
            if h > 23 || mi > 59 { return None; }
            let expire = crate::time::local_to_unix(y, m, d, h, mi) * 1000;
            InlineReminder { expire, notify: expire, should_notify: notify, whole_day: false }
        }
    })
}

/// `rest` 开头的方言标签和它占的长度；认不出（或写坏了）是 None，照普通文字读。
pub(crate) fn tag_at(rest: &str) -> Option<(Tag<'_>, usize)> {
    let s = rest.strip_prefix('<')?;
    let name_len = s.bytes().take_while(u8::is_ascii_alphabetic).count();
    let name = &s[..name_len];
    if !matches!(name, "br" | "u" | "video" | "file" | "reminder" | "doc" | "keep") { return None; }
    let mut i = 1 + name_len;
    let mut attrs: HashMap<&str, String> = HashMap::new();
    let closed = loop {
        let t = &rest[i..];
        let skip = t.len() - t.trim_start().len();
        i += skip;
        let t = &rest[i..];
        if let Some(r) = t.strip_prefix("/>") { i += t.len() - r.len(); break true; }
        if let Some(r) = t.strip_prefix('>') { i += t.len() - r.len(); break false; }
        if skip == 0 && !attrs.is_empty() { return None; }
        let key_len = t.bytes().take_while(|b| b.is_ascii_alphanumeric() || *b == b'-' || *b == b'_').count();
        if key_len == 0 { return None; }
        let key = &t[..key_len];
        let v = t[key_len..].strip_prefix('=')?;
        let q = v.chars().next().filter(|c| matches!(c, '"' | '\''))?;
        let end = v[1..].find(q)?;
        attrs.insert(key, unescape(&v[1..1 + end]));
        i += key_len + 1 + 1 + end + 1;
    };
    // 成对的标签：里面的内容和 `</name>` 之后。
    let inner = |i: &mut usize| -> Option<&str> {
        if closed { return Some(""); }
        let close = format!("</{name}>");
        let end = rest[*i..].find(&close)?;
        let s = &rest[*i..*i + end];
        *i += end + close.len();
        Some(s)
    };
    let tag = match name {
        "br" => Tag::Break,
        "u" if !closed => Tag::Under(inner(&mut i)?),
        "video" | "file" => {
            let (src_kind, src) = match (attrs.get("src"), attrs.get("token")) {
                (Some(s), _) => ("path", s.clone()),
                (None, Some(t)) => ("token", t.clone()),
                _ => return None,
            };
            if src.is_empty() { return None; }
            let name = unescape(inner(&mut i)?.trim());
            let name = if name.is_empty() { src.rsplit('/').next().unwrap_or(&src).to_string() } else { name };
            Tag::Obj(crate::docxml::to_inline(InlineKind::File(FileBlock { src_kind: src_kind.into(), src, name, view: FileView::Card })))
        }
        "reminder" => {
            let r = reminder_at(attrs.get("at")?, attrs.get("notify").is_none_or(|v| v.trim() != "false"))?;
            if !closed { inner(&mut i)?; }
            Tag::Obj(crate::docxml::to_inline(InlineKind::Reminder(r)))
        }
        "doc" => {
            let id = attrs.get("id").filter(|s| !s.is_empty())?.clone();
            let title = unescape(inner(&mut i)?);
            Tag::Obj(crate::docxml::to_inline(InlineKind::DocLink(InlineDocLink { id, title })))
        }
        "keep" => {
            let (n, _) = ref_prefix(&format!("[{}]", attrs.get("ref")?.trim().trim_matches(['[', ']'])))?;
            if !closed { inner(&mut i)?; }
            Tag::Obj(Inline { kind: InlineKind::Other { tag: "keep".into() }, xml: format!("<keep ref=\"b{n}\"/>") })
        }
        _ => return None,
    };
    Some((tag, i))
}

/// 一个 `<keep/>` 对象的短号。
fn keep_of(obj: &Inline) -> Option<u32> {
    if !matches!(&obj.kind, InlineKind::Other { tag } if tag == "keep") { return None; }
    let n = obj.xml.strip_prefix("<keep ref=\"b")?.split('"').next()?;
    n.parse().ok()
}

/// `blocks`（`doc` 的正文或其中一段）读成方言文本，短号记在 `refs` 里。
pub fn read(doc: &str, blocks: &[Block], refs: &mut Refs) -> String {
    let opts = Options { in_place: true, ..Default::default() };
    let mut w = dialect_writer(&opts, Pass { refs, doc, record: true });
    w.blocks(blocks)
}

/// 写回出错：短号不对。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// 这次 run 里没有这个短号，或者它是别的文档的块。
    Unknown(String),
    /// 有这个短号，但那一块已经不在文档里了。
    Gone(String),
    /// 同一个短号用了两次。
    Twice(String),
    /// `<keep/>` 没有单独占一行。
    KeepInline(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Unknown(s) => write!(f, "没有块 {s}：短号只能用这篇文档这次读出来的"),
            Error::Gone(s) => write!(f, "块 {s} 已经不在文档里了，请重新读一遍"),
            Error::Twice(s) => write!(f, "块 {s} 出现了两次：同一个短号只能用一次，新块不要带短号"),
            Error::KeepInline(s) => write!(f, "<keep ref=\"{s}\"/> 要单独占一行"),
        }
    }
}

impl std::error::Error for Error {}

/// 方言文本写回成块，替换读出时的那些块：`blocks` 是 `doc` 现在的正文（找原块用）。
/// 带短号且没改的块原样返回；改过的留原块标识；没带短号的是新块。
pub fn write(doc: &str, text: &str, blocks: &[Block], refs: &mut Refs) -> Result<Vec<Block>, Error> {
    let parsed = crate::paste::markdown_to_blocks(text, crate::paste::Off { file: true, alt: true, dialect: true, ..Default::default() });
    let mut index = HashMap::new();
    fn walk<'a>(bs: &'a [Block], index: &mut HashMap<BlockId, &'a Block>) {
        for b in bs {
            index.insert(b.id, b);
            walk(&b.children, index);
        }
    }
    walk(blocks, &mut index);
    let mut fix = Fix { doc, refs, index, used: HashSet::new() };
    fix.blocks(parsed)
}

struct Fix<'a> {
    doc: &'a str,
    refs: &'a mut Refs,
    index: HashMap<BlockId, &'a Block>,
    used: HashSet<u32>,
}

impl<'a> Fix<'a> {
    /// 短号 `n` 的原块和读出时的样子。
    fn take(&mut self, n: u32) -> Result<(&'a Block, String, u32), Error> {
        let short = format!("b{n}");
        let e = self.refs.entries.get(&n).filter(|e| e.doc == self.doc).ok_or_else(|| Error::Unknown(short.clone()))?;
        let b = *self.index.get(&e.id).ok_or_else(|| Error::Gone(short.clone()))?;
        let (text, seq) = (e.text.clone(), e.seq);
        if !self.used.insert(n) { return Err(Error::Twice(short)); }
        Ok((b, text, seq))
    }

    fn render(&mut self, b: &Block, seq: u32) -> String {
        let opts = Options { in_place: true, ..Default::default() };
        dialect_writer(&opts, Pass { refs: &mut *self.refs, doc: self.doc, record: false }).block(b, seq)
    }

    fn blocks(&mut self, bs: Vec<Block>) -> Result<Vec<Block>, Error> {
        let mut out = vec![];
        for mut b in bs {
            b.children = self.blocks(std::mem::take(&mut b.children))?;
            // 一行只有一个对象（前后的空格不算）。
            let lone = |b: &Block| (b.kind == BlockKind::Text && b.text.text.trim() == crate::editor::core::EQUATION).then(|| b.text.runs.iter().find_map(|r| r.marks.inline.clone())).flatten();
            // 单独一行的 `<keep/>`：原块。
            if let Some(n) = lone(&b).as_ref().and_then(keep_of) {
                out.push(self.take(n)?.0.clone());
                continue;
            }
            if let Some(n) = b.text.runs.iter().find_map(|r| r.marks.inline.as_ref().and_then(keep_of)) {
                return Err(Error::KeepInline(format!("b{n}")));
            }
            // 单独一行的视频 / 文件是一块。
            if let Some(Inline { kind: InlineKind::File(f), .. }) = lone(&b).filter(|_| b.children.is_empty()) {
                let view = if is_video(&f) { FileView::Preview } else { FileView::Card };
                b.kind = BlockKind::File(FileBlock { view, ..f });
                b.text = RichText::default();
            }
            if let BlockKind::Image { src, .. } = &mut b.kind && !src.contains("://") {
                *src = crate::markdown::percent_decode(src);
            }
            let Some(n) = untag(b.id) else { out.push(b); continue };
            let (orig, was, seq) = self.take(n)?;
            // 行尾的空格读进来就没了，比的时候不算。
            let norm = |s: &str| s.lines().map(str::trim_end).collect::<Vec<_>>().join("\n").trim_end().to_string();
            if norm(&self.render(&b, seq)) == norm(&was) {
                out.push(orig.clone());
                continue;
            }
            b.id = orig.id;
            carry(&mut b, orig, seq);
            out.push(b);
        }
        Ok(out)
    }
}

/// 改过的块接着用原块里方言看不见的东西：对齐、底色、字体、折叠，图片尺寸，代码块的标题和换行，
/// 表格列宽，高亮块的样式，还有标签没变的行内对象（提醒的提前量、@文档……）。
fn carry(b: &mut Block, orig: &Block, seq: u32) {
    if let (BlockKind::Callout { emoji, .. }, BlockKind::Quote) = (&orig.kind, &b.kind) {
        let first = b.children.first().is_some_and(|c| c.kind == BlockKind::Text && c.children.is_empty() && c.text.text.trim() == emoji.trim());
        if first && !emoji.trim().is_empty() { b.children.remove(0); }
        if first || emoji.trim().is_empty() { b.kind = orig.kind.clone(); }
    }
    if std::mem::discriminant(&b.kind) == std::mem::discriminant(&orig.kind) {
        // 标题折起来的是它下面那一节，没有子块也算。
        let folds = !b.children.is_empty() || matches!(b.kind, BlockKind::Heading { .. });
        (b.align, b.bg, b.font, b.folded) = (orig.align, orig.bg.clone(), orig.font.clone(), orig.folded && folds);
    }
    match (&mut b.kind, &orig.kind) {
        (BlockKind::Ordered { seq: s }, BlockKind::Ordered { seq: None }) if *s == Some(seq) => *s = None,
        (BlockKind::Heading { seq_auto, .. }, BlockKind::Heading { seq_auto: was, .. }) => *seq_auto = *was,
        (BlockKind::Code { caption, wrap, .. }, BlockKind::Code { caption: c, wrap: w, .. }) => (*caption, *wrap) = (c.clone(), *w),
        (BlockKind::Divider { rule }, BlockKind::Divider { rule: r }) => *rule = *r,
        (BlockKind::Image { src_kind, src, width, height, crop, .. }, BlockKind::Image { src_kind: k, src: s, width: w, height: h, crop: c, .. }) if src == s => {
            (*src_kind, *width, *height, *crop) = (k.clone(), *w, *h, *c);
        }
        (BlockKind::File(f), BlockKind::File(o)) if f.src == o.src => *f = FileBlock { name: f.name.clone(), ..o.clone() },
        (BlockKind::Table(t), BlockKind::Table(o)) => {
            if t.col_widths.len() == o.col_widths.len() { t.col_widths = o.col_widths.clone(); }
            t.header_col = o.header_col;
        }
        _ => {}
    }
    // 标签一样的行内对象用回原来的。
    let old: HashMap<String, &Inline> = orig.text.runs.iter().filter_map(|r| r.marks.inline.as_ref()).filter_map(|i| inline_text(i).map(|t| (t, i))).collect();
    if old.is_empty() || !b.text.runs.iter().any(|r| r.marks.inline.is_some()) { return; }
    let mut t = RichText::default();
    for (r, m) in b.text.spans() {
        let mut m = m.clone();
        if let Some(o) = m.inline.as_ref().and_then(inline_text).and_then(|k| old.get(&k)) { m.inline = Some((*o).clone()); }
        t.push(&b.text.text[r], m);
    }
    b.text = t;
}

#[cfg(test)]
mod tests;
