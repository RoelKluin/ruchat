use crate::cli::prompt::PromptArgs;
use crate::agent::event::{AgentEvent, StreamItem};
use crate::io::Io;
use crate::ollama::OllamaArgs;
use crate::orchestrator::Orchestrator;
use crate::{Result, RuChatError};
use clap::Parser;
use futures_util::TryStreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use ollama_rs::{Ollama, generation::completion::request::GenerationRequest, models::ModelOptions};
use std::pin::Pin;
use std::io::Write as _;
use tokio_stream::Stream;
use tokio_stream::StreamExt;
use serde_json::Value;

type LlamaStream = Pin<Box<dyn Stream<Item = Result<StreamItem>> + Send>>;

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
    #[arg(long, hide_short_help = true, hide_long_help = false, help_heading = "Debugging")]
    debug_sequence: Option<String>,

    #[command(flatten)]
    prompt: PromptArgs,

    #[command(flatten)]
    ollama: OllamaArgs,
}

// Reusable generation logic for Agents
pub(crate) async fn generate_oneshot(
    ollama: &Ollama,
    model: &str,
    prompt: &str,
    options: Option<ModelOptions>,
) -> Result<String> {
    // Resolve model name if strictly needed, or trust the Agent's config
    // For safety, we verify the model exists or use default if empty, similar to pipe
    let model_name = if model.is_empty() {
        DEFAULT_MODEL.to_string()
    } else {
        model.to_string()
    };

    let request =
        GenerationRequest::new(model_name, prompt.to_string()).options(options.unwrap_or_default());

    // We collect the stream here because agents need the full context for post-processing
    let mut stream = ollama.generate_stream(request).await?;
    let mut buffer = String::new();

    while let Some(responses) = stream.next().await.transpose()? {
        for resp in responses {
            buffer.push_str(&resp.response);
        }
    }
    Ok(buffer)
}

impl AskArgs {
    pub fn into_config(self, default_model: &str) -> Result<serde_json::Value> {
        // 1. Start with base: either provided JSON or empty object
        let mut config: serde_json::Value = if let Some(ref json_str) = self.agentic {
            serde_json::from_str(json_str)
                .map_err(|e| {
                    eprintln!("Provided agentic config: {json_str}");
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
                "status_msg": "Searching knowledge base..."
            });
            // Ensure the librarian uses the correct collection in the prompt
            config["task_hint"] = serde_json::json!(format!("Query the {} collection", col));
        }

        // Handle Multiple Critics
        if !self.critic.is_empty() {
            let mut critics_array = Vec::new();
            for c_name in self.critic {
                critics_array.push(serde_json::json!({
                    "model": self.team_model.clone().unwrap_or_else(|| default_model.to_string()),
                    "task": format!("Review the implementation specifically for {} concerns.", c_name),
                    "status_msg": format!("{} Critic is reviewing...", c_name)
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
    pub(crate) async fn ask(&self,cfg: &Value) -> Result<()> {
        let mut cio = Io::new();
        let prompt = match self.prompt.get_prompt() {
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
        };

        let (ollama, model) = self.ollama.init("", cfg).await?;
        let model_name = model
            .first()
            .cloned()
            .unwrap_or_else(|| DEFAULT_MODEL.to_string());
        let config = self.clone().into_config(&model_name)?;

        let mut stream: LlamaStream =
            if config.get("Architect").is_some() || config.get("Worker").is_some() {
                let orchestrator = Orchestrator::new(config, std::sync::Arc::new(ollama), cfg).await?;
                Box::pin(orchestrator.run_task_stream(prompt, self.debug_sequence.clone()))
            } else {
                // ... existing single-shot logic ...
                let request = self
                    .ollama
                    .build_generation_request(model[0].clone(), prompt, cfg)
                    .await?;
                Box::pin(
                    ollama
                        .generate_stream(request)
                        .await
                        .map(|s|
                            s.map_ok(StreamItem::Chunk)
                                .map_err(RuChatError::OllamaError))
                        .map_err(RuChatError::OllamaError)?,
                )
            };

        let pb = ProgressBar::new_spinner();
        // Falls back to a plain default style if the template string doesn't
        // parse against this indicatif version — kept deliberately simple.
        if let Ok(style) = ProgressStyle::with_template("{spinner:.cyan} {msg}") {
            pb.set_style(style.tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏"));
        }
        pb.enable_steady_tick(std::time::Duration::from_millis(80));
        pb.set_message("working...");

        // Chunk/color output is raw, partial-token text — not whole lines —
        // so it can't go through `pb.println` (which is line-oriented).
        // `pb.suspend` clears the bar, runs the closure, and redraws after;
        // it's synchronous, so raw stdout writes (not the async `Io` type)
        // are used inside it. Not stress-tested under very high token
        // throughput — if flicker becomes visible in practice, batch writes
        // across several chunks before suspending.
        let mut stream_err = None;
        while let Some(res) = stream.next().await {
            match res {
                Ok(StreamItem::Chunk(responses)) => {
                    pb.suspend(|| {
                        let mut out = std::io::stdout();
                        for resp in &responses {
                            let _ = out.write_all(resp.response.as_bytes());
                        }
                        let _ = out.flush();
                    });
                }
                Ok(StreamItem::ChatChunk(chunk)) => {
                    cio.write_line(&chunk.message.content).await?;
                }
                Ok(StreamItem::Event(AgentEvent::ColorChange(ansi_code))) => {
                    pb.suspend(|| {
                        let mut out = std::io::stdout();
                        let _ = out.write_all(ansi_code.as_bytes());
                        let _ = out.flush();
                    });
                }
                Ok(StreamItem::Event(AgentEvent::StatusUpdate(msg))) => {
                    pb.set_message(msg);
                }
                Ok(StreamItem::Event(AgentEvent::Trace(msg))) => {
                    pb.println(format!("\x1b[90m[TRACE] {msg}\x1b[0m"));
                }
                Ok(StreamItem::Event(AgentEvent::Progress(pct))) => {
                    pb.set_message(format!("{pct:.0}%"));
                }
                Err(e) => {
                    stream_err = Some(e);
                    break;
                }
            }
        }
        pb.finish_and_clear();
        cio.write_line("\x1b[0m").await?;
        match stream_err {
            Some(e) => Err(e),
            None => Ok(()),
        }
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
