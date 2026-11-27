use std::sync::Arc;

use serde_json::Value;

use super::*;

fn shelf() -> Shelf {
    let doc = |id: &str, title: &str, text: &str, modified| Doc { id: id.into(), title: title.into(), folder: None, modified, text: Arc::from(text) };
    Shelf { space: "我的空间".into(), docs: vec![doc("a", "周报", "本周完成了导出功能。下周做同步。", 10), doc("b", "读书笔记", "《沉默的大多数》里写道……", 20)], now: Some((2026, 11, 27, 9, 30)), ..Default::default() }
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
        Reply { text: String::new(), calls: vec![call("c1", "search_documents", r#"{"query":"导出"}"#)] },
        Reply { text: "先读一下".into(), calls: vec![call("c2", "read_document", r#"{"id":"a"}"#)] },
        Reply { text: "本周完成了**导出**。".into(), calls: vec![] },
    ];
    let (text, asks, used) = drive(&mut core, &shelf(), "这周做了什么", script);
    assert_eq!(text, "本周完成了**导出**。");
    assert_eq!(used, ["搜索「导出」", "读取《周报》"]);
    assert_eq!(asks.len(), 3);
    assert!(asks.iter().all(|a| a["tools"].as_array().is_some_and(|t| t.len() == 3)));
    // 最后一份请求里：系统、用户、助手调用、工具结果、助手调用、工具结果。
    let msgs = asks[2]["messages"].as_array().unwrap();
    let roles: Vec<&str> = msgs.iter().map(|m| m["role"].as_str().unwrap()).collect();
    assert_eq!(roles, ["system", "user", "assistant", "tool", "assistant", "tool"]);
    assert!(msgs[0]["content"].as_str().unwrap().contains("《读书笔记》(id: b)"));
    assert_eq!(msgs[2]["content"], Value::Null);
    assert_eq!(msgs[3]["tool_call_id"], "c1");
    assert!(msgs[3]["content"].as_str().unwrap().contains("id: a"));
    assert!(msgs[5]["content"].as_str().unwrap().contains("本周完成了导出功能"));
    let shaped = core.shape(&text, "g", false);
    assert!(!shaped.blocks.is_empty());
}

#[test]
fn agent_stops_offering_tools_after_rounds() {
    let mut core = agent::Agent::new(agent::ROUNDS);
    let mut script: Vec<Reply> = (0..agent::ROUNDS).map(|i| Reply { text: String::new(), calls: vec![call(&format!("c{i}"), "list_documents", "{}")] }).collect();
    // 到了轮数还要调：不再执行，它说的就是回答。
    script.push(Reply { text: "就这些".into(), calls: vec![call("x", "list_documents", "{}")] });
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
    s.attached = vec![focus(Some("下周做同步。")), Attached::Folder { name: "工作".into(), docs: vec!["b".into()] }];
    let both = agent::system(&s);
    assert!(both.contains("里的这些内容") && both.contains("下周做同步。") && both.contains("文件夹「工作」") && both.contains("《读书笔记》(id: b)"));
    assert!(tavern::material(&shelf()).contains("2 篇文档"));
    s.attached = vec![focus(None)];
    assert!(tavern::material(&s).contains("本周完成了导出功能"));
    // 附带的那篇读的是 Markdown。
    assert!(tools::run(&s, &call("1", "read_document", r#"{"id":"a"}"#)).contains("# 周报"));
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
fn reading_pages_long_documents() {
    let mut s = shelf();
    s.docs[0].text = Arc::from("字".repeat(tools::PAGE + 10));
    let first = tools::run(&s, &call("1", "read_document", r#"{"id":"a"}"#));
    assert!(first.contains(&format!("offset={}", tools::PAGE)));
    let rest = tools::run(&s, &call("1", "read_document", &format!(r#"{{"id":"a","offset":{}}}"#, tools::PAGE)));
    assert!(rest.contains("共 6010 字") && !rest.contains("offset="));
    assert!(tools::run(&s, &call("1", "read_document", r#"{"id":"zzz"}"#)).starts_with("错误"));
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
