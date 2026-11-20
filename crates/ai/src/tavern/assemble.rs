//! 拼装：前文（已切成消息）、本次要求、预设、角色和人设 → 发给模型的 messages 和参数。
//! 顺序和规则照 SillyTavern 的 Chat Completion（`openai.js` 的 `prepareOpenAIMessages` 一路）：
//! 1. 本次要求先跑「存下来时」的用户输入正则，再跑宏（它发送时就是这样存进聊天的）；
//! 2. 聊天记录跑「发送前」的正则，按离末尾的条数算深度；
//! 3. 按 `prompt_order` 准备启用的提示词（宏从前往后），再填角色、人设这些标记；
//! 4. 按深度插入的条目插进聊天记录；
//! 5. 预算从最新的消息往前装，装不下的旧消息丢掉；开场提示放在最前；
//! 6. 按 `prompt_order` 的位置排出来，空消息不发。
//!
//! 世界书、示例对话、群聊、继续生成、附加到已有消息（`injection_position` 2）第一期不做。
use std::collections::BTreeMap;

use serde_json::Value;

use super::macros::{Env, Macros};
use super::preset::{Params, Position, Preset, Prompt, Role};
use super::regex::{AI_OUTPUT, Compiled, Scope, Script, USER_INPUT};

/// 聊天记录里的一条（按时间顺序，不含本次要求）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Turn { pub role: Role, pub text: String }

/// 角色或人设：一个名字加一段描述。
#[derive(Clone, Debug, Default)]
pub struct Identity { pub name: String, pub description: String }

pub struct Request<'a> {
    pub preset: &'a Preset,
    pub char: Identity,
    pub persona: Identity,
    pub history: Vec<Turn>,
    /// 本次要求；空的就不发这条 user 消息（预设的 `send_if_empty` 可能补一条）。
    pub input: String,
    pub model: String,
    /// 模型自己的上下文长度（`/models` 给的），和预设的取小。
    pub model_context: Option<u64>,
    /// 发 `top_k` 这类非标准参数。
    pub extended_params: bool,
    /// 发预设的 `assistant_prefill`（SillyTavern 只在 Claude 来源下发）。
    pub prefill: bool,
    pub now: Option<(i64, u32, u32, u32, u32)>,
    pub seed: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message { pub role: Role, pub content: String, pub name: Option<String> }

impl Message {
    pub fn to_json(&self) -> Value {
        let mut m = serde_json::Map::new();
        m.insert("role".into(), self.role.as_str().into());
        m.insert("content".into(), self.content.clone().into());
        if let Some(n) = &self.name { m.insert("name".into(), n.clone().into()); }
        Value::Object(m)
    }
}

#[derive(Debug)]
pub struct Assembled {
    pub messages: Vec<Message>,
    pub params: Params,
    /// 预设里用到了但这里没照做的东西，给界面和诊断看。
    pub notes: Vec<String>,
    /// 超出预算没发的聊天记录条数（含插入的条目）。
    pub dropped: usize,
    pub unknown_macros: Vec<String>,
    /// 本次要求跑完「存下来时」的正则和宏的样子：生成记录里存它，下次还原历史时原样放回。
    pub request: String,
}

/// SillyTavern 认的相对位置条目：这些 id 和用户自建的（`system_prompt` 为 false）。别的系统条目（TauriTavern 的 `agent*` 等）不发。
const KNOWN: [&str; 12] = ["worldInfoBefore", "main", "worldInfoAfter", "charDescription", "charPersonality", "scenario", "personaDescription", "nsfw", "jailbreak", "enhanceDefinitions", "chatHistory", "dialogueExamples"];
/// SillyTavern 的 `MAX_INJECTION_DEPTH`。
const MAX_DEPTH: usize = 10000;

