use crate::options::merge_options_json;
use crate::{Result, RuChatError};
use clap::Parser;
use ollama_rs::generation::completion::request::GenerationRequest;
use ollama_rs::Ollama;
use serde_json::Value;

pub(crate) fn get_dynamic_history_limit(model_name: &str) -> u64 {
    if model_name.contains("qwen2.5") {
        128_000
    } else if model_name.contains("llama3") {
        8_192
    } else {
        4_096
    } // Safe fallback
}

#[derive(Parser, Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct ModelArgs {
    /// Model(s) to (down)load and use.
    #[arg(short, long, value_delimiter = ',', help_heading = "Model Selection")]
    model: Vec<String>,

    /// Path to a JSON file or a JSON string to amend default generation options.
    #[arg(
        short,
        long,
        help_heading = "Advanced Model Options",
        hide_short_help = true,
        hide_long_help = false
    )]
    options: Option<String>,

    /// Size of the prompt context (tokens) used to generate the next token.
    #[arg(
        long,
        help_heading = "Advanced Generation Parameters",
        hide_short_help = true,
        hide_long_help = false
    )]
    pub num_ctx: Option<u64>,

    /// The temperature of the model. Increasing the temperature will make the model answer more creatively.
    #[arg(long, value_parser = clap::value_parser!(f32), help_heading = "Advanced Generation Parameters", hide_short_help = true, hide_long_help = false)]
    pub temperature: Option<f32>,

    /// Reduces the probability of generating nonsense. A higher value (e.g. 100) will give more diverse answers.
    #[arg(
        long,
        help_heading = "Advanced Generation Parameters",
        hide_short_help = true,
        hide_long_help = false
    )]
    pub top_k: Option<u32>,

    /// Works together with top-k. A higher value (e.g., 0.95) will lead to more diverse text.
    #[arg(long, value_parser = clap::value_parser!(f32), help_heading = "Advanced Generation Parameters", hide_short_help = true, hide_long_help = false)]
    pub top_p: Option<f32>,

    /// Sets how strongly to penalize repetitions. A higher value (e.g., 1.5) will penalize repetitions more strongly.
    #[arg(long, value_parser = clap::value_parser!(f32), help_heading = "Advanced Generation Parameters", hide_short_help = true, hide_long_help = false)]
    pub repeat_penalty: Option<f32>,

    /// Sets the stop sequences to use. When this pattern is encountered the LLM will stop generating text.
    #[arg(
        long,
        value_delimiter = ',',
        help_heading = "Advanced Generation Parameters",
        hide_short_help = true,
        hide_long_help = false
    )]
    pub stop: Option<Vec<String>>,

    /// Maximum number of tokens to predict when generating text. (-1 = infinite, -2 = fill context).
    #[arg(
        long,
        help_heading = "Advanced Generation Parameters",
        hide_short_help = true,
        hide_long_help = false
    )]
    pub num_predict: Option<i32>,

    /// Sets the random number seed to use for generation. Setting this to a specific number will make the model generate the same text for the same prompt.
    #[arg(
        long,
        help_heading = "Advanced Generation Parameters",
        hide_short_help = true,
        hide_long_help = false
    )]
    pub seed: Option<i32>,
}

impl ModelArgs {
    pub(crate) async fn build_generation_request(
        &self,
        model: String,
        prompt: String,
        cfg: &Value,
    ) -> Result<GenerationRequest<'_>> {
        // 1. Get base options from file/string or start with empty JSON
        let mut opts_val = if let Some(opts_raw) = cfg.get("model_options") {
            let (opts_val, _etc) = merge_options_json(format!("{opts_raw}").as_str()).await?;
            opts_val
        } else {
            serde_json::json!({})
        };

        // 2. Merge CLI flags into JSON to bypass private fields
        if let Some(v) = self.num_ctx {
            opts_val["num_ctx"] = serde_json::json!(v);
        }
        if let Some(v) = self.temperature {
            opts_val["temperature"] = serde_json::json!(v);
        }
        if let Some(v) = self.top_k {
            opts_val["top_k"] = serde_json::json!(v);
        }
        if let Some(v) = self.top_p {
            opts_val["top_p"] = serde_json::json!(v);
        }
        if let Some(v) = self.repeat_penalty {
            opts_val["repeat_penalty"] = serde_json::json!(v);
        }
        if let Some(v) = &self.stop {
            opts_val["stop"] = serde_json::json!(v);
        }
        if let Some(v) = self.num_predict {
            opts_val["num_predict"] = serde_json::json!(v);
        }
        if let Some(v) = self.seed {
            opts_val["seed"] = serde_json::json!(v);
        }

