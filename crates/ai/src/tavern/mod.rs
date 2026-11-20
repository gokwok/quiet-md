//! 酒馆核心：照 SillyTavern 的预设拼提示词，生成的文字整理成块。
//! 只按它的格式和行为重写，不搬它的代码（AGPL）。
pub mod preset;
pub mod macros;
pub mod regex;
pub mod record;
pub mod history;
pub mod assemble;
pub mod shape;
pub mod library;
