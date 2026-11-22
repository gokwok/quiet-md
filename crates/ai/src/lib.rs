//! Quiet.md 的 AI 部分，不依赖 GPUI。
//!
//! 一个 core 负责两件事：把前文、要求和素材拼成发给模型的消息（拼装），把模型的输出变回块（成形）。
//! 下面共用模型接入，上面共用「预览 → 经 `editor::Core` 写入文档」。现在只有酒馆核心（`tavern`）。

pub mod tavern;
pub mod imagine;
