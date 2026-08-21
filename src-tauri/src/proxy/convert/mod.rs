//! 各入口协议 → OpenAI Chat Completions 的转换实现。
//! 入口协议与上游一致时的零转换路径见 `super::passthrough`。

pub(crate) mod anthropic;
pub(crate) mod gemini;
pub(crate) mod openai;
pub(crate) mod responses;
