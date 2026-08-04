use crate::agent::llm_client::LlmClient;
use crate::agent::pipeline::AgentPipeline;
use crate::anthropic::AnthropicArgs;
use crate::cli::prompt::PromptArgs;
use crate::io::Io;
use crate::ollama::OllamaArgs;
use crate::orchestrator::{DebugBreakpoints, Orchestrator};
use crate::{Result, RuChatError};
use clap::{Parser, ValueEnum};
use serde_json::Value;
use std::sync::Arc;

/// Which backend a run's chat-capable roles (Architect/Worker/Validator/Critics/Summarizer/
/// Scoper/Librarian) use. Embeddings (RAG, memorize/recall) always stay on Ollama regardless —
/// see `Orchestrator`'s `chat`/`embed` client split (`core/orchestrator.rs`) — Anthropic has no
/// embeddings API, so this flag only ever affects the chat client.
#[derive(ValueEnum, Clone, Debug, Default, PartialEq)]
enum ChatProvider {
    #[default]
    Ollama,
    Anthropic,
}

const DEFAULT_MODEL: &str = "qwen2.5vl:latest";

/// Command-line arguments for asking a question to a model.
///
/// This struct defines the arguments required to ask a question
/// to a model, including model details, prompt, and input options.
#[derive(Parser, Debug, Clone, Default, PartialEq)]
pub(crate) struct AskArgs {
    /// Provide a full JSON config for the team
    #[arg(
        short,
        long,
        group = "agent_config",
        conflicts_with = "team_model",
        help_heading = "Agent Configuration"
    )]
    agentic: Option<String>,

    /// Quick-start: Just enable Worker+Architect with this model
    #[arg(
        long,
        group = "agent_config",
        conflicts_with = "agentic",
        help_heading = "Agent Configuration"
    )]
    team_model: Option<String>,

    /// Enable RAG by specifying a Chroma collection name
    #[arg(long, help_heading = "RAG Configuration")]
    collection: Option<String>,

    /// Model for the Librarian agent's query embeddings (default: all-minilm:l6-v2)
    #[arg(long, help_heading = "RAG Configuration")]
    librarian_model: Option<String>,

    /// Embedding model the Librarian (and Worker's `retrieve` tool) uses to
    /// vectorize query text against Chroma (default: all-minilm:l6-v2)
    #[arg(long, help_heading = "RAG Configuration")]
    librarian_embed_model: Option<String>,

    /// Override maximum iterations
    #[arg(long, help_heading = "Agent Configuration")]
    iterations: Option<u64>,

    /// Model for an optional Validator agent
    #[arg(long, help_heading = "Agent Configuration")]
    validator_model: Option<String>,

    /// Add one or more specific critics (e.g., --critic "Security" --critic "Performance")
    #[arg(long, action = clap::ArgAction::Append, help_heading = "Agent Configuration")]
    critic: Vec<String>,

    /// Path to a single JSON file defining debug sequence + context imputations.
    #[arg(
        long,
        hide_short_help = true,
        hide_long_help = false,
        help_heading = "Debugging"
    )]
    debug_sequence: Option<String>,

    /// With --debug-sequence: pause for interactive inspection after every role in the
    /// sequence, waiting on stdin (Enter to continue, 'c' to stop pausing, 'q' to abort).
    #[arg(
        long,
        requires = "debug_sequence",
        hide_short_help = true,
        hide_long_help = false,
        help_heading = "Debugging"
    )]
    step: bool,

    /// With --debug-sequence: pause for interactive inspection after this specific role (e.g.
    /// --breakpoint Worker). Repeatable.
    #[arg(
        long,
        requires = "debug_sequence",
        action = clap::ArgAction::Append,
        hide_short_help = true,
        hide_long_help = false,
        help_heading = "Debugging"
    )]
    breakpoint: Vec<String>,

    /// Resume the last interrupted agentic run from its checkpoint (ruchat_checkpoint.json)
    /// instead of starting a new one — the prompt argument is ignored; the resumed run
    /// continues toward its original goal. Errors clearly if no checkpoint exists.
    #[arg(long, conflicts_with_all = ["debug_sequence", "prompt"], help_heading = "Agent Configuration")]
    resume: bool,

    /// Interactive human-in-the-loop approval gate: pause before committing, print the latest
    /// plan and the pending (uncommitted) diff, and wait for an explicit 'y' on stdin before
    /// proceeding — anything else stops the run without committing. Off by default; ruchat's
    /// only approval mechanism otherwise is automated Critics plus post-hoc branch review.
    #[arg(long, help_heading = "Agent Configuration")]
    approve: bool,

    #[command(flatten)]
    prompt: PromptArgs,

    #[command(flatten)]
    ollama: OllamaArgs,

    /// Which backend chat-capable roles use — Ollama (default, fully local) or Anthropic
    /// (opt-in, requires --anthropic-model and an API key; embeddings/RAG/memory always stay on
    /// Ollama regardless, since Anthropic has no embeddings API).
    #[arg(long, value_enum, default_value_t = ChatProvider::Ollama, help_heading = "Agent Configuration")]
    chat_provider: ChatProvider,

    #[command(flatten)]
    anthropic: AnthropicArgs,
}

