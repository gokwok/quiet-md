//! 酒馆预设：SillyTavern 的 Chat Completion 预设 JSON。
//! 整份 JSON 原样留着（`raw`），读的时候按需取出带类型的视图，改的时候直接改 `raw`：
//! 不认识的字段和 `extensions` 里的东西（脚本、分组信息……）导出时原样带回。
//! 社区预设的字段类型不太讲究（数字写成字符串之类），读的时候宽松。
use serde_json::{Map, Value, json};

use super::regex::Script;

/// `prompt_order` 里全局那份的 `character_id`（SillyTavern 的约定）。
pub const GLOBAL_ORDER: u64 = 100001;
/// 按深度插入的默认深度和顺序（SillyTavern 的 `DEFAULT_DEPTH`、`DEFAULT_ORDER`）。
const DEFAULT_DEPTH: usize = 4;
const DEFAULT_ORDER: i64 = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role { System, User, Assistant }

impl Role {
    pub fn read(s: &str) -> Option<Role> {
        match s.trim() { "system" => Some(Role::System), "user" => Some(Role::User), "assistant" => Some(Role::Assistant), _ => None }
    }
    pub fn as_str(self) -> &'static str {
        match self { Role::System => "system", Role::User => "user", Role::Assistant => "assistant" }
    }
}

/// 一条提示词放在哪：按 `prompt_order` 的位置（相对），按消息深度插进聊天记录，或附加到已有消息上（不支持）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Position { Relative, Depth { depth: usize, order: i64 }, Attach }

#[derive(Clone, Debug)]
pub struct Prompt {
    pub id: String,
    pub name: String,
    /// 没写时按 system。
    pub role: Option<Role>,
    pub content: String,
    /// 标记：内容由应用填（角色描述、聊天记录……），预设里不写正文。
    pub marker: bool,
    pub system_prompt: bool,
    pub position: Position,
    /// 只在这些生成类型下生效；空的是都生效。
    pub triggers: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry { pub id: String, pub enabled: bool }

/// 发给模型的采样参数。没写或写成「自动」的留空，不发。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Params {
    pub temperature: Option<f64>,
    pub top_p: Option<f64>,
    pub frequency_penalty: Option<f64>,
    pub presence_penalty: Option<f64>,
    pub max_tokens: Option<u64>,
    pub reasoning_effort: Option<String>,
    pub seed: Option<i64>,
    /// OpenAI 规范外的采样参数，只有接受它们的服务商才发。
    pub top_k: Option<f64>,
    pub min_p: Option<f64>,
    pub top_a: Option<f64>,
    pub repetition_penalty: Option<f64>,
}

