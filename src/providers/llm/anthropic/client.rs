use super::sse::{SseEffect, parse_event};
use crate::agent::llm_client::{ChatStream, LlmClient};
use crate::{Result, RuChatError};
use async_trait::async_trait;
use eventsource_stream::Eventsource;
use futures_util::StreamExt;
use ollama_rs::generation::chat::{ChatMessage, ChatMessageResponse, MessageRole};
use ollama_rs::generation::embeddings::GenerateEmbeddingsResponse;
use ollama_rs::generation::embeddings::request::GenerateEmbeddingsRequest;
use serde_json::json;

const API_URL: &str = "https://api.anthropic.com/v1/messages";
const API_VERSION: &str = "2023-06-01";

/// `LlmClient` impl for Anthropic's Messages API — chat-only (see `generate_embeddings` below).
/// Built by `AnthropicArgs::build_client` (`providers/llm/anthropic.rs`), never constructed
/// directly from elsewhere.
pub(crate) struct AnthropicClient {
    http: reqwest::Client,
    api_key: String,
    max_tokens: u32,
    temperature: Option<f32>,
    top_p: Option<f32>,
}

// Manual impl (not `#[derive(Debug)]`) so `api_key` — a secret — is never printed in full if
// this ever ends up in a `{:?}`/log line, matching this codebase's existing discipline around
// `chroma_token`/`chroma_client`.
impl std::fmt::Debug for AnthropicClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AnthropicClient")
            .field("api_key", &"<redacted>")
            .field("max_tokens", &self.max_tokens)
            .field("temperature", &self.temperature)
            .field("top_p", &self.top_p)
            .finish()
    }
}

impl AnthropicClient {
    pub(crate) fn new(
        api_key: String,
        max_tokens: u32,
        temperature: Option<f32>,
        top_p: Option<f32>,
    ) -> Self {
        Self {
            http: reqwest::Client::new(),
            api_key,
            max_tokens,
            temperature,
            top_p,
        }
    }
}

#[async_trait]
impl LlmClient for AnthropicClient {
    async fn chat_stream(&self, model: &str, messages: Vec<ChatMessage>) -> Result<ChatStream> {
        // Anthropic's Messages API takes the system prompt as a top-level field, not a message
        // in the array. `Agent::query_stream` (`core/agent.rs`) only ever constructs exactly
        // `[ChatMessage::system(..), ChatMessage::user(..)]` per round — no assistant/tool
        // messages exist in ruchat's current single-turn-per-round design — so this mapping
        // just pulls the one System entry out and passes the rest through unchanged.
        let mut system = String::new();
        let mut api_messages = Vec::with_capacity(messages.len());
        for m in messages {
            match m.role {
                MessageRole::System => system.push_str(&m.content),
                MessageRole::User => {
                    api_messages.push(json!({"role": "user", "content": m.content}))
                }
                MessageRole::Assistant => {
                    api_messages.push(json!({"role": "assistant", "content": m.content}))
                }
                // Not produced anywhere in ruchat's message construction today — ruchat's tool
                // calling is a text convention the model is prompted into (a fenced JSON code
                // block), not the API's native tool-use message role. Mapped as a user turn
                // rather than silently dropped content, in case that ever changes.
                MessageRole::Tool => {
                    api_messages.push(json!({"role": "user", "content": m.content}))
                }
            }
        }

        let mut body = json!({
            "model": model,
            "max_tokens": self.max_tokens,
            "messages": api_messages,
            "stream": true,
        });
        if !system.is_empty() {
            body["system"] = json!(system);
        }
        if let Some(t) = self.temperature {
            body["temperature"] = json!(t);
        }
        if let Some(p) = self.top_p {
            body["top_p"] = json!(p);
        }

        let response = self
            .http
            .post(API_URL)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", API_VERSION)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| RuChatError::AnthropicError(format!("request failed: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(RuChatError::AnthropicError(format!(
                "HTTP {status}: {text}"
            )));
        }

        let model_name = model.to_string();
        let event_stream = response.bytes_stream().eventsource();
        // Only `content_block_delta`'s text deltas become a chunk `Agent::query_stream` sees;
        // `Ignore`d bookkeeping events (message_start/content_block_start/.../ping) and `Done`
        // (message_stop) produce nothing — that loop only accumulates `chunk.message.content`
        // and never inspects `done`, so there's no need for a special final marker chunk, and
        // the stream simply ends when Anthropic closes the connection after `message_stop`.
        let mapped = event_stream.filter_map(move |item| {
            let model_name = model_name.clone();
            async move {
                let parsed = match item {
                    Ok(event) => parse_event(&event.event, &event.data),
                    Err(e) => Err(RuChatError::AnthropicError(format!("SSE parse error: {e}"))),
                };
                match parsed {
                    Ok(SseEffect::TextDelta(text)) => Some(Ok(ChatMessageResponse {
                        model: model_name,
                        created_at: String::new(),
                        message: ChatMessage::assistant(text),
                        logprobs: None,
                        done: false,
                        final_data: None,
                    })),
                    Ok(SseEffect::Done | SseEffect::Ignore) => None,
                    Err(e) => Some(Err(e)),
                }
            }
        });
        Ok(Box::pin(mapped))
    }

    async fn generate_embeddings(
        &self,
        _req: GenerateEmbeddingsRequest,
    ) -> Result<GenerateEmbeddingsResponse> {
        // Intentionally unreachable in normal operation: `Orchestrator` keeps a separate,
        // always-Ollama-backed `embed` client for every embeddings call (RAG, memorize/recall,
        // the Worker's `retrieve` tool) specifically because Anthropic has no embeddings API —
        // see `Orchestrator`'s `chat`/`embed` field split in `core/orchestrator.rs`.
        Err(RuChatError::AnthropicError(
            "Anthropic has no embeddings API — configure an Ollama embed_model for RAG/memory \
             features instead"
                .into(),
        ))
    }
}
