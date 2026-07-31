mod git;
pub(super) mod task;

use crate::agent::protocol::{ToolCall, Validation};
use crate::agent::types::{Context, TurnKind};
use crate::agent::Agent;
use crate::providers::vector::chroma::ChromaClientConfigArgs;
use crate::{Result, RuChatError};
use chroma::ChromaHttpClient;
use ollama_rs::generation::completion::GenerationResponse;
use ollama_rs::Ollama;
use serde_json::Value;
pub(super) use task::TaskType;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::Stream;
// Define what the UI receives
pub type OrchestratorResult = Result<Vec<GenerationResponse>>;
use crate::providers::vector::chroma::query::Query;
use git::commit_feature_branch;
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq)]
enum Stage {
    Plan,
    Retrieve,
    Implement,
    Test,
    Validate,
    Critique,
    Reconcile,
    Accept,
    Commit,
    Retry,
    Escalate(String),
    Done,
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
    orchestrator_config: Value,
    ollama: Ollama,
    client: Option<ChromaHttpClient>,
}

impl Orchestrator {
    pub(crate) async fn new(
        mut orchestrator_config: Value,
        ollama: Ollama,
        cfg: &Value,
    ) -> Result<Self> {
        let task_type = TaskType::deserialize(&orchestrator_config).ok();
        let task_type = task_type.as_ref();
        // 1. Extract Core Agents
        let architect = Agent::new(
            &mut orchestrator_config,
            "Architect",
            true,
            task_type,
            cfg.clone(),
        )
        .await?;
        let worker = Agent::new(
            &mut orchestrator_config,
            "Worker",
            true,
            task_type,
            cfg.clone(),
        )
        .await?;

        let validator = Agent::new(
            &mut orchestrator_config,
            "Validator",
            false,
            task_type,
            cfg.clone(),
        )
        .await
        .ok();
        let summarizer = Agent::new(
            &mut orchestrator_config,
            "Summarizer",
            false,
            None,
            cfg.clone(),
        )
        .await
        .ok();

        let mut critics = Vec::new();
        if let Some(critic_list) = orchestrator_config
            .get("Critics")
            .and_then(|v| v.as_array())
        {
            for (i, c_val) in critic_list.iter().enumerate() {
                // We pass a copy of the specific critic's config
                let mut c_config = c_val.clone();
                if let Ok(agent) = Agent::new(
                    &mut c_config,
                    &format!("Critic_{}", i),
                    true,
                    task_type,
                    cfg.clone(),
                )
                .await
                {
                    critics.push(agent);
                }
            }
        }

        let mut librarian = None;
        let mut client = None;
        if let Ok(mut lib) = Agent::new(
            &mut orchestrator_config,
            "Librarian",
            false,
            None,
            cfg.clone(),
        )
        .await
        {
            let mut client_config = ChromaClientConfigArgs::default();
            lib.remove_str("chroma_client").and_then(|s| {
                let val = s.parse::<serde_json::Value>()?;
                client_config.update_from_json(&val).map_err(|e| {
                    eprintln!("{s}");
                    tracing::error!(error = ?e, "Failed to parse chroma_client config as JSON:");
                    e
                }).map_err(RuChatError::AnyhowError)
            })?;
            client = Some(
                client_config
                    .create_client(cfg)
                    .await
                    .map_err(RuChatError::AnyhowError)?,
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
            orchestrator_config,
            ollama,
            client,
        })
    }

    pub(crate) fn run_task_stream(
        mut self,
        goal: String,
        debug_sequence: Option<String>,
    ) -> impl Stream<Item = OrchestratorResult> {
        let (tx, rx) = mpsc::channel(100);
        tokio::spawn(async move {
            if let Some(path) = debug_sequence {
                if let Err(e) = self.debug_stage_machine(goal, path, tx.clone()).await {
                    let _ = tx.send(Err(e)).await;
                }
            } else {
                if let Err(e) = self.run_stage_machine(goal, tx.clone()).await {
                    let _ = tx.send(Err(e)).await;
                }
            }
        });

        ReceiverStream::new(rx)
    }

    async fn run_critics_parallel(
        &mut self,
        ctx: &mut Context,
        round: u64,
        tx: &mpsc::Sender<Result<Vec<GenerationResponse>>>,
    ) -> Result<()> {
        let snapshot_output = ctx.output.clone();
        let snapshot_plan_impl = ctx.context_view();
        let mut futs = Vec::new();
        for critic in &mut self.critics {
            let approval_signal = critic
                .get_str("approval_signal")
                .unwrap_or("APPROVED")
                .to_string();
            let mut scratch = Context::new(ctx.goal.clone());
            scratch.output = snapshot_output.clone();
            scratch.push_turn(
                round,
                TurnKind::Implementation,
                "snapshot",
                snapshot_plan_impl.clone(),
            );
            let ollama = &self.ollama;
            futs.push(async move {
                critic
                    .query_stream(ollama, &mut scratch, round, tx)
                    .await
                    .map(|_| (scratch.output, approval_signal))
            });
        }
        let results = futures_util::future::join_all(futs).await;
        for res in results {
            if let Ok((text, approval_signal)) = res {
                if !text.contains(&approval_signal) {
                    ctx.push_turn(round, TurnKind::Rejection, "Critic", text);
                }
            }
        }
        Ok(())
    }

