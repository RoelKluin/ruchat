use crate::agent::{Agent, Context};
use crate::{Result, RuChatError};
use std::process::Command;
use tokio_stream::{Stream, StreamExt};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use ollama_rs::generation::completion::GenerationResponse;
use ollama_rs::Ollama;
use std::collections::HashMap;
use serde_json::Value;

// Define what the UI receives
pub type OrchestratorResult = Result<Vec<GenerationResponse>>;

pub(crate) fn get_agent_color(role: &str) -> String {
    let color = match role.to_lowercase().as_str() {
        "architect" => "\x1b[1;32m",
        "worker"    => "\x1b[1;34m",
        "validator" => "\x1b[1;33m",
        "critic"    => "\x1b[1;31m",
        "summary"   => "\x1b[1;35m",
        "performance"=> "\x1b[1;94m",
        _           => "\x1b[0m",
    };
    color.to_string()
}

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
    agent: HashMap<String, Agent>,
    config: HashMap<String, Value>,
    ollama: Ollama,
}

impl Orchestrator {
    pub(crate) async fn new(
        mut config: HashMap<String, Value>,
        ollama: Ollama
    ) -> Result<Self> {
        let mut agent: HashMap<String, Agent> = HashMap::new();
        for role in ["Architect", "Worker", "Critic"] {
            if let Some(options) = config.remove(role) {
                if let Some(options_str) = options.as_str() {
                    let agent_instance = Agent::new(role, options_str).await?;
                    agent.insert(role.to_string(), agent_instance);
                } else {
                    return Err(RuChatError::Is(format!("Options for {role} must be a string")));
                }
            } else if role != "Critic" {
                return Err(RuChatError::MissingAgent(role.to_string()));
            }
        }
        Ok(Self { agent, config, ollama })
    }

    fn get_str(&self, key: &str) -> Result<&str> {
        self.config.get(key).and_then(|s| s.as_str()).ok_or(RuChatError::Is(format!("No {key}: &str in agent config")))
    }
    fn get_u64(&self, key: &str) -> Result<u64> {
        self.config.get(key).and_then(|s| s.as_u64()).ok_or(RuChatError::Is(format!("No {key}: u64 in agent config")))
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
        let iteration = self.get_u64("iteration").and_then(|i| usize::try_from(i).map_err(RuChatError::TryFromIntError))?;
        let mut context = Context::new(goal);

        for _ in 0..iteration {
            for role in ["Architect", "Worker", "Critic"] {
                if let Some(agent) = self.agent.get_mut(role) {

                    // Initialize the stream from the LLM
                    let mut stream = agent.query_stream(&self.ollama, &context).await?;

                    context.output.clear();

                    // Pipe tokens from the LLM stream to the Orchestrator channel
                    while let Some(res) = stream.next().await {
                        let chunk = res.map_err(RuChatError::OllamaError)?;
                        for resp in &chunk {
                            context.output.push_str(&resp.response);
                        }
                        tx.send(Ok(chunk)).await.map_err(|e| RuChatError::Is(format!("Failed to send response: {e}")))?;
                    }
                    if !agent.update(&mut context) {
                        return Ok(());
                    }
                }
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
