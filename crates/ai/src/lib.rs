//! Quiet.md 的 AI 部分，不依赖 GPUI。
//!
//! 一个 core 负责两件事：把前文、要求和素材拼成发给模型的消息（拼装），把模型的输出变回块（成形）。
//! 下面共用模型接入，上面共用「预览 → 经 `editor::Core` 写入文档」。现在只有酒馆核心（`tavern`）。
//! 助手的素材（写作设置和预设、图像库、语音角色）放在所有空间共用的全局库里（`global`）。
//! 助手（MarginAI）的一次 run 由可换的核心驱动（`run`）：酒馆核心一次请求，Agent 核心带文档工具循环。

pub mod global;
pub mod run;
pub mod tavern;
pub mod imagine;
