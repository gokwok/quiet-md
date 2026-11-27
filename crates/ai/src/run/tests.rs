use std::collections::HashMap;
use std::sync::Arc;

use fdl_core::model::Document;

use serde_json::Value;

use super::*;

fn at(y: i64, m: u32, d: u32) -> u64 { fdl_core::time::local_to_unix(y, m, d, 10, 0) as u64 }

/// 一篇：快照里的那条和它的块（正文用 Markdown 写）。
fn mk(id: &str, title: &str, md: &str, parent: Option<&str>, tags: &[&str], modified: u64) -> (Doc, Document) {
    let doc = fdl_core::markdown::parse_file(md, title, None);
    let text: String = doc.blocks.iter().map(|b| b.text.plain_text()).collect::<Vec<_>>().join("\n");
    (Doc { id: id.into(), title: title.into(), parent: parent.map(Into::into), created: modified, modified, tags: tags.iter().map(|t| t.to_string()).collect(), text: Arc::from(text), ..Default::default() }, doc)
}

/// 「工作 / 周报」里一篇周报（带待办和提醒），顶层一篇读书笔记（@ 了周报，下面挂一篇子文档）。
fn shelf() -> Shelf {
    let (mut a, mut weekly) = mk("a", "周报", "# 本周\n\n本周完成了导出功能。\n\n- [ ] 写下周计划\n- [x] 发布 0.1.9\n\n下周做同步。", Some("f2"), &["工作"], at(2026, 11, 25));
    a.folder = Some("周报".into());
    // 「写下周计划」带一个 11 月 28 日 09:00 的提醒。
    let r = fdl_core::model::InlineReminder { expire: fdl_core::time::local_to_unix(2026, 11, 28, 9, 0) * 1000, notify: 0, should_notify: true, whole_day: false };
    weekly.blocks[2].text.push(fdl_core::editor::core::EQUATION, fdl_core::model::Marks { inline: Some(fdl_core::docxml::to_inline(fdl_core::model::InlineKind::Reminder(r))), ..Default::default() });
    let (mut b, notes) = mk("b", "读书笔记", "《沉默的大多数》里写道……\n\n读完再对照周报。", None, &["读书"], at(2026, 11, 10));
    b.links = vec![("a".into(), "读完再对照周报。".into())];
    let (c, sub) = mk("c", "子文档", "挂在读书笔记下面。", Some("b"), &[], at(2026, 11, 1));
    let docs: HashMap<String, Document> = [("a", weekly), ("b", notes), ("c", sub)].into_iter().map(|(k, v)| (k.to_string(), v)).collect();
    Shelf {
        space: "我的空间".into(),
        docs: vec![a, b, c],
        folders: vec![Folder { id: "f1".into(), name: "工作".into(), parent: None }, Folder { id: "f2".into(), name: "周报".into(), parent: Some("f1".into()) }],
        trash: vec![Trashed { id: "t".into(), title: "旧草稿".into(), deleted: at(2026, 11, 20), folder: false }],
        library: Library::new(move |id| docs.get(id).cloned()),
        now: Some((2026, 11, 27, 9, 30)),
        ..Default::default()
    }
}

/// 宿主的循环：照剧本一轮轮交回模型说的，记下每个请求体和执行过的工具。
fn drive(core: &mut dyn Core, shelf: &Shelf, input: &str, mut script: Vec<Reply>) -> (String, Vec<Value>, Vec<String>) {
    let (mut asks, mut used) = (vec![], vec![]);
    script.reverse();
    let mut act = core.begin(input, &[], shelf);
    loop {
        act = match act {
            Act::Ask(body) => { asks.push(body); core.reply(script.pop().expect("剧本说完了还在要")) }
            Act::Use(calls) => {
                let results = calls.into_iter().map(|c| { used.push(tools::label(shelf, &c)); let out = tools::run(shelf, &c); (c, out) }).collect();
                core.results(shelf, results)
            }
            Act::Finish(text) => return (text, asks, used),
        };
    }
}

fn call(id: &str, name: &str, args: &str) -> Call { Call { id: id.into(), name: name.into(), args: args.into() } }