        // 3. Deserialize back to ModelOptions
        let model_opts: ollama_rs::models::ModelOptions = serde_json::from_value(opts_val)
            .map_err(|e| {
                tracing::error!(error = ?e, "failed to deserialize JSON into ModelOptions");
                e
            })
            .map_err(RuChatError::SerdeError)?;

        Ok(GenerationRequest::new(model, prompt).options(model_opts))
    }
    #[cfg(test)]
    pub(crate) fn new(model: &str, options: Option<&str>) -> Self {
        let model = model.split(',').map(|s| s.trim().to_string()).collect();
        let options = options.map(|s| s.to_string());
        Self {
            model,
            options,
            ..Default::default()
        }
    }
    pub(super) async fn get_model(
        &self,
        ollama: &Ollama,
        nr: usize,
        default: &str,
    ) -> Result<String> {
        let model = match self.model.get(nr).map(|s| s.as_str()) {
            Some("") => default,
            Some(m) => m,
            None => return Err(RuChatError::NoModelSpecified),
        };
        if model.is_empty() {
            Err(RuChatError::NoModelSpecified)
        } else {
            get_model_name(ollama, model).await
        }
    }
    pub(super) fn get_nr_of_models(&self) -> usize {
        self.model.len()
    }
}

async fn get_model_name(ollama: &Ollama, name: &str) -> Result<String> {
    get_model_name_inner(ollama, name, 0).await
}

async fn get_model_name_inner(ollama: &Ollama, name: &str, pull_attempts: u8) -> Result<String> {
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_alphanumeric() || c == ':' || c == '-' || c == '.' || c == '/')
    {
        return Err(RuChatError::ModelError(format!("invalid name: {name}")));
    }
    let model_list = ollama
        .list_local_models()
        .await
        .map_err(|_| RuChatError::ModelError(format!("{name} not found")))?;
    let model = model_list.iter().find(|m| {
        if name.contains(":") {
            m.name == name
        } else {
            m.name.starts_with(name)
        }
    });

    match model {
        Some(model) => Ok(model.name.clone()),
        None => {
            const MAX_PULL_ATTEMPTS: u8 = 1;
            if pull_attempts >= MAX_PULL_ATTEMPTS {
                return Err(RuChatError::ModelError(format!(
                    "{name} still not found in local models after {pull_attempts} pull attempt(s)"
                )));
            }
            ollama.pull_model(name.to_string(), false).await?;
            Box::pin(get_model_name_inner(ollama, name, pull_attempts + 1)).await
        }
    }
}

#[cfg(test)]
mod build_generation_request_tests {
    use super::*;

    #[tokio::test]
    async fn cli_flags_only() {
        let args = ModelArgs {
            temperature: Some(0.5),
            seed: Some(42),
            ..Default::default()
        };
        let req = args
            .build_generation_request("m".into(), "p".into(), &serde_json::json!({}))
            .await
            .unwrap();
        let v = serde_json::to_value(&req).unwrap();
        assert_eq!(v["options"]["temperature"], 0.5);
        assert_eq!(v["options"]["seed"], 42);
    }

    #[tokio::test]
    async fn cli_flag_wins_over_config_model_options() {
        let args = ModelArgs {
            temperature: Some(0.25),
            ..Default::default()
        };
        let cfg = serde_json::json!({ "model_options": { "temperature": 0.1 } });
        let req = args
            .build_generation_request("m".into(), "p".into(), &cfg)
            .await
            .unwrap();
        let v = serde_json::to_value(&req).unwrap();
        assert_eq!(v["options"]["temperature"], 0.25);
    }

    // Pins a pre-existing (not introduced by the round-trip removal above) bug:
    // `ModelOptions::default()` serializes to `{}` (every field is `None` and
    // `skip_serializing_if`-omitted), so `merge_options_json`'s
    // `defaults.contains_key(&k)` gate is never true for ANY key coming from a
    // `model_options` file/string — config-supplied values silently land in the
    // discarded `remain` map instead of `defaults`, so they never reach the
    // final `ModelOptions`. Only CLI-flag values (set unconditionally, no gate)
    // actually take effect. See TODO.md.
    #[tokio::test]
    async fn config_only_model_options_are_currently_silently_dropped() {
        let args = ModelArgs::default();
        let cfg = serde_json::json!({ "model_options": { "seed": 7 } });
        let req = args
            .build_generation_request("m".into(), "p".into(), &cfg)
            .await
            .unwrap();
        let v = serde_json::to_value(&req).unwrap();
        assert_eq!(v["options"]["seed"], serde_json::Value::Null);
    }
}