impl Params {
    /// 放进请求体的字段。`extended` 时带上 OpenAI 规范外的那几个。
    pub fn body(&self, extended: bool) -> Map<String, Value> {
        let mut m = Map::new();
        let mut put = |k: &str, v: Option<Value>| { if let Some(v) = v { m.insert(k.into(), v); } };
        put("temperature", self.temperature.map(Value::from));
        put("top_p", self.top_p.map(Value::from));
        put("frequency_penalty", self.frequency_penalty.map(Value::from));
        put("presence_penalty", self.presence_penalty.map(Value::from));
        put("max_tokens", self.max_tokens.map(Value::from));
        put("reasoning_effort", self.reasoning_effort.clone().map(Value::from));
        put("seed", self.seed.map(Value::from));
        if extended {
            put("top_k", self.top_k.map(Value::from));
            put("min_p", self.min_p.map(Value::from));
            put("top_a", self.top_a.map(Value::from));
            put("repetition_penalty", self.repetition_penalty.map(Value::from));
        }
        m
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Preset { raw: Map<String, Value> }

pub(crate) fn text(v: &Value, k: &str) -> Option<String> { v.get(k).and_then(Value::as_str).map(str::to_owned) }
pub(crate) fn num(v: &Value, k: &str) -> Option<f64> {
    match v.get(k)? { Value::Number(n) => n.as_f64(), Value::String(s) => s.trim().parse().ok(), _ => None }
}
pub(crate) fn flag(v: &Value, k: &str) -> Option<bool> {
    match v.get(k)? {
        Value::Bool(b) => Some(*b),
        Value::String(s) => match s.trim() { "true" => Some(true), "false" => Some(false), _ => None },
        Value::Number(n) => n.as_f64().map(|n| n != 0.),
        _ => None,
    }
}

impl Prompt {
    fn read(v: &Value) -> Option<Prompt> {
        let id = text(v, "identifier").filter(|s| !s.is_empty())?;
        let depth = num(v, "injection_depth").filter(|d| *d >= 0.).map_or(DEFAULT_DEPTH, |d| d as usize);
        let order = num(v, "injection_order").map_or(DEFAULT_ORDER, |o| o as i64);
        // 认不出的位置按相对算，和 SillyTavern 读旧预设一样。
        let position = match num(v, "injection_position").map(|p| p as i64) { Some(1) => Position::Depth { depth, order }, Some(2) => Position::Attach, _ => Position::Relative };
        let triggers = v.get("injection_trigger").and_then(Value::as_array).map_or_else(Vec::new, |a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect());
        Some(Prompt {
            name: text(v, "name").unwrap_or_else(|| id.clone()),
            role: text(v, "role").and_then(|r| Role::read(&r)),
            content: text(v, "content").unwrap_or_default(),
            marker: flag(v, "marker").unwrap_or(false),
            // 没写的不算用户自建：SillyTavern 只认明写的 `false`。
            system_prompt: flag(v, "system_prompt").unwrap_or(true),
            position, triggers, id,
        })
    }
}

impl Preset {
    pub fn parse(bytes: &[u8]) -> Result<Preset, String> {
        match serde_json::from_slice::<Value>(bytes).map_err(|e| e.to_string())? {
            Value::Object(raw) => Ok(Preset { raw }),
            _ => Err("预设不是一个 JSON 对象".into()),
        }
    }

    /// 导出：整份 JSON，带着所有不认识的字段。键按字母排（serde_json 的默认）。
    pub fn to_json(&self) -> Vec<u8> { serde_json::to_vec_pretty(&self.raw).unwrap_or_default() }
    pub fn raw(&self) -> &Map<String, Value> { &self.raw }
    fn get(&self) -> Value { Value::Object(self.raw.clone()) }

    /// 预设自己写的名字（导出的文件常有 `name`；没有就由调用方用文件名）。
    pub fn name(&self) -> Option<String> { self.raw.get("name").and_then(Value::as_str).map(str::to_owned) }

    pub fn prompts(&self) -> Vec<Prompt> {
        self.raw.get("prompts").and_then(Value::as_array).map_or_else(Vec::new, |a| a.iter().filter_map(Prompt::read).collect())
    }
    pub fn prompt(&self, id: &str) -> Option<Prompt> { self.prompts().into_iter().find(|p| p.id == id) }

    fn order_list(&self) -> Option<&Vec<Value>> {
        let lists = self.raw.get("prompt_order")?.as_array()?;
        let pick = lists.iter().find(|o| num(o, "character_id") == Some(GLOBAL_ORDER as f64)).or_else(|| lists.first())?;
        pick.get("order")?.as_array()
    }

    /// 提示词的顺序和开关。预设没写 `prompt_order` 时按 `prompts` 的顺序，全部打开。
    pub fn order(&self) -> Vec<Entry> {
        match self.order_list() {
            Some(a) => a.iter().filter_map(|e| Some(Entry { id: text(e, "identifier")?, enabled: flag(e, "enabled").unwrap_or(true) })).collect(),
            None => self.prompts().into_iter().map(|p| Entry { id: p.id, enabled: true }).collect(),
        }
    }

    pub fn params(&self) -> Params {
        let v = self.get();
        let pos = |k: &str| num(&v, k).filter(|x| *x > 0.);
        Params {
            temperature: num(&v, "temperature"),
            top_p: num(&v, "top_p"),
            frequency_penalty: num(&v, "frequency_penalty"),
            presence_penalty: num(&v, "presence_penalty"),
            max_tokens: pos("openai_max_tokens").map(|x| x as u64),
            // 和 SillyTavern 的自定义来源一样：「自动」不发，其余原样发。
            reasoning_effort: text(&v, "reasoning_effort").filter(|e| !e.is_empty() && e != "auto"),
            seed: num(&v, "seed").map(|s| s as i64).filter(|s| *s >= 0),
            top_k: pos("top_k"),
            min_p: pos("min_p"),
            top_a: pos("top_a"),
            repetition_penalty: num(&v, "repetition_penalty").filter(|x| *x != 1. && *x > 0.),
        }
    }

    /// 预设的上下文长度（token）。
    pub fn max_context(&self) -> Option<u64> { num(&self.get(), "openai_max_context").filter(|x| *x > 0.).map(|x| x as u64) }
    pub fn setting(&self, key: &str) -> String { text(&self.get(), key).unwrap_or_default() }
    pub fn setting_flag(&self, key: &str) -> bool { flag(&self.get(), key).unwrap_or(false) }
    /// 聊天记录里名字的写法：-1 不写，0 默认，1 放进 `name` 字段，2 写在正文前面。
    pub fn names_behavior(&self) -> i64 { num(&self.get(), "names_behavior").map_or(0, |n| n as i64) }

    /// 预设带的正则脚本（`extensions.regex_scripts`），按原来的顺序。
    pub fn regex_scripts(&self) -> Vec<Script> {
        self.raw.get("extensions").and_then(|e| e.get("regex_scripts")).and_then(Value::as_array).map_or_else(Vec::new, |a| a.iter().map(Script::read).collect())
    }

    /// `extensions` 里不会执行、只原样保留的东西（脚本之类），给界面提示用。
    pub fn ignored_extensions(&self) -> Vec<String> {
        self.raw.get("extensions").and_then(Value::as_object).map_or_else(Vec::new, |m| m.keys().filter(|k| *k != "regex_scripts").cloned().collect())
    }

    // —— 编辑：都直接改 `raw` ——

    fn order_mut(&mut self) -> &mut Vec<Value> {
        if self.order_list().is_none() {
            let order: Vec<Value> = self.prompts().iter().map(|p| json!({ "identifier": p.id, "enabled": true })).collect();
            let lists = self.raw.entry("prompt_order").or_insert_with(|| json!([]));
            if !lists.is_array() { *lists = json!([]); }
            let lists = lists.as_array_mut().unwrap();
            match lists.iter_mut().find(|o| num(o, "character_id") == Some(GLOBAL_ORDER as f64)) {
                Some(o) => { o["order"] = Value::Array(order); }
                None => lists.insert(0, json!({ "character_id": GLOBAL_ORDER, "order": order })),
            }
        }
        let lists = self.raw.get_mut("prompt_order").and_then(Value::as_array_mut).unwrap();
        let i = lists.iter().position(|o| num(o, "character_id") == Some(GLOBAL_ORDER as f64)).unwrap_or(0);
        lists[i].get_mut("order").and_then(Value::as_array_mut).unwrap()
    }

    fn prompts_mut(&mut self) -> &mut Vec<Value> {
        let p = self.raw.entry("prompts").or_insert_with(|| json!([]));
        if !p.is_array() { *p = json!([]); }
        p.as_array_mut().unwrap()
    }

    /// 打开或关上一条；它不在顺序里时加到末尾。
    pub fn set_enabled(&mut self, id: &str, on: bool) {
        let order = self.order_mut();
        match order.iter_mut().find(|e| e.get("identifier").and_then(Value::as_str) == Some(id)) {
            Some(e) => { e["enabled"] = Value::Bool(on); }
            None => order.push(json!({ "identifier": id, "enabled": on })),
        }
    }

    /// 把顺序里第 `from` 条挪到第 `to` 条。
    pub fn move_entry(&mut self, from: usize, to: usize) {
        let order = self.order_mut();
        if from >= order.len() { return; }
        let e = order.remove(from);
        order.insert(to.min(order.len()), e);
    }

    /// 改一条提示词的某个字段（`name`、`role`、`content`、`injection_position`……）。没有这条时返回 false。
    pub fn set_prompt_field(&mut self, id: &str, key: &str, value: Value) -> bool {
        let Some(p) = self.prompts_mut().iter_mut().find(|p| p.get("identifier").and_then(Value::as_str) == Some(id)) else { return false };
        match p.as_object_mut() { Some(m) => { m.insert(key.into(), value); true } None => false }
    }

    /// 新加一条自己的提示词（不是标记），放在顺序末尾并打开。
    pub fn add_prompt(&mut self, id: &str, name: &str, role: Role, content: &str) {
        self.prompts_mut().push(json!({ "identifier": id, "name": name, "role": role.as_str(), "content": content, "system_prompt": false, "marker": false }));
        self.set_enabled(id, true);
    }

    /// 删掉一条自己的提示词。标记删不掉（它们是应用填内容的位置），返回 false。
    pub fn remove_prompt(&mut self, id: &str) -> bool {
        if self.prompt(id).is_none_or(|p| p.marker) { return false; }
        self.prompts_mut().retain(|p| p.get("identifier").and_then(Value::as_str) != Some(id));
        self.order_mut().retain(|e| e.get("identifier").and_then(Value::as_str) != Some(id));
        true
    }

    /// 改一个顶层设置（温度、最大输出……）。
    pub fn set_setting(&mut self, key: &str, value: Value) { self.raw.insert(key.into(), value); }

    /// 改第 `i` 条正则的某个字段（`disabled`、`findRegex`、`replaceString`……）。
    pub fn set_regex_field(&mut self, i: usize, key: &str, value: Value) -> bool {
        let Some(s) = self.raw.get_mut("extensions").and_then(|e| e.get_mut("regex_scripts")).and_then(Value::as_array_mut).and_then(|a| a.get_mut(i)) else { return false };
        match s.as_object_mut() { Some(m) => { m.insert(key.into(), value); true } None => false }
    }

    fn regex_mut(&mut self) -> &mut Vec<Value> {
        let ext = self.raw.entry("extensions").or_insert_with(|| json!({}));
        if !ext.is_object() { *ext = json!({}); }
        let list = ext.as_object_mut().unwrap().entry("regex_scripts").or_insert_with(|| json!([]));
        if !list.is_array() { *list = json!([]); }
        list.as_array_mut().unwrap()
    }

    /// 新加一条正则（作用于模型的输出，开着），放在最后；返回它的位置。
    pub fn add_regex(&mut self, id: &str, name: &str) -> usize {
        let list = self.regex_mut();
        list.push(json!({ "id": id, "scriptName": name, "findRegex": "", "replaceString": "", "trimStrings": [], "placement": [super::regex::AI_OUTPUT],
            "disabled": false, "markdownOnly": false, "promptOnly": false, "runOnEdit": true, "substituteRegex": 0, "minDepth": null, "maxDepth": null }));
        list.len() - 1
    }

    /// 删掉第 `i` 条正则。
    pub fn remove_regex(&mut self, i: usize) -> bool {
        let list = self.regex_mut();
        if i >= list.len() { return false; }
        list.remove(i);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SMALL: &str = r#"{
        "temperature": 1, "top_p": "0.9", "top_k": 64, "openai_max_tokens": 300, "reasoning_effort": "auto", "seed": -1,
        "mystery": {"kept": [1, 2]},
        "prompts": [
            {"identifier": "main", "name": "Main", "role": "system", "content": "hi", "system_prompt": true},
            {"identifier": "chatHistory", "name": "Chat History", "marker": true, "system_prompt": true},
            {"identifier": "x", "name": "Deep", "role": "user", "content": "d", "injection_position": 1, "injection_depth": "2"}
        ],
        "prompt_order": [{"character_id": 100000, "order": []}, {"character_id": 100001, "order": [
            {"identifier": "main", "enabled": true}, {"identifier": "chatHistory", "enabled": true}, {"identifier": "x", "enabled": false}
        ]}],
        "extensions": {"regex_scripts": [{"scriptName": "a", "findRegex": "/a/g", "replaceString": "b", "placement": [2]}], "tavern_helper": {"scripts": []}}
    }"#;

    #[test]
    fn reads_leniently_and_round_trips() {
        let p = Preset::parse(SMALL.as_bytes()).unwrap();
        assert_eq!(p.order().iter().map(|e| (e.id.as_str(), e.enabled)).collect::<Vec<_>>(), [("main", true), ("chatHistory", true), ("x", false)]);
        assert_eq!(p.prompt("x").unwrap().position, Position::Depth { depth: 2, order: 100 });
        let params = p.params();
        assert_eq!((params.top_p, params.max_tokens, params.reasoning_effort.clone(), params.seed), (Some(0.9), Some(300), None, None));
        assert!(!params.body(false).contains_key("top_k"));
        assert_eq!(params.body(true)["top_k"], json!(64.0));
        assert_eq!(p.ignored_extensions(), ["tavern_helper"]);
        let back = Preset::parse(&p.to_json()).unwrap();
        assert_eq!(back, p);
        assert_eq!(serde_json::from_slice::<Value>(&p.to_json()).unwrap(), serde_json::from_str::<Value>(SMALL).unwrap());
    }

    #[test]
    fn edits_touch_only_their_field() {
        let mut p = Preset::parse(SMALL.as_bytes()).unwrap();
        p.set_enabled("x", true);
        p.move_entry(2, 0);
        assert!(p.set_prompt_field("main", "content", json!("hello")));
        p.add_prompt("mine", "Mine", Role::Assistant, "ok");
        assert!(!p.remove_prompt("chatHistory"));
        assert!(p.set_regex_field(0, "disabled", json!(true)));
        assert_eq!(p.order().iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), ["x", "main", "chatHistory", "mine"]);
        assert_eq!(p.prompt("main").unwrap().content, "hello");
        assert!(p.regex_scripts()[0].disabled);
        assert!(p.remove_prompt("mine"));
        let n = p.regex_scripts().len();
        assert_eq!(p.add_regex("r9", "新的"), n);
        assert_eq!((p.regex_scripts()[n].name.as_str(), p.regex_scripts()[n].placement.as_slice()), ("新的", [2].as_slice()));
        assert!(p.remove_regex(n) && !p.remove_regex(n));
        assert_eq!(p.raw()["mystery"], json!({"kept": [1, 2]}));
        let mut bare = Preset::parse(b"{}").unwrap();
        assert_eq!(bare.add_regex("a", "a"), 0);
        assert_eq!(p.raw()["prompt_order"][0], json!({"character_id": 100000, "order": []}));
    }

    #[test]
    fn missing_order_follows_prompts() {
        let mut p = Preset::parse(br#"{"prompts": [{"identifier": "a", "content": "1"}, {"identifier": "b", "content": "2"}]}"#).unwrap();
        assert_eq!(p.order().len(), 2);
        p.set_enabled("b", false);
        assert_eq!(p.order(), [Entry { id: "a".into(), enabled: true }, Entry { id: "b".into(), enabled: false }]);
    }
}
