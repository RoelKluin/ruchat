pub(crate) mod client;
pub(crate) mod sse;

use crate::{Result, RuChatError};
pub(crate) use client::AnthropicClient;
use clap::Parser;
use serde::Deserialize;
use serde_json::Value;

/// CLI/config surface for the Anthropic (Claude) chat provider — an opt-in alternative to
/// Ollama for chat roles only (see `Orchestrator`'s `chat`/`embed` client split in
/// `core/orchestrator.rs`; embeddings/RAG/memory always stay on Ollama since Anthropic has no
/// embeddings API). Mirrors `OllamaArgs`'s CLI shape, not its field set — Anthropic's parameter
/// surface (`max_tokens` required, `temperature`/`top_p` only) is genuinely different from
/// Ollama's `ModelOptions`, not a drop-in replacement. Field names (and clap arg ids, which
/// default to the field name regardless of an explicit `long`) are all `anthropic_*`-prefixed:
/// this struct is flattened into the same `AskArgs` as `OllamaArgs`'s `ModelArgs`, which already
/// owns plain `model`/`temperature`/`top_p` ids — reusing those names here would be a clap arg-id
/// collision, not just a flag-name one.
#[derive(Parser, Debug, Clone, Default, PartialEq, Deserialize)]
pub(crate) struct AnthropicArgs {
    /// Anthropic API key — prefer the ANTHROPIC_API_KEY env var over this flag (a value passed
    /// here is visible in shell history/process listings). Separate from a Claude Pro
    /// subscription: this needs a console.anthropic.com API key with its own billing.
    #[arg(
        long = "anthropic-api-key",
        env = "ANTHROPIC_API_KEY",
        help_heading = "Anthropic (Claude) Configuration"
    )]
    api_key: Option<String>,

    /// Claude model id (e.g. claude-sonnet-4-5-20250929) — required when --chat-provider
    /// anthropic is selected. No default: model ids are versioned strings that go stale, so
    /// this deliberately isn't guessed at.
    #[arg(long, help_heading = "Anthropic (Claude) Configuration")]
    anthropic_model: Option<String>,

    /// Maximum tokens Claude may generate for one response — required by the Messages API
    /// (unlike Ollama, which has no equivalent mandatory cap).
    #[arg(long, default_value_t = 4096, help_heading = "Anthropic (Claude) Configuration")]
    anthropic_max_tokens: u32,

    #[arg(long, help_heading = "Anthropic (Claude) Configuration")]
    anthropic_temperature: Option<f32>,

    #[arg(long, help_heading = "Anthropic (Claude) Configuration")]
    anthropic_top_p: Option<f32>,
}

impl AnthropicArgs {
    /// Resolves the model id this run should use — required, no guessed default (see the
    /// field's doc comment above). `cfg`'s `"anthropic": {"model": ...}` takes precedence over
    /// `--anthropic-model`, matching how other config-file values override CLI defaults
    /// elsewhere in this codebase (e.g. `ChromaClientConfigArgs::create_client`'s
    /// `cfg.get("chroma")` merge).
    pub(crate) fn model(&self, cfg: &Value) -> Result<String> {
        cfg.get("anthropic")
            .and_then(|v| v.get("model"))
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .or_else(|| self.anthropic_model.clone())
            .ok_or(RuChatError::NoModelSpecified)
    }

    /// Builds an Anthropic `LlmClient`. Resolves the API key from
    /// `--anthropic-api-key`/`ANTHROPIC_API_KEY` (clap's `env` attribute already gives the env
    /// var the right precedence against the flag) first, then a config
    /// `"anthropic": {"api_key": ...}` value as a fallback — mirroring `ChromaClientConfigArgs`'s
    /// `chroma_token` handling. The resolved key is never logged. Does not itself validate the
    /// model id — that's `Self::model`'s job, called separately by `AskArgs::ask`
    /// (`providers/llm/ollama/ask.rs`) to resolve the per-role default model string, since
    /// `chat_stream`'s model name is a call-time parameter, not something stored on the client.
    pub(crate) fn build_client(&self, cfg: &Value) -> Result<AnthropicClient> {
        let api_key = self
            .api_key
            .clone()
            .or_else(|| {
                cfg.get("anthropic")
                    .and_then(|v| v.get("api_key"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            })
            .ok_or_else(|| {
                RuChatError::AnthropicError(
                    "no Anthropic API key configured — set ANTHROPIC_API_KEY, pass \
                     --anthropic-api-key, or set \"anthropic\": {\"api_key\": ...} in the \
                     config file"
                        .into(),
                )
            })?;
        Ok(AnthropicClient::new(
            api_key,
            self.anthropic_max_tokens,
            self.anthropic_temperature,
            self.anthropic_top_p,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn model_prefers_config_value_over_the_cli_flag() {
        let args = AnthropicArgs {
            anthropic_model: Some("claude-from-flag".to_string()),
            ..Default::default()
        };
        let cfg = json!({"anthropic": {"model": "claude-from-config"}});
        assert_eq!(args.model(&cfg).unwrap(), "claude-from-config");
    }

    #[test]
    fn model_falls_back_to_the_cli_flag_without_a_config_value() {
        let args = AnthropicArgs {
            anthropic_model: Some("claude-from-flag".to_string()),
            ..Default::default()
        };
        assert_eq!(args.model(&json!({})).unwrap(), "claude-from-flag");
    }

    #[test]
    fn model_errors_clearly_when_neither_is_set() {
        let args = AnthropicArgs::default();
        assert!(matches!(
            args.model(&json!({})),
            Err(RuChatError::NoModelSpecified)
        ));
    }

    #[test]
    fn build_client_errors_clearly_without_an_api_key() {
        let args = AnthropicArgs {
            anthropic_model: Some("claude-x".to_string()),
            ..Default::default()
        };
        let result = args.build_client(&json!({}));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("no Anthropic API key"));
    }

    #[test]
    fn build_client_succeeds_with_just_an_api_key_regardless_of_model() {
        // build_client deliberately doesn't validate the model — see its doc comment.
        let args = AnthropicArgs {
            api_key: Some("sk-test".to_string()),
            ..Default::default()
        };
        assert!(args.build_client(&json!({})).is_ok());
    }
}
