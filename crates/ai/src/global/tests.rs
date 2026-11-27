use super::*;

fn temp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fdl-ai-global-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
fn card(id: &str, name: &str, d: &str) -> Card { Card { id: id.into(), name: name.into(), description: d.into() } }
fn names(cards: &[Card]) -> Vec<&str> { cards.iter().map(|c| c.name.as_str()).collect() }

/// 放哪：没 iCloud 是本机；有 iCloud 默认放 iCloud；选了本机就本机；选了 iCloud 但这会儿没有就先用本机。换位置复制过去、不盖那边已有的。
#[test]
fn place_and_move() {
    let t = temp("place");
    let (home, cloud) = (t.join("home"), t.join("cloud"));
    assert_eq!(dir_in(&home, None), home.join(DIR));
    assert_eq!(dir_in(&home, Some(&cloud)), cloud.join(DIR));
    assert_eq!(place_in(&home, Some(&cloud)), Place::ICloud);
    assert!(set_place_in(&home, None, Place::ICloud).is_err());

    std::fs::create_dir_all(cloud.join(DIR).join("presets")).unwrap();
    std::fs::write(cloud.join(DIR).join("imagine.json"), b"{\"style\":\"a\"}").unwrap();
    std::fs::write(cloud.join(DIR).join("presets").join("p1.json"), b"{}").unwrap();
    std::fs::write(cloud.join(DIR).join(".gone.json.icloud"), b"").unwrap();
    std::fs::create_dir_all(home.join(DIR)).unwrap();
    std::fs::write(home.join(DIR).join("imagine.json"), b"{\"style\":\"local\"}").unwrap();
    assert_eq!(set_place_in(&home, Some(&cloud), Place::Local).unwrap(), home.join(DIR));
    assert_eq!(chosen_in(&home), Some(Place::Local));
    assert_eq!(dir_in(&home, Some(&cloud)), home.join(DIR));
    // 本机已有的不盖；没有的复制过来；占位不复制；原处还在。
    assert_eq!(std::fs::read(home.join(DIR).join("imagine.json")).unwrap(), b"{\"style\":\"local\"}");
    assert!(home.join(DIR).join("presets").join("p1.json").is_file());
    assert!(!home.join(DIR).join(".gone.json.icloud").exists() && !home.join(DIR).join("gone.json").exists());
    assert!(cloud.join(DIR).join("imagine.json").is_file());
    // 选了 iCloud，iCloud 没了：先用本机。
    set_place_in(&home, Some(&cloud), Place::ICloud).unwrap();
    assert_eq!(dir_in(&home, Some(&cloud)), cloud.join(DIR));
    assert_eq!(dir_in(&home, None), home.join(DIR));

    assert!(!device_migrated_in(&home));
    set_device(&home, "device_migrated", true.into()).unwrap();
    assert!(device_migrated_in(&home) && chosen_in(&home) == Some(Place::ICloud));
    std::fs::remove_dir_all(&t).unwrap();
}

