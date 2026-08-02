pub(crate) mod git;
pub(crate) mod fs;
pub(crate) mod search;
pub(crate) mod scope;
pub(crate) mod cargo;
pub(super) mod task;

use crate::agent::event::StreamItem;
use crate::agent::protocol::Validation;
use crate::agent::tools::{self, ToolName};
use crate::agent::types::{Context, TurnKind};
use crate::agent::Agent;
use crate::providers::vector::chroma::ChromaClientConfigArgs;
use crate::{Result, RuChatError};
use serde_json::Value;
pub(super) use task::TaskType;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::Stream;
// Define what the UI receives
pub type OrchestratorResult = Result<StreamItem>;
use crate::providers::vector::chroma::query::Query;
use super::agent::json_extract::strip_json_fences;
use git::commit_feature_branch;
use serde::Deserialize;
use crate::retry_transient;
use std::sync::Arc;
use crate::agent::llm_client::{LlmClient, VectorStore};

#[derive(Debug, Clone, PartialEq)]
enum Stage {
    Scope,
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

#[derive(serde::Deserialize)]
struct ValidatorVerdict {
    verdict: String,
    #[serde(default)]
    reason: String,
}

pub(crate) struct Orchestrator {
    // Core pipeline
    scoper: Option<Agent>,
    architect: Agent,
    librarian: Option<Agent>,
    worker: Agent,
    // Consensus pipeline: All of these must return their specific approval signal
    critics: Vec<Agent>,
    summarizer: Option<Agent>,
    validator: Option<Agent>,
    orchestrator_config: Value,
    ollama: Arc<dyn LlmClient>,
    client: Option<Arc<dyn VectorStore>>,
}

impl Orchestrator {
    pub(crate) async fn new(
        mut orchestrator_config: Value,
        ollama: Arc<dyn LlmClient>,
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
        let scoper = Agent::new(&mut orchestrator_config, "Scoper", false, task_type, cfg.clone()).await.ok();

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
            let concrete_client = client_config
                .create_client(cfg)
                .await
                .map_err(RuChatError::AnyhowError)?;
            client = Some(Arc::new(concrete_client) as Arc<dyn VectorStore>);

            librarian = Some(lib);
        }

