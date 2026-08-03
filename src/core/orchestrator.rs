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
                    // Deliberately not logging `s` itself: `chroma_client` config legitimately
                    // carries `chroma_token` (a secret), and the parse error already includes
                    // enough position/context to debug without echoing the raw string.
                    tracing::error!(error = ?e, "Failed to parse chroma_client config as JSON");
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
            let label = critic
                .get_str("name")
                .or_else(|_| critic.get_str("role"))
                .unwrap_or("Critic")
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
                // Critics run concurrently (`join_all` below), but `query_stream` streams
                // token-by-token into whatever `tx` it's given — forwarding all of them into
                // one shared channel interleaves multiple critics' output character-by-
                // character with no way for a renderer to tell them apart. So each critic
                // streams into its own local channel instead; nobody renders it live, a
                // background task just drains it to avoid blocking `query_stream` on a full
                // buffer. The caller emits one clearly-labeled, complete block per critic on
                // the real `tx` afterward, sequentially, once every critic has finished.
                let (local_tx, mut local_rx) = mpsc::channel(100);
                let drain = async { while local_rx.recv().await.is_some() {} };
                let query = async {
                    let r = retry_transient!(critic.query_stream(ollama, &mut scratch, &local_tx));
                    drop(local_tx);
                    r
                };
                let (result, ()) = tokio::join!(query, drain);
                result.map(|_| (label, scratch.output, approval_signal))
            });
        }
        let results = futures_util::future::join_all(futs).await;
        for res in results {
            match res {
                Ok((label, text, approval_signal)) => {
                    ctx.trace(tx, format!("[Critic '{label}']:\n{text}")).await;
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

    /// Recalls prior memories relevant to this run's goal, if any, before the stage machine
    /// begins. Unlike `run_librarian_retrieval` (the Librarian's on-demand, LLM-shaped query
    /// during `Stage::Retrieve`), this is deterministic — the goal text itself is the query,
    /// no LLM call needed to write a query spec, since there's no other context yet at session
    /// start to reason about narrowing it further. Reuses the Librarian's Chroma client and
    /// `embed_model`, so it's a no-op when no Librarian is configured for this run — see
    /// `TODO.md` for why a memorize-only, Librarian-less run can't recall yet. Pushed as a
    /// `TurnKind::Retrieval` turn tagged "Memory" (not "Librarian") so it's distinguishable in
    /// `history_view`/traces from an on-demand retrieval, though both feed `documents_view`
    /// identically. Never fails the run: an empty/missing collection (e.g. the very first run,
    /// before anything has ever been memorized) is the normal case, not an error, so a query
    /// failure is traced and swallowed rather than propagated.
    async fn recall_prior_memories(&self, ctx: &mut Context, tx: &mpsc::Sender<OrchestratorResult>) {
        let (Some(client), Some(librarian)) = (self.client.as_ref(), self.librarian.as_ref()) else {
            return;
        };
        let mut q = Query::default();
        let _ = q.update_from_json(serde_json::json!({
            "query": [ctx.goal.clone()],
            "n_results": 3,
        }));
        match librarian.retrieve_and_generate(client, &self.ollama, q).await {
            Ok(docs) if !docs.trim().is_empty() => {
                ctx.push_turn(TurnKind::Retrieval, "Memory", docs);
            }
            Ok(_) => {}
            Err(e) => {
                ctx.trace(tx, format!("Memory recall skipped: {e}")).await;
            }
        }
    }

    /// One-line, once-per-run summary of which model each configured role uses. Printed a
    /// single time at the start of the run (see `run_stage_machine`) instead of repeating
    /// "querying 'model'..." on every single turn (every role, every round) — each role's own
    /// colored banner already identifies who's speaking once the run is underway, so restating
    /// the model there added noise without new information.
    fn model_summary(&self) -> String {
        let mut parts = vec![
            format!("Architect={}", self.architect.get_str("model").unwrap_or("?")),
            format!("Worker={}", self.worker.get_str("model").unwrap_or("?")),
        ];
        if let Some(a) = self.scoper.as_ref() {
            parts.push(format!("Scoper={}", a.get_str("model").unwrap_or("?")));
        }
        if let Some(a) = self.librarian.as_ref() {
            parts.push(format!("Librarian={}", a.get_str("model").unwrap_or("?")));
        }
        if let Some(a) = self.validator.as_ref() {
            parts.push(format!("Validator={}", a.get_str("model").unwrap_or("?")));
        }
        for c in &self.critics {
            let label = c
                .get_str("name")
                .or_else(|_| c.get_str("role"))
                .unwrap_or("Critic");
            parts.push(format!("{label}={}", c.get_str("model").unwrap_or("?")));
        }
        if let Some(a) = self.summarizer.as_ref() {
            parts.push(format!("Summarizer={}", a.get_str("model").unwrap_or("?")));
        }
        format!(
            "Models: {} — full prompts logged to .ruchat_trace.md as the run progresses.",
            parts.join(", ")
        )
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
        self.recall_prior_memories(ctx, &tx).await;
        ctx.trace(&tx, self.model_summary()).await;

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
                            // Without this, context_view() never finds a Plan
                            // turn in a real run (only debug_stage_machine
                            // pushed one) — the Worker, Critics, and the
                            // Architect's own next round all read an empty
                            // "PLAN:" section and effectively improvise from
                            // scratch each round instead of building on it.
                            ctx.push_turn(TurnKind::Plan, "Architect", ctx.output.clone());
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
                            | ToolName::CargoCheck | ToolName::CargoClippy | ToolName::CargoDupes
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
                        match self.handle_structured_tool(&call, ctx).await {
                            Err(e) => {
                                ctx.push_turn(
                                    TurnKind::System,
                                    "Orchestrator",
                                    format!("tool call failed: {e}"),
                                );
                            }
                            // Explicit, immediate reminder right before the reask — not just
                            // documented once in the prompt — since a rule stated at the top of
                            // a long prompt is easy for smaller local models to lose track of by
                            // generation time. Local models reliably keep calling the same (or
                            // another) read-only tool again instead of switching to act on a
                            // result they already have; `run_implement_patch_loop` rejects that
                            // if it happens anyway, but heading it off here avoids burning the
                            // round on a rejection at all.
                            Ok(()) => {
                                ctx.push_turn(
                                    TurnKind::System,
                                    "Orchestrator",
                                    "Tool result is above. You've used this round's one \
                                    information-lookup — you must now emit exactly one \
                                    apply_patch (or memorize) tool_call. Do not call another \
                                    read-only tool.".into(),
                                );
                            }
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
                    self.run_implement_patch_loop(ctx, &tx).await?
                }
                Stage::Test => {
                    let report = Validation::run_build_and_test(&cancel).await?;
                    if !report.compiled || !report.tests_passed {
                        ctx.push_turn(TurnKind::Rejection, "Tester", report.rejection_message());
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
                        ctx.revert_pending_patches(&tx).await;
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
                    // Optional dedicated model for commit-message generation
                    // (`commit_message_model`); falls back to the Worker's model — always
                    // configured, since Worker is a required agent — rather than a made-up
                    // default, so the message-writer uses whatever the user already trusted
                    // enough to implement the change.
                    let commit_model = self
                        .orchestrator_config
                        .get("commit_message_model")
                        .and_then(|v| v.as_str())
                        .or_else(|| self.worker.get_str("model").ok())
                        .unwrap_or("qwen2.5-coder:14b")
                        .to_string();
                    commit_feature_branch(ctx, self.ollama.as_ref(), &commit_model).await?;
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
            ToolName::CargoClippy => {
                let out = crate::orchestrator::cargo::cargo_clippy().await?;
                ctx.push_turn(TurnKind::Retrieval, "CargoClippy", out);
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

    /// `Stage::Implement`'s patch loop, run once the Worker's turn is already pushed as an
    /// `Implementation` turn. Allows up to a per-round `patch_budget` of sequential
    /// `apply_patch` calls (reset fresh every time this stage is entered, i.e. every new round —
    /// unlike `retrieve_budget`, which is a conservative cap for the whole run) so a plan naming
    /// multiple files in its `FILES:` line can land as one commit instead of only ever touching
    /// the first of them (see `should_continue_patch_loop`). Ends the same way a single-patch
    /// round always did on `Failure` (reject/retry) or a successful `Memorize`/no-op reask after
    /// at least one patch already landed (proceed to Test) — the one new case is a Worker turn
    /// that produced no recognized tool_call *at all* on its first attempt this round (e.g. a
    /// narrative walkthrough instead of an actual tool call): rejected immediately with a
    /// precise, deterministic reason rather than silently proceeding to a Test cycle that will
    /// trivially pass (nothing changed) and hoping the Validator LLM happens to notice.
    async fn run_implement_patch_loop(
        &mut self,
        ctx: &mut Context,
        tx: &mpsc::Sender<OrchestratorResult>,
    ) -> Result<Stage> {
        let mut patch_budget: u32 = 3;
        let mut any_patch_applied = false;
        loop {
            let is_apply_patch = matches!(
                tools::parse_tool_call(&ctx.output),
                Ok(tools::StructuredToolCall { tool: ToolName::ApplyPatch, .. })
            );
            match self.worker.execute_and_verify(ctx).await? {
                Validation::Failure(err) => {
                    ctx.push_turn(TurnKind::Rejection, "ApplyPatch", err);
                    return Ok(Stage::Retry);
                }
                Validation::Success if is_apply_patch => {
                    any_patch_applied = true;
                    patch_budget = patch_budget.saturating_sub(1);
                    if !should_continue_patch_loop(ctx, patch_budget) {
                        return Ok(Stage::Test);
                    }
                    ctx.push_turn(
                        TurnKind::System,
                        "Orchestrator",
                        "Patch applied. The plan's FILES: line names more files than you've \
                        changed so far — call apply_patch for the next one now, or emit no \
                        tool call if you're done."
                            .into(),
                    );
                    retry_transient!(self.worker.query_stream(&self.ollama, ctx, tx))?;
                    ctx.push_turn(TurnKind::Implementation, "Worker", ctx.output.clone());
                }
                // `Validation::Skip` means `parse_tool_call` found nothing it recognized
                // anywhere in the Worker's output — no tool_call fence, no bare-diff fallback
                // either. On a *follow-up* reask within this same round (after at least one
                // apply_patch already succeeded) that's the Worker's normal way of signaling
                // "I'm done" — fine, proceed to Test. On the *first* attempt this round it means
                // the Worker did nothing actionable at all.
                Validation::Skip if !any_patch_applied => {
                    ctx.push_turn(
                        TurnKind::Rejection,
                        "Worker",
                        NO_TOOL_CALL_REJECTION.to_string(),
                    );
                    return Ok(Stage::Retry);
                }
                _ => return Ok(Stage::Test),
            }
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

/// Whether `Stage::Implement`'s multi-file patch loop should re-ask the Worker for another
/// `apply_patch` call after a successful one, instead of finalizing the round. Deliberately
/// count-based rather than matching planned paths against patched ones exactly (that would
/// duplicate `protocol.rs::file_in_scope`'s suffix-matching for little practical benefit here):
/// a plan with no `FILES:` line (or exactly one file) always returns `false` once one patch has
/// landed, so a single-file round behaves identically to before this loop existed.
fn should_continue_patch_loop(ctx: &Context, remaining_patch_budget: u32) -> bool {
    remaining_patch_budget > 0 && ctx.planned_files().len() > ctx.pending_patches.len()
}

/// Rejection reason for a Worker turn with no recognized tool call anywhere in it — see
/// `Orchestrator::run_implement_patch_loop`.
const NO_TOOL_CALL_REJECTION: &str = "refused: no recognized tool_call found anywhere in your \
    output — every round you must emit exactly one tool_call (e.g. apply_patch, memorize). A \
    narrative walkthrough, an explanation of what you would do, or any other prose without an \
    actual tool_call accomplishes nothing on its own.";

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

    // Regression: `Orchestrator::new`'s Librarian setup used to log the raw `chroma_client`
    // config string verbatim on parse failure (`config = %s`). That string legitimately carries
    // `chroma_token` (a secret), so a malformed-but-token-bearing config leaked it to logs.
    // Uses the real `Orchestrator::new` (not `build_test_orchestrator`) since the bug is in its
    // Librarian-construction branch specifically.
    #[test]
    fn librarian_chroma_client_parse_failure_does_not_log_the_raw_config() {
        use crate::agent::llm_client::FakeLlmClient;
        use std::io::Write;
        use std::sync::{Arc, Mutex};

        #[derive(Clone)]
        struct BufWriter(Arc<Mutex<Vec<u8>>>);
        impl Write for BufWriter {
            fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(data);
                Ok(data.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
        let writer = BufWriter(buf.clone());
        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || writer.clone())
            .finish();

        let secret = "super-secret-chroma-token-xyz";
        // Valid JSON syntax (so the outer `s.parse::<Value>()` succeeds) but neither a string
        // nor an object, so `ChromaClientConfigArgs::update_from_json` — the call this test
        // targets — rejects it via its `val.as_object().ok_or_else(...)` branch, with the
        // secret embedded in the string that would have been logged pre-fix.
        let malformed = format!(r#"["chroma_token", "{secret}"]"#);
        let config = json!({
            "Architect": { "model": "fake" },
            "Worker": { "model": "fake" },
            "Librarian": { "model": "fake", "embed_model": "fake-embed", "chroma_client": malformed },
        });

        // Current-thread runtime, deliberately: `with_default` sets a thread-local subscriber,
        // so `Orchestrator::new`'s `tracing::error!` call must run on this same thread to be
        // captured — a multi-thread runtime would poll it on a worker thread instead, silently
        // making this assertion vacuous (nothing captured, so it trivially "contains no secret").
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let result = tracing::subscriber::with_default(subscriber, || {
            rt.block_on(Orchestrator::new(
                config,
                Arc::new(FakeLlmClient::new(vec![])),
                &json!({}),
            ))
        });

        assert!(result.is_err(), "malformed chroma_client JSON should still be rejected");
        let logged = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
        assert!(
            !logged.contains(secret),
            "secret leaked into log output: {logged}"
        );
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

    #[test]
    fn patch_loop_does_not_continue_when_the_plan_named_no_files() {
        // No FILES: line at all (or a plan the parser found none in) — the common/legacy case.
        // Must behave exactly like the single-patch-per-round flow this loop replaced.
        let mut ctx = Context::new("goal".to_string());
        ctx.push_turn(TurnKind::Plan, "Architect", "just do it, no files line".to_string());
        ctx.record_patch("src/foo.rs".to_string(), "original".to_string());
        assert!(!should_continue_patch_loop(&ctx, 2));
    }

    #[test]
    fn patch_loop_does_not_continue_when_the_plan_named_exactly_one_file() {
        let mut ctx = Context::new("goal".to_string());
        ctx.push_turn(TurnKind::Plan, "Architect", "FILES: src/foo.rs".to_string());
        ctx.record_patch("src/foo.rs".to_string(), "original".to_string());
        assert!(!should_continue_patch_loop(&ctx, 2));
    }

    #[test]
    fn patch_loop_continues_when_the_plan_named_more_files_than_patched_so_far() {
        let mut ctx = Context::new("goal".to_string());
        ctx.push_turn(
            TurnKind::Plan,
            "Architect",
            "FILES: src/foo.rs, src/bar.rs".to_string(),
        );
        ctx.record_patch("src/foo.rs".to_string(), "original".to_string());
        assert!(should_continue_patch_loop(&ctx, 2));
    }

    #[test]
    fn patch_loop_stops_once_every_planned_file_is_patched_even_with_budget_left() {
        let mut ctx = Context::new("goal".to_string());
        ctx.push_turn(
            TurnKind::Plan,
            "Architect",
            "FILES: src/foo.rs, src/bar.rs".to_string(),
        );
        ctx.record_patch("src/foo.rs".to_string(), "original".to_string());
        ctx.record_patch("src/bar.rs".to_string(), "original".to_string());
        assert!(!should_continue_patch_loop(&ctx, 5));
    }

    #[test]
    fn patch_loop_stops_when_budget_is_exhausted_even_with_files_still_unplanned() {
        let mut ctx = Context::new("goal".to_string());
        ctx.push_turn(
            TurnKind::Plan,
            "Architect",
            "FILES: src/foo.rs, src/bar.rs, src/baz.rs".to_string(),
        );
        ctx.record_patch("src/foo.rs".to_string(), "original".to_string());
        assert!(!should_continue_patch_loop(&ctx, 0));
    }

    #[tokio::test]
    async fn model_summary_lists_every_configured_role_once() {
        let mut config = base_config();
        config["Validator"] = json!({ "model": "validator-model" });
        config["Critics"] = json!([{ "model": "critic-model", "name": "Security" }]);
        let orchestrator = build_test_orchestrator(config, vec![], None).await;

        let summary = orchestrator.model_summary();
        assert!(summary.contains("Architect=fake"));
        assert!(summary.contains("Worker=fake"));
        assert!(summary.contains("Validator=validator-model"));
        assert!(summary.contains("Security=critic-model"));
        assert!(summary.contains(".ruchat_trace.md"));
    }

    // Regression test for a real failure: the Worker replied with a narrative walkthrough
    // ("### Identified First Warning... ### Applying the Fix...") wrapped around fenced blocks
    // that weren't actually a tool_call (a ```bash block naming a tool as if it were a shell
    // command, then a ```rust block with raw source, not a diff) — `parse_tool_call` correctly
    // found nothing, but the orchestrator used to treat that identically to a successful
    // `memorize` and silently proceed to `Stage::Test`, wasting a cycle and leaving the Worker
    // with no specific feedback about what it did wrong.
    #[tokio::test]
    async fn implement_patch_loop_rejects_a_worker_turn_with_no_tool_call_at_all() {
        let mut orchestrator = build_test_orchestrator(base_config(), vec![], None).await;
        let mut ctx = Context::new("goal".to_string());
        ctx.output = "### Identified First Warning\n\nAssuming cargo_clippy has been run, \
            here's what I would do next. If the warning is resolved, proceed with the next \
            steps."
            .to_string();
        let (tx, _rx) = mpsc::channel(100);

        let stage = orchestrator
            .run_implement_patch_loop(&mut ctx, &tx)
            .await
            .unwrap();

        assert_eq!(stage, Stage::Retry);
        let rejection = ctx
            .turns
            .iter()
            .find(|t| t.kind == TurnKind::Rejection)
            .expect("a no-tool-call Worker turn should be rejected");
        assert_eq!(rejection.source, "Worker");
        assert!(rejection.content.contains("no recognized tool_call"));
    }

    // Regression test for a second real failure, reported right after the one above: the Worker
    // called cargo_clippy, got its result, then called cargo_clippy *again* instead of switching
    // to apply_patch — repeated across rounds until the exact-match stall guard escalated the
    // whole run. `execute_and_verify` only handles ApplyPatch/Memorize; any other (valid, known)
    // tool reaching it at verify-time used to produce a cryptic "Unexpected tool at verify
    // stage: CargoClippy" debug dump. This exercises that same code path — CargoClippy isn't
    // handled specially in `execute_and_verify`, so no real subprocess ever runs here.
    #[tokio::test]
    async fn implement_patch_loop_rejects_a_second_read_only_tool_call_with_actionable_guidance()
    {
        let mut orchestrator = build_test_orchestrator(base_config(), vec![], None).await;
        let mut ctx = Context::new("goal".to_string());
        ctx.output = "```tool_call\n{\"tool\": \"cargo_clippy\"}\n```".to_string();
        let (tx, _rx) = mpsc::channel(100);

        let stage = orchestrator
            .run_implement_patch_loop(&mut ctx, &tx)
            .await
            .unwrap();

        assert_eq!(stage, Stage::Retry);
        let rejection = ctx
            .turns
            .iter()
            .find(|t| t.kind == TurnKind::Rejection)
            .expect("a repeated read-only tool call should be rejected");
        assert!(rejection.content.contains("already used this round's one information-lookup"));
        assert!(rejection.content.contains("apply_patch"));
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

        // Regression: `query_stream` used to send a "[Role's input] querying 'model'..." trace
        // on every single turn — redundant noise once each role already gets its own colored
        // banner (`ColorChange`). That per-turn announcement is gone now (`.ruchat_trace.md`
        // still gets refreshed, just silently); a model summary is only ever printed once, at
        // the start of a real (non-debug) run — `debug_stage_machine`, which this fixture runs
        // through, doesn't call `model_summary` at all, so no trace events are expected here.
        let has_querying_trace = items.iter().any(|item| {
            matches!(item, StreamItem::Event(AgentEvent::Trace(msg)) if msg.contains("querying"))
        });
        assert!(
            !has_querying_trace,
            "no turn should announce '...querying \\'model\\'...' anymore"
        );
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

    // `recall_prior_memories` is tested directly rather than through a fixture: it's not part
    // of the fixed debug-sequence mechanism (`debug_stage_machine`), it runs unconditionally
    // once per real `run_stage_machine` call before any sequence starts. Unlike the Librarian's
    // own on-demand retrieval, it never calls `query_stream`, so no `responses` entries are
    // needed — the query is built deterministically from `ctx.goal`, not an LLM-authored spec.
    #[tokio::test]
    async fn recall_prior_memories_pushes_a_retrieval_turn_when_librarian_configured() {
        let mut config = base_config();
        config["Librarian"] = json!({ "model": "fake", "embed_model": "fake-embed" });
        let orchestrator =
            build_test_orchestrator(config, vec![], Some(fake_query_response())).await;
        let mut ctx = Context::new("fix the flaky test".to_string());
        let (tx, _rx) = mpsc::channel(100);

        orchestrator.recall_prior_memories(&mut ctx, &tx).await;

        let recalled = ctx
            .turns
            .iter()
            .find(|t| t.kind == TurnKind::Retrieval && t.source == "Memory")
            .expect("recall_prior_memories should push a Memory retrieval turn");
        assert!(recalled.content.contains("fake retrieved document"));
    }

    #[tokio::test]
    async fn recall_prior_memories_is_a_noop_without_a_librarian() {
        let orchestrator = build_test_orchestrator(base_config(), vec![], None).await;
        let mut ctx = Context::new("fix the flaky test".to_string());
        let (tx, _rx) = mpsc::channel(100);

        orchestrator.recall_prior_memories(&mut ctx, &tx).await;

        assert!(ctx.turns.is_empty());
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

    // Regression test for `run_critics_parallel` specifically — NOT via a fixture: fixtures run
    // through `debug_stage_machine`, whose "Critic_N" branch calls each critic's `query_stream`
    // directly and sequentially (see the `starts_with("Critic")` arm above), which never had
    // the bug and doesn't exercise `run_critics_parallel` at all (confirmed by temporarily
    // reverting the fix and finding `multiple_critics_dispatches_each_critic_once` above still
    // passed unchanged — it was testing the wrong code path). `run_critics_parallel` only runs
    // from the real `run_stage_machine`'s `Stage::Critique`, so it's called directly here.
    //
    // Before the fix: critics ran concurrently (`join_all`) but all streamed their responses
    // token-by-token onto the one shared `tx`, so two critics' output could interleave
    // character-by-character with no way for a renderer to tell them apart. Each critic's full
    // response must now arrive as one contiguous, clearly-labeled `Trace` block instead.
    #[tokio::test]
    async fn run_critics_parallel_emits_one_undamaged_labeled_trace_per_critic() {
        let mut config = base_config();
        config["Critics"] = json!([
            { "model": "fake", "name": "Security", "task": "security review" },
            { "model": "fake", "name": "Performance", "task": "performance review" },
        ]);
        let mut orchestrator = build_test_orchestrator(
            config,
            vec!["No issues found.\nAPPROVED", "Looks efficient.\nAPPROVED"],
            None,
        )
        .await;
        let mut ctx = Context::new("goal".to_string());
        ctx.output = "some implementation".to_string();
        let (tx, mut rx) = mpsc::channel(100);

        orchestrator
            .run_critics_parallel(&mut ctx, &tx)
            .await
            .unwrap();
        drop(tx);
        let mut traces = Vec::new();
        while let Some(item) = rx.recv().await {
            if let Ok(StreamItem::Event(AgentEvent::Trace(msg))) = item {
                traces.push(msg);
            }
        }

        // Exactly one trace block per critic — neither merged into the other nor dropped.
        assert_eq!(
            traces.len(),
            2,
            "expected exactly one trace per critic, got: {traces:?}"
        );

        // Deliberately NOT asserting which critic got which canned response: both critics race
        // on `FakeLlmClient`'s shared scripted-response queue (a plain `VecDeque`, popped in
        // call order), and since they now genuinely run concurrently (the fix this test
        // guards), which one's `chat_stream` call wins that race isn't deterministic — this
        // was a flaky test bug caught by running the full suite repeatedly, not a code bug.
        // What must hold regardless: each critic is clearly labeled, and each trace contains
        // exactly one critic's response, never both spliced together and never neither.
        assert!(traces.iter().any(|t| t.starts_with("[Critic 'Security']:")));
        assert!(traces.iter().any(|t| t.starts_with("[Critic 'Performance']:")));
        for t in &traces {
            let has_security_text = t.contains("No issues found.");
            let has_performance_text = t.contains("Looks efficient.");
            assert!(
                has_security_text ^ has_performance_text,
                "each trace should contain exactly one critic's response, not both or neither: {t:?}"
            );
        }

        // Both approved, so no rejection turns.
        assert!(!ctx.turns.iter().any(|t| t.kind == TurnKind::Rejection));
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
