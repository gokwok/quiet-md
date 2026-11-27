use super::*;

fn para(s: &str) -> Block {
    Block::new(BlockKind::Text, RichText::plain(s))
}

fn obj(kind: InlineKind) -> RichText {
    let mut t = RichText::default();
    t.push(crate::editor::core::EQUATION, Marks { inline: Some(crate::docxml::to_inline(kind)), ..Default::default() });
    t
}

fn with(text: &str, kind: InlineKind, tail: &str) -> Block {
    let mut t = RichText::plain(text);
    t.append(&obj(kind));
    t.append(&RichText::plain(tail));
    Block::new(BlockKind::Text, t)
}

fn video() -> Block {
    Block::new(BlockKind::File(FileBlock { src_kind: "path".into(), src: "assets/clip one.mp4".into(), name: "海边".into(), view: FileView::Preview }), RichText::default())
}

fn at(y: i64, m: u32, d: u32, h: u32, mi: u32) -> i64 {
    crate::time::local_to_unix(y, m, d, h, mi) * 1000
}

/// 读出再原样写回。
fn again(blocks: &[Block]) -> (String, Vec<Block>, Refs) {
    let mut refs = Refs::new();
    let text = read("d", blocks, &mut refs);
    let back = write("d", &text, blocks, &mut refs).unwrap_or_else(|e| panic!("{e}\n{text}"));
    (text, back, refs)
}

fn reminders(bs: &[Block]) -> Vec<InlineReminder> {
    bs.iter().flat_map(|b| b.text.runs.iter().filter_map(|r| r.marks.inline.as_ref().and_then(Inline::reminder)).copied().collect::<Vec<_>>()).collect()
}

#[test]
fn video_round_trips() {
    let blocks = vec![para("前面"), video()];
    let (text, back, refs) = again(&blocks);
    assert_eq!(text, "[b1] 前面\n\n[b2] <video src=\"assets/clip one.mp4\">海边</video>");
    assert_eq!(back, blocks);
    // 改了说明：还是那个视频块，来源和预览视图都在。
    let edited = text.replace("海边", "海边日落");
    let back = write("d", &edited, &blocks, &mut refs.clone()).unwrap();
    assert_eq!(back[1].id, blocks[1].id);
    let BlockKind::File(f) = &back[1].kind else { panic!("{:?}", back[1].kind) };
    assert_eq!((f.src.as_str(), f.name.as_str(), f.view), ("assets/clip one.mp4", "海边日落", FileView::Preview));
    // 新写的视频。
    let back = write("d", "<video src=\"assets/a.MOV\">旅行</video>", &blocks, &mut Refs::new()).unwrap();
    let BlockKind::File(f) = &back[0].kind else { panic!("{:?}", back[0].kind) };
    assert_eq!((f.src.as_str(), f.name.as_str(), f.view, f.src_kind.as_str()), ("assets/a.MOV", "旅行", FileView::Preview, "path"));
    // 段落里的视频（文本视图）也不丢。
    let inline = vec![with("看 ", InlineKind::File(FileBlock { src_kind: "path".into(), src: "assets/v.m4v".into(), name: "v.m4v".into(), view: FileView::Card }), " 这段")];
    let (text, back, refs) = again(&inline);
    assert_eq!(text, "[b1] 看 <video src=\"assets/v.m4v\">v.m4v</video> 这段");
    assert_eq!(back, inline);
    let back = write("d", &text.replace("这段", "那段"), &inline, &mut refs.clone()).unwrap();
    assert_eq!(back[0].text.runs[1].marks.inline, inline[0].text.runs[1].marks.inline);
}

#[test]
fn file_round_trips() {
    let file = |kind: &str, src: &str| Block::new(BlockKind::File(FileBlock { src_kind: kind.into(), src: src.into(), name: "报告 <终版>.pdf".into(), view: FileView::Card }), RichText::default());
    let blocks = vec![file("path", "assets/r.pdf"), file("token", "boxcn123")];
    let (text, back, _) = again(&blocks);
    assert_eq!(text, "[b1] <file src=\"assets/r.pdf\">报告 &lt;终版&gt;.pdf</file>\n\n[b2] <file token=\"boxcn123\">报告 &lt;终版&gt;.pdf</file>");
    assert_eq!(back, blocks);
    let back = write("d", "<file src=\"assets/x.zip\"></file>\n\n<file src=\"assets/y.mp4\"/>", &[], &mut Refs::new()).unwrap();
    let names: Vec<_> = back.iter().map(|b| match &b.kind { BlockKind::File(f) => (f.name.clone(), f.view), k => panic!("{k:?}") }).collect();
    assert_eq!(names, [("x.zip".to_string(), FileView::Card), ("y.mp4".to_string(), FileView::Preview)]);
}

