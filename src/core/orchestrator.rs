pub(crate) mod git;
pub(crate) mod fs;
pub(crate) mod search;
pub(crate) mod scope;
pub(crate) mod cargo;
pub(super) mod task;

use crate::agent::event::{StreamItem, AgentEvent};
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
                // `Agent::new` looks up `config.get(role)` — it expects the role's
                // config nested under its own key (as every other role's config is
                // in `orchestrator_config`), not the flat per-critic object itself.
                // Passing `c_val.clone()` directly here used to mean `config.get(role)`
                // could never find anything, so `Agent::new` always returned
                // `Err(MissingAgent)`, silently swallowed below — Critics were never
                // actually constructed via `--agentic`/`--critic`, regardless of how
                // many were configured.
                let role = format!("Critic_{i}");
                let mut c_config = serde_json::json!({ role.clone(): c_val.clone() });
                if let Ok(agent) =
                    Agent::new(&mut c_config, &role, true, task_type, cfg.clone()).await
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
        let (done_tx, done_rx) = tokio::sync::oneshot::channel::<()>();

        // Watcher now races two exits: receiver dropped early (cancellation),
        // or the task below finished normally (done_rx). Either way this task
        // exits and drops `watcher_tx` — without that second branch, watcher_tx
        // is held alive forever on a clean run, since closed() only fires on
        // receiver drop, and the receiver only drops once the stream ends,
        // which never happens while watcher_tx keeps the channel open. That
        // was the deadlock.
        let watcher_tx = tx.clone();
        let watcher_cancel = cancel.clone();
        tokio::spawn(async move {
            tokio::select! {
                _ = watcher_tx.closed() => {
                    watcher_cancel.cancel();
                }
                _ = done_rx => {
                    // Normal completion — just exit and drop watcher_tx.
                }
            }
        });

        let task_cancel = cancel.clone();
        tokio::spawn(async move {
            let result = if let Some(path) = debug_sequence {
                self.debug_stage_machine(goal, path, tx.clone(), task_cancel).await
            } else {
                self.run_stage_machine(goal, tx.clone(), task_cancel).await
            };
            if let Err(e) = result
                && !matches!(e, RuChatError::Cancelled)
            {
                let _ = tx.send(Err(e)).await;
            }
            let _ = done_tx.send(());
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
            .unwrap_or(7);
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
        let mut last_scope_output: Option<String> = None;
        let mut last_architect_output: Option<String> = None;
        let mut last_worker_output: Option<String> = None;

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
                Stage::Done => {
                    let _ = tx.send(Ok(StreamItem::Event(AgentEvent::Done))).await;
                    break;
                }
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
                        if let Some(prev) = &last_architect_output && prev == &ctx.output {
                            ctx.push_turn(
                                TurnKind::Rejection,
                                "Orchestrator",
                                "Architect repeated identical plan with no new information — likely stalled, escalating".into(),
                            );
                            Stage::Escalate("Architect stalled: repeated identical output across rounds".into())
                        } else {
                            last_architect_output = Some(ctx.output.clone());
                            Stage::Retrieve
                        }
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
                        // A failing tool call (bad git args, missing ripgrep, a
                        // vanished file) must not abort the whole run — same
                        // posture as the Scoper's identical dispatch below.
                        // Record the failure as a turn and let the Worker see
                        // it and try something else, instead of propagating a
                        // fatal error out of the stage machine entirely.
                        if let Err(e) = self.handle_structured_tool(&call, ctx).await {
                            ctx.push_turn(
                                TurnKind::System,
                                "Orchestrator",
                                format!("tool call failed: {e}"),
                            );
                        }
                        retry_transient!(self.worker.query_stream(&self.ollama, ctx, &tx))?;
                        if let Some(prev) = &last_worker_output && prev == &ctx.output {
                            ctx.push_turn(
                                TurnKind::Rejection,
                                "Orchestrator",
                                "Worker repeated identical plan with no new information — likely stalled, escalating".into(),
                            );
                            stage = Stage::Escalate("Worker stalled: repeated identical output across rounds".into());
                            continue;
                        }
                        last_worker_output = Some(ctx.output.clone());
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
                        // Looping back for another attempt — a patch this round already
                        // applied (Test/Validate/Critique rejected it after the fact) must
                        // not be left in place, or the next Worker round starts editing an
                        // unreviewed mutation instead of the last known-good state.
                        ctx.revert_pending_patch(&tx).await;
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
                        if !ctx.turns.iter().any(|t| t.kind == TurnKind::Retrieval) {
                            // Scope gave up with zero successful lookups — don't send Architect
                            // into a vacuum. Force one deterministic, tool-free grounding step.
                            ctx.push_turn(
                                TurnKind::System,
                                "Orchestrator",
                                "Scope stage produced no retrieved information — forcing a repo listing before planning".into(),
                            );
                            let listing = crate::orchestrator::fs::list_dir(".").await.unwrap_or_default();
                            ctx.push_turn(TurnKind::Retrieval, "Orchestrator", listing);
                        }
                        Stage::Plan
                    } else {
                        scope_round += 1;
                        let stage = self.run_scope_stage(ctx, &tx).await?;
                        if let Some(prev) = &last_scope_output && prev == &ctx.output {
                            ctx.trace(&tx, "Scoper repeated identical output — forcing progression to Plan".into()).await;
                            Stage::Plan
                        } else {
                            last_scope_output = Some(ctx.output.clone());
                            stage
                        }
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
                        let reason = strip_json_fences(&ctx.output)
                            .to_string();
                        ctx.trace(&tx, format!("[REJECTED] {reason}")).await;
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
                        let reason = strip_json_fences(&ctx.output)
                            .to_string();
                        ctx.trace(&tx, format!("[REJECTED] {reason}")).await;
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
                let path = opt_str(&call.args, "path");
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
                let path = opt_str(&call.args, "path");
                let staged = call.args.get("staged").and_then(|v| v.as_bool()).unwrap_or(false);
                let out = git::git_diff(path, staged).await?;
                ctx.push_turn(TurnKind::Retrieval, "GitDiff", out);
                Ok(())
            }
            ToolName::GitSearchHistory => {
                let pattern = call.args["pattern"].as_str().unwrap_or_default();
                let mode = call.args["mode"].as_str().unwrap_or("message");
                let path = opt_str(&call.args, "path");
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
                let path = opt_str(&call.args, "path");
                let glob = opt_str(&call.args, "glob");
                let max_count = call.args.get("max_count").and_then(|v| v.as_u64()).map(|v| v as u32);
                let out = crate::orchestrator::search::ripgrep(pattern, path, glob, max_count).await?;
                ctx.push_turn(TurnKind::Retrieval, "Ripgrep", out);
                Ok(())
            }
            ToolName::ReadTags => {
                let symbol = opt_str(&call.args, "symbol");
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
            if let Some(reason) = looks_like_placeholder(&item) {
                ctx.push_turn(
                    TurnKind::System,
                    "Scoper",
                    format!(
                        "rejected lookup request: {reason}. You must use a real value — if \
                        INFORMATION GATHERED SO FAR already contains a path from an earlier \
                        ripgrep/list_dir result, copy that exact path; otherwise run ripgrep or \
                        list_dir first to discover one."
                    ),
                );
                continue;
            }
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

/// Treats an explicit empty string the same as an omitted optional field.
/// Models reliably emit `"path": ""` instead of leaving an optional arg out
/// entirely, and downstream commands (e.g. `git log -- ""`) reject an empty
/// pathspec outright rather than treating it as "no restriction" — this
/// normalizes that before it ever reaches them.
fn opt_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(|v| v.as_str()).filter(|s| !s.is_empty())
}

/// Catches values the model copied from prompt scaffolding instead of
/// producing real ones — angle-bracket placeholders (`<path>`), the literal
/// word `placeholder`, or template-looking segments like `path/to/`. Cheap
/// heuristic, not exhaustive; it exists to turn a wasted I/O round-trip into
/// an immediate, specific correction instead.
fn looks_like_placeholder(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => {
            let lower = s.to_lowercase();
            if s.contains('<') && s.contains('>') {
                Some(format!("'{s}' looks like a placeholder, not a real value"))
            } else if lower.contains("path/to") || lower.contains("<") {
                Some(format!("'{s}' looks like a template, not a real value"))
            } else {
                None
            }
        }
        Value::Object(map) => map.values().find_map(looks_like_placeholder),
        Value::Array(arr) => arr.iter().find_map(looks_like_placeholder),
        _ => None,
    }
}

