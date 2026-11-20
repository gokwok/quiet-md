//! 还原历史：锚点之前的前文（按块，带「是不是模型写的」）→ 聊天记录。
//! 连续的同一来源合成一条：用户自己写的连成一条 user，同一次生成的连成一条 assistant。
//! 每次生成时的要求放回那次生成之前，作为一条 user。
//! 一次生成的块都没被改过、也没被拆开时，发它的原文（显示时去掉的思考、标签都还在，「只改发送内容」的正则要用）；
//! 改过就发现在的文字，以用户的修改为准。
use std::{borrow::Cow, collections::HashMap};

use fdl_core::model::{Block, BlockId, BlockKind, Document, RichText};

use super::assemble::Turn;
use super::preset::Role;
use super::record::{Record, hash};

/// 前文里的一块。
pub enum Piece<'a> {
    Written(Cow<'a, str>),
    /// 模型写的；`pristine` 是这次生成的块和写进文档时一样（`pieces` 比对记录里的哈希）。
    Generated { id: &'a str, text: Cow<'a, str>, pristine: bool },
}

/// 文档里锚点（含它的子块）以前的块，切成 `Piece`。一块里有生成标记就算那次生成写的；
/// 这次生成各块的全文和记录里的哈希一致，就是没改过。
pub fn pieces<'a>(doc: &'a Document, anchor: Option<BlockId>, records: &HashMap<String, Record>) -> Vec<Piece<'a>> {
    fn walk<'a>(blocks: &'a [Block], anchor: Option<BlockId>, out: &mut Vec<&'a Block>) -> bool {
        for b in blocks {
            out.push(b);
            let mut done = walk(&b.children, anchor, out);
            if let BlockKind::Table(t) = &b.kind { for cell in t.rows.iter().flatten() { done |= walk(&cell.blocks, anchor, out); } }
            if done || Some(b.id) == anchor { return true; }
        }
        false
    }
    let mut blocks = vec![];
    walk(&doc.blocks, anchor, &mut blocks);
    let mut clean: HashMap<&str, bool> = HashMap::new();
    blocks.into_iter().map(|b| match b.text.spans().find_map(|(_, m)| m.generated.as_deref()) {
        Some(id) => {
            let pristine = *clean.entry(id).or_insert_with(|| records.get(id).is_some_and(|r| r.hash == hash(&doc.gen_text(id))));
            Piece::Generated { id, text: readable(&b.text), pristine }
        }
        None => Piece::Written(readable(&b.text)),
    }).collect()
}

/// 发给模型的字：行内对象在文字里是 U+FFFC，@文档 换成标题，公式换成 LaTeX，别的去掉。
fn readable(t: &RichText) -> Cow<'_, str> {
    if !t.text.contains('\u{fffc}') { return Cow::Borrowed(&t.text); }
    let mut out = String::with_capacity(t.text.len());
    for (r, m) in t.spans() {
        match m.inline.as_ref() {
            Some(i) => if let Some(l) = i.doc_link() { out.push_str(&l.title) } else if let Some(src) = i.equation() { out.push_str(src) },
            None => out.push_str(&t.text[r]),
        }
    }
    Cow::Owned(out.replace('\u{fffc}', ""))
}

/// 文档里锚点以前的聊天记录：读出用到的生成记录（`dir` 是 `record::dir()`），切块，合成消息。
pub fn turns(doc: &Document, anchor: Option<BlockId>, dir: &std::path::Path) -> Vec<Turn> {
    let mut records = HashMap::new();
    doc.for_each_block(|b| for (_, m) in b.text.spans() {
        if let Some(id) = &m.generated && !records.contains_key(id) && let Some(r) = Record::load(dir, id) { records.insert(id.clone(), r); }
    });
    rebuild(&pieces(doc, anchor, &records), &records)
}

pub fn rebuild(pieces: &[Piece], records: &HashMap<String, Record>) -> Vec<Turn> {
    // 先分段：连续的用户文字，或连续的同一次生成。没有文字的块（分隔线、折叠块的外壳）打不上标记，不算分界。
    let mut runs: Vec<(Option<&str>, Vec<&str>, bool)> = vec![];
    for p in pieces {
        let (id, text, pristine) = match p { Piece::Written(t) => (None, &**t, true), Piece::Generated { id, text, pristine } => (Some(*id), &**text, *pristine) };
        if id.is_none() && text.trim().is_empty() { continue; }
        match runs.last_mut() {
            Some((last, texts, clean)) if *last == id => { texts.push(text); *clean &= pristine; }
            _ => runs.push((id, vec![text], pristine)),
        }
    }
    let mut seen: HashMap<&str, usize> = HashMap::new();
    for (id, _, _) in &runs { if let Some(id) = id { *seen.entry(id).or_default() += 1; } }
    let mut asked: Vec<&str> = vec![];
    let mut turns = vec![];
    let push = |turns: &mut Vec<Turn>, role: Role, text: String| { if !text.trim().is_empty() { turns.push(Turn { role, text }); } };
    for (id, texts, pristine) in runs {
        let joined = texts.iter().map(|t| t.trim_end()).filter(|t| !t.trim().is_empty()).collect::<Vec<_>>().join("\n\n");
        let Some(id) = id else { push(&mut turns, Role::User, joined); continue };
        let record = records.get(id);
        if let Some(r) = record && !asked.contains(&id) { asked.push(id); push(&mut turns, Role::User, r.request.clone()); }
        let text = match record { Some(r) if pristine && seen[id] == 1 => r.raw.clone(), _ => joined };
        push(&mut turns, Role::Assistant, text);
    }
    turns
}

