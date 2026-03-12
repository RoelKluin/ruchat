mod git;
pub(super) mod task;

use crate::agent::Agent;
use crate::agent::protocol::Validation;
use crate::agent::types::Context;
use crate::providers::vector::chroma::ChromaClientConfigArgs;
use crate::{Result, RuChatError};
use chroma::ChromaHttpClient;
use ollama_rs::Ollama;
use ollama_rs::generation::completion::GenerationResponse;
use serde_json::Value;
pub(super) use task::TaskType;
use tokio::sync::mpsc;
use tokio_stream::Stream;
use tokio_stream::wrappers::ReceiverStream;
// Define what the UI receives
pub type OrchestratorResult = Result<Vec<GenerationResponse>>;
use git::commit_feature_branch;
use serde::Deserialize;

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
    pub(crate) async fn new(mut config: Value, ollama: Ollama) -> Result<Self> {
        let task_type = TaskType::deserialize(&config).unwrap_or(TaskType::ShellAutomation);
        // 1. Extract Core Agents
        let architect = Agent::new(&mut config, "Architect", true, Some(&task_type)).await?;
        let worker = Agent::new(&mut config, "Worker", true, Some(&task_type)).await?;

        let validator = Agent::new(&mut config, "Validator", false, Some(&task_type))
            .await
            .ok();
        let summarizer = Agent::new(&mut config, "Summarizer", false, None)
            .await
            .ok();

        let mut critics = Vec::new();
        if let Some(critic_list) = config.get("Critics").and_then(|v| v.as_array()) {
            for (i, c_val) in critic_list.iter().enumerate() {
                // We pass a copy of the specific critic's config
                let mut c_config = c_val.clone();
                if let Ok(agent) = Agent::new(
                    &mut c_config,
                    &format!("Critic_{}", i),
                    true,
                    Some(&task_type),
                )
                .await
                {
                    critics.push(agent);
                }
            }
        }

        let mut librarian = None;
        let mut client = None;
        if let Ok(mut lib) = Agent::new(&mut config, "Librarian", false, None).await {
            client = Some(
                lib.remove_str("chroma_client")
                    .and_then(|s| serde_json::from_str(&s).map_err(RuChatError::SerdeError))
                    .and_then(|c: ChromaClientConfigArgs| {
                        c.create_client().map_err(RuChatError::ChromaError)
                    })?,
            );

            librarian = Some(lib);
        }

        // 2. Extract Critics (can be a list or individual named keys in JSON)

        Ok(Self {
            architect,
            worker,
            validator,
            summarizer,
            critics,
            librarian,
            config,
            ollama,
            client,
        })
    }

    pub(crate) fn run_task_stream(
        mut self,
        goal: String,
    ) -> impl Stream<Item = OrchestratorResult> {
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
        tx: mpsc::Sender<Result<Vec<GenerationResponse>>>,
    ) -> Result<()> {
        let iterations = self
            .config
            .get("iterations")
            .and_then(|v| v.as_u64())
            .unwrap_or(3);
        let history_limit = self.config.get("history_limit").and_then(|v| v.as_u64());
        let mut ctx = Context::new(goal);
        let ollama = &self.ollama;
        let client = self.client.as_ref();

        for round in 1..=iterations {
            let ctx = &mut ctx;
            self.architect.query_stream(ollama, round, ctx, &tx).await?;

            if round == 1
                && let Some(librarian) = self.librarian.as_mut()
            {
                let client = client.ok_or(RuChatError::Is(
                    "Librarian provided without chroma client config".into(),
                ))?;

                // Ask the LLM to formulate the query
                librarian.query_stream(ollama, round, ctx, &tx).await?;

                let q =
                    serde_json::from_str(ctx.output.as_str()).map_err(RuChatError::SerdeError)?;
                ctx.documents = librarian.retrieve_and_generate(client, ollama, q).await?;
            }
            self.worker.query_stream(ollama, round, ctx, &tx).await?;
            if let Validation::Failure(err) = self.worker.execute_and_verify(ctx).await? {
                ctx.rejections
                    .push_str(&format!("\nRound {round} failed verification: {err}"));
                continue;
            }

            if let Some(validator) = self.validator.as_mut() {
                validator.query_stream(ollama, round, ctx, &tx).await?;

                // Auto-Rejection Logic
                if ctx.output.contains("REJECTED") {
                    ctx.rejections
                        .push_str(&format!("\nValidation Failed: {}", ctx.output));
                    // Logic to skip Critics and jump back to Architect/Worker
                    continue;
                }
            }
            for critic in &mut self.critics {
                critic.query_stream(ollama, round, ctx, &tx).await?;
            }

            if ctx.is_approved() {
                commit_feature_branch(ctx).await?;
                break;
            } else {
                if let Some(summarizer) = self.summarizer.as_mut()
                    && ctx.history.len() as u64
                        > history_limit.unwrap_or(summarizer.get_dynamic_history_limit())
                {
                    summarizer.query_stream(ollama, round, ctx, &tx).await?;
                }
                ctx.history.push_str("\nREJECTIONS: ");
                ctx.history.push_str(&ctx.rejections);
                ctx.rejections.clear();
            }
        }
        Ok(())
    }
}
