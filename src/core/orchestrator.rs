use crate::agent::Agent;
use crate::{Result, RuChatError};
use std::process::Command;
use tokio_stream::Stream;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use ollama_rs::generation::completion::GenerationResponse;
use ollama_rs::Ollama;
use serde_json::Value;
use crate::providers::vector::chroma::ChromaClientConfigArgs;
use chroma::ChromaHttpClient;
use crate::core::embed::UpsertMode;
use log::info;
use crate::agent::protocol::{Tool, ToolCall};
use crate::agent::types::Context;
// Define what the UI receives
pub type OrchestratorResult = Result<Vec<GenerationResponse>>;

#[derive(Debug, Clone)]
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
    librarian: Option<Agent>,
    worker: Agent,
    // Consensus pipeline: All of these must return their specific approval signal
    critics: Vec<Agent>,
    summarizer: Option<Agent>,
    validator: Option<Agent>,
    config: Value,
    ollama: Ollama,
    client: Option<ChromaHttpClient>,
}

impl Orchestrator {
    pub(crate) async fn new(
        mut config: Value,
        ollama: Ollama
    ) -> Result<Self> {

        let task_type = Self::detect_task_type(&config);
        // 1. Extract Core Agents
        let mut architect = Self::build_agent(&mut config, "Architect", true).await?;
        let mut librarian = None;
        let mut client = None;
        if let Ok(mut lib) = Self::build_agent(&mut config, "Librarian", false).await {
            client = Some(lib.remove_str("chroma_client")
                .and_then(|s| serde_json::from_str(&s).map_err(RuChatError::SerdeError))
                .and_then(|c: ChromaClientConfigArgs| c.create_client().map_err(RuChatError::ChromaError))?);

            librarian = Some(lib);
        }
        let mut worker = Self::build_agent(&mut config, "Worker", true).await?;
        let mut validator = Self::build_agent(&mut config, "Validator", false).await.ok();

        architect.apply_task_context(task_type.clone());
        validator.as_mut().map(|v| v.apply_task_context(task_type.clone()));
        worker.apply_task_context(task_type);


        // 2. Extract Critics (can be a list or individual named keys in JSON)
        let mut critics = Vec::new();
        let critic_roles = ["Validator", "Critic", "Safety Critic", "Performance Critic"];

        for role in critic_roles {
            if let Ok(agent) = Self::build_agent(&mut config, role, false).await {
                critics.push(agent);
            }
        }
        let summarizer = Self::build_agent(&mut config, "Summarizer", false).await.ok();

        Ok(Self { architect, librarian, worker, critics, config, ollama, summarizer, client, validator })
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
    fn detect_task_type(config: &serde_json::Value) -> TaskType {
        let goal = config.get("goal").and_then(|v| v.as_str()).unwrap_or_default().to_lowercase();

        if goal.contains("refactor") || goal.contains("rust") {
            TaskType::RustRefactor
        } else if goal.contains("bisect") || goal.contains("git") {
            TaskType::GitBisect
        } else if goal.contains("debug") || goal.contains("fix") {
            TaskType::DebugCore
        } else {
            TaskType::ShellAutomation
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
        let history_limit = self.config.get("history_limit").and_then(|v| v.as_u64());
        let mut ctx = Context::new(goal);
        let ollama = &self.ollama;
        let client = self.client.as_ref();

        for round in 1..=iterations {
            let ctx = &mut ctx;
            self.architect.query_stream(ollama, round, ctx, &tx).await?;

            if round == 1 && let Some(librarian) = self.librarian.as_mut() {
                let client = client.ok_or(RuChatError::Is("Librarian provided without chroma client config".into()))?;

                // Ask the LLM to formulate the query
                librarian.query_stream(ollama, round, ctx, &tx).await?;

                let q = serde_json::from_str(ctx.output.as_str()).map_err(RuChatError::SerdeError)?;
                ctx.documents = librarian.retrieve_and_generate(client, ollama, q).await?;
            }
            self.worker.query_stream(ollama, round, ctx, &tx).await?;
            match self.execute_and_verify(ctx).await? {
                Validation::Success => {
                    // If we reach the final round or a Critic approves, mark success
                }
                Validation::Failure(err) => {
                    ctx.rejections.push_str(&format!("\nRound {round} failed verification: {err}"));
                    continue;
                }
                Validation::Skip => {}
            }

            if let Some(validator) = self.validator.as_mut() {
                validator.query_stream(ollama, round, ctx, &tx).await?;

                // Auto-Rejection Logic
                if ctx.output.contains("REJECTED") {
                    ctx.rejections.push_str(&format!("\nValidation Failed: {}", ctx.output));
                    // Logic to skip Critics and jump back to Architect/Worker
                    continue;
                }
            }
            for critic in &mut self.critics {
                critic.query_stream(ollama, round, ctx, &tx).await?;
            }

            if ctx.is_approved() {
                self.commit_feature_branch(ctx).await?;
                break;
            } else {
                if let Some(summarizer) = self.summarizer.as_mut() && ctx.history.len() as u64 > history_limit.unwrap_or(summarizer.get_dynamic_history_limit()) {
                    summarizer.query_stream(ollama, round, ctx, &tx).await?;
                }
                ctx.history.push_str("\nREJECTIONS: ");
                ctx.history.push_str(&ctx.rejections);
                ctx.rejections.clear();
            }
        }
        Ok(())
    }
    async fn execute_and_verify(&self, ctx: &mut Context) -> Result<Validation> {
        let tool_call = match ToolCall::parse(&ctx.output) {
            Some(call) => call,
            None => return Ok(Validation::Skip),
        };

        match tool_call.to_tool() {
            Some(Tool::Shell { command }) => {
                let shell_res = self.execute_shell_script(&command).await?;

                match shell_res {
                    Validation::Success => {
                        // If Rust code was touched, run cargo check
                        if command.contains(".rs") {
                            let check_res = self.run_cargo_check().await?;
                            if let Validation::Failure(ref err) = check_res {
                                ctx.rejections.push_str(&format!("\nCargo Check Failed: {err}"));
                            }
                            Ok(check_res)
                        } else {
                            Ok(Validation::Success)
                        }
                    }
                    Validation::Failure(err) => {
                        ctx.rejections.push_str(&format!("\nShell Error: {err}"));
                        Ok(Validation::Failure(err))
                    }
                    Validation::Skip => Ok(Validation::Skip),
                }
            }
            Some(Tool::Memorize { content }) => {
                match self.worker.embed(&content, UpsertMode::Upsert).await {
                    Ok(_) => {
                        ctx.history.push_str("\n[SYSTEM]: Information memorized.");
                        Ok(Validation::Success)
                    }
                    Err(e) => Ok(Validation::Failure(e.to_string())),
                }
            }
            None => Ok(Validation::Failure(format!("Unknown tool: {}", tool_call.name))),
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
    async fn run_git_command(&self, args: Vec<&str>) -> Result<()> {
        let output = tokio::process::Command::new("git")
            .args(&args)
            .output()
            .await
            .map_err(|e| RuChatError::InternalError(format!("Git exec failed: {e}")))?;

        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            return Err(RuChatError::InternalError(format!("Git error: {err}")));
        }
        Ok(())
    }
    async fn commit_feature_branch(&self, ctx: &Context) -> Result<()> {
        // 1. Sanitize Branch Name
        let timestamp = chrono::Utc::now().timestamp();
        let branch_name = format!("ai/feature-{}", timestamp);
        let goal = ctx.get_goal();

        // 2. Prepare the Summary Entry
        let summary_entry = format!(
            "\n--- \n### 🤖 AI Update: {}\n**Date:** {}\n**Goal:** {}\n**Changes:** \n{}\n",
            branch_name,
            chrono::Utc::now().to_rfc2822(),
            goal,
            ctx.output.lines().take(5).collect::<Vec<_>>().join("\n") // Take first 5 lines of worker output as summary
        );

        // 3. Execution Sequence
        // We use a helper to run commands and check status
        // Create branch and switch
        self.run_git_command(vec!["checkout", "-b", &branch_name]).await?;

        // Append to featured_changes.md
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("featured_changes.md")
            .await?;

        tokio::io::AsyncWriteExt::write_all(&mut file, summary_entry.as_bytes()).await?;

        // Finalize Git sequence
        self.run_git_command(vec!["add", "."]).await?;
        self.run_git_command(vec!["commit", "-m", &format!("AI Success: {}", goal)]).await?;
        self.run_git_command(vec!["checkout", "-"]).await?; // Return to main

        info!("🚀 Changes logged in featured_changes.md and committed to {}", branch_name);
        Ok(())
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
