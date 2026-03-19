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
use crate::providers::vector::chroma::query::Query;

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
        let task_type = TaskType::deserialize(&config).ok();
        let task_type = task_type.as_ref();
        // 1. Extract Core Agents
        let architect = Agent::new(&mut config, "Architect", true, task_type).await?;
        let worker = Agent::new(&mut config, "Worker", true, task_type).await?;

        let validator = Agent::new(&mut config, "Validator", false, task_type)
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
                    task_type,
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
            let mut client_config = ChromaClientConfigArgs::default();
            lib.remove_str("chroma_client")
                .and_then(|s| client_config.update_from_json(&s).map_err(|e| {
                    eprintln!("{s}");
                    tracing::error!(error = ?e, "Failed to parse chroma_client config as JSON:");
                    e
                }).map_err(RuChatError::AnyhowError)
                )?;
            client = Some(client_config.create_client().map_err(RuChatError::AnyhowError)?);

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
        let ctx = &mut ctx;
        let ollama = &self.ollama;
        let client = self.client.as_ref();
        if let Some(librarian) = self.librarian.as_ref() {
            ctx.read_config_file(librarian.get_str("db_config_path").unwrap_or("db_config.json"))?;
        }

        for round in 1..=iterations {
            self.architect.query_stream(ollama, ctx, &tx).await?;

            if round == 1
                && let Some(librarian) = self.librarian.as_mut()
            {
                let client = client.ok_or(RuChatError::Is(
                    "Librarian provided without chroma client config".into(),
                ))?;

                // Ask the LLM to formulate the query
                librarian.query_stream(ollama, ctx, &tx).await?;

                ctx.trace(&tx, format!("Librarian formulated query: {}", ctx.output.as_str())).await;

                let mut q = Query::default();
                if let Ok(json_val) = serde_json::from_str::<Value>(&ctx.output) {
                    let _ = q.update_from_json(json_val);
                } else {
                    ctx.trace(&tx, "Librarian did not output valid JSON query — skipping RAG".to_string()).await;
                }
                ctx.documents = librarian.retrieve_and_generate(client, ollama, q).await?;

                let num_docs = ctx.documents.lines().filter(|l| l.trim().starts_with(|c: char| c.is_ascii_digit() || c == ' ')).count().saturating_sub(2);
                ctx.trace(&tx, format!("✅ Librarian found {} results. Documents now in context for Worker.", num_docs)).await;
            }

            // Worker now generates implementation (using documents/plan from Architect + Librarian RAG)
            self.worker.query_stream(ollama, ctx, &tx).await?;

            if let Validation::Failure(err) = self.worker.execute_and_verify(ctx).await? {
                ctx.trace(&tx, format!("Round {round} failed verification: {err}")).await;
                continue;
            }

            if let Some(validator) = self.validator.as_mut() {
                validator.query_stream(ollama, ctx, &tx).await?;

                // Auto-Rejection Logic
                if ctx.output.contains("REJECTED") {
                    ctx.trace(&tx, format!("Round {round} rejected by validator: {}", ctx.output)).await;
                    continue;
                }
            }
            for critic in &mut self.critics {
                critic.query_stream(ollama, ctx, &tx).await?;
            }

            if ctx.is_approved() {
                commit_feature_branch(ctx).await?;
                break;
            } else {
                if let Some(summarizer) = self.summarizer.as_mut()
                    && ctx.history.len() as u64
                        > history_limit.unwrap_or(summarizer.get_dynamic_history_limit())
                {
                    summarizer.query_stream(ollama, ctx, &tx).await?;
                }
                ctx.history.push_str("\nREJECTIONS: ");
                ctx.history.push_str(&ctx.rejections);
                ctx.rejections.clear();
            }
        }
        ctx.trace(&tx, String::new()).await;
        Ok(())
    }

    pub(crate) fn run_task_stream(
        mut self,
        goal: String,
        debug_sequence: Option<String>,
    ) -> impl Stream<Item = OrchestratorResult> {
        let (tx, rx) = mpsc::channel(100);
        tokio::spawn(async move {
            if let Some (path) = debug_sequence {
                if let Err(e) = self.debug_orchestration(goal, path, tx.clone()).await {
                    let _ = tx.send(Err(e)).await;
                }
            } else {
                if let Err(e) = self.execute_orchestration(goal, tx.clone()).await {
                    let _ = tx.send(Err(e)).await;
                }
            }
        });

        ReceiverStream::new(rx)
    }

    async fn debug_orchestration(
        &mut self,
        goal: String,
        path: String,
        tx: mpsc::Sender<Result<Vec<GenerationResponse>>>,
    ) -> Result<()> {
        let debug_json: Value = serde_json::from_str(
            &tokio::fs::read_to_string(path).await?
        )?;
        let sequence: Vec<String> = debug_json["sequence"]
            .as_array()
            .ok_or(RuChatError::Is("missing 'sequence' array".into()))?
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();

        let imputations = debug_json.get("context_imputations").cloned().unwrap_or_default();

        let mut ctx = Context::new(goal);
        ctx.apply_debug_imputations(&imputations);

        for role in sequence {
            let agent = match role.as_str() {
                "Architect" => &mut self.architect,
                "Worker" => &mut self.worker,
                "Librarian" => self.librarian.as_mut().ok_or(RuChatError::Is("Librarian not enabled".into()))?,
                "Validator" => self.validator.as_mut().ok_or(RuChatError::Is("Validator not enabled".into()))?,
                "Summarizer" => self.summarizer.as_mut().ok_or(RuChatError::Is("Summarizer not enabled".into()))?,
                r if r.starts_with("Critic") => {
                    let idx: usize = r[5..].parse().unwrap_or(0); // Critic0, Critic1...
                    self.critics.get_mut(idx).ok_or(RuChatError::Is("Critic index out of bounds".into()))?
                }
                _ => return Err(RuChatError::Is(format!("Unknown agent: {role}"))),
            };
            agent.query_stream(&self.ollama, &mut ctx, &tx).await?;
            if role == "Librarian" {
                let client = self.client.as_ref().ok_or(RuChatError::Is(
                    "Librarian provided without chroma client config".into(),
                ))?;

                let mut q = Query::default();
                if let Ok(json_val) = serde_json::from_str::<Value>(&ctx.output) {
                    let _ = q.update_from_json(json_val);
                } else {
                    ctx.trace(&tx, "Librarian did not output valid JSON query — skipping RAG".to_string()).await;
                }
                ctx.documents = agent.retrieve_and_generate(client, &self.ollama, q).await?;
            }
            ctx.print_debug_info(&tx, &role).await;
        }

        ctx.trace(&tx, "DEBUG SEQUENCE COMPLETE — real Librarian query used when present".to_string()).await;
        Ok(())
    }
}