/// 估算 token：中日韩字符一个算一个，其余四个字符算一个，每条消息另加 3。第一期不接分词器。
pub fn estimate(s: &str) -> u64 {
    let (mut wide, mut other) = (0u64, 0u64);
    for c in s.chars() {
        if matches!(c as u32, 0x3000..=0x9fff | 0xac00..=0xd7af | 0xf900..=0xfaff | 0xff00..=0xffef | 0x20000..=0x3ffff) { wide += 1; } else { other += 1; }
    }
    wide + other.div_ceil(4)
}
fn cost(m: &Message) -> u64 { estimate(&m.content) + 3 + m.name.as_ref().map_or(0, |_| 1) }

/// 聊天记录的一条，拼装中途的样子。
struct Chat { role: Role, content: String, name: Option<String> }

fn sanitize(name: &str) -> String { name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).take(64).collect() }

fn scripts_for(all: &[(Script, Option<Compiled>)], placement: i64, scope: Scope, depth: Option<usize>) -> impl Iterator<Item = (&Script, &Compiled)> {
    all.iter().filter_map(move |(s, c)| Some((s, c.as_ref()?))).filter(move |(s, _)| s.applies(placement, scope, depth))
}

pub fn assemble(req: &Request) -> Assembled {
    let preset = req.preset;
    let params = preset.params();
    let mut notes = vec![];
    let last_char = req.history.iter().rev().find(|t| t.role == Role::Assistant).map(|t| t.text.clone()).unwrap_or_default();
    let last_user = if req.input.is_empty() { req.history.iter().rev().find(|t| t.role == Role::User).map(|t| t.text.clone()).unwrap_or_default() } else { req.input.clone() };
    let context = match (preset.max_context(), req.model_context) { (Some(a), Some(b)) => Some(a.min(b)), (a, b) => a.or(b) };
    let mut m = Macros::new(Env {
        user: req.persona.name.clone(), char: req.char.name.clone(),
        description: req.char.description.clone(), persona: req.persona.description.clone(),
        // SillyTavern 拼装时输入框已经清空，`{{input}}` 是空的。
        model: req.model.clone(),
        last_message: if req.input.is_empty() { req.history.last().map(|t| t.text.clone()).unwrap_or_default() } else { req.input.clone() },
        last_user_message: last_user, last_char_message: last_char,
        max_context: context.unwrap_or(0), max_response: params.max_tokens.unwrap_or(0),
        now: req.now, seed: req.seed, ..Default::default()
    });

    // 正则：一次拼装里每条脚本只编译一次。
    let scripts: Vec<(Script, Option<Compiled>)> = preset.regex_scripts().into_iter().map(|s| {
        let c = if s.disabled { None } else { s.compile(&mut m).ok() };
        if !s.disabled && c.is_none() { notes.push(format!("正则「{}」编译不了，没有执行", s.name)); }
        (s, c)
    }).collect();

    // 1. 本次要求：存下来时的用户输入正则，再宏。
    let mut failed: Vec<String> = vec![];
    let mut input = req.input.clone();
    for (s, c) in scripts_for(&scripts, USER_INPUT, Scope::Stored, None) {
        match s.run_compiled(c, &input, &mut m) { Ok(t) => input = t, Err(_) => failed.push(s.name.clone()) }
    }
    let input = m.eval(&input);
    let request = input.clone();

    // 2. 聊天记录（含本次要求）跑发送前的正则，深度是离末尾的条数。
    let mut turns: Vec<(Role, String)> = req.history.iter().map(|t| (t.role, t.text.clone())).collect();
    if !input.is_empty() { turns.push((Role::User, input)); }
    let n = turns.len();
    let names = preset.names_behavior();
    let chat: Vec<Chat> = turns.into_iter().enumerate().map(|(i, (role, mut text))| {
        let placement = if role == Role::User { USER_INPUT } else { AI_OUTPUT };
        for (s, c) in scripts_for(&scripts, placement, Scope::Prompt, Some(n - i - 1)) {
            match s.run_compiled(c, &text, &mut m) { Ok(t) => text = t, Err(_) => failed.push(s.name.clone()) }
        }
        let name = match role { Role::User => &req.persona.name, Role::Assistant => &req.char.name, Role::System => &req.char.name };
        if names == 2 { text = format!("{name}: {text}"); }
        Chat { role, content: text.replace('\r', ""), name: (names == 1 && role != Role::System).then(|| sanitize(name)) }
    }).collect();

    failed.dedup();
    for f in failed { notes.push(format!("正则「{f}」在有的段落上运行出错（回溯太多），那些段落没处理")); }

    // 3. 启用的提示词，按 prompt_order，宏从前往后。主提示词关了也占位（内容为空）。
    let mut prompts: Vec<(Prompt, String)> = vec![];
    for e in preset.order() {
        let Some(p) = preset.prompt(&e.id) else { continue };
        let on = e.enabled && (p.triggers.is_empty() || p.triggers.iter().any(|t| t == "normal"));
        if on { let c = m.eval(&p.content); prompts.push((p, c)); } else if e.id == "main" { prompts.push((p, String::new())); }
    }
    // 标记：内容由这里填，预设里写的不算（世界书、性格、场景、示例对话第一期都是空的）。
    // SillyTavern 在准备完所有条目之后才填，宏也在那时跑。
    for (p, c) in prompts.iter_mut().filter(|(p, _)| p.marker) {
        *c = match p.id.as_str() { "charDescription" => m.eval(&req.char.description), "personaDescription" => m.eval(&req.persona.description), _ => String::new() };
    }
    for (p, _) in &prompts {
        if p.marker && !["charDescription", "personaDescription", "chatHistory", "charPersonality", "scenario"].contains(&p.id.as_str()) && !p.id.starts_with("agent") {
            notes.push(format!("「{}」第一期不填，留空", p.name));
        }
        if p.position == Position::Attach { notes.push(format!("「{}」附加到已有消息，第一期不做", p.name)); }
        if p.position == Position::Relative && p.system_prompt && !KNOWN.contains(&p.id.as_str()) && !p.id.starts_with("agent") {
            notes.push(format!("「{}」不是 SillyTavern 认的条目，不发", p.name));
        }
    }
    if preset.setting_flag("squash_system_messages") { notes.push("合并系统消息第一期不做".into()); }

    // 4. 按深度插入。照 SillyTavern：在「最新在前」的列表里，深度 i 处依次插入，同深度按顺序从大到小、再按 system、user、assistant 各合成一条。
    let mut latest: Vec<Chat> = chat.into_iter().rev().collect();
    let depth_prompts: Vec<(usize, i64, Role, &str)> = prompts.iter().filter_map(|(p, c)| match p.position {
        Position::Depth { depth, order } if !c.is_empty() && depth <= MAX_DEPTH => Some((depth, order, p.role.unwrap_or(Role::System), c.as_str())),
        _ => None,
    }).collect();
    let mut inserted = 0;
    let mut depths: Vec<usize> = depth_prompts.iter().map(|d| d.0).collect();
    depths.sort_unstable();
    depths.dedup();
    for i in depths {
        let mut groups: BTreeMap<i64, Vec<(Role, &str)>> = BTreeMap::new();
        for &(d, order, role, c) in &depth_prompts { if d == i { groups.entry(order).or_default().push((role, c)); } }
        let mut msgs = vec![];
        for (_, group) in groups.iter().rev() {
            for role in [Role::System, Role::User, Role::Assistant] {
                let joined = group.iter().filter(|(r, _)| *r == role).map(|(_, c)| *c).collect::<Vec<_>>().join("\n");
                let joined = joined.trim();
                if !joined.is_empty() { msgs.push(Chat { role, content: joined.to_owned(), name: None }); }
            }
        }
        let at = (i + inserted).min(latest.len());
        inserted += msgs.len();
        latest.splice(at..at, msgs);
    }

    // 5. 预算：上下文减回复长度，先扣掉固定条目和开场提示，剩下的从最新往前装聊天记录。
    let relative = |p: &Prompt| p.position == Position::Relative && (KNOWN.contains(&p.id.as_str()) || !p.system_prompt);
    let fixed: Vec<Message> = prompts.iter().filter(|(p, _)| relative(p) && p.id != "chatHistory").map(|(p, c)| Message { role: p.role.unwrap_or(Role::System), content: c.clone(), name: None }).collect();
    let new_chat = Message { role: Role::System, content: m.eval(&preset.setting("new_chat_prompt")), name: None };
    let empty_reply = preset.setting("send_if_empty");
    let mut budget = context.map(|c| c as i64 - params.max_tokens.unwrap_or(0) as i64 - 3 - fixed.iter().map(|x| cost(x) as i64).sum::<i64>() - cost(&new_chat) as i64);
    let has_history = prompts.iter().any(|(p, _)| p.id == "chatHistory");
    let mut history: Vec<Message> = vec![];
    let mut dropped = 0;
    if has_history {
        if latest.first().is_some_and(|c| c.role == Role::Assistant) && !empty_reply.is_empty() {
            let msg = Message { role: Role::User, content: empty_reply, name: None };
            if budget.is_none_or(|b| b >= cost(&msg) as i64) {
                if let Some(b) = &mut budget { *b -= cost(&msg) as i64; }
                history.push(msg);
            }
        }
        // 宏从最新往前跑（SillyTavern 也是这样）。
        for (k, c) in latest.iter().enumerate() {
            let msg = Message { role: c.role, content: m.eval(&c.content), name: c.name.clone() };
            if let Some(b) = &mut budget {
                if *b < cost(&msg) as i64 { dropped = latest.len() - k; break; }
                *b -= cost(&msg) as i64;
            }
            history.push(msg);
        }
        history.push(new_chat);
        history.reverse();
        if budget.is_some_and(|b| b < 0) { notes.push("固定的提示词已经超出上下文长度".into()); }
    } else if !latest.is_empty() {
        notes.push("预设里没有聊天记录（chatHistory），前文和要求都没发".into());
    }

    // 6. 按 prompt_order 排出来。
    let mut messages = vec![];
    for (p, c) in &prompts {
        if p.id == "chatHistory" { messages.append(&mut history); continue; }
        if relative(p) { messages.push(Message { role: p.role.unwrap_or(Role::System), content: c.clone(), name: None }); }
    }
    if req.prefill {
        let prefill = m.eval(&preset.setting("assistant_prefill"));
        if !prefill.is_empty() { messages.push(Message { role: Role::Assistant, content: prefill, name: None }); }
    }
    messages.retain(|x| !x.content.is_empty());
    notes.dedup();
    Assembled { messages, params, notes, dropped, unknown_macros: m.unknown.iter().cloned().collect(), request }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn preset(v: Value) -> Preset { Preset::parse(v.to_string().as_bytes()).unwrap() }

    fn req<'a>(p: &'a Preset, history: Vec<Turn>, input: &str) -> Request<'a> {
        Request {
            preset: p, char: Identity { name: "助手".into(), description: "帮 {{user}} 写东西".into() }, persona: Identity { name: "我".into(), description: String::new() },
            history, input: input.into(), model: "m".into(), model_context: None, extended_params: false, prefill: false, now: None, seed: 1,
        }
    }

    fn turn(role: Role, text: &str) -> Turn { Turn { role, text: text.into() } }

    fn show(a: &Assembled) -> Vec<String> { a.messages.iter().map(|x| format!("{}:{}", x.role.as_str(), x.content)).collect() }

    fn base(extra: Vec<Value>, order: Vec<Value>) -> Value {
        let mut prompts = vec![
            json!({"identifier": "main", "name": "Main", "role": "system", "content": "主{{setvar::k::v}}", "system_prompt": true}),
            json!({"identifier": "charDescription", "name": "Char", "marker": true, "system_prompt": true}),
            json!({"identifier": "personaDescription", "name": "Persona", "marker": true, "system_prompt": true}),
            json!({"identifier": "chatHistory", "name": "Chat History", "marker": true, "system_prompt": true}),
            json!({"identifier": "agentTask", "name": "Agent", "marker": true, "system_prompt": true}),
            json!({"identifier": "secret", "name": "Unknown system", "role": "system", "content": "不该发", "system_prompt": true}),
        ];
        prompts.extend(extra);
        json!({"prompts": prompts, "prompt_order": [{"character_id": 100001, "order": order}], "new_chat_prompt": "开始", "names_behavior": 0})
    }
    fn on(id: &str) -> Value { json!({"identifier": id, "enabled": true}) }

    #[test]
    fn relative_order_markers_and_macros() {
        let p = preset(base(vec![json!({"identifier": "u1", "name": "尾", "role": "user", "content": "{{getvar::k}}-{{char}}", "system_prompt": false})],
            vec![on("main"), on("charDescription"), on("personaDescription"), on("secret"), on("agentTask"), on("chatHistory"), on("u1")]));
        let a = assemble(&req(&p, vec![turn(Role::Assistant, "前文")], "继续 {{user}}"));
        assert_eq!(show(&a), ["system:主", "system:帮 我 写东西", "system:开始", "assistant:前文", "user:继续 我", "user:v-助手"]);
        assert_eq!(a.dropped, 0);
    }

    #[test]
    fn depth_injection_matches_sillytavern() {
        let d = |id: &str, role: &str, depth: u32, order: i64| json!({"identifier": id, "name": id, "role": role, "content": id, "system_prompt": false, "injection_position": 1, "injection_depth": depth, "injection_order": order});
        let p = preset(base(vec![d("a", "system", 0, 100), d("b", "user", 0, 100), d("c", "system", 0, 200), d("e", "system", 2, 100), d("far", "system", 9, 100)],
            vec![on("chatHistory"), on("a"), on("b"), on("c"), on("e"), on("far")]));
        let h = vec![turn(Role::User, "1"), turn(Role::Assistant, "2"), turn(Role::User, "3")];
        let a = assemble(&req(&p, h, "4"));
        // 最新在前的列表里，深度 0 依次插入 [c(200), a+合成 system, b]，反过来就是 b、a、c 在最后。
        assert_eq!(show(&a), ["system:开始", "system:far", "user:1", "assistant:2", "system:e", "user:3", "user:4", "user:b", "system:a", "system:c"]);
    }

    #[test]
    fn budget_drops_oldest_and_names() {
        let mut v = base(vec![], vec![on("main"), on("chatHistory")]);
        v["openai_max_context"] = json!(40);
        v["openai_max_tokens"] = json!(10);
        v["names_behavior"] = json!(2);
        let p = preset(v);
        let h = vec![turn(Role::User, "很早很早很早很早的一段话"), turn(Role::Assistant, "较新")];
        let a = assemble(&req(&p, h, "要求"));
        assert_eq!(show(&a), ["system:主", "system:开始", "assistant:助手: 较新", "user:我: 要求"]);
        assert_eq!(a.dropped, 1);
    }

    #[test]
    fn prompt_regex_by_depth_and_send_if_empty() {
        let mut v = base(vec![], vec![on("chatHistory")]);
        v["send_if_empty"] = json!("[继续]");
        v["extensions"] = json!({"regex_scripts": [
            {"scriptName": "去标签", "findRegex": "/<x>[\\s\\S]*?<\\/x>/g", "replaceString": "", "placement": [2], "promptOnly": true, "minDepth": 1},
            {"scriptName": "显示用", "findRegex": "/正文/g", "replaceString": "<b>正文</b>", "placement": [2], "markdownOnly": true},
        ]});
        let p = preset(v);
        let h = vec![turn(Role::Assistant, "旧<x>思考</x>正文"), turn(Role::Assistant, "新<x>思考</x>正文")];
        let a = assemble(&req(&p, h, ""));
        assert_eq!(show(&a), ["system:开始", "assistant:旧正文", "assistant:新<x>思考</x>正文", "user:[继续]"]);
    }
}
