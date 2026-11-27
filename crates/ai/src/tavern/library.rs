//! 助手的素材库：一个目录（现在是全局库 `global` 的目录，以前是每个空间的 `tavern/`）。
//! - `presets/<标识>.json`：导入的预设，原样存（导出时还是那份文件，只少了我们记文件名的那个字段）。
//!   标识就是文件名，导入时取内容的哈希：同一份导入两次不会多一份；同名的文件已经改过了，就另起一个标识，不盖掉改过的。
//! - `settings.json`：选中的预设、助手和人设，助手和人设的列表，上一次的要求。
//!
//! 没导入过预设时用内置的「默认」（`default-preset.json`，本项目自己写的，很短）；标识是空串。
//! 目录可能在 iCloud 里：读写都经过 `file_io`（协调访问），还没下载的预设只有 `.<标识>.json.icloud` 占位。
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use fdl_core::file_io::{self, DownloadState};
use serde_json::{Value, json};

use super::preset::{Preset, text};
use super::record::hash;

/// 助手（角色）或人设：名字加描述。
#[derive(Clone, Debug, PartialEq)]
pub struct Card { pub id: String, pub name: String, pub description: String }

impl Card {
    fn read(v: &Value) -> Option<Card> {
        Some(Card { id: text(v, "id").filter(|s| !s.is_empty())?, name: text(v, "name").unwrap_or_default(), description: text(v, "description").unwrap_or_default() })
    }
    fn to_json(&self) -> Value { json!({ "id": self.id, "name": self.name, "description": self.description }) }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    /// 选中的预设；空的是内置的「默认」。
    pub preset: String,
    pub char: String,
    pub persona: String,
    pub chars: Vec<Card>,
    pub personas: Vec<Card>,
    /// 上一次的要求，下次打开时预填。
    pub last: String,
    /// 见过的预设名字（标识 → 名字）：另一台设备上还没下载的预设，列表里也有名字可显示。
    pub names: BTreeMap<String, String>,
}

impl Default for Settings {
    fn default() -> Settings {
        Settings {
            preset: String::new(), char: "assistant".into(), persona: "me".into(),
            chars: vec![Card { id: "assistant".into(), name: "写作助手".into(), description: String::new() }],
            personas: vec![Card { id: "me".into(), name: "我".into(), description: String::new() }],
            last: String::new(), names: BTreeMap::new(),
        }
    }
}

/// 先写临时文件再改名（在对目标的同一次协调写里，和文档库的写法一样）。
pub(crate) fn write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() { file_io::create_dir_all(dir)?; }
    file_io::with_write(path, |p| {
        let tmp = p.with_extension("json.tmp");
        std::fs::write(&tmp, bytes)?;
        Ok(std::fs::rename(&tmp, p)?)
    }).map_err(std::io::Error::other)
}

impl Settings {
    /// 读得宽松：缺的按默认，列表空了补上默认的那一个，选中的不在列表里就选第一个。
    pub fn load(dir: &Path) -> Settings {
        let d = Settings::default();
        let Some(v) = file_io::read(dir.join("settings.json")).ok().and_then(|b| serde_json::from_slice::<Value>(&b).ok()) else { return d };
        let cards = |k: &str, fallback: &Vec<Card>| {
            let list: Vec<Card> = v.get(k).and_then(Value::as_array).map_or_else(Vec::new, |a| a.iter().filter_map(Card::read).collect());
            if list.is_empty() { fallback.clone() } else { list }
        };
        let (chars, personas) = (cards("chars", &d.chars), cards("personas", &d.personas));
        let pick = |k: &str, list: &[Card]| text(&v, k).filter(|id| list.iter().any(|c| c.id == *id)).unwrap_or_else(|| list[0].id.clone());
        Settings { preset: text(&v, "preset").unwrap_or_default(), char: pick("char", &chars), persona: pick("persona", &personas), chars, personas, last: text(&v, "last").unwrap_or_default(),
            names: v.get("names").and_then(Value::as_object).map(|m| m.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned()))).collect()).unwrap_or_default() }
    }

    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        let v = json!({
            "preset": self.preset, "char": self.char, "persona": self.persona,
            "chars": self.chars.iter().map(Card::to_json).collect::<Vec<_>>(), "personas": self.personas.iter().map(Card::to_json).collect::<Vec<_>>(),
            "last": self.last, "names": self.names,
        });
        write(&dir.join("settings.json"), &serde_json::to_vec_pretty(&v).map_err(std::io::Error::other)?)
    }

    pub fn char(&self) -> &Card { self.chars.iter().find(|c| c.id == self.char).unwrap_or(&self.chars[0]) }
    pub fn persona(&self) -> &Card { self.personas.iter().find(|c| c.id == self.persona).unwrap_or(&self.personas[0]) }
}