#[cfg(test)]
mod tests {
    use super::*;

    fn show(t: &[Turn]) -> Vec<String> { t.iter().map(|x| format!("{}:{}", x.role.as_str(), x.text)).collect() }

    #[test]
    fn runs_requests_and_raw() {
        let mut rec = HashMap::new();
        let r = |raw: &str, request: &str| Record { raw: raw.into(), request: request.into(), ..Default::default() };
        rec.insert("g1".to_owned(), r("<think>想</think>甲\n\n乙", "写两段"));
        rec.insert("g2".to_owned(), r("丙原文", ""));
        rec.insert("g3".to_owned(), r("丁原文", "再写"));
        let g = |id, text, pristine| Piece::Generated { id, text: Cow::Borrowed(text), pristine };
        let w = |t| Piece::Written(Cow::Borrowed(t));
        let pieces = [
            w("开头"), w(""), w("第二段"),
            g("g1", "甲", true), g("g1", "乙", true),
            g("g2", "丙改过", false),
            g("g3", "丁一", true), w("插一句"), g("g3", "丁二", true),
            g("missing", "无记录", true),
        ];
        assert_eq!(show(&rebuild(&pieces, &rec)), [
            "user:开头\n\n第二段", "user:写两段", "assistant:<think>想</think>甲\n\n乙",
            "assistant:丙改过",
            // 被拆开的生成：要求只放一次，两段都用现在的文字。
            "user:再写", "assistant:丁一", "user:插一句", "assistant:丁二",
            "assistant:无记录",
        ]);
    }

    #[test]
    fn reads_the_document_up_to_the_anchor() {
        let doc = fdl_core::docxml::parse("<title>t</title><p>开头</p><p><span gen=\"g\">甲</span></p><p><span gen=\"g\">乙</span>改<p>子</p></p><p>锚点后</p>").unwrap();
        let mut rec = HashMap::new();
        rec.insert("g".to_owned(), Record { raw: "原文".into(), request: "要求".into(), hash: hash("甲\n乙"), ..Default::default() });
        let anchor = doc.blocks[2].id;
        let turns = |rec: &HashMap<String, Record>| show(&rebuild(&pieces(&doc, Some(anchor), rec), rec));
        // 第二块里多了自己打的「改」：全文对不上哈希，用现在的文字；锚点的子块也算前文。
        assert_eq!(turns(&rec), ["user:开头", "user:要求", "assistant:甲\n\n乙改", "user:子"]);
        rec.get_mut("g").unwrap().hash = hash("甲\n乙改");
        assert_eq!(turns(&rec), ["user:开头", "user:要求", "assistant:原文", "user:子"]);
        assert_eq!(pieces(&doc, None, &rec).len(), 5);
    }

    #[test]
    fn inline_objects_read_as_text() {
        // @文档 发标题、公式发 LaTeX，不发 U+FFFC。
        let doc = fdl_core::docxml::parse("<title>t</title><p>见<cite type=\"doc\" doc-id=\"a\" file-type=\"docx\">笔记</cite>和<latex>x^2</latex></p>").unwrap();
        assert_eq!(show(&rebuild(&pieces(&doc, None, &HashMap::new()), &HashMap::new())), ["user:见笔记和x^2"]);
    }

    #[test]
    fn textless_blocks_do_not_split_a_generation() {
        // 分隔线、折叠块的外壳没有文字，打不上标记：不该把一次生成拆成两段。
        let doc = fdl_core::docxml::parse("<title>t</title><p><span gen=\"g\">甲</span></p><hr/><callout emoji=\"📦\"><p folded=\"true\"><span gen=\"g\">卡</span><p><span gen=\"g\">行</span></p></p></callout><p><span gen=\"g\">乙</span></p>").unwrap();
        let mut rec = HashMap::new();
        rec.insert("g".to_owned(), Record { raw: "原文".into(), request: "要求".into(), hash: hash(&doc.gen_text("g")), ..Default::default() });
        assert_eq!(show(&rebuild(&pieces(&doc, None, &rec), &rec)), ["user:要求", "assistant:原文"]);
    }
}