#[test]
fn agent_loops_through_tools() {
    let mut core = agent::Agent::new(agent::ROUNDS);
    let script = vec![
        Reply { text: String::new(), calls: vec![call("c1", "search", r#"{"query":"导出"}"#)] },
        Reply { text: "先读一下".into(), calls: vec![call("c2", "read_document", r#"{"id":"a"}"#)] },
        Reply { text: "本周完成了**导出**。".into(), calls: vec![] },
    ];
    let (text, asks, used) = drive(&mut core, &shelf(), "这周做了什么", script);
    assert_eq!(text, "本周完成了**导出**。");
    assert_eq!(used, ["搜索「导出」", "读取《周报》"]);
    assert_eq!(asks.len(), 3);
    assert!(asks.iter().all(|a| a["tools"].as_array().is_some_and(|t| t.len() == 6)));
    // 最后一份请求里：系统、用户、助手调用、工具结果、助手调用、工具结果。
    let msgs = asks[2]["messages"].as_array().unwrap();
    let roles: Vec<&str> = msgs.iter().map(|m| m["role"].as_str().unwrap()).collect();
    assert_eq!(roles, ["system", "user", "assistant", "tool", "assistant", "tool"]);
    assert!(msgs[0]["content"].as_str().unwrap().contains("《读书笔记》(id: b)"));
    assert_eq!(msgs[2]["content"], Value::Null);
    assert_eq!(msgs[3]["tool_call_id"], "c1");
    assert!(msgs[3]["content"].as_str().unwrap().contains("id: a"));
    assert!(msgs[5]["content"].as_str().unwrap().contains("[b2] 本周完成了导出功能"));
    let shaped = core.shape(&text, "g", false);
    assert!(!shaped.blocks.is_empty());
}

#[test]
fn agent_stops_offering_tools_after_rounds() {
    let mut core = agent::Agent::new(agent::ROUNDS);
    let mut script: Vec<Reply> = (0..agent::ROUNDS).map(|i| Reply { text: String::new(), calls: vec![call(&format!("c{i}"), "list_folder", "{}")] }).collect();
    // 到了轮数还要调：不再执行，它说的就是回答。
    script.push(Reply { text: "就这些".into(), calls: vec![call("x", "list_folder", "{}")] });
    let (text, asks, used) = drive(&mut core, &shelf(), "列一下", script);
    assert_eq!(text, "就这些");
    assert_eq!(used.len(), agent::ROUNDS);
    assert!(asks.last().unwrap().get("tools").is_none());
}

fn focus(picked: Option<&str>) -> Attached {
    Attached::Doc(Focus { id: "a".into(), title: "周报".into(), markdown: "# 周报\n\n本周完成了导出功能。".into(), picked: picked.map(str::to_owned) })
}

#[test]
fn resources_fill_context() {
    let mut s = shelf();
    assert!(agent::system(&s).contains("没有附带资源"));
    s.attached = vec![focus(None)];
    let doc = agent::system(&s);
    assert!(doc.contains("《周报》(id: a)") && doc.contains("# 周报"));
    s.attached = vec![focus(Some("下周做同步。")), Attached::Folder { id: "f".into(), name: "工作".into(), docs: vec!["b".into()] }];
    let both = agent::system(&s);
    assert!(both.contains("里的这些内容") && both.contains("下周做同步。") && both.contains("文件夹「工作」") && both.contains("《读书笔记》(id: b)"));
    assert!(tavern::material(&shelf()).contains("3 篇文档"));
    s.attached = vec![focus(None)];
    assert!(tavern::material(&s).contains("本周完成了导出功能"));
    assert!(both.contains("文件夹「工作」(id: f)"));
}

/// 两个核心的请求里都有空间说明、助手的设定和附带的资源。
#[test]
fn both_cores_carry_guide_persona_and_resources() {
    let mut s = shelf();
    s.guide = "这里放读书笔记，用简体中文回答。".into();
    s.assistant = crate::tavern::assemble::Identity { name: "小书童".into(), description: "说话像一位老派的图书管理员。".into() };
    s.attached = vec![focus(None)];
    let (_, asks, _) = drive(&mut agent::Agent::new(agent::ROUNDS), &s, "总结", vec![Reply { text: "好".into(), calls: vec![] }]);
    let sys = asks[0]["messages"][0]["content"].as_str().unwrap().to_owned();
    assert!(sys.starts_with("你叫小书童") && sys.contains("老派的图书管理员") && sys.contains("用简体中文回答") && sys.contains("《周报》(id: a)"));
    let preset = crate::tavern::library::default_preset();
    let me = crate::tavern::assemble::Identity { name: "我".into(), description: String::new() };
    let (_, asks, _) = drive(&mut tavern::Tavern::new(preset, s.assistant.clone(), me, "m".into(), 1), &s, "总结", vec![Reply { text: "好".into(), calls: vec![] }]);
    let all = asks[0]["messages"].to_string();
    assert!(all.contains("小书童") && all.contains("老派的图书管理员") && all.contains("用简体中文回答") && all.contains("本周完成了导出功能"));
    assert!(asks[0].get("tools").is_none());
}

#[test]
fn tavern_asks_once_with_material() {
    let preset = crate::tavern::library::default_preset();
    let id = |n: &str| crate::tavern::assemble::Identity { name: n.into(), description: String::new() };
    let mut core = tavern::Tavern::new(preset, id("写作助手"), id("我"), "m".into(), 1);
    let (text, asks, used) = drive(&mut core, &shelf(), "你好", vec![Reply { text: "你好！".into(), calls: vec![] }]);
    assert_eq!((text.as_str(), asks.len(), used.len()), ("你好！", 1, 0));
    let all = asks[0]["messages"].to_string();
    assert!(all.contains("我的空间") && all.contains("你好"));
    assert!(asks[0].get("tools").is_none());
}

#[test]
fn chat_round_trips() {
    let dir = std::env::temp_dir().join(format!("fdl-ai-chat-{}", std::process::id()));
    let mut c = chat::Chat::new("c1".into(), 5);
    (c.assistant, c.core, c.permission) = (Some("helper".into()), Some(Kind::Tavern), Permission::Write);
    let blocks = Resource::Blocks { doc: "a".into(), blocks: vec!["b1".into(), "b2".into()], text: "- 第一段\n- 第二段".into() };
    c.items.push(chat::Item { role: Role::Assistant, text: "你好，我是小书童。".into(), at: 5, ..Default::default() });
    c.items.push(chat::Item { role: Role::User, text: "总结一下这几段".into(), resources: vec![blocks.clone(), Resource::Folder { id: "f".into() }], at: 6, ..Default::default() });
    c.items.push(chat::Item { role: Role::Assistant, text: "好的".into(), steps: vec!["读取《周报》".into()], at: 7, model: "m".into(), ..Default::default() });
    c.items.push(chat::Item { role: Role::Assistant, text: String::new(), at: 8, failed: Some("超时".into()), ..Default::default() });
    chat::save(&dir, &c).unwrap();
    assert_eq!(chat::load(&dir, "c1").as_ref(), Some(&c));
    assert_eq!(chat::list(&dir).len(), 1);
    assert_eq!(c.history().len(), 3);
    assert_eq!(c.name(), "总结一下这几段");
    assert_eq!(c.last_resources().unwrap()[0], blocks);
    // 只有开场白的不进历史。
    let mut empty = chat::Chat::new("c2".into(), 9);
    empty.items.push(chat::Item { role: Role::Assistant, text: "你好".into(), at: 9, ..Default::default() });
    chat::save(&dir, &empty).unwrap();
    assert_eq!(chat::list(&dir).len(), 1);
    chat::delete(&dir, "c1").unwrap();
    chat::delete(&dir, "c2").unwrap();
    assert!(chat::list(&dir).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// 旧格式：整段一个范围、选中的块记在 `picked`、一定写了核心。
#[test]
fn old_chats_still_load() {
    let v: Value = serde_json::from_str(r#"{"id":"o","title":"","scope":{"kind":"blocks","id":"a","blocks":["x"]},"core":"agent","created":1,"picked":"- 选中",
        "items":[{"role":"user","text":"这段讲什么","at":2},{"role":"assistant","text":"讲的是……","at":3}]}"#).unwrap();
    let c = chat::Chat::read(&v).unwrap();
    assert_eq!(c.core, Some(Kind::Agent));
    assert_eq!((c.assistant.as_deref(), c.permission), (None, Permission::Read));
    assert_eq!(c.items[0].resources, [Resource::Blocks { doc: "a".into(), blocks: vec!["x".into()], text: "- 选中".into() }]);
    let space: Value = serde_json::from_str(r#"{"id":"s","scope":{"kind":"space"},"core":"tavern","items":[{"role":"user","text":"hi"}]}"#).unwrap();
    let c = chat::Chat::read(&space).unwrap();
    assert!(c.items[0].resources.is_empty() && c.core == Some(Kind::Tavern));
}

#[test]
fn agent_settles_at_paragraphs() {
    let a = agent::Agent::new(agent::ROUNDS);
    assert_eq!(a.settled("第一段"), 0);
    assert_eq!(a.settled("第一段\n\n第二"), "第一段".len());
    // 没合上的代码块不算稳。
    assert_eq!(a.settled("开头\n\n```rust\nfn a() {}\n\n还在写"), "开头".len());
}

fn tool(s: &Shelf, name: &str, args: &str) -> String { tools::run(s, &call("t", name, args)) }

#[test]
fn list_folder_overview_folders_and_pages() {
    let s = shelf();
    let all = tool(&s, "list_folder", "{}");
    assert!(all.contains("空间「我的空间」：3 篇文档，2 个文件夹") && all.contains("#工作（1）") && all.contains("#读书（1）"), "{all}");
    assert!(all.contains("文件夹「工作」 id: f1 | 1 篇") && all.contains("《读书笔记》 id: b") && all.contains("下面有 1 篇子文档") && all.contains("《沉默的大多数》"));
    assert!(!all.contains("《周报》"));
    // 按名字找文件夹，往下展开一层。
    let work = tool(&s, "list_folder", r#"{"folder":"工作","depth":2}"#);
    assert!(work.contains("文件夹「工作」(id: f1)") && work.contains("  - 《周报》 id: a") && work.contains("标签：#工作"), "{work}");
    let weekly = tool(&s, "list_folder", r#"{"folder":"f2"}"#);
    assert!(weekly.contains("文件夹「工作 / 周报」") && weekly.contains("本周 本周完成了导出功能"), "{weekly}");
    assert!(tool(&s, "list_folder", r#"{"folder":"b"}"#).contains("《子文档》"));
    assert!(tool(&s, "list_folder", r#"{"folder":"没有这个"}"#).starts_with("错误：没有叫「没有这个」的文件夹"));
    // 空空间。
    assert!(tool(&Shelf::default(), "list_folder", "{}").contains("空间里还什么都没有"));
    // 分页。
    let mut many = Shelf::default();
    many.docs = (0..35).map(|i| Doc { id: format!("d{i}"), title: format!("第 {i} 篇"), modified: 100 - i, ..Default::default() }).collect();
    let first = tool(&many, "list_folder", "{}");
    assert!(first.contains("《第 0 篇》") && !first.contains("《第 30 篇》") && first.contains("cursor=\"30\""), "{first}");
    let rest = tool(&many, "list_folder", r#"{"cursor":"30"}"#);
    assert!(rest.contains("《第 34 篇》") && !rest.contains("cursor="));
    assert!(tool(&many, "list_folder", r#"{"cursor":"99"}"#).contains("超出了"));
    assert!(tool(&many, "list_folder", r#"{"cursor":"x"}"#).starts_with("错误：cursor"));
}

#[test]
fn search_across_and_inside_documents() {
    let s = shelf();
    let hit = tool(&s, "search", r#"{"query":"导出"}"#);
    assert!(hit.contains("共 1 篇") && hit.contains("《周报》 id: a") && hit.contains("…本周 本周完成了导出功能"), "{hit}");
    // 没有关键词，只按时间、标签、文件夹筛。
    let week = tool(&s, "search", r#"{"since":"2026-11-20","until":"2026-11-27"}"#);
    assert!(week.contains("符合条件的文档共 1 篇") && week.contains("《周报》"), "{week}");
    assert!(tool(&s, "search", r##"{"tag":"#读书"}"##).contains("《读书笔记》"));
    let work = tool(&s, "search", r#"{"folder":"工作"}"#);
    assert!(work.contains("《周报》") && !work.contains("《读书笔记》"));
    assert!(tool(&s, "search", r#"{"query":"笔记","title_only":true}"#).contains("《读书笔记》"));
    assert!(tool(&s, "search", r#"{"query":"同步","title_only":true}"#).starts_with("没有"));
    assert!(tool(&s, "search", r#"{"since":"上周"}"#).starts_with("错误：since"));
    let recent = tool(&s, "search", r#"{"sort":"title","limit":2}"#);
    assert!(recent.contains("共 3 篇，列出前 2 篇"));
    // 一篇里找：命中的块和前后各一块，带块号。
    let inside = tool(&s, "search", r#"{"doc":"a","query":"下周","context":1}"#);
    assert!(inside.contains("有 2 块含「下周」") && inside.contains("[b") && inside.contains("- [ ] 写下周计划") && inside.contains("下周做同步。"), "{inside}");
    assert!(tool(&s, "search", r#"{"doc":"a"}"#).starts_with("错误：在一篇文档里找要给 query"));
    assert!(tool(&s, "search", r#"{"doc":"a","query":"火星"}"#).contains("没有「火星」"));
    assert!(tool(&s, "search", r#"{"doc":"zzz","query":"x"}"#).starts_with("错误：没有标识为 zzz"));
}

#[test]
fn read_document_pages_outlines_and_space_md() {
    let mut s = shelf();
    let all = tool(&s, "read_document", r#"{"id":"a"}"#);
    assert!(all.contains("《周报》 id: a | 文件夹：工作 / 周报 | 标签：#工作") && all.contains("共 5 块"), "{all}");
    assert!(all.contains("[b1] # 本周\n\n[b2] 本周完成了导出功能。\n\n[b3] - [ ] 写下周计划<reminder at=\"2026-11-28 09:00\" notify=\"true\"/>\n[b4] - [x] 发布 0.1.9"), "{all}");
    assert!(!all.contains("还有"));
    // 分页：照块数，接着读用 from_block（第几块或块号）。
    let page = tool(&s, "read_document", r#"{"id":"a","limit":2}"#);
    assert!(page.contains("第 1–2 块") && page.contains("[还有 3 块没读，接着读用 from_block=2]"), "{page}");
    assert!(tool(&s, "read_document", r#"{"id":"a","from_block":2}"#).starts_with("《周报》") && tool(&s, "read_document", r#"{"id":"a","from_block":"2"}"#).contains("第 3–5 块"));
    assert!(tool(&s, "read_document", r#"{"id":"a","from_block":"b4"}"#).contains("第 4–5 块"));
    assert!(tool(&s, "read_document", r#"{"id":"a","from_block":9}"#).contains("这篇只有 5 块"));
    assert!(tool(&s, "read_document", r#"{"id":"a","from_block":"b99"}"#).starts_with("错误：没有块 b99"));
    assert!(tool(&s, "read_document", r#"{"id":"b","from_block":"b1"}"#).contains("不在这篇里"));
    assert_eq!(tool(&s, "read_document", r#"{"id":"a","outline":true}"#).lines().last(), Some("[b1] # 本周"));
    assert!(tool(&s, "read_document", r#"{"id":"zzz"}"#).starts_with("错误：没有标识为 zzz"));
    // 读得出标识但读不出正文（比如还没下载）。
    s.docs.push(Doc { id: "cloud".into(), title: "云端".into(), ..Default::default() });
    assert!(tool(&s, "read_document", r#"{"id":"cloud"}"#).contains("现在读不出来"));
    // SPACE.md。
    assert!(tool(&s, "read_document", r#"{"id":"SPACE.md"}"#).contains("还没有 SPACE.md"));
    s.guide = format!("# 空间说明\n\n{}", "长".repeat(agent::GUIDE + 50));
    assert!(tool(&s, "read_document", r#"{"id":"space.md"}"#).ends_with(&"长".repeat(agent::GUIDE + 50)));
    let sys = agent::system(&s);
    assert!(sys.contains("read_document(\"SPACE.md\")") && !sys.contains(&"长".repeat(agent::GUIDE)), "系统提示词里只放开头");
    s.guide = "短说明".into();
    assert!(!agent::system(&s).contains("太长"));
}

#[test]
fn tasks_backlinks_and_trash() {
    let s = shelf();
    // 这次 run 里头一个报块号的就是 b1。
    let open = tool(&s, "list_tasks", "{}");
    assert!(open.contains("共 1 项") && open.contains("《周报》 id: a") && open.contains("- [ ] [b1] 写下周计划（提醒 2026-11-28 09:00）") && !open.contains("发布"), "{open}");
    assert!(tool(&s, "list_tasks", r#"{"status":"done"}"#).contains("] 发布 0.1.9") && tool(&s, "list_tasks", r#"{"status":"done"}"#).contains("- [x] [b"));
    assert!(tool(&s, "list_tasks", r#"{"status":"all"}"#).contains("共 2 项"));
    assert!(tool(&s, "list_tasks", r#"{"due_before":"2026-11-28 08:00"}"#).starts_with("没有"));
    assert!(tool(&s, "list_tasks", r#"{"due_before":"2026-11-29"}"#).contains("写下周计划"));
    assert!(tool(&s, "list_tasks", r#"{"folder":"读书"}"#).starts_with("错误：没有叫「读书」"));
    assert!(tool(&s, "list_tasks", r#"{"status":"later"}"#).starts_with("错误：status"));
    let back = tool(&s, "backlinks", r#"{"id":"a"}"#);
    assert!(back.contains("有 1 篇文档链接了《周报》") && back.contains("《读书笔记》 id: b") && back.contains("「读完再对照周报。」"), "{back}");
    assert!(tool(&s, "backlinks", r#"{"id":"b"}"#).starts_with("没有文档链接《读书笔记》"));
    assert!(tool(&s, "backlinks", r#"{"id":"zzz"}"#).starts_with("错误"));
    let trash = tool(&s, "list_trash", "{}");
    assert!(trash.contains("文档「旧草稿」 id: t | 删除于 2026-11-20"), "{trash}");
    assert!(tool(&Shelf::default(), "list_trash", "{}").contains("是空的"));
    assert!(tool(&s, "rm", "{}").starts_with("错误：没有叫 rm 的工具"));
}

/// 一次 run 里块号各工具一致：待办报的、在一篇里搜到的、读出来的都是同一个号，换个顺序先读别的也一样。
#[test]
fn block_ids_agree_across_tools() {
    let s = shelf();
    let first = tool(&s, "list_tasks", r#"{"status":"done"}"#);
    let short = first.split("[x] [").nth(1).and_then(|t| t.split(']').next()).unwrap().to_owned();
    assert!(tool(&s, "search", r#"{"doc":"a","query":"发布","context":0}"#).contains(&format!("[{short}] - [x] 发布")));
    assert!(tool(&s, "read_document", r#"{"id":"a"}"#).contains(&format!("[{short}] - [x] 发布")));
    let (doc, id) = s.refs.lock().unwrap().resolve(&short).map(|(d, i)| (d.to_owned(), i)).unwrap();
    assert_eq!(doc, "a");
    let blocks = s.document("a").unwrap();
    assert!(blocks.blocks.iter().any(|b| b.id == id && b.text.text.contains("发布")));
    // 克隆的 Shelf（宿主交给后台线程的那份）共用同一份短号。
    let clone = s.clone();
    assert!(tool(&clone, "read_document", r#"{"id":"a"}"#).contains(&format!("[{short}] - [x] 发布")));
}

/// 假模型跑一遍多轮：先看空间，一轮里同时搜索和读 SPACE.md，再读一篇，最后回答。
#[test]
fn agent_runs_read_tools_over_rounds() {
    let mut s = shelf();
    s.guide = "周报放在「工作 / 周报」。".into();
    let script = vec![
        Reply { text: String::new(), calls: vec![call("c1", "list_folder", "{}")] },
        Reply { text: String::new(), calls: vec![call("c2", "search", r#"{"since":"2026-11-20"}"#), call("c3", "read_document", r#"{"id":"SPACE.md"}"#)] },
        Reply { text: String::new(), calls: vec![call("c4", "read_document", r#"{"id":"a","outline":true}"#), call("c5", "list_tasks", "{}")] },
        Reply { text: "这周写了《周报》，还有一项待办：写下周计划（b3）。".into(), calls: vec![] },
    ];
    let (text, asks, used) = drive(&mut agent::Agent::new(agent::ROUNDS), &s, "我这周写了什么", script);
    assert!(text.contains("写下周计划"));
    assert_eq!(used, ["查看空间", "筛选文档", "读取 SPACE.md", "读取《周报》", "查看待办"]);
    assert_eq!(asks.len(), 4);
    let msgs = asks[3]["messages"].as_array().unwrap();
    let roles: Vec<&str> = msgs.iter().map(|m| m["role"].as_str().unwrap()).collect();
    assert_eq!(roles, ["system", "user", "assistant", "tool", "assistant", "tool", "tool", "assistant", "tool", "tool"]);
    assert_eq!(msgs[4]["tool_calls"].as_array().unwrap().len(), 2);
    assert!(msgs[6]["content"].as_str().unwrap().contains("周报放在"));
    assert!(msgs[9]["content"].as_str().unwrap().contains("- [ ] [b2] 写下周计划"));
    let sys = msgs[0]["content"].as_str().unwrap();
    assert!(sys.contains("list_tasks") && sys.contains("块号") && sys.contains("只读不写"));
}