impl AskArgs {
    pub fn into_config(self, default_model: &str) -> Result<serde_json::Value> {
        // 1. Start with base: either provided JSON or empty object
        let mut config: serde_json::Value = if let Some(ref json_str) = self.agentic {
            serde_json::from_str(json_str)
                .map_err(|e| {
                    // Deliberately not logging `json_str` itself: an `--agentic` config can
                    // embed a Librarian's `chroma_client` (which carries `chroma_token`, a
                    // secret) as a nested string, and the parse error already includes enough
                    // position/context to debug without echoing the raw config.
                    tracing::error!(error = ?e, "Failed to parse agentic JSON config");
                    e
                })
                .map_err(RuChatError::SerdeError)?
        } else {
            serde_json::json!({})
        };
        // Inject Librarian if collection is provided via flag
        if let Some(col) = self.collection {
            config["Librarian"] = serde_json::json!({
                "chroma_client": "{\"chroma_server\": \"http://localhost:8000\"}", // Default server
                "status_msg": "Searching knowledge base...",
                // `recall_prior_memories`'s ad-hoc pre-run recall doesn't go through the
                // Librarian's own LLM-driven query (which picks a collection name itself,
                // guided by the `task_hint` below) — it needs this collection name as plain,
                // structured config it can actually read, or it falls back to
                // `ChromaCollectionConfigArgs::default()`'s literal collection named "default",
                // which has nothing to do with what `--collection` here actually configured.
                "memory_collection": col,
            });
            // Ensure the librarian uses the correct collection in the prompt
            config["task_hint"] = serde_json::json!(format!("Query the {} collection", col));
        }

        // Librarian needs a CHAT model (`model`, defaulted below like every
        // other role) to write the JSON query, and a separate EMBEDDING
        // model (`embed_model`) to vectorize it — these must never be the
        // same value, one is generative, the other embedding-only.
        if let Some(librarian) = config.get_mut("Librarian") {
            if let Some(m) = &self.librarian_model {
                librarian["model"] = serde_json::json!(m);
            }
            let embed_model = self
                .librarian_embed_model
                .clone()
                .unwrap_or_else(|| "all-minilm:l6-v2".to_string());
            librarian["embed_model"] = serde_json::json!(embed_model);
        }

        // Handle Multiple Critics
        if !self.critic.is_empty() {
            let mut critics_array = Vec::new();
            for c_name in self.critic {
                critics_array.push(serde_json::json!({
                    "model": self.team_model.clone().unwrap_or_else(|| default_model.to_string()),
                    "task": format!("Review the implementation specifically for {} concerns.", c_name),
                    "status_msg": format!("{} Critic is reviewing...", c_name),
                    "name": c_name,
                }));
            }
            config["Critics"] = serde_json::Value::Array(critics_array);
        }

        // Handle team_model shortcut
        if let Some(model) = self.team_model {
            if config.get("Architect").is_none() {
                config["Architect"] = serde_json::json!({ "model": model });
            }
            if config.get("Worker").is_none() {
                config["Worker"] = serde_json::json!({ "model": model });
            }
            if config.get("Scoper").is_none() {
                config["Scoper"] = serde_json::json!({ "model": model });
            }
        }

        // Handle validator shortcut
        if let Some(v_model) = self.validator_model {
            config["Validator"] = serde_json::json!({ "model": v_model });
        }

        // Override iterations if flag is present
        if let Some(iters) = self.iterations {
            config["iterations"] = serde_json::json!(iters);
        }

        // Inject global model as fallback for agents missing one
        for role in [
            "Scoper",
            "Architect",
            "Worker",
            "Librarian",
            "Validator",
            "Summarizer",
        ] {
            if let Some(agent) = config.get_mut(role)
                && agent.get("model").is_none()
            {
                agent["model"] = default_model.into();
            }
        }

        Ok(config)
    }