    async fn run_librarian_retrieval(
        &mut self,
        ctx: &mut Context,
        round: u64,
        tx: &mpsc::Sender<Result<Vec<GenerationResponse>>>,
    ) -> Result<()> {
        let client = self.client.as_ref().ok_or_else(|| {
            RuChatError::Is("Librarian provided without chroma client config".into())
        })?;
        let librarian = self
            .librarian
            .as_mut()
            .ok_or_else(|| RuChatError::Is("Librarian not enabled".into()))?;

        librarian.query_stream(&self.ollama, ctx, round, tx).await?;

        let mut q = Query::default();
        if let Ok(json_val) = serde_json::from_str::<Value>(&ctx.output) {
            let _ = q.update_from_json(json_val);
        } else {
            ctx.trace(
                tx,
                "Librarian did not output valid JSON query — skipping RAG".to_string(),
            )
            .await;
        }

        let docs = librarian
            .retrieve_and_generate(client, &self.ollama, q)
            .await?;
        ctx.push_turn(round, TurnKind::Retrieval, "Librarian", docs);
        Ok(())
    }

    async fn run_stage_machine(
        &mut self,
        goal: String,
        tx: mpsc::Sender<Result<Vec<GenerationResponse>>>,
    ) -> Result<()> {
        let max_iterations = self
            .orchestrator_config
            .get("iterations")
            .and_then(|v| v.as_u64())
            .unwrap_or(3);
        let mut ctx = Context::new(goal);
        let ctx = &mut ctx;

        if let Some(librarian) = self.librarian.as_ref() {
            ctx.read_config_file(
                librarian
                    .get_str("db_config_path")
                    .unwrap_or("db_config.json"),
            )?;
        }

        let mut round: u64 = 0;
        let mut retrieve_budget: u32 = 2; // conservative cap on Worker-initiated retrievals per run
        let mut stage = Stage::Plan;

        loop {
            stage = match stage {
                Stage::Done => break,
                Stage::Escalate(reason) => {
                    ctx.trace(&tx, format!("ESCALATED: {reason}")).await;
                    break;
                }
                Stage::Plan => {
                    round += 1;
                    if round > max_iterations {
                        Stage::Escalate("max iterations reached without acceptance".into())
                    } else {
                        self.architect
                            .query_stream(&self.ollama, ctx, round, &tx)
                            .await?;
                        ctx.push_turn(round, TurnKind::Plan, "Architect", ctx.output.clone());
                        Stage::Retrieve
                    }
                }
                Stage::Retrieve => {
                    if round == 1 && self.librarian.is_some() {
                        self.run_librarian_retrieval(ctx, round, &tx).await?;
                    }
                    Stage::Implement
                }
                Stage::Implement => {
                    self.worker
                        .query_stream(&self.ollama, ctx, round, &tx)
                        .await?;

                    if let Some(call) = ToolCall::parse(&ctx.output) {
                        if call.name == "RETRIEVE" && retrieve_budget > 0 {
                            retrieve_budget -= 1;
                            self.handle_retrieve(&call.content, ctx, round).await?;
                            self.worker
                                .query_stream(&self.ollama, ctx, round, &tx)
                                .await?;
                        }
                    }
                    ctx.push_turn(
                        round,
                        TurnKind::Implementation,
                        "Worker",
                        ctx.output.clone(),
                    );

                    match self.worker.execute_and_verify(ctx).await? {
                        Validation::Failure(err) => {
                            ctx.push_turn(round, TurnKind::Rejection, "ApplyPatch", err);
                            Stage::Retry
                        }
                        _ => Stage::Test,
                    }
                }
                Stage::Test => {
                    let report = Validation::run_build_and_test().await?;
                    if !report.compiled || !report.tests_passed {
                        ctx.push_turn(round, TurnKind::Rejection, "Tester", report.diagnostics);
                        Stage::Retry
                    } else {
                        Stage::Validate
                    }
                }
                Stage::Validate => {
                    if let Some(validator) = self.validator.as_mut() {
                        validator
                            .query_stream(&self.ollama, ctx, round, &tx)
                            .await?;
                        if ctx.output.trim_start().starts_with("REJECTED") {
                            ctx.push_turn(
                                round,
                                TurnKind::Rejection,
                                "Validator",
                                ctx.output.clone(),
                            );
                            Stage::Retry
                        } else {
                            Stage::Critique
                        }
                    } else {
                        Stage::Critique
                    }
                }
                Stage::Critique => {
                    self.run_critics_parallel(ctx, round, &tx).await?;
                    Stage::Reconcile
                }
                Stage::Reconcile => {
                    if ctx.reconcile_rejections(round) {
                        Stage::Retry
                    } else {
                        Stage::Accept
                    }
                }
                Stage::Retry => {
                    if round >= max_iterations {
                        Stage::Escalate("repeated rejections, iteration budget exhausted".into())
                    } else {
                        if let Some(summarizer) = self.summarizer.as_mut() {
                            const CHARS_PER_TOKEN: u64 = 4;
                            let approx_tokens: u64 = ctx
                                .turns
                                .iter()
                                .map(|t| t.content.len() as u64)
                                .sum::<u64>()
                                / CHARS_PER_TOKEN;
                            if approx_tokens > summarizer.get_dynamic_history_limit() {
                                summarizer
                                    .query_stream(&self.ollama, ctx, round, &tx)
                                    .await?;
                                ctx.collapse_to_summary(ctx.output.clone(), round);
                            }
                        }
                        Stage::Plan
                    }
                }
                Stage::Accept => Stage::Commit,
                Stage::Commit => {
                    commit_feature_branch(ctx).await?;
                    Stage::Done
                }
            };
        }
        ctx.trace(&tx, String::new()).await;
        Ok(())
    }