        Ok(Self {
            scoper,
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
                retry_transient!(critic.query_stream(ollama, &mut scratch, tx))
                    .map(|_| (scratch.output, approval_signal))
            });
        }
        let results = futures_util::future::join_all(futs).await;
        for res in results {
            match res {
                Ok((text, approval_signal)) => {
                    if !text.contains(&approval_signal) {
                        ctx.push_turn(TurnKind::Rejection, "Critic", text);
                    }
                }
                Err(e) => {
                    // A critic that exhausts retries must count as a
                    // rejection, not a silent no-op — otherwise an
                    // unreachable/erroring critic is indistinguishable from
                    // an approving one, inverting the consensus gate's intent.
                    ctx.push_turn(
                        TurnKind::Rejection,
                        "Critic",
                        format!("critic failed to produce a verdict: {e}"),
                    );
                }
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

        retry_transient!(librarian.query_stream(&self.ollama, ctx, tx))?;

        let mut q = Query::default();
        match serde_json::from_str::<Value>(strip_json_fences(&ctx.output)) {
            Ok(json_val) => {
                let _ = q.update_from_json(json_val);
            }
            Err(parse_err) => {
                // One corrective re-ask before giving up, mirroring the
                // Validator's "unparseable == not silently ignored" stance.
                ctx.trace(
                    tx,
                    format!(
                        "Librarian output was not valid JSON ({parse_err}); re-prompting once."
                    ),
                )
                .await;
                ctx.push_turn(
                    crate::agent::types::TurnKind::System,
                    "System",
                    format!(
                        "Your previous response was not valid JSON: {parse_err}. \
                         Return ONLY the JSON object described in your instructions, \
                         no fences, no preamble."
                    ),
                );
                retry_transient!(librarian.query_stream(&self.ollama, ctx, tx))?;
                match serde_json::from_str::<Value>(strip_json_fences(&ctx.output)) {
                    Ok(json_val) => {
                        let _ = q.update_from_json(json_val);
                    }
                    Err(e2) => {
                        ctx.trace(
                            tx,
                            format!("Librarian still not valid JSON after retry ({e2}) — skipping RAG"),
                        )
                        .await;
                    }
                }
            }
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
        let max_scope_iterations = self
            .orchestrator_config
            .get("scope_iterations")
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
        let mut scope_round = 0;
        let mut stage = Stage::Scope;

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
                        retry_transient!(self.architect.query_stream(&self.ollama, ctx, &tx))?;
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
                    retry_transient!(self.worker.query_stream(&self.ollama, ctx, &tx))?;

                    if let Ok(call) = tools::parse_tool_call(&ctx.output)
                        && matches!(
                            call.tool,
                            ToolName::Retrieve | ToolName::GitLog | ToolName::GitBlame 
                            | ToolName::GitDiff | ToolName::GitSearchHistory | ToolName::ReadFile
                            | ToolName::ListDir | ToolName::Ripgrep | ToolName::ReadTags
                            | ToolName::CargoCheck | ToolName::CargoDupes
                        )
                        && retrieve_budget > 0
                    {
                        retrieve_budget -= 1;
                        self.handle_structured_tool(&call, ctx).await?;
                        retry_transient!(self.worker.query_stream(&self.ollama, ctx, &tx))?;
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
                        retry_transient!(validator.query_stream(&self.ollama, ctx, &tx))?;
                        let stripped = strip_json_fences(&ctx.output);
                        match serde_json::from_str::<ValidatorVerdict>(stripped).ok() {
                            Some(v) if v.verdict.eq_ignore_ascii_case("REJECTED") => {
                                ctx.push_turn(TurnKind::Rejection, "Validator", v.reason);
                                Stage::Retry
                            }
                            Some(_) => Stage::Critique,
                            None => {
                                // Conservative: unparseable verdict is treated
                                // as a rejection rather than silently passing.
                                ctx.push_turn(
                                    TurnKind::Rejection,
                                    "Validator",
                                    format!(
                                        "Validator produced unparseable output: {}",
                                        ctx.output
                                    ),
                                );
                                Stage::Retry
                            }
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
                        if ctx.turns.iter().any(|t| t.kind == TurnKind::Implementation) {
                            ctx.trace(&tx, "Iteration budget exhausted — surfacing best-known implementation, NOT committed, unresolved feedback remains.".into()).await;
                            Stage::Done // deliberately not Commit — don't auto-commit an unvalidated patch
                        } else {
                            Stage::Escalate("repeated rejections, iteration budget exhausted, no implementation produced".into())
                        }
                    } else {
                        if let Some(summarizer) = self.summarizer.as_mut() {
                            let approx_tokens: u64 = ctx
                                .turns
                                .iter()
                                .map(|t| crate::agent::tokens::count_tokens(&t.content))
                                .sum();
                            if approx_tokens > summarizer.get_dynamic_history_limit() {
                                retry_transient!(summarizer.query_stream(&self.ollama, ctx, &tx))?;
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
                Stage::Scope => {
                    if self.scoper.is_none() || scope_round >= max_scope_iterations {
                        Stage::Plan
                    } else {
                        scope_round += 1;
                        self.run_scope_stage(ctx, &tx).await?
                    }
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
                    "Scoper" => {
                        self.scoper
                            .as_mut()
                            .ok_or(RuChatError::Is("Scoper not enabled".into()))?
                            .query_stream(&self.ollama, &mut ctx, &tx)
                            .await?;
                        TurnKind::Plan
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
            .and_then(|l| l.get_str("embed_model").ok())
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
            ToolName::GitSearchHistory => {
                let pattern = call.args["pattern"].as_str().unwrap_or_default();
                let mode = call.args["mode"].as_str().unwrap_or("message");
                let path = call.args.get("path").and_then(|v| v.as_str());
                let max_count = call.args.get("max_count").and_then(|v| v.as_u64()).map(|v| v as u32);
                let out = git::git_search_history(pattern, mode, path, max_count).await?;
                ctx.push_turn(TurnKind::Retrieval, "GitSearchHistory", out);
                Ok(())
            }
            ToolName::ReadFile => {
                let path = call.args["path"].as_str().unwrap_or_default();
                let start = call.args.get("start").and_then(|v| v.as_u64()).map(|v| v as u32);
                let end = call.args.get("end").and_then(|v| v.as_u64()).map(|v| v as u32);
                let out = crate::orchestrator::fs::read_file(path, start, end).await?;
                ctx.push_turn(TurnKind::Retrieval, "ReadFile", out);
                Ok(())
            }
            ToolName::ListDir => {
                let path = call.args["path"].as_str().unwrap_or_default();
                let out = crate::orchestrator::fs::list_dir(path).await?;
                ctx.push_turn(TurnKind::Retrieval, "ListDir", out);
                Ok(())
            }
            ToolName::Ripgrep => {
                let pattern = call.args["pattern"].as_str().unwrap_or_default();
                let path = call.args.get("path").and_then(|v| v.as_str());
                let glob = call.args.get("glob").and_then(|v| v.as_str());
                let max_count = call.args.get("max_count").and_then(|v| v.as_u64()).map(|v| v as u32);
                let out = crate::orchestrator::search::ripgrep(pattern, path, glob, max_count).await?;
                ctx.push_turn(TurnKind::Retrieval, "Ripgrep", out);
                Ok(())
            }
            ToolName::ReadTags => {
                let symbol = call.args.get("symbol").and_then(|v| v.as_str());
                let out = crate::orchestrator::search::read_tags(symbol).await?;
                ctx.push_turn(TurnKind::Retrieval, "ReadTags", out);
                Ok(())
            }
            ToolName::CargoCheck => {
                let out = crate::orchestrator::cargo::cargo_check().await?;
                ctx.push_turn(TurnKind::Retrieval, "CargoCheck", out);
                Ok(())
            }
            ToolName::CargoDupes => {
                let out = crate::orchestrator::cargo::cargo_dupes().await?;
                ctx.push_turn(TurnKind::Retrieval, "CargoDupes", out);
                Ok(())
            }
            ToolName::Memorize | ToolName::ApplyPatch => Ok(()),
        }
    }
    async fn run_scope_stage(
        &mut self,
        ctx: &mut Context,
        tx: &mpsc::Sender<OrchestratorResult>,
    ) -> Result<Stage> {
        let scoper = self
            .scoper
            .as_mut()
            .ok_or_else(|| RuChatError::Is("Scoper not enabled".into()))?;

        retry_transient!(scoper.query_stream(&self.ollama, ctx, tx))?;

        let Some(verdict) = scope::parse_scope_verdict(&ctx.output) else {
            ctx.trace(
                tx,
                "Scoper output was not valid JSON — proceeding to Plan with the goal as-is".into(),
            )
            .await;
            return Ok(Stage::Plan);
        };

        if !verdict.notes.is_empty() {
            ctx.push_turn(TurnKind::System, "Scoper", verdict.notes.clone());
        }
        if !verdict.clarified_goal.is_empty() {
            ctx.goal = verdict.clarified_goal;
        }

        if verdict.verdict.eq_ignore_ascii_case("READY") {
            return Ok(Stage::Plan);
        }

        for item in verdict.information_needed {
            match tools::structured_call_from_value(item) {
                Ok(call) => {
                    if let Err(e) = self.handle_structured_tool(&call, ctx).await {
                        ctx.push_turn(TurnKind::System, "Scoper", format!("lookup failed: {e}"));
                    }
                }
                Err(e) => {
                    ctx.push_turn(
                        TurnKind::System,
                        "Scoper",
                        format!("invalid information_needed entry: {e}"),
                    );
                }
            }
        }
        Ok(Stage::Scope)
    }
}

/// Bundles an `Orchestrator` with the goal/debug-sequence it needs to run,
/// so it can implement `AgentPipeline`'s fixed `run(&mut self, ...)` signature
/// without changing `Orchestrator::new`'s existing constructor (which
/// `ask.rs` already calls directly and unchanged).
pub(crate) struct OrchestratorRun {
    orchestrator: Orchestrator,
    goal: String,
    debug_sequence: Option<String>,
}

impl OrchestratorRun {
    pub(crate) fn new(orchestrator: Orchestrator, goal: String, debug_sequence: Option<String>) -> Self {
        Self { orchestrator, goal, debug_sequence }
    }
}