    /// The ask command handles prompted questions with context using a model.
    ///
    /// This function connects to a model using the provided arguments,
    /// generates a response to the specified prompt, and outputs the response.
    ///
    /// # Returns
    ///
    /// A `Result` indicating success or failure.
    pub(crate) async fn ask(&self, cfg: &Value) -> Result<()> {
        let mut cio = Io::new();
        // `--resume` ignores the prompt entirely (the checkpoint's own goal is what continues)
        // — skip prompt resolution altogether rather than blocking on interactive stdin for a
        // value that will never be used. `clap`'s `conflicts_with_all` on `resume` already
        // rejects `--resume` combined with an explicit prompt/`--explicit-prompt`, so this is
        // purely about not reading stdin, not a redundant validation.
        let prompt = if self.resume {
            String::new()
        } else {
            match self.prompt.get_prompt() {
                Ok(p) => p,
                Err(RuChatError::NoPromptProvided) => {
                    let mut input = String::new();
                    while let Ok(line) = cio.read_line().await {
                        if line == "---" {
                            cio.write_error_line("End marker received, finishing input...")
                                .await?;
                            break;
                        }
                        input += line.as_str();
                    }
                    input
                }
                Err(e) => return Err(e),
            }
        };

        // `embed` (RAG/memorize/recall) always stays on Ollama — Anthropic has no embeddings
        // API — so this client is resolved unconditionally regardless of --chat-provider.
        let (ollama, ollama_models) = self.ollama.init("", cfg).await?;
        let embed: Arc<dyn LlmClient> = Arc::new(ollama);

        // The per-role "model" config fallback (`into_config`'s default-model injection) must
        // be a name the actually-selected chat provider understands — an Ollama model tag would
        // be meaningless sent to Claude's API, and vice versa.
        let (chat, model_name): (Arc<dyn LlmClient>, String) = match self.chat_provider {
            ChatProvider::Anthropic => (
                Arc::new(self.anthropic.build_client(cfg)?),
                self.anthropic.model(cfg)?,
            ),
            ChatProvider::Ollama => (
                embed.clone(),
                ollama_models
                    .first()
                    .cloned()
                    .unwrap_or_else(|| DEFAULT_MODEL.to_string()),
            ),
        };
        let config = self.clone().into_config(&model_name)?;

        let pipeline = if config.get("Architect").is_some() || config.get("Worker").is_some() {
            let orchestrator = Orchestrator::new(config, chat, embed, cfg).await?;
            AgentPipeline::Orchestrator {
                orchestrator,
                goal: prompt,
                debug_sequence: self.debug_sequence.clone(),
                breakpoints: DebugBreakpoints::new(self.step, self.breakpoint.clone()),
                resume: self.resume,
                approve_commit: self.approve,
            }
        } else if self.resume {
            // OneShot has no Context/Stage machine at all — nothing a checkpoint could resume.
            // `--resume` needs the same `--team-model`/`--agentic` (or config-file equivalent)
            // the original interrupted run used, to reconstruct the same Architect/Worker roles
            // the checkpoint's turn history was talking to.
            return Err(RuChatError::Is(
                "--resume requires --team-model or --agentic (or a config file with Architect/\
                 Worker) — the checkpoint only recovers conversation state, not which roles/\
                 models to reconstruct."
                    .into(),
            ));
        } else {
            AgentPipeline::OneShot {
                ollama: chat,
                model: model_name,
                prompt,
            }
        };
        crate::tui::render::render_pipeline_stream(pipeline.run(), &mut cio).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_ask_args_default() {
        let args = AskArgs::default();
        assert_eq!(args.agentic, None);
    }
    // Regression: `into_config`'s parse-failure branch used to log the raw `--agentic` string
    // verbatim (`config = %json_str`). That string can legitimately embed a Librarian's
    // `chroma_client` config, which carries `chroma_token` (a secret) — so a malformed-but-
    // token-bearing config leaked the token to logs. Verifies both that the error path still
    // works and that the secret never reaches the tracing output.
    #[test]
    fn agentic_parse_failure_does_not_log_the_raw_config() {
        use std::io::Write;
        use std::sync::{Arc, Mutex};

        #[derive(Clone)]
        struct BufWriter(Arc<Mutex<Vec<u8>>>);
        impl Write for BufWriter {
            fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(data);
                Ok(data.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
        let writer = BufWriter(buf.clone());
        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || writer.clone())
            .finish();

        let secret = "super-secret-chroma-token-xyz";
        // Deliberately malformed (unterminated object) so parsing fails, with the secret
        // embedded in the string that would have been logged pre-fix.
        let malformed =
            format!(r#"{{"Librarian":{{"chroma_client":"{{\"chroma_token\":\"{secret}\"}}"}}"#);
        let args = AskArgs {
            agentic: Some(malformed),
            ..Default::default()
        };

        let result =
            tracing::subscriber::with_default(subscriber, || args.into_config("default-model"));

        assert!(result.is_err(), "malformed JSON should still be rejected");
        let logged = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
        assert!(
            !logged.contains(secret),
            "secret leaked into log output: {logged}"
        );
    }

    #[tokio::test]
    async fn test_agentic_config_merging() {
        let args = AskArgs {
            team_model: Some("codellama".to_string()),
            iterations: Some(5),
            ..Default::default()
        };

        let config = args.into_config("default-model").unwrap();

        assert_eq!(config["iterations"], 5);
        assert_eq!(config["Architect"]["model"], "codellama");
        assert_eq!(config["Worker"]["model"], "codellama");
    }
    #[tokio::test]
    #[ignore = "requires a live Ollama server on localhost:11434 — runs a full 3-round Orchestrator against real qwen2.5 models"]
    async fn test_agentic() {
        let agentic = Some(json!({
                "iterations": 3,
                "Architect": {
                    "model": "qwen2.5:latest",
                    "status_msg": "Architecting technical blueprint...",
                    "temperature": 0.0,
                    "task": "Plan the solution for the Worker agent to implement",
                    "dense_signal": "Use markdown headers."
                },
                "Worker": {
                    "model": "qwen2.5:latest",
                    "temperature": 0.7,
                    "task": "Follow the Architect agent's plan precisely",
                    "dense_signal": "OUTPUT RAW CODE ONLY. NO CHAT."
                },
                "Critic": {
                    "model": "qwen2.5:latest",
                    "temperature": 0.0,
                    "task": "Respond with APPROVED or give feedback",
                    "dense_signal": "Explain your reasoning then end with APPROVED or REJECTED.",
                    "approval_signal": "APPROVED"
                },
                "Summarizer": {
                    "model": "qwen2.5:latest",
                    "temperature": 0.0,
                    "task": "Summarize the following history of changes and feedback into a dense technical state"
                }
            }).to_string());
        let cfg = json!({});
        let args = AskArgs {
            agentic,
            prompt: PromptArgs::default(),
            ollama: OllamaArgs::default(),
            ..Default::default()
        };
        assert!(args.ask(&cfg).await.is_ok());
        assert!(args.agentic.is_some());
    }
}