pub fn default_preset() -> Preset { Preset::parse(include_bytes!("default-preset.json")).expect("内置预设") }

pub(crate) fn presets_dir(dir: &Path) -> PathBuf { dir.join("presets") }
pub(crate) fn path(dir: &Path, id: &str) -> PathBuf { presets_dir(dir).join(format!("{id}.json")) }
fn valid(id: &str) -> bool { !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') }
/// 我们自己加的字段：预设没写 `name` 时记导入的文件名。导出时去掉。
const FILE_NAME: &str = "quiet_md_file_name";

/// 列表里的一份预设。`state` 不是 `Ready` 时还没下载（或下载失败），名字是上次见过的（没见过就是标识）。
#[derive(Clone, Debug, PartialEq)]
pub struct Listed { pub id: String, pub name: String, pub state: DownloadState }

fn name_of(p: &Preset) -> Option<String> { p.name().or_else(|| text(&Value::Object(p.raw().clone()), FILE_NAME)).filter(|n| !n.trim().is_empty()) }

/// 导入过的预设，按名字排；内置的「默认」不在里面。还没下载的也列出来（顺手请求下载），名字用 `names` 里记着的。
pub fn presets(dir: &Path, names: &BTreeMap<String, String>) -> Vec<Listed> {
    let Ok(entries) = file_io::read_dir(presets_dir(dir)) else { return vec![] };
    let mut out: Vec<Listed> = entries.filter_map(Result::ok).filter_map(|e| {
        let file = e.file_name().into_string().ok()?;
        // 占位文件：`.<标识>.json.icloud`。
        let (id, placeholder) = match file.strip_prefix('.').and_then(|f| f.strip_suffix(".icloud")) { Some(real) => (real.strip_suffix(".json")?.to_owned(), true), None => (file.strip_suffix(".json")?.to_owned(), false) };
        if !valid(&id) { return None; }
        let state = if placeholder { file_io::download_state(&path(dir, &id)) } else { DownloadState::Ready };
        if placeholder && !state.ready() { let _ = file_io::request_download(&path(dir, &id)); }
        let name = if state.ready() { load(dir, &id).ok().and_then(|p| name_of(&p)) } else { None };
        Some(Listed { name: name.or_else(|| names.get(&id).cloned()).unwrap_or_else(|| id.clone()), id, state })
    }).collect();
    out.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.cmp(&b.id)));
    out.dedup_by(|a, b| a.id == b.id);
    out
}

/// 读一份预设：空标识是内置的「默认」。没下载好、找不到、读不了都报出来，不悄悄换成「默认」（不然另一台设备上会拿错预设去写）。
pub fn load(dir: &Path, id: &str) -> Result<Preset, String> {
    if id.is_empty() { return Ok(default_preset()); }
    if !valid(id) { return Err("预设的标识不对".into()); }
    let path = path(dir, id);
    match file_io::download_state(&path) {
        DownloadState::Ready => {}
        DownloadState::Waiting | DownloadState::Downloading => { let _ = file_io::request_download(&path); return Err("预设还在从 iCloud 下载".into()); }
        DownloadState::Missing => return Err("找不到这份预设".into()),
        DownloadState::Failed(e) => return Err(format!("预设下载失败：{e}")),
    }
    Preset::parse(&file_io::read(&path).map_err(|e| e.to_string())?)
}

