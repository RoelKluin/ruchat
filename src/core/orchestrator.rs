pub(crate) mod git;
pub(super) mod task;

use crate::agent::event::StreamItem;
use crate::agent::protocol::Validation;
use crate::agent::tools::{self, ToolName};
use crate::agent::types::{Context, TurnKind};
use crate::agent::Agent;
use crate::providers::vector::chroma::ChromaClientConfigArgs;
use crate::{Result, RuChatError};
use chroma::ChromaHttpClient;
use ollama_rs::Ollama;
use serde_json::Value;
pub(super) use task::TaskType;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::Stream;
// Define what the UI receives
pub type OrchestratorResult = Result<StreamItem>;
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
        let cancel = CancellationToken::new();

        // Dropping the returned `ReceiverStream` drops `rx`, which makes
        // `tx.closed()` resolve — that's our cancellation trigger. A
        // separate watcher task (rather than polling `tx.is_closed()` inline)
        // means cancellation is detected even while the stage machine is
        // blocked awaiting a long-running future (LLM call, cargo test).
        let watcher_tx = tx.clone();
        let watcher_cancel = cancel.clone();
        tokio::spawn(async move {
            watcher_tx.closed().await;
            watcher_cancel.cancel();
        });

        let task_cancel = cancel.clone();
        tokio::spawn(async move {
            let result = if let Some(path) = debug_sequence {
                self.debug_stage_machine(goal, path, tx.clone(), task_cancel).await
            } else {
                self.run_stage_machine(goal, tx.clone(), task_cancel).await
            };
            // `Cancelled` is an expected early-exit, not worth surfacing as
            // an error to a receiver that's already gone (or going).
            if let Err(e) = result
                && !matches!(e, RuChatError::Cancelled)
            {
                let _ = tx.send(Err(e)).await;
            }
        });

        ReceiverStream::new(rx)
    }

    async fn run_critics_parallel(
        &mut self,
        ctx: &mut Context,
        tx: &mpsc::Sender<OrchestratorResult>,
    ) -> Result<()> {
        let snapshot_output = ctx.output.clone();
        let snapshot_plan_impl = ctx.context_view();
        let mut futs = Vec::new();
        let round = ctx.round;
        for critic in &mut self.critics {
            let approval_signal = critic
                .get_str("approval_signal")
                .unwrap_or("APPROVED")
                .to_string();
            let mut scratch = Context::new(ctx.goal.clone());
            scratch.output = snapshot_output.clone();
            scratch.push_turn(
                TurnKind::Implementation,
                "snapshot",
                snapshot_plan_impl.clone(),
            );
            scratch.round = round;
            let ollama = &self.ollama;
            futs.push(async move {
                critic
                    .query_stream(ollama, &mut scratch, tx)
                    .await
                    .map(|_| (scratch.output, approval_signal))
            });
        }
        let results = futures_util::future::join_all(futs).await;
        for res in results {
            if let Ok((text, approval_signal)) = res
                && !text.contains(&approval_signal) {
                    ctx.push_turn(TurnKind::Rejection, "Critic", text);
                }
        }
        Ok(())
    }

    async fn run_librarian_retrieval(
        &mut self,
        ctx: &mut Context,
        tx: &mpsc::Sender<OrchestratorResult>,
    ) -> Result<()> {
        let client = self.client.as_ref().ok_or_else(|| {
            RuChatError::Is("Librarian provided without chroma client config".into())
        })?;
        let librarian = self
            .librarian
            .as_mut()
            .ok_or_else(|| RuChatError::Is("Librarian not enabled".into()))?;

        librarian.query_stream(&self.ollama, ctx, tx).await?;

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
        ctx.push_turn(TurnKind::Retrieval, "Librarian", docs);
        Ok(())
    }

    async fn run_stage_machine(
        &mut self,
        goal: String,
        tx: mpsc::Sender<OrchestratorResult>,
        cancel: CancellationToken,
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

        let mut retrieve_budget: u32 = 2; // conservative cap on Worker-initiated retrievals per run
        let mut stage = Stage::Plan;

        loop {
            // Checked once per stage transition — this is the boundary the
            // success metric refers to. It does NOT preempt a stage already
            // in flight (e.g. a 120s `cargo test` won't be killed mid-run by
            // this check alone); see `run_build_and_test`'s own cancellation
            // wrapping below for that case.
            if cancel.is_cancelled() {
                return Err(RuChatError::Cancelled);
            }
            stage = match stage {
                Stage::Done => break,
                Stage::Escalate(reason) => {
                    ctx.trace(&tx, format!("ESCALATED: {reason}")).await;
                    break;
                }
                Stage::Plan => {
                    ctx.round += 1;
                    if ctx.round > max_iterations {
                        Stage::Escalate("max iterations reached without acceptance".into())
                    } else {
                        self.architect.query_stream(&self.ollama, ctx, &tx).await?;
                        ctx.push_turn(TurnKind::Plan, "Architect", ctx.output.clone());
                        Stage::Retrieve
                    }
                }
                Stage::Retrieve => {
                    if ctx.round == 1 && self.librarian.is_some() {
                        self.run_librarian_retrieval(ctx, &tx).await?;
                    }
                    Stage::Implement
                }
                Stage::Implement => {
                    self.worker.query_stream(&self.ollama, ctx, &tx).await?;

                    if let Ok(call) = tools::parse_tool_call(&ctx.output)
                        && matches!(
                            call.tool,
                            ToolName::Retrieve | ToolName::GitLog | ToolName::GitBlame | ToolName::GitDiff
                        )
                        && retrieve_budget > 0
                    {
                        retrieve_budget -= 1;
                        self.handle_structured_tool(&call, ctx).await?;
                        self.worker.query_stream(&self.ollama, ctx, &tx).await?;
                    }
                    ctx.push_turn(TurnKind::Implementation, "Worker", ctx.output.clone());

                    match self.worker.execute_and_verify(ctx).await? {
                        Validation::Failure(err) => {
                            ctx.push_turn(TurnKind::Rejection, "ApplyPatch", err);
                            Stage::Retry
                        }
                        _ => Stage::Test,
                    }
                }
                Stage::Test => {
                    let report = Validation::run_build_and_test(&cancel).await?;
                    if !report.compiled || !report.tests_passed {
                        ctx.push_turn(TurnKind::Rejection, "Tester", report.diagnostics);
                        Stage::Retry
                    } else {
                        Stage::Validate
                    }
                }
                Stage::Validate => {
                    if let Some(validator) = self.validator.as_mut() {
                        validator.query_stream(&self.ollama, ctx, &tx).await?;
                        if ctx.output.trim_start().starts_with("REJECTED") {
                            ctx.push_turn(TurnKind::Rejection, "Validator", ctx.output.clone());
                            Stage::Retry
                        } else {
                            Stage::Critique
                        }
                    } else {
                        Stage::Critique
                    }
                }
                Stage::Critique => {
                    self.run_critics_parallel(ctx, &tx).await?;
                    Stage::Reconcile
                }
                Stage::Reconcile => {
                    if ctx.reconcile_rejections() {
                        Stage::Retry
                    } else {
                        Stage::Accept
                    }
                }
                Stage::Retry => {
                    if ctx.round >= max_iterations {
                        Stage::Escalate("repeated rejections, iteration budget exhausted".into())
                    } else {
                        if let Some(summarizer) = self.summarizer.as_mut() {
                            let approx_tokens: u64 = ctx
                                .turns
                                .iter()
                                .map(|t| crate::agent::tokens::count_tokens(&t.content))
                                .sum();
                            if approx_tokens > summarizer.get_dynamic_history_limit() {
                                summarizer.query_stream(&self.ollama, ctx, &tx).await?;
                                ctx.collapse_to_summary(ctx.output.clone());
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
        tx: mpsc::Sender<OrchestratorResult>,
        cancel: CancellationToken,
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
            if cancel.is_cancelled() {
                return Err(RuChatError::Cancelled);
            }
            ctx.round = step as u64 + 1;

            if role == "Librarian" {
                self.run_librarian_retrieval(&mut ctx, &tx).await?;
            } else {
                let kind = match role.as_str() {
                    "Architect" => {
                        self.architect
                            .query_stream(&self.ollama, &mut ctx, &tx)
                            .await?;
                        TurnKind::Plan
                    }
                    "Worker" => {
                        self.worker
                            .query_stream(&self.ollama, &mut ctx, &tx)
                            .await?;
                        TurnKind::Implementation
                    }
                    "Validator" => {
                        self.validator
                            .as_mut()
                            .ok_or(RuChatError::Is("Validator not enabled".into()))?
                            .query_stream(&self.ollama, &mut ctx, &tx)
                            .await?;
                        TurnKind::Rejection
                    }
                    "Summarizer" => {
                        self.summarizer
                            .as_mut()
                            .ok_or(RuChatError::Is("Summarizer not enabled".into()))?
                            .query_stream(&self.ollama, &mut ctx, &tx)
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
                            .query_stream(&self.ollama, &mut ctx, &tx)
                            .await?;
                        TurnKind::Rejection
                    }
                    _ => return Err(RuChatError::Is(format!("Unknown agent: {role}"))),
                };
                ctx.push_turn(kind, &role, ctx.output.clone());
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

    async fn handle_retrieve(&mut self, query_text: &str, ctx: &mut Context) -> Result<()> {
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
        ctx.push_turn(TurnKind::Retrieval, "Retrieve", docs);
        Ok(())
    }

    /// Dispatches a validated structured tool call from `Stage::Implement`.
    /// Only the read-only tools reach here; `Memorize`/`ApplyPatch` are
    /// handled later by `Agent::execute_and_verify` since they mutate state
    /// tied to the agent's own config, not the orchestrator's.
    async fn handle_structured_tool(
        &mut self,
        call: &tools::StructuredToolCall,
        ctx: &mut Context,
    ) -> Result<()> {
        match call.tool {
            ToolName::Retrieve => {
                let query = call.args["query"].as_str().unwrap_or_default();
                self.handle_retrieve(query, ctx).await
            }
            ToolName::GitLog => {
                let path = call.args.get("path").and_then(|v| v.as_str());
                let max_count = call
                    .args
                    .get("max_count")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as u32);
                let out = git::git_log(path, max_count).await?;
                ctx.push_turn(TurnKind::Retrieval, "GitLog", out);
                Ok(())
            }
            ToolName::GitBlame => {
                let path = call.args.get("path").and_then(|v| v.as_str()).unwrap_or_default();
                let out = git::git_blame(path).await?;
                ctx.push_turn(TurnKind::Retrieval, "GitBlame", out);
                Ok(())
            }
            ToolName::GitDiff => {
                let path = call.args.get("path").and_then(|v| v.as_str());
                let staged = call.args.get("staged").and_then(|v| v.as_bool()).unwrap_or(false);
                let out = git::git_diff(path, staged).await?;
                ctx.push_turn(TurnKind::Retrieval, "GitDiff", out);
                Ok(())
            }
            ToolName::Memorize | ToolName::ApplyPatch => Ok(()),
        }
    }
}