/// 两个空间并进全局库：同名同内容合并，同名不同内容加「（2）」；同标识不同内容的预设另起标识，引用跟着改；
/// 第一个空间的选择带过来，第二个的不覆盖；并过的不再并。
#[test]
fn merge_two_spaces() {
    let t = temp("merge");
    let (global, a, b) = (t.join("global"), t.join("a"), t.join("b"));
    let (ta, tb) = (a.join("tavern"), b.join("tavern"));

    let p1 = library::import(&ta, br#"{"name":"P1","prompts":[]}"#, "p1.json").unwrap();
    let mut s = Settings::default();
    s.chars.push(card("c-edit", "编辑", "挑错"));
    s.char = "c-edit".into();
    s.preset = p1.clone();
    s.last = "接着写".into();
    s.names.insert(p1.clone(), "P1".into());
    s.save(&ta).unwrap();
    let mut lib = Library::default();
    lib.styles.push(crate::imagine::Style { id: "s1".into(), name: "水墨".into(), positive: "ink".into(), negative: String::new() });
    lib.style = "s1".into();
    lib.characters.push(crate::imagine::Character { id: "lin".into(), name: "林".into(), look: "girl".into(), ..Default::default() });
    lib.quality = Some("best".into());
    lib.save(&ta).unwrap();

    // 第二个空间：同一份 P1，一份标识撞了但内容不同的，一样的「编辑」、不一样的「编辑」，不一样的「林」。
    library::import(&tb, br#"{"name":"P1","prompts":[]}"#, "p1.json").unwrap();
    library::write(&library::path(&tb, &p1), r#"{"name":"P1 改过","prompts":[]}"#.as_bytes()).unwrap();
    let p2 = library::import(&tb, br#"{"name":"P2","prompts":[]}"#, "p2.json").unwrap();
    let mut s = Settings::default();
    s.chars.push(card("c-edit", "编辑", "挑错"));
    s.chars.push(card("c-x", "编辑", "润色"));
    s.personas.push(card("me", "我", "写小说的"));
    s.char = "c-x".into();
    s.preset = p2.clone();
    s.save(&tb).unwrap();
    let mut lib = Library::default();
    lib.characters.push(crate::imagine::Character { id: "lin".into(), name: "林".into(), look: "boy".into(), ..Default::default() });
    lib.style = "builtin-anime".into();
    lib.save(&tb).unwrap();

    let spaces = vec![("A".to_owned(), a.clone()), ("B".to_owned(), b.clone()), ("C".to_owned(), t.join("none"))];
    let out = migrate_in(&global, &spaces).unwrap();
    assert!(out.iter().all(|(_, r)| r.is_ok()), "{out:?}");
    assert_eq!(out[0].1.as_ref().unwrap(), &Merged { presets: 1, chars: 1, personas: 0, styles: 1, characters: 1 });
    assert_eq!(out[1].1.as_ref().unwrap(), &Merged { presets: 2, chars: 1, personas: 1, styles: 0, characters: 1 });

    let g = Settings::load(&global);
    assert_eq!(names(&g.chars), ["写作助手", "编辑", "编辑（2）"]);
    assert_eq!(g.chars[2].id, "c-x");
    assert_eq!(names(&g.personas), ["我", "我（2）"]);
    assert_eq!(g.personas[1].id, "me-m2");
    // 选择是第一个空间的。
    assert_eq!((g.char.as_str(), g.preset.as_str(), g.last.as_str()), ("c-edit", p1.as_str(), "接着写"));
    // 预设：P1 一份；撞标识的另起 `-m2`；P2 照拷。
    let ids: BTreeSet<String> = library::presets(&global, &g.names).into_iter().map(|l| l.id).collect();
    assert_eq!(ids, [p1.clone(), format!("{p1}-m2"), p2.clone()].into_iter().collect());
    assert_eq!(library::load(&global, &format!("{p1}-m2")).unwrap().name().as_deref(), Some("P1 改过"));

    let lib = Library::load(&global);
    assert_eq!(lib.characters.iter().map(|c| (c.name.as_str(), c.look.as_str())).collect::<Vec<_>>(), [("林", "girl"), ("林（2）", "boy")]);
    assert_eq!((lib.style.as_str(), lib.quality.as_deref()), ("s1", Some("best")));

    // 再跑一次：都并过了，什么都不做。
    assert_eq!(migrated(&global), ["A", "B", "C"].into_iter().map(String::from).collect());
    assert!(migrate_in(&global, &spaces).unwrap().is_empty());
    std::fs::remove_dir_all(&t).unwrap();
}

/// 空间里的文件还在从 iCloud 下载：这次跳过，不记成并过，全局库里什么都没写。
#[test]
fn skip_while_downloading() {
    let t = temp("waiting");
    let (global, a) = (t.join("global"), t.join("a"));
    std::fs::create_dir_all(a.join("tavern")).unwrap();
    std::fs::write(a.join("tavern").join(".settings.json.icloud"), b"").unwrap();
    let out = migrate_in(&global, &[("A".to_owned(), a.clone())]).unwrap();
    assert!(out[0].1.is_err());
    assert!(migrated(&global).is_empty() && !global.exists());

    // 预设没下载好也一样。
    std::fs::remove_file(a.join("tavern").join(".settings.json.icloud")).unwrap();
    std::fs::create_dir_all(a.join("tavern").join("presets")).unwrap();
    std::fs::write(a.join("tavern").join("presets").join(".abc.json.icloud"), b"").unwrap();
    assert!(merge_space(&global, &a.join("tavern")).is_err());
    std::fs::remove_dir_all(&t).unwrap();
}

/// 头像搬家：同名的不盖。
#[test]
fn copy_avatar_files() {
    let t = temp("avatars");
    let (from, to) = (t.join("old"), t.join("new"));
    std::fs::create_dir_all(&from).unwrap();
    std::fs::create_dir_all(&to).unwrap();
    std::fs::write(from.join("a.png"), b"A").unwrap();
    std::fs::write(from.join("b.png"), b"B").unwrap();
    std::fs::write(to.join("b.png"), b"kept").unwrap();
    assert_eq!(copy_files(&from, &to).unwrap(), 1);
    assert_eq!(std::fs::read(to.join("b.png")).unwrap(), b"kept");
    assert_eq!(copy_files(&t.join("missing"), &to).unwrap(), 0);
    std::fs::remove_dir_all(&t).unwrap();
}