#[test]
fn reminders_round_trip() {
    let timed = InlineReminder { expire: at(2026, 11, 28, 9, 0), notify: at(2026, 11, 28, 8, 45), should_notify: true, whole_day: false };
    let day = at(2026, 11, 29, 0, 0);
    let whole = InlineReminder { expire: day + 86_400_000 - 1000, notify: day + 9 * 3_600_000 - 86_400_000, should_notify: false, whole_day: true };
    let blocks = vec![with("开会 ", InlineKind::Reminder(timed), ""), with("交稿 ", InlineKind::Reminder(whole), " 之前")];
    let (text, back, refs) = again(&blocks);
    assert_eq!(text, "[b1] 开会 <reminder at=\"2026-11-28 09:00\" notify=\"true\"/>\n\n[b2] 交稿 <reminder at=\"2026-11-29\" notify=\"false\"/> 之前");
    assert_eq!(back, blocks);
    // 改了别的字：提醒还是原来那个（提前 15 分钟、提前一天这些都在）。
    let back = write("d", &text.replace("开会", "周会").replace("交稿", "交初稿"), &blocks, &mut refs.clone()).unwrap();
    assert_eq!(reminders(&back), [timed, whole]);
    // 新写的：定时到点提醒，全天当天 09:00 提醒；notify 不写是提醒。
    let back = write("d", "a <reminder at=\"2026-12-01 18:30\"/>\n\nb <reminder at=\"2026-12-02\" notify=\"false\"/>", &[], &mut Refs::new()).unwrap();
    let day = at(2026, 12, 2, 0, 0);
    assert_eq!(reminders(&back), [
        InlineReminder { expire: at(2026, 12, 1, 18, 30), notify: at(2026, 12, 1, 18, 30), should_notify: true, whole_day: false },
        InlineReminder { expire: day + 86_400_000 - 1000, notify: day + 9 * 3_600_000, should_notify: false, whole_day: true },
    ]);
    // 日期写坏了：照文字留着。
    let back = write("d", "c <reminder at=\"2026-13-01\"/>", &[], &mut Refs::new()).unwrap();
    assert_eq!(back[0].text.text, "c <reminder at=\"2026-13-01\"/>");
}

#[test]
fn keep_blocks_survive_or_go() {
    let grid = Block::new(BlockKind::Grid, RichText::default())
        .with_children(vec![Block::new(BlockKind::Column { ratio: 0.5 }, RichText::default()).with_children(vec![para("左")]), Block::new(BlockKind::Column { ratio: 0.5 }, RichText::default()).with_children(vec![para("右")])]);
    let countdown = Block::new(BlockKind::Countdown(Countdown { duration: 61., ..Default::default() }), RichText::default());
    let blocks = vec![para("一"), grid, countdown, para("二")];
    let (text, back, refs) = again(&blocks);
    assert_eq!(text, "[b1] 一\n\n<keep ref=\"b2\"/>\n\n<keep ref=\"b3\"/>\n\n[b4] 二");
    assert_eq!(back, blocks);
    // 去掉倒计时、把分栏挪到最后：分栏原样，倒计时没了。
    let back = write("d", "[b1] 一\n\n[b4] 二\n\n<keep ref=\"b2\"/>", &blocks, &mut refs.clone()).unwrap();
    assert_eq!(back, vec![blocks[0].clone(), blocks[3].clone(), blocks[1].clone()]);
    // 列表里的占位块。
    let nested = vec![Block::new(BlockKind::Bullet, RichText::plain("项")).with_children(vec![blocks[2].clone()])];
    let (text, back, _) = again(&nested);
    assert_eq!(text, "[b1] - 项\n    <keep ref=\"b2\"/>");
    assert_eq!(back, nested);
    // 写在一行字中间不行。
    assert_eq!(write("d", "看 <keep ref=\"b2\"/> 这里", &blocks, &mut refs.clone()), Err(Error::KeepInline("b2".into())));
}

