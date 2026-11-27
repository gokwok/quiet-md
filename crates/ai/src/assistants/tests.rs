use super::*;
use crate::tavern::library::Card;

fn temp(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("fdl-ai-assistants-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// 没有文件是只有内置的；读得宽松：坏条目、重复的标识跳过，缺的字段按默认，不认识的核心按 Agent 跑、原样存回去；内置的排最前、删不了。
#[test]
fn load_save_and_builtin() {
    let t = temp("load");
    let a = Assistants::load(&t);
    assert_eq!(a.list.len(), 1);
    assert!(a.list[0].is_builtin() && a.list[0].core.kind() == Kind::Agent);

    std::fs::write(t.join(FILE), r#"{"assistants":[
        {"id":"muse","name":"缪斯","core":{"kind":"dream","config":{"rounds":3,"x":1}},"permission":"write","voice":"v1","model":{"provider":"p","model":"m"}},
        {"name":"没有标识"}, 7,
        {"id":"marginai","name":"我的 MarginAI","greeting":"你好"},
        {"id":"muse","name":"重复"}]}"#).unwrap();
    let mut a = Assistants::load(&t);
    assert_eq!(a.list.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), ["marginai", "muse"]);
    assert_eq!((a.list[0].name.as_str(), a.list[0].greeting.as_str()), ("我的 MarginAI", "你好"));
    let m = a.get("muse").unwrap().clone();
    assert_eq!((m.core.kind(), m.core.rounds(), m.permission, m.voice.as_deref(), m.character.as_deref()), (Kind::Agent, 3, Permission::Write, Some("v1"), None));
    assert_eq!(m.model.as_ref().map(|x| x.model.as_str()), Some("m"));
    a.save(&t).unwrap();
    let raw: Value = serde_json::from_slice(&std::fs::read(t.join(FILE)).unwrap()).unwrap();
    assert_eq!(raw["assistants"][1]["core"]["kind"], "dream");
    assert_eq!(Assistants::load(&t), a);

    // 换核心：另一种的配置留着。
    let muse = a.get_mut("muse").unwrap();
    muse.core.set_kind(Kind::Tavern);
    muse.core.set_preset("p1");
    muse.core.set_kind(Kind::Agent);
    assert_eq!((muse.core.preset(), muse.core.rounds()), ("p1", 3));

    assert!(!a.remove(BUILTIN) && a.list.len() == 2);
    assert!(a.duplicate("muse", "muse2", "缪斯 副本") && a.list[2].id == "muse2" && a.list[2].voice.as_deref() == Some("v1"));
    assert!(!a.duplicate("muse", "muse2", "又一份"));
    assert!(a.remove("muse"));
    // 用哪个：对话的 > 空间的 > 内置的；不在了的跳过。
    assert_eq!(a.resolve(&[Some("gone"), Some("muse2")]).id, "muse2");
    assert_eq!(a.resolve(&[None, Some("muse")]).id, BUILTIN);
    std::fs::remove_dir_all(&t).unwrap();
}

/// 写作设置里的助手卡片变成酒馆核心的助手：没动过的「写作助手」不变；同名同设定的合并；标识撞了另起；
/// 「/」卡片选上原来选中的那个；并过的不再并，后来加进来的卡片下次照样并。
#[test]
fn migrate_cards() {
    let t = temp("migrate");
    let mut s = Settings::default();
    s.chars.push(Card { id: "c-edit".into(), name: "编辑".into(), description: "挑错".into() });
    s.chars.push(Card { id: "marginai".into(), name: "撞名".into(), description: "".into() });
    s.char = "c-edit".into();
    s.preset = "p1".into();
    s.save(&t).unwrap();
    let mut a = Assistants::load(&t);
    a.list.push(Assistant { description: "挑错".into(), ..Assistant::new("old", "编辑", Core::agent()) });
    a.save(&t).unwrap();

    assert_eq!(migrate(&t).unwrap(), 1);
    let a = Assistants::load(&t);
    assert_eq!(a.list.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), ["marginai", "old", "marginai-m2"]);
    let x = a.get("marginai-m2").unwrap();
    assert_eq!((x.name.as_str(), x.core.kind(), x.core.preset()), ("撞名", Kind::Tavern, "p1"));
    assert_eq!(Settings::load(&t).assistant, "old");

    assert_eq!(migrate(&t).unwrap(), 0);
    let mut s = Settings::load(&t);
    s.chars.push(Card { id: "late".into(), name: "后来的".into(), description: "".into() });
    s.assistant = "marginai-m2".into();
    s.save(&t).unwrap();
    assert_eq!(migrate(&t).unwrap(), 1);
    assert!(Assistants::load(&t).get("late").is_some());
    // 已经选过的不改。
    assert_eq!(Settings::load(&t).assistant, "marginai-m2");
    std::fs::remove_dir_all(&t).unwrap();
}

/// 还在从 iCloud 下载时不并。
#[test]
fn skip_while_downloading() {
    let t = temp("waiting");
    std::fs::write(t.join(".settings.json.icloud"), b"").unwrap();
    assert_eq!(migrate(&t).unwrap(), 0);
    assert!(!t.join(FILE).exists());
    std::fs::remove_dir_all(&t).unwrap();
}
