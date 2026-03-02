use crate::agent::Agent;
use crate::{Result, RuChatError};
use std::process::Command;
use async_stream::try_stream;
use tokio_stream::{Stream, StreamExt};
use ollama_rs::generation::completion::GenerationResponse;
use ollama_rs::Ollama;
use ollama_rs::models::ModelOptions;
use std::collections::HashMap;

pub(crate) struct AgentConfig {
    model: String,
    temperature: f32,
    init_prompt: String,
}

impl AgentConfig {
    pub(crate) fn new(model: String, temperature: f32, init_prompt: String) -> Self {
        Self { model, temperature, init_prompt }
    }
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
    ollama: Ollama,
}

impl Orchestrator {
    pub(crate) fn new(
        config: HashMap<String, AgentConfig>,
        ollama: Ollama
    ) -> Result<Self> {
        let mut agent: HashMap<String, Agent> = HashMap::new();
        for role in ["Architect", "Worker", "Critic"] {
            if let Some(agent_config) = config.get(role) {
                agent.insert(role.to_string(), Agent {
                    name: role.to_string(),
                    model: agent_config.model.clone(),
                    options: ModelOptions::default().temperature(agent_config.temperature),
                    system_prompt: agent_config.init_prompt.clone(),
                });
            } else if role != "Critic" {
                return Err(RuChatError::MissingAgent(role.to_string()));
            }
        }
        Ok(Self { agent, ollama })
    }
    pub(crate) fn run_task_stream(
        self,
        goal: String,
        iterations: usize
    ) -> impl Stream<Item = Result<Vec<GenerationResponse>>> {
        try_stream! {
            let mut current_context = goal;
            let mut history = String::new();

            'iteration: for i in 1..=iterations {
                let mut plan = String::new();
                let architect = self.agent.get("Architect").ok_or_else(|| RuChatError::MissingAgent("Architect".to_string()))?;
                let mut arch_stream = architect.query_stream(&self.ollama, &history, &current_context).await
                    .map_err(RuChatError::OllamaError)?;
                while let Some(res) = arch_stream.next().await {
                    let chunk = res.map_err(RuChatError::OllamaError)?;
                    for resp in &chunk {
                        plan.push_str(&resp.response);
                    }
                    yield chunk; // Streaming Architect's thoughts to the UI
                }

                let mut worker_output = String::new();
                let worker = self.agent.get("Worker").ok_or_else(|| RuChatError::MissingAgent("Worker".to_string()))?;
                let mut work_stream = worker.query_stream(&self.ollama, "Plan text...", "Execute").await?;
                while let Some(res) = work_stream.next().await {
                    let chunk = res?;
                    for resp in &chunk {
                        worker_output.push_str(&resp.response);
                    }
                    yield chunk;
                }
                if let Some(ref critic) = self.agent.get("Critic") {
                    let mut review = String::new();
                    let mut review_stream = critic.query_stream(&self.ollama, &worker_output, "Review for safety and correctness.").await
                        .map_err(RuChatError::OllamaError)?;
                    while let Some(resp) = review_stream.next().await {
                        let review_chunk = resp?;
                        for r in &review_chunk {
                            if review.contains("APPROVED") {
                                break 'iteration;
                            }
                            review.push_str(&r.response);
                        }
                    }
                    current_context = format!("CRITIQUE: {}\nAdjust and retry.", review);
                    history.push_str(&format!("\nRound {}:\nPlan: {}\nResult: {}\n", i, plan, worker_output));
                } else {
                    break;
                }
            }
        }
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