/// Runs each `agent_debug/*.json` fixture through the real stage machine
/// (`debug_stage_machine`, via `run_task_stream`) against a scripted
/// `FakeLlmClient`/`FakeVectorStore` instead of a live Ollama/Chroma server.
/// These were sitting unused as fixtures on disk before this — nothing
/// exercised them, which is exactly how `multiple_critics.json`/`critic.json`
/// carried a naming bug (`"Critic0"` instead of `"Critic_0"`, silently
/// falling back to critic index 0 every time) for who knows how long.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::llm_client::fake_vector_store::FakeVectorStore;
    use crate::agent::llm_client::FakeLlmClient;
    use chroma::types::QueryResponse;
    use serde_json::json;

    /// Builds an `Orchestrator` by hand (not `Orchestrator::new`, which always
    /// constructs a real `ChromaHttpClient` for the Librarian) so Librarian
    /// fixtures can run against a `FakeVectorStore` instead of a live Chroma
    /// server. Each role's `Agent` is still built through the real
    /// `Agent::new`, so config parsing/merging is exercised exactly as in
    /// production — only the network-facing clients are swapped.
    async fn build_test_orchestrator(
        mut config: Value,
        responses: Vec<&str>,
        query_response: Option<QueryResponse>,
    ) -> Orchestrator {
        let architect = Agent::new(&mut config, "Architect", true, None, json!({}))
            .await
            .unwrap();
        let worker = Agent::new(&mut config, "Worker", true, None, json!({}))
            .await
            .unwrap();
        let validator = Agent::new(&mut config, "Validator", false, None, json!({}))
            .await
            .ok();
        let summarizer = Agent::new(&mut config, "Summarizer", false, None, json!({}))
            .await
            .ok();
        let scoper = Agent::new(&mut config, "Scoper", false, None, json!({}))
            .await
            .ok();
        let librarian = Agent::new(&mut config, "Librarian", false, None, json!({}))
            .await
            .ok();

        let mut critics = Vec::new();
        if let Some(critic_list) = config.get("Critics").and_then(|v| v.as_array()) {
            for (i, c_val) in critic_list.iter().enumerate() {
                let role = format!("Critic_{i}");
                let mut c_config = json!({ role.clone(): c_val.clone() });
                if let Ok(agent) = Agent::new(&mut c_config, &role, true, None, json!({})).await {
                    critics.push(agent);
                }
            }
        }

        let client: Option<Arc<dyn VectorStore>> = query_response
            .map(|response| Arc::new(FakeVectorStore { response }) as Arc<dyn VectorStore>);

        Orchestrator {
            scoper,
            architect,
            worker,
            librarian,
            critics,
            summarizer,
            validator,
            orchestrator_config: config,
            ollama: Arc::new(FakeLlmClient::new(responses)),
            client,
        }
    }

    fn fake_query_response() -> QueryResponse {
        QueryResponse {
            ids: vec![vec!["doc1".to_string()]],
            embeddings: None,
            documents: Some(vec![vec![Some("fake retrieved document".to_string())]]),
            uris: None,
            metadatas: None,
            distances: None,
            include: vec![],
        }
    }

    /// Runs a fixture to completion and returns every streamed item —
    /// panics (failing the test) if the stage machine ever surfaces an
    /// `Err`, which is the thing debug-mode is meant to make crisp to catch.
    async fn run_fixture(
        fixture: &str,
        config: Value,
        responses: Vec<&str>,
        query_response: Option<QueryResponse>,
    ) -> Vec<StreamItem> {
        let orchestrator = build_test_orchestrator(config, responses, query_response).await;
        let path = format!("agent_debug/{fixture}");
        let stream = orchestrator.run_task_stream("test goal".to_string(), Some(path));
        tokio_stream::StreamExt::collect::<Vec<Result<StreamItem>>>(stream)
            .await
            .into_iter()
            .map(|r| r.expect("debug sequence produced an error"))
            .collect()
    }

    fn base_config() -> Value {
        json!({
            "Architect": { "model": "fake" },
            "Worker": { "model": "fake" },
        })
    }

    #[tokio::test]
    async fn architect_only_completes() {
        let items = run_fixture(
            "architect_only.json",
            base_config(),
            vec!["Plan: refactor error handling to use `?`."],
            None,
        )
        .await;
        assert!(!items.is_empty());
    }

    #[tokio::test]
    async fn librarian_only_completes() {
        let mut config = base_config();
        config["Librarian"] = json!({ "model": "fake", "embed_model": "fake-embed" });
        let items = run_fixture(
            "librarian_only.json",
            config,
            vec!["{\"query\": \"error handling\", \"n_results\": 5, \"collection\": \"repo\"}"],
            Some(fake_query_response()),
        )
        .await;
        assert!(!items.is_empty());
    }

    #[tokio::test]
    async fn librarian_and_worker_completes() {
        let mut config = base_config();
        config["Librarian"] = json!({ "model": "fake", "embed_model": "fake-embed" });
        let items = run_fixture(
            "librarian_and_worker.json",
            config,
            vec![
                "{\"query\": \"error handling\", \"n_results\": 5, \"collection\": \"repo\"}",
                "Replaced unwrap() with `?` and anyhow::Context.",
            ],
            Some(fake_query_response()),
        )
        .await;
        assert!(!items.is_empty());
    }

    #[tokio::test]
    async fn worker_and_validator_completes() {
        let mut config = base_config();
        config["Validator"] = json!({ "model": "fake" });
        let items = run_fixture(
            "worker_and_validator.json",
            config,
            vec![
                "fn foo() -> Result<()> { do_thing()?; Ok(()) }",
                "{\"verdict\": \"VALIDATED\", \"reason\": \"\"}",
            ],
            None,
        )
        .await;
        assert!(!items.is_empty());
    }

    #[tokio::test]
    async fn worker_and_validator_rejection_completes() {
        let mut config = base_config();
        config["Validator"] = json!({ "model": "fake" });
        let items = run_fixture(
            "worker_and_validator_rejection.json",
            config,
            vec![
                "fn foo() { do_thing().unwrap(); }",
                "{\"verdict\": \"REJECTED\", \"reason\": \"still uses unwrap()\"}",
            ],
            None,
        )
        .await;
        assert!(!items.is_empty());
    }

    #[tokio::test]
    async fn summarizer_completes() {
        let mut config = base_config();
        config["Summarizer"] = json!({ "model": "fake" });
        let items = run_fixture(
            "summarizer.json",
            config,
            vec!["Summary: several rejections over unwrap() usage; not yet resolved."],
            None,
        )
        .await;
        assert!(!items.is_empty());
    }

    #[tokio::test]
    async fn validator_only_completes() {
        let mut config = base_config();
        config["Validator"] = json!({ "model": "fake" });
        let items = run_fixture(
            "validator_only.json",
            config,
            vec!["{\"verdict\": \"REJECTED\", \"reason\": \"unwrap() on line 42\"}"],
            None,
        )
        .await;
        assert!(!items.is_empty());
    }

    /// Regression test for the `"Critic0"` vs `"Critic_0"` naming bug this
    /// fixture originally carried (see the module doc comment) — with two
    /// distinct scripted responses queued, if the sequence's second step
    /// silently dispatched to critic index 0 again instead of index 1, the
    /// `FakeLlmClient` queue would desync and either this call would consume
    /// the wrong response or the queue would run dry, panicking.
    #[tokio::test]
    async fn multiple_critics_dispatches_each_critic_once() {
        let mut config = base_config();
        config["Critics"] = json!([
            { "model": "fake", "task": "security review" },
            { "model": "fake", "task": "performance review" },
        ]);
        let items = run_fixture(
            "multiple_critics.json",
            config,
            vec!["No issues found.\nAPPROVED", "Looks efficient.\nAPPROVED"],
            None,
        )
        .await;
        assert!(!items.is_empty());
    }

    #[tokio::test]
    async fn critic_only_completes() {
        let mut config = base_config();
        config["Critics"] = json!([{ "model": "fake", "task": "security review" }]);
        let items = run_fixture(
            "critic.json",
            config,
            vec!["No issues found.\nAPPROVED"],
            None,
        )
        .await;
        assert!(!items.is_empty());
    }
}
