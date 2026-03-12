use crate::cli::prompt::PromptArgs;
use crate::io::Io;
use crate::ollama::OllamaArgs;
use crate::orchestrator::Orchestrator;
use crate::{Result, RuChatError};
use clap::Parser;
use futures_util::TryStreamExt;
use ollama_rs::generation::completion::GenerationResponse;
use ollama_rs::{Ollama, generation::completion::request::GenerationRequest, models::ModelOptions};
use std::pin::Pin;
use tokio_stream::Stream;
use tokio_stream::StreamExt;

type LlamaStream = Pin<Box<dyn Stream<Item = Result<Vec<GenerationResponse>>> + Send>>;

const DEFAULT_MODEL: &str = "qwen2.5vl:latest";

/// Command-line arguments for asking a question to a model.
///
/// This struct defines the arguments required to ask a question
/// to a model, including model details, prompt, and input options.
#[derive(Parser, Debug, Clone, Default, PartialEq)]
pub(crate) struct AskArgs {
    /// Provide a full JSON config for the team
    #[arg(short, long, group = "agent_config")]
    agentic: Option<String>,

    /// Quick-start: Just enable Worker+Architect with this model
    #[arg(long, group = "agent_config")]
    team_model: Option<String>,

    /// Enable RAG by specifying a Chroma collection name
    #[arg(long)]
    collection: Option<String>,

    /// Override maximum iterations
    #[arg(long, default_value = "3")]
    iterations: Option<u64>,

    /// Model for an optional Validator agent
    #[arg(long)]
    validator_model: Option<String>,

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
            serde_json::from_str(json_str).map_err(RuChatError::SerdeError)?
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
        // 2. Handle team_model shortcut
        if let Some(model) = self.team_model {
            if config.get("Architect").is_none() {
                config["Architect"] = serde_json::json!({ "model": model });
            }
            if config.get("Worker").is_none() {
                config["Worker"] = serde_json::json!({ "model": model });
            }
        }

        // 3. Handle validator shortcut
        if let Some(v_model) = self.validator_model {
            config["Validator"] = serde_json::json!({ "model": v_model });
        }

        // 4. Override iterations if flag is present
        if let Some(iters) = self.iterations {
            config["iterations"] = serde_json::json!(iters);
        }

        // 5. Inject global model as fallback for agents missing one
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
    /// # Parameters
    ///
    /// - `end_marker`: The marker indicating the end of user or stdin input.
    ///
    /// # Returns
    ///
    /// A `Result` indicating success or failure.
    pub(crate) async fn ask(&self, end_marker: &str) -> Result<()> {
        let mut cio = Io::new();
        let prompt = match self.prompt.get_prompt() {
            Ok(p) => p,
            Err(RuChatError::NoPromptProvided) => {
                let mut input = String::new();
                if end_marker.is_empty() {
                    // indicates user mode
                    cio.write_line("Enter your question (empty line to finish):")
                        .await?;
                }
                while let Ok(line) = cio.read_line().await {
                    if line == end_marker {
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

        let (ollama, model) = self.ollama.init("").await?;
        let model_name = model
            .first()
            .cloned()
            .unwrap_or_else(|| DEFAULT_MODEL.to_string());

        let config = self.clone().into_config(&model_name)?;

        let mut stream: LlamaStream =
            if config.get("Architect").is_some() || config.get("Worker").is_some() {
                let orchestrator = Orchestrator::new(config, ollama).await?;
                Box::pin(orchestrator.run_task_stream(prompt))
            } else {
                // ... existing single-shot logic ...
                let request = self
                    .ollama
                    .build_generation_request(model[0].clone(), prompt)
                    .await?;
                Box::pin(
                    ollama
                        .generate_stream(request)
                        .await
                        .map(|res| res.map_err(RuChatError::OllamaError))
                        .map_err(RuChatError::OllamaError)?,
                )
            };
        while let Some(res) = stream.next().await {
            match res {
                Ok(responses) => {
                    for resp in responses {
                        cio.write_line(&resp.response).await?;
                    }
                }
                Err(RuChatError::ColorChange(ansi_code)) => {
                    // Write the color code directly to the output without a newline
                    cio.write_line(ansi_code).await?;
                }
                Err(RuChatError::StatusUpdate(msg)) => {
                    // Print a dim status message that gets overwritten by the next line
                    cio.write_line(&format!("\x1b[2m   ... {} \x1b[0m\r", msg))
                        .await?;
                }
                Err(e) => return Err(e), // Real errors still break the loop
            }
        }
        cio.write_line("\x1b[0m").await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_ask_args_default() {
        let args = AskArgs::default();
        assert_eq!(args.agentic, "");
        assert_eq!(args.output_format, "text");
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
        let agentic = json!({
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
            }).to_string();
        let args = AskArgs {
            agentic,
            output_format: "text".to_string(),
            prompt: PromptArgs::default(),
            ollama: OllamaArgs::default(),
        };
        assert!(args.ask("").await.is_ok());
        assert_eq!(args.output_format, "text");
        assert!(!args.agentic.is_empty());
    }
}