#[test]
fn short_ids_and_refs() {
    let list = Block::new(BlockKind::Bullet, RichText::plain("父")).with_children(vec![Block::new(BlockKind::Bullet, RichText::plain("子"))]);
    let quote = Block::new(BlockKind::Quote, RichText::default()).with_children(vec![para("引")]);
    let blocks = vec![Block::new(BlockKind::Heading { level: 2, seq_auto: false }, RichText::plain("标题")), list, quote, para("")];
    let mut refs = Refs::new();
    let text = read("d", &blocks, &mut refs);
    // 照文档顺序编号，父块在子块前面。
    assert_eq!(text, "[b1] ## 标题\n\n[b2] - 父\n    [b3] - 子\n\n[b4] > [b5] 引\n\n[b6]");
    assert_eq!(refs.resolve("b3"), Some(("d", blocks[1].children[0].id)));
    assert_eq!(refs.resolve("[b4]"), Some(("d", blocks[2].id)));
    assert_eq!(refs.short("d", blocks[3].id).as_deref(), Some("b6"));
    assert_eq!(refs.resolve("b99"), None);
    // 再读一遍号不变；别的文档接着编。
    assert_eq!(read("d", &blocks, &mut refs), text);
    assert_eq!(read("e", &[para("x")], &mut refs), "[b7] x");
    assert_eq!(write("d", &text, &blocks, &mut refs.clone()).unwrap(), blocks);
    // 短号不对。
    assert_eq!(write("d", "[b99] x", &blocks, &mut refs.clone()), Err(Error::Unknown("b99".into())));
    assert_eq!(write("d", "[b7] x", &blocks, &mut refs.clone()), Err(Error::Unknown("b7".into())));
    assert_eq!(write("d", "[b1] a\n\n[b1] b", &blocks, &mut refs.clone()), Err(Error::Twice("b1".into())));
    assert_eq!(write("d", "[b1] a", &blocks[1..], &mut refs.clone()), Err(Error::Gone("b1".into())));
    assert!(Error::Unknown("b99".into()).to_string().contains("b99"));
}

#[test]
fn edits_keep_ids_and_hidden_attrs() {
    let mut p = para("旧的");
    p.align = Align::Center;
    p.bg = Some(ColorName("light-yellow".into()));
    let mut img = Block::new(BlockKind::Image { src_kind: "path".into(), src: "assets/a b.png".into(), width: Some(120.), height: Some(80.), caption: Some("图".into()), crop: None }, RichText::default());
    img.align = Align::Center;
    let callout = Block::new(BlockKind::Callout { emoji: "💡".into(), bg: Some(ColorName("light-blue".into())), border: None, text_color: None }, RichText::default()).with_children(vec![para("提示")]);
    let ol = |s: &str| Block::new(BlockKind::Ordered { seq: None }, RichText::plain(s));
    let blocks = vec![p, img, callout, ol("一"), ol("二")];
    let (text, back, refs) = again(&blocks);
    assert_eq!(text, "[b1] 旧的\n\n[b2] ![图](assets/a%20b.png)\n\n[b3] > 💡\n> [b4] 提示\n\n[b5] 1. 一\n[b6] 2. 二");
    assert_eq!(back, blocks);
    let edited = text.replace("旧的", "新的").replace("![图]", "![新图]").replace("提示", "新提示").replace("二", "贰");
    let back = write("d", &edited, &blocks, &mut refs.clone()).unwrap();
    assert_eq!(back.len(), 5);
    assert_eq!((back[0].id, back[0].align, &back[0].bg, back[0].text.text.as_str()), (blocks[0].id, Align::Center, &blocks[0].bg, "新的"));
    assert_eq!(back[1].kind, BlockKind::Image { src_kind: "path".into(), src: "assets/a b.png".into(), width: Some(120.), height: Some(80.), caption: Some("新图".into()), crop: None });
    assert_eq!((back[2].id, &back[2].kind), (blocks[2].id, &blocks[2].kind));
    assert_eq!(back[2].children.len(), 1);
    assert_eq!((back[2].children[0].id, back[2].children[0].text.text.as_str()), (blocks[2].children[0].id, "新提示"));
    assert_eq!(back[3], blocks[3]);
    assert_eq!((back[4].id, &back[4].kind, back[4].text.text.as_str()), (blocks[4].id, &BlockKind::Ordered { seq: None }, "贰"));
}

#[test]
fn new_blocks_and_deletions() {
    let blocks = vec![para("留"), para("删")];
    let mut refs = Refs::new();
    read("d", &blocks, &mut refs);
    let back = write("d", "# 新标题\n\n[b1] 留\n\n- [ ] 新待办 #进度", &blocks, &mut refs).unwrap();
    assert_eq!(back.len(), 3);
    assert_eq!(back[1], blocks[0]);
    assert_eq!(back[0].kind, BlockKind::Heading { level: 1, seq_auto: false });
    assert_eq!(back[2].kind, BlockKind::Todo { done: false });
    assert_eq!(back[2].text.runs.last().and_then(|r| r.marks.inline.as_ref()).and_then(Inline::tag), Some("进度"));
    assert!(back.iter().all(|b| untag(b.id).is_none()));
}

