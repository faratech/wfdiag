//! DeepSeek provider constants.
//!
//! DeepSeek's API is OpenAI-compatible — all traffic goes through the shared
//! [`super::openai_compat`] client (which appends `/v1`, landing on
//! DeepSeek's documented OpenAI-compatible root). No client code lives here.

/// Base URL; the compat client appends `/v1`.
pub const DEEPSEEK_API_BASE: &str = "https://api.deepseek.com";

/// Default model when the `deepseekModel` setting is empty.
/// Alternative: deepseek-reasoner (chain-of-thought, slower).
pub const DEEPSEEK_DEFAULT_MODEL: &str = "deepseek-chat";
