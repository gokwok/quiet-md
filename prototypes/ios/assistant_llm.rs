//! OpenAI 兼容接口的客户端：列出模型、流式对话。协议和 SSE 解析交给 async-openai，网络是 reqwest（系统 TLS，走 Security.framework）。
//! 请求跑在一个只有一个工作线程的 tokio 运行时上；调用方拿到的 future 和 channel 在 GPUI 的执行器上等就行。
//! 各家服务的模型信息出入很大，只有 `id` 是一定有的：请求和回复都用 async-openai 的 byot（自带类型），按 JSON 宽松地读。
use std::{sync::OnceLock, time::Duration};

use async_openai::{Client, config::{Config, OpenAIConfig}, error::OpenAIError};
use futures::{StreamExt, channel::{mpsc, oneshot}};
use serde_json::{Value, json};

use crate::settings::tr;

#[derive(Clone)]
pub struct Model {
    pub id: String,
    /// 服务给的显示名（`display_name` 或 `name`），和 id 一样时不留。
    pub name: Option<String>,
    pub owner: Option<String>,
    /// 上下文长度（token）。
    pub context: Option<u64>,
    pub reasoning: bool,
    /// 支持的推理强度（`reasoning_levels`），没给就是空。
    pub efforts: Vec<String>,
}

impl Model {
    fn read(v: &Value) -> Option<Model> {
        let text = |k: &str| v.get(k).and_then(Value::as_str).map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned);
        let id = text("id")?;
        let name = text("display_name").or_else(|| text("name")).filter(|n| *n != id);
        let context = ["context_length", "context_window", "max_context_length"].iter().find_map(|k| v.get(*k).and_then(Value::as_u64))
            .or_else(|| v.pointer("/top_provider/context_length").and_then(Value::as_u64));
        let efforts: Vec<String> = v.get("reasoning_levels").and_then(Value::as_array).map_or_else(Vec::new, |a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect());
        let reasoning = v.get("reasoning").and_then(Value::as_bool).unwrap_or(false) || !efforts.is_empty()
            || v.get("supported_parameters").and_then(Value::as_array).is_some_and(|a| a.iter().any(|p| p.as_str() == Some("reasoning")));
        Some(Model { id, name, owner: text("owned_by"), context, reasoning, efforts })
    }
    pub fn label(&self) -> &str { self.name.as_deref().unwrap_or(&self.id) }
}

/// 流式对话里陆续收到的东西。
pub enum Event { Thinking, Text(String), Done, Failed(String) }

pub(crate) fn runtime() -> &'static tokio::runtime::Runtime {
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME.get_or_init(|| tokio::runtime::Builder::new_multi_thread().worker_threads(1).thread_name("quiet-md-llm").enable_all().build().expect("tokio runtime"))
}

/// OpenAI 兼容的配置；没有 Key 时不发 `Authorization`（async-openai 会发一个空的 `Bearer `，有的服务因此拒绝）。
struct Endpoint { config: OpenAIConfig, keyed: bool }

impl Config for Endpoint {
    fn headers(&self) -> reqwest::header::HeaderMap {
        let mut headers = self.config.headers();
        if !self.keyed { headers.remove(reqwest::header::AUTHORIZATION); }
        headers
    }
    fn url(&self, path: &str) -> String { self.config.url(path) }
    fn query(&self) -> Vec<(&str, &str)> { self.config.query() }
    fn api_base(&self) -> &str { self.config.api_base() }
    fn api_key(&self) -> &secrecy::SecretString { self.config.api_key() }
}

fn client(base: &str, key: &str) -> Result<Client<Endpoint>, String> {
    // 不跟随重定向：Key 只发往填写的这个地址。读超时按两段数据之间算，推理模型想得久也不会被掐断。
    let http = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10)).read_timeout(Duration::from_secs(90)).build().map_err(|e| e.to_string())?;
    Ok(Client::build(http, Endpoint { config: OpenAIConfig::new().with_api_base(base).with_api_key(key), keyed: !key.is_empty() }))
}