#[test]
fn inline_objects_and_marks_round_trip() {
    let mut t = RichText::plain("见 ");
    t.append(&obj(InlineKind::DocLink(InlineDocLink { id: "doc-1".into(), title: "周报".into() })));
    t.push(" 和 ", Marks::default());
    t.append(&obj(InlineKind::Tag("项目".into())));
    t.push("，", Marks::default());
    t.append(&obj(InlineKind::Equation("a_b".into())));
    t.push("x", Marks { underline: true, ..Default::default() });
    t.push("；价格 $5 和 #1 号", Marks::default());
    let blocks = vec![Block::new(BlockKind::Text, t), para("第一行\n第二行")];
    let (text, back, _) = again(&blocks);
    assert_eq!(text, "[b1] 见 <doc id=\"doc-1\">周报</doc> 和 #项目，$a_b$<u>x</u>；价格 \\$5 和 \\#1 号\n\n[b2] 第一行<br>第二行");
    assert_eq!(back, blocks);
}

#[test]
fn unknown_tags_stay_text() {
    let back = write("d", "<foo>bar</foo> <video>无来源</video> <doc>无 id</doc> <keep/> 1 < 2", &[], &mut Refs::new()).unwrap();
    assert_eq!(back.len(), 1);
    assert_eq!(back[0].text.text, "<foo>bar</foo> <video>无来源</video> <doc>无 id</doc> <keep/> 1 < 2");
    assert!(back[0].text.runs.iter().all(|r| r.marks.inline.is_none()));
    // 代码里的标签不动。
    let back = write("d", "`<video src=\"a.mp4\">x</video>`\n\n```\n[b9] <keep ref=\"b9\"/>\n```", &[], &mut Refs::new()).unwrap();
    assert_eq!(back[0].text.text, "<video src=\"a.mp4\">x</video>");
    assert_eq!(back[1].text.text, "[b9] <keep ref=\"b9\"/>");
}

/// DocxXML 全部块类型的 fixture：读出再写回，一块不差。
#[test]
fn all_blocks_round_trip() {
    let doc = crate::docxml::parse(include_str!("../../tests/fixtures/all-blocks.xml")).unwrap();
    let (text, back, _) = again(&doc.blocks);
    let bad: Vec<String> = doc.blocks.iter().zip(&back).filter(|(a, b)| a != b).map(|(a, b)| format!("{a:?}\n  →  {b:?}")).collect();
    assert!(bad.is_empty() && back.len() == doc.blocks.len(), "{} / {}\n{}\n\n{text}", back.len(), doc.blocks.len(), bad.join("\n\n"));
    assert!(text.contains("<keep ref="), "{text}");
}

#[test]
fn nesting_round_trips() {
    let item = |k: BlockKind, s: &str| Block::new(k, RichText::plain(s));
    let code = Block::new(BlockKind::Code { lang: "rust".into(), caption: None, wrap: false }, RichText::plain("fn main() {\n    x();\n}"));
    let ol = BlockKind::Ordered { seq: None };
    let blocks = vec![
        item(BlockKind::Bullet, "父").with_children(vec![item(BlockKind::Bullet, "子一"), item(BlockKind::Bullet, "子二").with_children(vec![item(BlockKind::Bullet, "孙")]), code.clone()]),
        item(ol.clone(), "甲").with_children(vec![item(ol.clone(), "甲一"), item(ol.clone(), "甲二")]),
        item(BlockKind::Todo { done: true }, "做完").with_children(vec![para("说明"), para("第二段")]),
        para("段落").with_children(vec![para("段落的子块"), item(BlockKind::Bullet, "段落下的列表")]),
        item(BlockKind::Heading { level: 2, seq_auto: false }, "标题").with_children(vec![para("标题的子块")]),
        Block::new(BlockKind::Callout { emoji: String::new(), bg: None, border: None, text_color: None }, RichText::default()).with_children(vec![para("没 emoji 的高亮块")]),
    ];
    let (text, back, mut refs) = again(&blocks);
    assert_eq!(back, blocks, "{text}");
    // 再来两遍也不变（代码不会越缩越多）。
    let text2 = read("d", &back, &mut refs);
    assert_eq!(text2, text);
    assert_eq!(write("d", &text2, &back, &mut refs).unwrap(), blocks);
    // 子项里加一段没短号的新段落：父块留着原标识，原来的子块都在，新段落排在后面。
    let edited = text.replace("[b2] - 子一", "[b2] - 子一\n\n      新的一段");
    let back = write("d", &edited, &blocks, &mut refs.clone()).unwrap();
    assert_eq!(back[1..], blocks[1..]);
    assert_eq!(back[0].id, blocks[0].id);
    let kids: Vec<_> = back[0].children.iter().map(|c| c.id).collect();
    assert_eq!(kids.len(), 3, "{edited}\n{:?}", back[0].children);
    assert_eq!(kids[1..], [blocks[0].children[1].id, blocks[0].children[2].id]);
    assert_eq!(back[0].children[0].children.len(), 1);
    assert_eq!(back[0].children[0].children[0].text.text, "新的一段");
}
