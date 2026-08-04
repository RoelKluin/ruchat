pub(crate) mod ask;
pub(crate) mod func;
mod model;
pub(super) mod server;
use crate::Result;
use clap::Parser;
use ollama_rs::Ollama;
use ollama_rs::generation::completion::request::GenerationRequest;
use serde::Deserialize;
use serde_json::Value;

pub(crate) use model::{ModelArgs, get_dynamic_history_limit};
pub(crate) use server::ServerArgs;

#[derive(Parser, Debug, Clone, Default, PartialEq, Deserialize)]
pub(crate) struct OllamaArgs {
    #[command(flatten)]
    server: ServerArgs,

    #[command(flatten)]
    model: ModelArgs,
}

/// How many model slots `OllamaArgs::init`'s loop should resolve: normally exactly
/// `nr_of_models` (the number of `-m`/`--model` flags given), except when none were given at
/// all AND the caller passed a real, usable `default` (e.g.
/// `EmbedArgs::embed_with_metadata_items`'s "all-minilm:l6-v2") — then exactly one, so that
/// default actually gets a chance to resolve instead of `init` returning an empty `models` Vec
/// and the caller failing downstream with "no model found".
///
/// Critically, this must stay 0 when `default` is also empty: `AskArgs::ask` (the `pipe`
/// command) calls `init("", cfg)` deliberately — it doesn't need a model resolved from *this*
/// call at all (Architect/Worker models come from `--team-model`/`--validator-model` instead)
/// and relies on an empty `models` Vec being a silent, harmless outcome. Forcing at least one
/// iteration unconditionally (regardless of `default`) was a real regression found from a live
/// run: `get_model(0, "")` ran anyway, resolved nothing, and errored with "No model specified"
/// before `pipe` ever reached its own per-role models — breaking every `pipe` invocation, which
/// (correctly) never passes a top-level `-m`/`--model` flag.
fn resolve_model_slot_count(nr_of_models: usize, default: &str) -> usize {
    if nr_of_models == 0 && !default.is_empty() {
        1
    } else {
        nr_of_models
    }
}

impl OllamaArgs {
    /// see [ServerArgs::init]
    pub(crate) async fn init(&self, default: &str, cfg: &Value) -> Result<(Ollama, Vec<String>)> {
        let mut server = self.server.clone();
        if let Some(s) = cfg.get("ollama").or_else(|| cfg.get("server")) {
            server.update_from_json(s)?;
        }

        let ollama = server.init()?;
        let mut models = Vec::new();
        for nr in 0..resolve_model_slot_count(self.model.get_nr_of_models(), default) {
            let model = self.model.get_model(&ollama, nr, default).await?;
            models.push(model);
        }
        Ok((ollama, models))
    }
    /// The first explicitly configured model name, or `default` — a synchronous config read,
    /// unlike `init`'s `get_model`, which validates against a live Ollama server via
    /// `list_local_models`. Used by `EmbedArgs::embed_model_name` (`core/embed.rs`).
    pub(crate) fn model_name_or(&self, default: &str) -> String {
        self.model.first_model_name().unwrap_or(default).to_string()
    }
    /// Subcommand to remove a model from the local Ollama instance.
    ///
    /// This function connects to the local Ollama instance, retrieves the specified
    /// model, and removes it from the local environment.
    ///
    /// # Returns
    ///
    /// A `Result` indicating success or failure.
    pub(crate) async fn delete_model(&self, cfg: &Value) -> Result<()> {
        let (ollama, models) = self.init("", cfg).await?;
        ollama.delete_model(models[0].clone()).await?;
        Ok(())
    }
    /// Subcommand to pull a model from the main Ollama server.
    ///
    /// This function connects to the Ollama server, retrieves the specified
    /// model, and pulls it to the local environment.
    ///
    /// # Returns
    ///
    /// A `Result` indicating success or failure.
    pub(crate) async fn pull(&self, cfg: &Value) -> Result<()> {
        let (ollama, models) = self.init("", cfg).await?;
        ollama.pull_model(models[0].clone(), false).await?;
        Ok(())
    }
    pub(crate) fn init_server(&self) -> Result<Ollama> {
        self.server.init()
    }
    /// see [ModelArgs::build_generation_request]
    pub(crate) async fn build_generation_request(
        &self,
        model: String,
        prompt: String,
        cfg: &Value,
    ) -> Result<GenerationRequest<'_>> {
        self.model
            .build_generation_request(model, prompt, cfg)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Regression: `pipe` (`AskArgs::ask`) calls `init("", cfg)` — zero `-m` flags, empty
    // default — and must get zero model-resolution attempts (an empty `models` Vec), not an
    // error. See `resolve_model_slot_count`'s doc comment for the full story.
    #[test]
    fn resolve_model_slot_count_stays_zero_when_default_is_also_empty() {
        assert_eq!(resolve_model_slot_count(0, ""), 0);
    }

    // The companion case this must not regress: `ruchat index`/`ruchat embed` pass a real
    // default and must get exactly one resolution attempt even with zero `-m` flags given.
    #[test]
    fn resolve_model_slot_count_becomes_one_when_a_real_default_exists() {
        assert_eq!(resolve_model_slot_count(0, "all-minilm:l6-v2"), 1);
    }

    #[test]
    fn resolve_model_slot_count_is_unchanged_when_models_were_already_given() {
        assert_eq!(resolve_model_slot_count(2, ""), 2);
        assert_eq!(resolve_model_slot_count(3, "all-minilm:l6-v2"), 3);
    }
}
