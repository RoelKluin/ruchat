use crate::agent::{Agent, Context};
use crate::{Result, RuChatError};
use std::process::Command;
use tokio_stream::Stream;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use ollama_rs::generation::completion::GenerationResponse;
use ollama_rs::Ollama;
use serde_json::Value;

// Define what the UI receives
pub type OrchestratorResult = Result<Vec<GenerationResponse>>;

pub(crate) enum TaskType {
    RustRefactor,
    GitBisect,
    ShellAutomation,
    DebugCore,
}

pub(crate) enum Validation {
    Success,
    Failure(String),
    Skip,
}

pub(crate) struct Orchestrator {
    // Core pipeline
    architect: Agent,
    worker: Agent,
    // Consensus pipeline: All of these must return their specific approval signal
    critics: Vec<Agent>,
    summarizer: Option<Agent>,
    config: Value,
    ollama: Ollama,
}

impl Orchestrator {
    pub(crate) async fn new(
        mut config: Value,
        ollama: Ollama
    ) -> Result<Self> {
        // 1. Extract Core Agents
        let architect = Self::build_agent(&mut config, "Architect", true).await?;
        let worker = Self::build_agent(&mut config, "Worker", true).await?;

        // 2. Extract Critics (can be a list or individual named keys in JSON)
        let mut critics = Vec::new();
        let critic_roles = ["Validator", "Critic", "Safety Critic", "Performance Critic"];

        for role in critic_roles {
            if let Ok(agent) = Self::build_agent(&mut config, role, false).await {
                critics.push(agent);
            }
        }
        let summarizer = match Self::build_agent(&mut config, "Summarizer", false).await {
            Ok(agent) => Some(agent),
            Err(_) => None,
        };

        Ok(Self { architect, worker, critics, config, ollama, summarizer })
    }
    async fn build_agent(config: &mut Value, role: &str, required: bool) -> Result<Agent> {
        if let Some(agent_val) = config.get(role) {
            // Check if it's a raw JSON string (from CLI) or an Object (from json! macro)
            let options_str = if agent_val.is_string() {
                agent_val.as_str().unwrap().to_string()
            } else {
                agent_val.to_string()
            };
            Agent::new(role, &options_str).await
        } else if required {
            Err(RuChatError::MissingAgent(role.to_string()))
        } else {
            Err(RuChatError::Is("Optional agent missing".into()))
        }
    }

    pub(crate) fn run_task_stream(mut self, goal: String) -> impl Stream<Item = OrchestratorResult> {
        let (tx, rx) = mpsc::channel(100);

        tokio::spawn(async move {
            if let Err(e) = self.execute_orchestration(goal, tx.clone()).await {
                let _ = tx.send(Err(e)).await;
            }
        });

        ReceiverStream::new(rx)
    }
    async fn execute_orchestration(
        &mut self,
        goal: String,
        tx: mpsc::Sender<Result<Vec<GenerationResponse>>>
    ) -> Result<()> {
        let iterations = self.config.get("iterations").and_then(|v| v.as_u64()).unwrap_or(3);
        let mut ctx = Context::new(goal);
        let ollama = &self.ollama;

        for round in 1..=iterations {
            let ctx = &mut ctx;
            self.architect.query_stream(ollama, round, ctx, &tx).await?;
            self.worker.query_stream(ollama, round, ctx, &tx).await?;

            for critic in &mut self.critics {
                critic.query_stream(ollama, round, ctx, &tx).await?;
            }

            if ctx.is_approved() {
                break;
            } else {
                if let Some(summarizer) = self.summarizer.as_mut() {
                    summarizer.query_stream(ollama, round, ctx, &tx).await?;
                }
                ctx.rejections.clear();
            }
        }
        Ok(())
    }
    async fn execute_shell_script(&self, script: &str) -> Result<Validation> {
        // Logic to run sed and awk script and capture output
        match Command::new("bash")
            .arg("-c")
            .arg(script)
            .output() {
                Ok(output) if output.status.success() => Ok(Validation::Success),
                Ok(output) => {
                    Ok(Validation::Failure(String::from_utf8_lossy(&output.stderr).to_string()))
                }
                Err(e) => {
                    Ok(Validation::Failure(format!("Failed to execute sed/awk: {e}")))
                }
        }
    }
    async fn run_cargo_check(&self) -> Result<Validation> {
        let output = Command::new("cargo")
            .args(["check"])
            .output()
            .expect("failed to execute cargo check");

        if output.status.success() {
            Ok(Validation::Success)
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            Ok(Validation::Failure(stderr))
        }
    }
}
