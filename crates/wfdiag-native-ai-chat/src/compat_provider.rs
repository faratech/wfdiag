//! Chat-provider adapter over the shared chat-completions client.
//!
//! Both desktop shells stream API and subscription providers identically:
//! OpenAI-compatible endpoints, native Anthropic/Gemini/DeepSeek transports,
//! and the installed Codex/Claude Code CLIs. Phi Silica remains an explicit
//! shell adapter because it owns package-bound Windows runtime state.

use crate::ai_service::AIProvider;
use crate::engine::ChatProvider;
use crate::model::{ChatRequest, ChatTurn};
use crate::openai_compat;
use crate::provider_config::ResolvedProviderConfig;
use std::future::Future;
use std::pin::Pin;
use tokio::sync::mpsc;

/// Streams the chat-completions provider set through the included client.
pub struct CompatChatProvider {
    pub provider: AIProvider,
    pub cfg: ResolvedProviderConfig,
}

impl ChatProvider for CompatChatProvider {
    fn stream<'a>(
        &'a self,
        request: &'a ChatRequest,
        tx: mpsc::Sender<String>,
    ) -> Pin<Box<dyn Future<Output = Result<ChatTurn, String>> + Send + 'a>> {
        Box::pin(async move {
            match self.provider {
                AIProvider::OpenAI
                | AIProvider::FoundryLocal
                | AIProvider::Ollama
                | AIProvider::CustomOpenAI => {
                    openai_compat::chat_stream(self.provider, &self.cfg, request, tx).await
                }
                AIProvider::Anthropic => {
                    crate::anthropic::chat_stream(&self.cfg, request, tx).await
                }
                AIProvider::Gemini => crate::gemini::chat_stream(&self.cfg, request, tx).await,
                AIProvider::DeepSeek => crate::deepseek::chat_stream(&self.cfg, request, tx).await,
                AIProvider::CodexCli => {
                    crate::codex::chat_single_shot(&self.cfg, request, tx).await
                }
                AIProvider::ClaudeCode => {
                    crate::claude_cli::chat_single_shot(&self.cfg, request, tx).await
                }
                other => Err(format!(
                    "{other} chat requires a provider transport this shell does not provide yet"
                )),
            }
        })
    }
}