/// 预设的名字（列表和记录里用）。
pub fn display_name(p: &Preset, id: &str) -> String { name_of(p).unwrap_or_else(|| id.to_owned()) }

/// 导入一份预设，返回它的标识。预设里没写 `name` 时，把文件名记在 `quiet_md_file_name` 里给列表用（只多这一个字段）。
pub fn import(dir: &Path, bytes: &[u8], file_name: &str) -> Result<String, String> {
    let p = Preset::parse(bytes)?;
    let bytes = if p.name().is_some() { bytes.to_vec() } else {
        let mut raw = p.raw().clone();
        raw.insert(FILE_NAME.into(), file_name.trim_end_matches(".json").into());
        serde_json::to_vec_pretty(&raw).map_err(|e| e.to_string())?
    };
    let mut id = hash(&String::from_utf8_lossy(&bytes));
    // 同一个标识已经有了：内容一样就是同一份；不一样（导入后改过）就另起一个，不盖掉。
    let mut n = 1;
    while let Ok(old) = file_io::read(path(dir, &id)) {
        if old == bytes { return Ok(id); }
        n += 1;
        id = format!("{}-{n}", hash(&String::from_utf8_lossy(&bytes)));
    }
    write(&path(dir, &id), &bytes).map_err(|e| e.to_string())?;
    Ok(id)
}

/// 存一份新的预设（「默认」改动后另存、复制一份），返回新标识。
pub fn create(dir: &Path, p: &Preset) -> Result<String, String> {
    let seed = format!("{}{:?}", String::from_utf8_lossy(&p.to_json()), std::time::SystemTime::now());
    let id = hash(&seed);
    write(&path(dir, &id), &p.to_json()).map_err(|e| e.to_string())?;
    Ok(id)
}

/// 改过的预设写回原处。内置的「默认」（空标识）不能写回，调用方先 `create`。
pub fn save(dir: &Path, id: &str, p: &Preset) -> Result<(), String> {
    if !valid(id) { return Err("内置的预设不能直接改".into()); }
    write(&path(dir, id), &p.to_json()).map_err(|e| e.to_string())
}

/// 删一份导入的预设（连同还没下载的占位）。
pub fn delete(dir: &Path, id: &str) -> Result<(), String> {
    if !valid(id) { return Err("内置的预设删不掉".into()); }
    let path = path(dir, id);
    let placeholder = presets_dir(dir).join(format!(".{id}.json.icloud"));
    let target = if file_io::metadata(&path).is_ok() { path } else { placeholder };
    file_io::remove_file(&target).map_err(|e| e.to_string())
}