/// 列出模型，按服务给的顺序。
pub async fn models(base: String, key: String) -> Result<Vec<Model>, String> {
    let (tx, rx) = oneshot::channel();
    runtime().spawn(async move {
        let result = async {
            let list: Value = tokio::time::timeout(Duration::from_secs(20), client(&base, &key)?.models().list_byot()).await
                .map_err(|_| tr("连接超时", "The connection timed out").to_owned())?.map_err(describe)?;
            let data = list.get("data").and_then(Value::as_array).ok_or_else(|| tr("这个地址返回的不是模型列表", "This address did not return a model list").to_owned())?;
            let mut seen = std::collections::HashSet::new();
            Ok(data.iter().filter_map(Model::read).filter(|m| seen.insert(m.id.clone())).collect())
        }.await;
        let _ = tx.send(result);
    });
    rx.await.unwrap_or_else(|_| Err(tr("请求中断了", "The request was interrupted").into()))
}

/// 流式对话。收的那头丢掉（`Receiver` 被 drop）时请求跟着停。
pub fn chat(base: String, key: String, model: String, effort: Option<String>, messages: Vec<(&'static str, String)>) -> mpsc::UnboundedReceiver<Event> {
    let (tx, rx) = mpsc::unbounded();
    runtime().spawn(async move {
        let send = |e: Event| tx.unbounded_send(e).is_ok();
        let messages: Vec<Value> = messages.into_iter().map(|(role, content)| json!({ "role": role, "content": content })).collect();
        let mut body = json!({ "model": model, "messages": messages, "stream": true });
        if let Some(effort) = effort { body["reasoning_effort"] = effort.into(); }
        let client = match client(&base, &key) { Ok(c) => c, Err(e) => { send(Event::Failed(e)); return; } };
        let mut stream = match client.chat().create_stream_byot::<Value, Value>(body).await { Ok(s) => s, Err(e) => { send(Event::Failed(describe(e))); return; } };
        while let Some(chunk) = stream.next().await {
            let chunk = match chunk { Ok(c) => c, Err(e) => { send(Event::Failed(describe(e))); return; } };
            // 有的服务把错误放在流里。
            if let Some(message) = chunk.pointer("/error/message").and_then(Value::as_str) { send(Event::Failed(message.into())); return; }
            let Some(delta) = chunk.pointer("/choices/0/delta") else { continue };
            let thinking = ["reasoning_content", "reasoning"].iter().any(|k| delta.get(*k).and_then(Value::as_str).is_some_and(|s| !s.is_empty()));
            let ok = match delta.get("content").and_then(Value::as_str) {
                Some(text) if !text.is_empty() => send(Event::Text(text.into())),
                _ if thinking => send(Event::Thinking),
                _ => true,
            };
            if !ok { return; }
        }
        send(Event::Done);
    });
    rx
}

/// 出错时给人看的一句：服务自己的说法优先。
fn describe(e: OpenAIError) -> String {
    let short = |s: &str| { let s = s.trim(); if s.chars().count() > 160 { format!("{}…", s.chars().take(160).collect::<String>()) } else { s.to_owned() } };
    match e {
        OpenAIError::ApiError(r) if !r.api_error.message.trim().is_empty() => short(&r.api_error.message),
        OpenAIError::ApiError(r) => match r.status_code.as_u16() {
            401 | 403 => tr("API Key 无效或没有权限", "The API Key is invalid or lacks permission").into(),
            404 => tr("这个地址上没有找到接口", "No API was found at this address").into(),
            429 => tr("请求过多，请稍后重试", "Too many requests. Try again shortly").into(),
            code => format!("HTTP {code}"),
        },
        OpenAIError::Reqwest(e) if e.is_timeout() => tr("连接超时", "The connection timed out").into(),
        OpenAIError::Reqwest(e) if e.is_connect() => tr("无法连接到这个地址", "Could not connect to this address").into(),
        OpenAIError::Reqwest(e) => match e.status() { Some(s) => format!("HTTP {}", s.as_u16()), None => tr("网络连接中断了", "The connection was interrupted").into() },
        // 错误的 JSON 和 async-openai 期望的不一样（比如 `code` 是数字）：照样把服务的说法拿出来。
        OpenAIError::JSONDeserialize(_, content) if let Some(message) = serde_json::from_str::<Value>(&content).ok()
            .and_then(|v| ["/error/message", "/message", "/error"].iter().find_map(|k| v.pointer(k).and_then(Value::as_str).map(str::to_owned))) => short(&message),
        OpenAIError::JSONDeserialize(..) => tr("这个地址返回的不是 OpenAI 兼容的内容", "This address did not answer like an OpenAI-compatible API").into(),
        e => short(&e.to_string()),
    }
}
