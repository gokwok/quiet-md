//! 用一份真实的社区预设检查：`FDL_TAVERN_PRESET=<预设.json> cargo test --release -p fdl_ai user_preset -- --ignored --nocapture`。
//! 预设是使用者自己的，不进仓库。`FDL_TAVERN_EXPECT=<请求.json>` 给出 SillyTavern 同样输入下实际发出的请求（或只有 messages）时，逐条比对。
//! 同样的输入：角色「对照助手」（描述见下）、人设「对照者」（描述放进提示词）、前文一问一答，再发本次要求。
use fdl_ai::tavern::assemble::{Identity, Request, Turn, assemble};
use fdl_ai::tavern::macros::{Env, Macros};
use fdl_ai::tavern::preset::{Preset, Role};
use serde_json::Value;

#[test]
#[ignore]
fn user_preset() {
    let Ok(path) = std::env::var("FDL_TAVERN_PRESET") else { panic!("设置 FDL_TAVERN_PRESET") };
    let bytes = std::fs::read(&path).unwrap();
    let preset = Preset::parse(&bytes).unwrap();

    // 原样往返：解析再导出，JSON 值完全相同。
    let original: Value = serde_json::from_slice(&bytes).unwrap();
    let back: Value = serde_json::from_slice(&preset.to_json()).unwrap();
    assert_eq!(original, back, "导出后内容变了");

    // 所有正则（包括关着的）都能编译。
    let scripts = preset.regex_scripts();
    let mut m = Macros::new(Env::default());
    for s in &scripts {
        if let Err(e) = s.compile(&mut m) { panic!("正则「{}」编译失败：{e}\n{}", s.name, s.find); }
    }
    println!("正则 {} 条全部能编译", scripts.len());
    // 和 JS 对照：`FDL_TAVERN_REGEX_CASES=<字符串数组.json>` 时，把每条脚本在每段输入上的结果写到 `FDL_TAVERN_REGEX_OUT`。
    if let (Ok(cases), Ok(out)) = (std::env::var("FDL_TAVERN_REGEX_CASES"), std::env::var("FDL_TAVERN_REGEX_OUT")) {
        let cases: Vec<String> = serde_json::from_slice(&std::fs::read(cases).unwrap()).unwrap();
        // 运行出错的写成 null。
        let results: Vec<Vec<Option<String>>> = scripts.iter().map(|s| {
            let t = std::time::Instant::now();
            let c = s.compile(&mut m).unwrap();
            let compiled = t.elapsed();
            let mut each = vec![];
            let r = cases.iter().map(|x| { let t = std::time::Instant::now(); let r = s.run_compiled(&c, x, &mut m).ok(); each.push(t.elapsed().as_millis()); r }).collect();
            if t.elapsed().as_millis() > 100 { println!("  每段（毫秒）：{each:?}"); }
            if t.elapsed().as_millis() > 5 { println!("正则「{}」编译 {compiled:?}，共 {:?}", s.name, t.elapsed()); }
            r
        }).collect();
        std::fs::write(out, serde_json::to_vec(&results).unwrap()).unwrap();
    }

    let history = vec![
        Turn { role: Role::User, text: "写一个雨夜的开头。".into() },
        Turn { role: Role::Assistant, text: "<thinking>先定基调</thinking>\n雨下了一整夜。\n\n她推开门。".into() },
    ];
    let a = assemble(&Request {
        preset: &preset, char: Identity { name: "对照助手".into(), description: "一个帮 {{user}} 写小说的助手。".into() },
        persona: Identity { name: "对照者".into(), description: "正在写一部短篇。".into() },
        history, input: "接着写，她看见了什么。".into(), model: "test".into(), model_context: None,
        extended_params: false, prefill: false, now: Some((2026, 11, 19, 20, 30)), seed: 7,
    });
    let out: Vec<Value> = a.messages.iter().map(|x| x.to_json()).collect();
    println!("{} 条消息，丢掉历史 {} 条", out.len(), a.dropped);
    for n in &a.notes { println!("注：{n}"); }
    if !a.unknown_macros.is_empty() { println!("没认出的宏：{:?}", a.unknown_macros); }
    println!("参数：{}", Value::Object(a.params.body(false)));
    if let Ok(dump) = std::env::var("FDL_TAVERN_DUMP") { std::fs::write(dump, serde_json::to_vec_pretty(&out).unwrap()).unwrap(); }
    for x in &out { assert!(!x["content"].as_str().unwrap().contains("{{"), "有没替换掉的宏：{x}"); }

    // 成形对照：`FDL_TAVERN_SHAPE=<{"raw": 原文, "lines": SillyTavern 显示出来的各行}>`，成形后各块的文字要和显示的各行一致。
    if let Ok(path) = std::env::var("FDL_TAVERN_SHAPE") {
        let v: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        let mut m = Macros::new(Env::default());
        let stored = fdl_ai::tavern::shape::store(&preset, v["raw"].as_str().unwrap(), &mut m);
        let shaped = fdl_ai::tavern::shape::shape(&preset, &stored, "g", &mut m);
        let ours: Vec<String> = shaped.blocks.iter().map(|b| b.text.text.trim().to_owned()).filter(|t| !t.is_empty()).collect();
        let want: Vec<String> = v["lines"].as_array().unwrap().iter().map(|l| l.as_str().unwrap().to_owned()).collect();
        for (i, (a, b)) in ours.iter().zip(&want).enumerate() { assert_eq!(a, b, "成形后第 {i} 块不一样"); }
        assert_eq!(ours.len(), want.len(), "块数不一样");
        println!("成形：{} 块，和 SillyTavern 显示的一致；{:?}", ours.len(), shaped.notes);

        // 流式：每多几个字成形一次「稳了的前缀」，除最后一块外都得是最终结果的前几块（不会先显示、后变样）。
        let raw = v["raw"].as_str().unwrap();
        let shaper = fdl_ai::tavern::shape::Shaper::new(&preset, &mut m);
        let text = |b: &fdl_core::model::Block| b.text.text.trim().to_owned();
        let (mut slowest, mut passes) = (std::time::Duration::ZERO, 0);
        let ends: Vec<usize> = raw.char_indices().map(|(i, _)| i).step_by(5).chain([raw.len()]).collect();
        for end in ends {
            let t = std::time::Instant::now();
            let prefix = &raw[..shaper.settled(&raw[..end])];
            let s = shaper.shape(&shaper.store(prefix, &mut m.clone()), "g", &mut m.clone());
            slowest = slowest.max(t.elapsed());
            passes += 1;
            let got: Vec<String> = s.blocks.iter().map(text).filter(|t| !t.is_empty()).collect();
            for (i, g) in got.iter().enumerate().take(got.len().saturating_sub(1)) { assert_eq!(Some(g), ours.get(i), "前 {end} 字节时第 {i} 块和最终的不一样"); }
            if let (Some(g), Some(o)) = (got.last(), ours.get(got.len().wrapping_sub(1))) { assert!(o.starts_with(g.as_str()) || g == o, "前 {end} 字节时最后一块不是最终的开头：{g:?} / {o:?}"); }
        }
        println!("流式成形 {passes} 次，最慢一次 {slowest:?}");
    }

    if let Ok(expect) = std::env::var("FDL_TAVERN_EXPECT") {
        let want: Value = serde_json::from_slice(&std::fs::read(expect).unwrap()).unwrap();
        let want = want.get("messages").unwrap_or(&want).as_array().unwrap().clone();
        for (i, (got, want)) in out.iter().zip(&want).enumerate() { assert_eq!(got, want, "第 {i} 条不一样"); }
        assert_eq!(out.len(), want.len(), "条数不一样");
    }
}