/// 导出：整份 JSON（不认识的字段原样带着），去掉我们记文件名的字段。文件名用预设的名字。
pub fn export(dir: &Path, id: &str) -> Result<(String, Vec<u8>), String> {
    let p = load(dir, id)?;
    let mut raw = p.raw().clone();
    let name = display_name(&p, if id.is_empty() { "默认" } else { id });
    raw.remove(FILE_NAME);
    let file: String = name.chars().map(|c| if matches!(c, '/' | '\\' | ':' | '\0') { '-' } else { c }).collect();
    Ok((format!("{}.json", file.trim()), serde_json::to_vec_pretty(&raw).map_err(|e| e.to_string())?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tavern::{macros::{Env, Macros}, preset::Role};

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("fdl-ai-library-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }
    fn ids(dir: &Path) -> Vec<(String, String)> { presets(dir, &BTreeMap::new()).into_iter().map(|l| (l.id, l.name)).collect() }

    #[test]
    fn settings_presets_and_defaults() {
        let dir = temp("settings");
        assert_eq!(Settings::load(&dir), Settings::default());
        assert_eq!(default_preset().order().len(), 4);
        let id = import(&dir, br#"{"prompts":[],"temperature":0.7}"#, "我的预设.json").unwrap();
        assert_eq!(import(&dir, br#"{"prompts":[],"temperature":0.7}"#, "我的预设.json").unwrap(), id);
        assert_eq!(ids(&dir), [(id.clone(), "我的预设".to_owned())]);
        assert_eq!(load(&dir, &id).unwrap().params().temperature, Some(0.7));
        assert_eq!(load(&dir, "").unwrap().name().as_deref(), Some("默认"));
        assert!(load(&dir, "../x").is_err() && load(&dir, "gone").is_err());
        assert!(import(&dir, b"[]", "x.json").is_err());

        let mut s = Settings::default();
        s.preset = id.clone();
        s.last = "接着写".into();
        s.chars.push(Card { id: "c2".into(), name: "编辑".into(), description: "挑错".into() });
        s.char = "c2".into();
        s.names.insert(id.clone(), "我的预设".into());
        s.save(&dir).unwrap();
        let back = Settings::load(&dir);
        assert_eq!(back, s);
        assert_eq!(back.char().name, "编辑");
        // 选中的不在列表里：选第一个。
        std::fs::write(dir.join("settings.json"), br#"{"char":"gone","personas":[]}"#).unwrap();
        let s = Settings::load(&dir);
        assert_eq!((s.char.as_str(), s.persona().name.as_str()), ("assistant", "我"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    const PRESET: &str = r#"{
        "name": "试用", "temperature": 1, "mystery": {"kept": [1, {"deep": true}]},
        "prompts": [
            {"identifier": "main", "name": "Main", "role": "system", "content": "写好"},
            {"identifier": "chatHistory", "name": "Chat History", "marker": true},
            {"identifier": "extra", "name": "━━━━ 分组 ━━━━", "role": "system", "content": ""}
        ],
        "prompt_order": [{"character_id": 100001, "order": [
            {"identifier": "main", "enabled": true}, {"identifier": "extra", "enabled": true}, {"identifier": "chatHistory", "enabled": true}
        ]}],
        "extensions": {"regex_scripts": [{"id": "r1", "scriptName": "去掉星号", "findRegex": "/\\*/g", "replaceString": "", "placement": [2], "disabled": false}],
            "tavern_helper": {"scripts": [{"name": "不执行", "content": "alert(1)"}]}, "SPreset": {"x": 1}}
    }"#;

    /// 改一份导入的预设：开关、挪动、改正文、改正则、加一条；存回、重新读、导出，不认识的字段原样都在。
    #[test]
    fn edit_save_delete_export() {
        let dir = temp("edit");
        let id = import(&dir, PRESET.as_bytes(), "试用.json").unwrap();
        let mut p = load(&dir, &id).unwrap();
        p.set_enabled("extra", false);
        p.move_entry(2, 0);
        assert!(p.set_prompt_field("main", "content", "写得更好".into()));
        assert!(p.set_regex_field(0, "findRegex", "/~/g".into()));
        p.add_prompt("mine", "我的", Role::User, "{{user}} 的话");
        save(&dir, &id, &p).unwrap();

        let back = load(&dir, &id).unwrap();
        let order: Vec<(String, bool)> = back.order().into_iter().map(|e| (e.id, e.enabled)).collect();
        assert_eq!(order, [("chatHistory".into(), true), ("main".into(), true), ("extra".into(), false), ("mine".into(), true)]);
        assert_eq!(back.prompt("main").unwrap().content, "写得更好");
        assert_eq!(back.regex_scripts()[0].find, "/~/g");
        let original: Value = serde_json::from_str(PRESET).unwrap();
        for k in ["mystery", "name", "temperature"] { assert_eq!(back.raw()[k], original[k], "{k}"); }
        assert_eq!(back.raw()["extensions"]["tavern_helper"], original["extensions"]["tavern_helper"]);
        assert_eq!(back.raw()["extensions"]["SPreset"], original["extensions"]["SPreset"]);

        // 再导入原来那份：改过的那份不被盖掉，另起一个标识。
        let again = import(&dir, PRESET.as_bytes(), "试用.json").unwrap();
        assert_ne!(again, id);
        assert_eq!(load(&dir, &id).unwrap().prompt("main").unwrap().content, "写得更好");
        assert_eq!(import(&dir, PRESET.as_bytes(), "试用.json").unwrap(), again);

        // 「默认」改了要另存。
        let mut d = load(&dir, "").unwrap();
        assert!(save(&dir, "", &d).is_err());
        d.set_setting("name", "默认 2".into());
        let copy = create(&dir, &d).unwrap();
        assert_eq!(display_name(&load(&dir, &copy).unwrap(), &copy), "默认 2");

        let (file, bytes) = export(&dir, &id).unwrap();
        assert_eq!(file, "试用.json");
        assert_eq!(Preset::parse(&bytes).unwrap().raw(), back.raw());
        let (file, bytes) = export(&dir, "").unwrap();
        assert_eq!((file.as_str(), Preset::parse(&bytes).unwrap().name().as_deref()), ("默认.json", Some("默认")));
        // 没写名字的导入：导出时去掉我们记的文件名。
        let plain = import(&dir, br#"{"prompts":[]}"#, "无名.json").unwrap();
        let (file, bytes) = export(&dir, &plain).unwrap();
        assert_eq!((file.as_str(), bytes.windows(FILE_NAME.len()).any(|w| w == FILE_NAME.as_bytes())), ("无名.json", false));

        delete(&dir, &again).unwrap();
        assert!(load(&dir, &again).is_err());
        assert!(delete(&dir, "").is_err());
        assert_eq!(ids(&dir).len(), 3);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 另一台设备导入、这台还没下载：列出来（名字用记着的），读的时候报「还在下载」，删得掉。
    #[test]
    fn not_downloaded() {
        let dir = temp("icloud");
        std::fs::create_dir_all(presets_dir(&dir)).unwrap();
        std::fs::write(presets_dir(&dir).join(".abc123.json.icloud"), b"").unwrap();
        let names = BTreeMap::from([("abc123".to_owned(), "远处的预设".to_owned())]);
        assert_eq!(presets(&dir, &names), [Listed { id: "abc123".into(), name: "远处的预设".into(), state: DownloadState::Waiting }]);
        assert_eq!(presets(&dir, &BTreeMap::new())[0].name, "abc123");
        assert!(load(&dir, "abc123").unwrap_err().contains("下载"));
        delete(&dir, "abc123").unwrap();
        assert!(presets(&dir, &names).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// 拿一份真实的预设跑一遍（文件不进仓库）：`FDL_TAVERN_PRESET=<路径> cargo test -p fdl_ai -- --ignored real_preset --nocapture`。
    #[test]
    #[ignore]
    fn real_preset() {
        let Some(src) = std::env::var_os("FDL_TAVERN_PRESET") else { return };
        let bytes = std::fs::read(&src).unwrap();
        let dir = temp("real");
        let id = import(&dir, &bytes, "real.json").unwrap();
        let mut p = load(&dir, &id).unwrap();
        let order = p.order();
        let first = order.iter().find(|e| e.enabled).unwrap().id.clone();
        p.set_enabled(&first, false);
        p.move_entry(0, order.len() - 1);
        p.set_regex_field(0, "disabled", Value::Bool(true));
        save(&dir, &id, &p).unwrap();
        let back = load(&dir, &id).unwrap();
        assert!(!back.order().iter().find(|e| e.id == first).unwrap().enabled);
        let original = Preset::parse(&bytes).unwrap();
        let skip = ["prompt_order", "extensions"];
        for (k, v) in original.raw() { if !skip.contains(&k.as_str()) { assert_eq!(&back.raw()[k], v, "{k}"); } }
        for (k, v) in original.raw()["extensions"].as_object().unwrap() { if k != "regex_scripts" { assert_eq!(&back.raw()["extensions"][k], v, "extensions.{k}"); } }
        let (_, out) = export(&dir, &id).unwrap();
        assert_eq!(Preset::parse(&out).unwrap().raw(), back.raw());
        let mut m = Macros::new(Env::default());
        let scripts = original.regex_scripts();
        let bad: Vec<(String, String)> = scripts.iter().filter_map(|s| s.compile(&mut m).err().map(|e| (s.name.clone(), e))).collect();
        println!("条目 {}（开着 {}），提示词 {}，正则 {}，编译不过 {}：{bad:?}", order.len(), order.iter().filter(|e| e.enabled).count(), original.prompts().len(), scripts.len(), bad.len());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