    async fn debug_stage_machine(
        &mut self,
        goal: String,
        path: String,
        tx: mpsc::Sender<Result<Vec<GenerationResponse>>>,
    ) -> Result<()> {
        let debug_json: Value = serde_json::from_str(&tokio::fs::read_to_string(path).await?)?;
        let sequence: Vec<String> = debug_json["sequence"]
            .as_array()
            .ok_or(RuChatError::Is("missing 'sequence' array".into()))?
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();

        let imputations = debug_json
            .get("context_imputations")
            .cloned()
            .unwrap_or_default();

        let mut ctx = Context::new(goal);
        ctx.apply_debug_imputations(&imputations);

        // Debug sequences have no natural "round"; number each step so round-scoped
        // views (history_view/documents_view/context_view) still window correctly.
        for (step, role) in sequence.into_iter().enumerate() {
            let round = step as u64 + 1;

            if role == "Librarian" {
                self.run_librarian_retrieval(&mut ctx, round, &tx).await?;
            } else {
                let kind = match role.as_str() {
                    "Architect" => {
                        self.architect
                            .query_stream(&self.ollama, &mut ctx, round, &tx)
                            .await?;
                        TurnKind::Plan
                    }
                    "Worker" => {
                        self.worker
                            .query_stream(&self.ollama, &mut ctx, round, &tx)
                            .await?;
                        TurnKind::Implementation
                    }
                    "Validator" => {
                        self.validator
                            .as_mut()
                            .ok_or(RuChatError::Is("Validator not enabled".into()))?
                            .query_stream(&self.ollama, &mut ctx, round, &tx)
                            .await?;
                        TurnKind::Rejection
                    }
                    "Summarizer" => {
                        self.summarizer
                            .as_mut()
                            .ok_or(RuChatError::Is("Summarizer not enabled".into()))?
                            .query_stream(&self.ollama, &mut ctx, round, &tx)
                            .await?;
                        TurnKind::Summary
                    }
                    r if r.starts_with("Critic") => {
                        // "Critic_0", "Critic_1", ... — matches the naming in Orchestrator::new.
                        let idx: usize = r
                            .strip_prefix("Critic_")
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(0);
                        self.critics
                            .get_mut(idx)
                            .ok_or(RuChatError::Is("Critic index out of bounds".into()))?
                            .query_stream(&self.ollama, &mut ctx, round, &tx)
                            .await?;
                        TurnKind::Rejection
                    }
                    _ => return Err(RuChatError::Is(format!("Unknown agent: {role}"))),
                };
                ctx.push_turn(round, kind, &role, ctx.output.clone());
            }

            ctx.print_debug_info(&tx, &role).await;
        }

        ctx.trace(
            &tx,
            "DEBUG SEQUENCE COMPLETE — real Librarian query used when present".to_string(),
        )
        .await;
        Ok(())
    }

    async fn handle_retrieve(
        &mut self,
        query_text: &str,
        ctx: &mut Context,
        round: u64,
    ) -> Result<()> {
        let client = self.client.as_ref().ok_or_else(|| {
            RuChatError::Is("Retrieve tool called but no Chroma client is configured".into())
        })?;
        let model = self
            .librarian
            .as_ref()
            .and_then(|l| l.get_str("model").ok())
            .unwrap_or("all-minilm:l6-v2")
            .to_string();

        let mut q = Query::default();
        q.update_from_json(serde_json::json!({ "query": [query_text] }))?;

        let docs = q.query(client, &self.ollama, &model).await?;
        ctx.push_turn(round, TurnKind::Retrieval, "Retrieve", docs);
        Ok(())
    }
}
