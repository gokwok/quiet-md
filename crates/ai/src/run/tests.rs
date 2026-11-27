use std::sync::Arc;

use serde_json::Value;

use super::*;

fn shelf() -> Shelf {
    let doc = |id: &str, title: &str, text: &str, modified| Doc { id: id.into(), title: title.into(), folder: None, modified, text: Arc::from(text) };
    Shelf { space: "我的空间".into(), docs: vec![doc("a", "周报", "本周完成了导出功能。下周做同步。", 10), doc("b", "读书笔记", "《沉默的大多数》里写道……", 20)], focus: None, now: Some((2026, 11, 27, 9, 30)) }
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
    let mut core = agent::Agent::new(Scope::Space);
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
    let mut core = agent::Agent::new(Scope::Space);
    let mut script: Vec<Reply> = (0..agent::ROUNDS).map(|i| Reply { text: String::new(), calls: vec![call(&format!("c{i}"), "list_documents", "{}")] }).collect();
    // 到了轮数还要调：不再执行，它说的就是回答。
    script.push(Reply { text: "就这些".into(), calls: vec![call("x", "list_documents", "{}")] });
    let (text, asks, used) = drive(&mut core, &shelf(), "列一下", script);
    assert_eq!(text, "就这些");
    assert_eq!(used.len(), agent::ROUNDS);
    assert!(asks.last().unwrap().get("tools").is_none());
}

#[test]
fn scope_fills_context() {
    let mut s = shelf();
    s.focus = Some(Focus { id: "a".into(), title: "周报".into(), markdown: "# 周报\n\n本周完成了导出功能。".into(), picked: Some("下周做同步。".into()) });
    let doc = agent::system(&Scope::Doc { id: "a".into() }, &s);
    assert!(doc.contains("《周报》(id: a)") && doc.contains("# 周报"));
    let picked = agent::system(&Scope::Blocks { id: "a".into(), blocks: vec!["x".into()] }, &s);
    assert!(picked.contains("选中了") && picked.contains("下周做同步。"));
    assert!(tavern::material(&Scope::Space, &s).contains("2 篇文档"));
    assert!(tavern::material(&Scope::Doc { id: "a".into() }, &s).contains("本周完成了导出功能"));
    // 进来的那篇读的是 Markdown。
    assert!(tools::run(&s, &call("1", "read_document", r#"{"id":"a"}"#)).contains("# 周报"));
}

#[test]
fn tavern_asks_once_with_material() {
    let preset = crate::tavern::library::default_preset();
    let id = |n: &str| crate::tavern::assemble::Identity { name: n.into(), description: String::new() };
    let mut core = tavern::Tavern::new(preset, id("写作助手"), id("我"), "m".into(), 1, Scope::Space);
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
    let mut c = chat::Chat::new("c1".into(), Scope::Blocks { id: "a".into(), blocks: vec!["b1".into(), "b2".into()] }, Kind::Agent, 5);
    c.picked = "- 第一段\n- 第二段".into();
    c.items.push(chat::Item { role: Role::User, text: "总结一下这几段".into(), at: 6, ..Default::default() });
    c.items.push(chat::Item { role: Role::Assistant, text: "好的".into(), steps: vec!["读取《周报》".into()], at: 7, model: "m".into(), ..Default::default() });
    c.items.push(chat::Item { role: Role::Assistant, text: String::new(), at: 8, failed: Some("超时".into()), ..Default::default() });
    chat::save(&dir, &c).unwrap();
    assert_eq!(chat::load(&dir, "c1").as_ref(), Some(&c));
    assert_eq!(chat::list(&dir).len(), 1);
    assert_eq!(c.history().len(), 2);
    assert_eq!(c.name(), "总结一下这几段");
    chat::delete(&dir, "c1").unwrap();
    assert!(chat::list(&dir).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn agent_settles_at_paragraphs() {
    let a = agent::Agent::new(Scope::Space);
    assert_eq!(a.settled("第一段"), 0);
    assert_eq!(a.settled("第一段\n\n第二"), "第一段".len());
    // 没合上的代码块不算稳。
    assert_eq!(a.settled("开头\n\n```rust\nfn a() {}\n\n还在写"), "开头".len());
}
