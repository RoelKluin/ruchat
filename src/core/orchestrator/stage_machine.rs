use super::stall_mitigation::{
    auto_ground_planned_file, is_near_duplicate, is_read_only_worker_tool,
    strip_architect_tool_call_hallucination,
};
use super::{Orchestrator, OrchestratorResult, Stage, checkpoint, git, run_summary};
use crate::Result;
use crate::RuChatError;
use crate::agent::event::{AgentEvent, StreamItem};
use crate::agent::json_extract::strip_json_fences;
use crate::agent::protocol::Validation;
use crate::agent::tools;
use crate::agent::types::{Context, TurnKind};
use crate::retry_transient;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// Per-run budgets/flags `run_stage_machine_loop` needs, bundled so passing them down doesn't
/// trip `clippy::too_many_arguments` — plain config read once before the loop starts, not state
/// the loop mutates, so a struct rather than individual fields costs nothing here.
struct StageLoopBudgets {
    max_iterations: u64,
    max_scope_iterations: u64,
    approve_commit: bool,
}

#[derive(serde::Deserialize)]
struct ValidatorVerdict {
    verdict: String,
    #[serde(default)]
    reason: String,
}

/// Whether a `--approve` commit-gate answer counts as approval — deliberately strict (an exact
/// "y"/"yes", case-insensitive-ish via explicit variants, trimmed) rather than "anything not
/// starting with n": a HITL approval gate that defaults to yes on ambiguous or accidental input
/// (a stray keystroke, a blank line from a fumbled Enter) would defeat the entire point of the
/// gate. Everything else, including an empty line, counts as rejection.
fn is_approval_yes(answer: &str) -> bool {
    matches!(answer.trim(), "y" | "Y" | "yes" | "Yes")
}

/// Computes `AgentEvent::Progress`'s round-based completion percentage, `[0.0, 100.0]`, for
/// `Stage::Plan`. A coarse, monotonically-increasing signal for a user watching a long run to
/// gauge proximity to the iteration budget — not a precise ETA, since a single round (e.g. one
/// with a slow `cargo test`) can still take arbitrarily long. Pulled out as its own function for
/// direct unit testing, the same tradeoff `implement::should_continue_patch_loop` makes:
/// exercising `run_stage_machine` itself through a full round requires either a live Ollama/Chroma round
/// trip or `Stage::Test`'s real `cargo test` invocation, both out of scope for a `--lib` unit
/// test per this file's existing test-placement precedent.
fn progress_pct(round: u64, max_iterations: u64) -> f32 {
    if max_iterations == 0 {
        return 100.0;
    }
    (round as f32 / max_iterations as f32 * 100.0).min(100.0)
}

impl Orchestrator {
    /// One-line, once-per-run summary of which model each configured role uses. Printed a
    /// single time at the start of the run (see `run_stage_machine`) instead of repeating
    /// "querying 'model'..." on every single turn (every role, every round) — each role's own
    /// colored banner already identifies who's speaking once the run is underway, so restating
    /// the model there added noise without new information.
    pub(super) fn model_summary(&self) -> String {
        let mut parts = vec![
            format!(
                "Architect={}",
                self.architect.get_str("model").unwrap_or("?")
            ),
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
            "Models: {} — full prompts logged to ruchat_traces/ as the run progresses.",
            parts.join(", ")
        )
    }

    pub(super) async fn run_stage_machine(
        &mut self,
        goal: String,
        tx: mpsc::Sender<OrchestratorResult>,
        cancel: CancellationToken,
        resume: bool,
        approve_commit: bool,
        trace_timings: bool,
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
        // See `checkpoint.rs` for the full "Resumable/crash-resilient runs" mechanism (ROADMAP.md
        // Phase 3). A fresh run starts from `Context::new`/`Stage::Scope`, same as always; `--
        // resume` reloads the last-completed stage's checkpoint instead — `goal` above is
        // ignored in that case, since resuming continues the *same* task, not a new one.
        let (mut ctx, stage) = if resume {
            checkpoint::Checkpoint::load(std::path::Path::new(checkpoint::CHECKPOINT_PATH))
                .await?
                .into_context_and_stage()
        } else {
            (Context::new(goal), Stage::Scope)
        };
        let ctx = &mut ctx;
        if !resume {
            // A resumed `Context` already carries the `trace_index` its checkpoint captured —
            // allocating a fresh one here would start a new trace file and lose the pre-crash
            // history the old one had.
            ctx.init_trace_index().await;
        }
        // Re-applied unconditionally (fresh or resumed) — `--trace-timings` isn't persisted
        // through `Checkpoint`, same as `--team-model`/etc. already must be re-specified on
        // `--resume`, so this always reflects *this* invocation's flag, not a stale one.
        ctx.trace_timings = trace_timings;

        if let Some(librarian) = self.librarian.as_ref() {
            ctx.read_config_file(
                librarian
                    .get_str("db_config_path")
                    .unwrap_or("db_config.json"),
            )?;
        }
        self.recall_prior_memories(ctx, &tx).await;
        ctx.trace(&tx, self.model_summary()).await;

        // Split out so a `?` anywhere in the loop below — an LLM call failing, cancellation, a
        // build error, any of the dozen fallible steps a round can hit — can no longer make the
        // run skip `finalize_trace` entirely by returning straight out of this function. That
        // used to be the common case: most archived runs never reached the summaries/successes/
        // failures directories at all (TODO.md item 19), because the trace file this analyzes
        // lives on `ctx`, which stayed alive here regardless of how the loop ended, but the old
        // single-function shape threw it away unread on every early `?`. `tx`/`cancel` are cheap
        // handles (`mpsc::Sender`/`CancellationToken` are both `Clone`), so cloning them into the
        // loop instead of taking them by reference sidesteps any lifetime gymnastics here.
        let result = self
            .run_stage_machine_loop(
                ctx,
                stage,
                tx.clone(),
                cancel.clone(),
                StageLoopBudgets {
                    max_iterations,
                    max_scope_iterations,
                    approve_commit,
                },
            )
            .await;
        if let Err(ref e) = result {
            ctx.trace(&tx, format!("Run aborted before completion: {e}"))
                .await;
        }
        let success = matches!(result, Ok(true));
        self.finalize_trace(ctx, &tx, success).await;
        result.map(|_| ())
    }

    /// The stage-machine loop itself, extracted from `run_stage_machine` so an early error can
    /// still be reported to `finalize_trace` by the caller — see the comment there. Behavior is
    /// unchanged from before the split: same stages, same budgets, same checkpointing.
    async fn run_stage_machine_loop(
        &mut self,
        ctx: &mut Context,
        mut stage: Stage,
        tx: mpsc::Sender<OrchestratorResult>,
        cancel: CancellationToken,
        budgets: StageLoopBudgets,
    ) -> Result<bool> {
        let StageLoopBudgets {
            max_iterations,
            max_scope_iterations,
            approve_commit,
        } = budgets;
        let mut retrieve_budget: u32 = 2; // conservative cap on Worker-initiated retrievals per run
        let mut scope_round = 0;
        let mut last_scope_output: Option<String> = None;
        let mut last_architect_output: Option<String> = None;
        // Set only by `Stage::Commit` succeeding — both `Stage::Escalate` and `Stage::Retry`'s
        // iteration-budget-exhausted branch reach `Stage::Done` without ever going through
        // Commit, and both are "the agents did not reach a successful, committed result" even
        // though only one of them is technically an escalation.
        let mut success = false;

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
                    // A deliberate, recorded outcome — not a crash — so there's nothing left to
                    // `--resume`; see `checkpoint.rs::Checkpoint::clear`'s doc comment.
                    checkpoint::Checkpoint::clear(std::path::Path::new(
                        checkpoint::CHECKPOINT_PATH,
                    ))
                    .await;
                    let _ = tx.send(Ok(StreamItem::Event(AgentEvent::Done))).await;
                    break;
                }
                Stage::Escalate(reason) => {
                    checkpoint::Checkpoint::clear(std::path::Path::new(
                        checkpoint::CHECKPOINT_PATH,
                    ))
                    .await;
                    ctx.trace(&tx, format!("ESCALATED: {reason}")).await;
                    break;
                }
                Stage::Plan => {
                    ctx.round += 1;
                    // Per-round progress signal for a user watching a long run — the natural
                    // per-round checkpoint the orchestrator already tracks via `ctx.round`/
                    // `max_iterations`. Fire-and-forget like the `Done` event below: a dropped
                    // receiver here is already handled by `cancel` on the next loop iteration,
                    // so there's nothing more useful to do with a send error than ignore it.
                    let pct = progress_pct(ctx.round, max_iterations);
                    let _ = tx
                        .send(Ok(StreamItem::Event(AgentEvent::Progress(pct))))
                        .await;
                    if ctx.round > max_iterations {
                        Stage::Escalate("max iterations reached without acceptance".into())
                    } else {
                        let architect_start = std::time::Instant::now();
                        retry_transient!(self.architect.query_stream(&self.chat, ctx, &tx))?;
                        // `architect.md` explicitly forbids the Architect from ever emitting a
                        // `tool_call` — it's plan-only, no tools — but a live-verified run (see
                        // TODO.md's pinned reliability item) showed it doing so anyway, embedding
                        // a full, hallucinated `apply_patch` diff (a phantom comment line that
                        // didn't exist in the real file) right inside its "plan." The Worker then
                        // copied that exact broken diff verbatim, unchanged, across 3 separate
                        // rounds — it wasn't reasoning about the real file content at all, just
                        // parroting the Architect's fabrication. Stripped deterministically here,
                        // before the plan is ever stored: with nothing ready-made to copy, the
                        // Worker has to construct its own diff from DOCUMENTS/real content.
                        ctx.output = strip_architect_tool_call_hallucination(&ctx.output);
                        // A repeated plan is NOT treated as fatal. It used to trigger an
                        // immediate `Stage::Escalate` on the very first repeat — measured against
                        // real runs (see TODO.md's pinned reliability item), that was killing the
                        // overwhelming majority of them after just 1-2 of a configured 5-round
                        // `max_iterations` budget, well before it was exhausted: a repeated plan
                        // doesn't mean no progress is possible, since the Worker/Test/Validate
                        // stages can still behave differently this round (e.g. producing a
                        // corrected diff informed by a rejection now in context that wasn't
                        // there last round). `ctx.round > max_iterations` above already bounds a
                        // genuine infinite stall without a separate fast-fail — same posture the
                        // Scoper's own identical-output handling already uses (forces
                        // progression instead of escalating).
                        let mut should_escalate = false;
                        if let Some(prev) = &last_architect_output
                            && is_near_duplicate(prev, &ctx.output)
                        {
                            // Count recent System notes from this round warning about duplicates
                            let duplicate_warnings = ctx
                                .turns
                                .iter()
                                .rev()
                                .take_while(|t| t.round == ctx.round && t.kind == TurnKind::System)
                                .filter(|t| t.content.contains("near-duplicate"))
                                .count();

                            // After 2 consecutive duplicate plans, escalate rather than keep
                            // advising (with history_view deduplication, model should now be able
                            // to change the plan; repeated duplicates despite that signal an actual stall)
                            if duplicate_warnings >= 1 {
                                should_escalate = true;
                            } else {
                                ctx.push_turn(
                                    TurnKind::System,
                                    "Orchestrator",
                                    "Note: this plan is a near-duplicate of the previous round's \
                                    (allowing for minor wording drift, not just byte-identical). If \
                                    the rejection reason above suggests a different approach, use it \
                                    now — otherwise proceeding with the same plan is fine as long \
                                    as the implementation actually changes this round."
                                        .into(),
                                );
                            }
                        }
                        last_architect_output = Some(ctx.output.clone());
                        // Without this, context_view() never finds a Plan
                        // turn in a real run (only debug_stage_machine
                        // pushed one) — the Worker, Critics, and the
                        // Architect's own next round all read an empty
                        // "PLAN:" section and effectively improvise from
                        // scratch each round instead of building on it.
                        ctx.push_turn_timed(
                            TurnKind::Plan,
                            "Architect",
                            ctx.output.clone(),
                            architect_start,
                        );
                        if should_escalate {
                            Stage::Escalate(
                                "Architect repeated a plan despite near-duplicate warning".into(),
                            )
                        } else {
                            Stage::Retrieve
                        }
                    }
                }
                Stage::Retrieve => {
                    if ctx.round == 1 && self.librarian.is_some() {
                        self.run_librarian_retrieval(ctx, &tx).await?;
                    }
                    auto_ground_planned_file(ctx).await;
                    Stage::Implement
                }
                Stage::Implement => {
                    // Re-assigned before each reask below, so whichever push_turn_timed call
                    // ends up storing `ctx.output` — inside the read-only-tool branch or the
                    // fallthrough at the bottom — always attributes it to the query that
                    // actually produced it, not an earlier one in the same round.
                    let mut worker_start = std::time::Instant::now();
                    retry_transient!(self.worker.query_stream(&self.chat, ctx, &tx))?;

                    if let Ok(call) = tools::parse_tool_call(&ctx.output)
                        && is_read_only_worker_tool(&call.tool)
                        && retrieve_budget > 0
                    {
                        retrieve_budget -= 1;
                        // Records the read-only tool-call *action* itself — before
                        // `handle_structured_tool` runs it and the reask below overwrites
                        // `ctx.output` with the Worker's next response. Without this, the trace
                        // only ever showed the tool's *output* (the Retrieval turn
                        // `handle_structured_tool` pushes below), never what was actually
                        // called or with what arguments — the console showed the action (via
                        // the streamed response) but the trace file didn't, making it unclear
                        // after the fact which tool produced which result.
                        ctx.push_turn_timed(
                            TurnKind::Implementation,
                            "Worker",
                            ctx.output.clone(),
                            worker_start,
                        );
                        // A failing tool call (bad git args, missing ripgrep, a
                        // vanished file) must not abort the whole run — same
                        // posture as the Scoper's identical dispatch below.
                        // Record the failure as a turn and let the Worker see
                        // it and try something else, instead of propagating a
                        // fatal error out of the stage machine entirely.
                        match self.handle_structured_tool(&call, ctx, &tx).await {
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
                                    read-only tool."
                                        .into(),
                                );
                            }
                        }
                        worker_start = std::time::Instant::now();
                        retry_transient!(self.worker.query_stream(&self.chat, ctx, &tx))?;

                        // Bounded second chance, in-round: real traces (see TODO.md's pinned
                        // reliability item) show the Worker very often ignoring the reminder
                        // above and calling a read-only tool again anyway (a mechanical local-
                        // model mistake — repeating a just-completed action) — which used to
                        // fall straight through to `execute_and_verify`'s rejection below and
                        // burn the *entire* round on it. One more sharper nudge-and-reask first:
                        // does NOT re-spend `retrieve_budget` or re-run the tool (its result is
                        // already in context, rerunning it adds nothing) — just a stronger,
                        // final reminder. If the Worker still won't switch after this, the
                        // existing `execute_and_verify` "called X again" rejection (agent.rs)
                        // takes over exactly as before and the round is spent via the normal
                        // `Stage::Retry` path — bounded, not an infinite loop.
                        if let Ok(repeat_call) = tools::parse_tool_call(&ctx.output)
                            && is_read_only_worker_tool(&repeat_call.tool)
                        {
                            ctx.push_turn_timed(
                                TurnKind::Implementation,
                                "Worker",
                                ctx.output.clone(),
                                worker_start,
                            );
                            ctx.push_turn(
                                TurnKind::System,
                                "Orchestrator",
                                format!(
                                    "You called '{:?}' again — its result is already shown \
                                    above, calling it again will not run it a second time and \
                                    will not add anything new. This is your last chance this \
                                    round: emit exactly one apply_patch (or memorize) \
                                    tool_call now, with no other tool calls.",
                                    repeat_call.tool
                                ),
                            );
                            worker_start = std::time::Instant::now();
                            retry_transient!(self.worker.query_stream(&self.chat, ctx, &tx))?;
                        }
                    }
                    ctx.push_turn_timed(
                        TurnKind::Implementation,
                        "Worker",
                        ctx.output.clone(),
                        worker_start,
                    );
                    self.run_implement_patch_loop(ctx, &tx).await?
                }
                Stage::Test => {
                    let test_start = std::time::Instant::now();
                    let report = Validation::run_build_and_test(&cancel).await?;
                    if !report.compiled || !report.tests_passed {
                        ctx.push_turn_timed(
                            TurnKind::Rejection,
                            "Tester",
                            report.rejection_message(),
                            test_start,
                        );
                        Stage::Retry
                    } else {
                        // Same posture as the Validator's own VALIDATED-verdict turn below:
                        // every stage's actual outcome should be visible in the trace, not just
                        // the ones that trigger a rejection — and `cargo check`+`cargo test` is
                        // typically one of the slowest steps in a round, exactly the kind of
                        // cost `--trace-timings` exists to surface.
                        ctx.push_turn_timed(
                            TurnKind::System,
                            "Tester",
                            "cargo check and cargo test both passed.".to_string(),
                            test_start,
                        );
                        Stage::Validate
                    }
                }
                Stage::Validate => {
                    if let Some(validator) = self.validator.as_mut() {
                        let validator_start = std::time::Instant::now();
                        retry_transient!(validator.query_stream(&self.chat, ctx, &tx))?;
                        let stripped = strip_json_fences(&ctx.output);
                        match serde_json::from_str::<ValidatorVerdict>(stripped).ok() {
                            Some(v) if v.verdict.eq_ignore_ascii_case("REJECTED") => {
                                ctx.push_turn_timed(
                                    TurnKind::Rejection,
                                    "Validator",
                                    v.reason,
                                    validator_start,
                                );
                                Stage::Retry
                            }
                            Some(_) => {
                                // Unlike the REJECTED/unparseable arms below, a VALIDATED
                                // verdict used to push no turn at all — the Validator's action
                                // was streamed live to the console but never recorded, so it
                                // was invisible in the trace file afterward even though nothing
                                // went wrong. Every agent's actual output should be visible in
                                // the trace, not just the ones that trigger a rejection.
                                ctx.push_turn_timed(
                                    TurnKind::System,
                                    "Validator",
                                    ctx.output.clone(),
                                    validator_start,
                                );
                                Stage::Critique
                            }
                            None => {
                                // Conservative: unparseable verdict is treated
                                // as a rejection rather than silently passing.
                                ctx.push_turn_timed(
                                    TurnKind::Rejection,
                                    "Validator",
                                    format!(
                                        "Validator produced unparseable output: {}",
                                        ctx.output
                                    ),
                                    validator_start,
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
                            // Real bug found 2026-08-04 (maintainer's own working tree): this arm
                            // used to skip `revert_pending_patches` entirely — only the normal
                            // still-has-budget retry loop below called it. That left the final
                            // round's applied-but-never-validated patch sitting on disk,
                            // uncommitted, indefinitely: not reverted (this arm), not committed
                            // (deliberately, see below), just silently mutated and forgotten. A
                            // real, reproduced instance: an invalid `allow_unused = true` clap
                            // builder call (doesn't exist in clap's API) got left in
                            // `src/cli/config.rs`, breaking the build for every subsequent run —
                            // including unrelated later `ruchat pipe` invocations, which then
                            // reported confusing pre-existing compile errors with no indication
                            // they weren't caused by anything in the current run. Same posture as
                            // the still-has-budget branch below: a patch that was never validated
                            // must not be left in place. The trace file (not the working tree)
                            // is where a failed attempt's diff should be reviewed from.
                            ctx.revert_pending_patches(&tx).await;
                            ctx.trace(&tx, "Iteration budget exhausted — NOT committed, unresolved feedback remains; working tree reverted to its pre-run state (see the trace file for what was attempted).".into()).await;
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
                                let summarizer_start = std::time::Instant::now();
                                retry_transient!(summarizer.query_stream(&self.chat, ctx, &tx))?;
                                ctx.collapse_to_summary(ctx.output.clone(), summarizer_start);
                            }
                        }
                        Stage::Plan
                    }
                }
                Stage::Accept => Stage::Commit,
                Stage::Commit => {
                    // Optional interactive human-in-the-loop approval gate — off by default
                    // (`--approve`). ruchat's only approval mechanism otherwise is automated
                    // Critics (an LLM-driven gate) plus post-hoc review of the committed branch;
                    // this closes the real gap AutoGen's UserProxy/LangGraph's interrupts cover
                    // (identified via `comparisons/*.md`) without adding open-ended
                    // interactivity elsewhere — just this one, well-known pause point. Rejecting
                    // just returns `Stage::Escalate` like any other escalation in this match —
                    // the top-level `Stage::Escalate` arm above already handles clearing the
                    // checkpoint, tracing, and breaking, so there's nothing special to do here.
                    let approved = if approve_commit {
                        let plan = ctx
                            .turns
                            .iter()
                            .rev()
                            .find(|t| t.kind == TurnKind::Plan)
                            .map(|t| t.content.clone())
                            .unwrap_or_else(|| "(no plan turn found)".to_string());
                        let diff = git::git_diff(None, false)
                            .await
                            .unwrap_or_else(|e| format!("(failed to compute diff: {e})"));
                        ctx.trace(
                            &tx,
                            format!(
                                "[APPROVAL REQUIRED] About to commit. Latest plan:\n{plan}\n\n\
                                 Pending diff:\n{diff}\n\nType 'y' to commit, anything else to \
                                 stop without committing."
                            ),
                        )
                        .await;
                        let mut io = crate::io::Io::new();
                        let answer = io.read_line().await.unwrap_or_default();
                        is_approval_yes(&answer)
                    } else {
                        true
                    };

                    if !approved {
                        Stage::Escalate("commit rejected by human approval gate".into())
                    } else if ctx.pending_patches.is_empty() {
                        // Real failure seen in 5 of 8 archived agent commits (2026-08-05): the run
                        // reached Commit having applied no patch at all, and committed anyway —
                        // because `commit_add_targets` always stages `featured_changes.md`, so
                        // there was something to commit even with zero source changes. Those land
                        // as branches whose only content is a changelog entry describing work that
                        // never happened, which also makes any success-rate measurement lie.
                        Stage::Escalate(
                            "nothing to commit: no file changes were applied this run".into(),
                        )
                    } else {
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
                        // Optional explicit branch name (`--feature-branch`). Absent, the commit
                        // continues the most recent `ai/feature-*` branch — see
                        // `git::resolve_feature_branch`.
                        let requested_branch = self
                            .orchestrator_config
                            .get("feature_branch")
                            .and_then(|v| v.as_str())
                            .map(str::to_string);
                        if let Err(e) = git::commit_feature_branch(
                            ctx,
                            self.chat.as_ref(),
                            &commit_model,
                            requested_branch.as_deref(),
                        )
                        .await
                        {
                            // Live-verified 2026-08-05: `commit_feature_branch`'s own checkout
                            // to a *continued* `ai/feature-*` branch (the default — see
                            // `git::resolve_feature_branch`) can fail outright ("Your local
                            // changes... would be overwritten by checkout") when that branch's
                            // committed version of a file differs from the round's own
                            // uncommitted `apply_patch` output — normal for two runs that both
                            // touch the same file. `commit_feature_branch` already best-efforts
                            // its own return-to-original-branch checkout on failure, but that
                            // only moves HEAD back; it never touches the working tree, so the
                            // round's applied-but-never-committed patch was left sitting on
                            // disk indefinitely — the same class of bug contributor #7's
                            // `revert_pending_patches` calls elsewhere in this loop exist to
                            // prevent, just not yet covering this call site.
                            ctx.revert_pending_patches(&tx).await;
                            return Err(e);
                        }
                        success = true;
                        Stage::Done
                    }
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
                            let listing = crate::orchestrator::fs::list_dir(".")
                                .await
                                .unwrap_or_default();
                            ctx.push_turn(TurnKind::Retrieval, "Orchestrator", listing);
                        }
                        Stage::Plan
                    } else {
                        scope_round += 1;
                        let stage = self.run_scope_stage(ctx, &tx).await?;
                        if let Some(prev) = &last_scope_output
                            && is_near_duplicate(prev, &ctx.output)
                        {
                            ctx.trace(
                                &tx,
                                "Scoper repeated a near-duplicate of its previous output — \
                                forcing progression to Plan"
                                    .into(),
                            )
                            .await;
                            Stage::Plan
                        } else {
                            last_scope_output = Some(ctx.output.clone());
                            stage
                        }
                    }
                }
            };
            // `Stage::Done`/`Stage::Escalate` both `break` above, before reaching here — this
            // only ever runs for a genuine, non-terminal transition, which is exactly "after
            // each stage transition" per the checkpoint's own scoping.
            checkpoint::Checkpoint::save(
                ctx,
                &stage,
                std::path::Path::new(checkpoint::CHECKPOINT_PATH),
            )
            .await;
        }
        Ok(success)
    }

    /// Analyzes the just-finished run's in-memory trace (`ctx.trace_body()` — never written to
    /// disk itself) and archives a single analysis file under `ruchat_traces/successes/` or
    /// `ruchat_traces/failures/`: how the run ended, plus a round-by-round review of the
    /// agents' decisions saying which were good calls and which were not
    /// (`run_summary::generate_step_review`). No raw trace text is ever written to disk.
    ///
    /// Two LLM calls rather than one asking for both parts at once: a local model does
    /// noticeably better on one focused instruction than on a two-section structured document,
    /// and it keeps the short outcome summary — the part echoed to the terminal and used as the
    /// archive header — from being held hostage to the much longer review's timeout.
    ///
    /// If either call fails (Ollama unreachable, timeout, empty response), the run is still
    /// archived, just with a placeholder note in place of that piece — a diagnostic nicety
    /// failing must never mask or replace the original outcome.
    ///
    /// Writes a placeholder archive *before* either LLM call, then overwrites it with the
    /// enriched version once they finish. A run that already succeeded (a real commit landed)
    /// must not lose its record to a slow or interrupted summary/review call — live-verified
    /// 2026-08-05: a landed gate commit had no archive at all because nothing was written to
    /// disk until after both calls returned.
    async fn finalize_trace(
        &self,
        ctx: &Context,
        tx: &mpsc::Sender<OrchestratorResult>,
        success: bool,
    ) {
        let placeholder = ctx.summary_body(
            "(pending — summary generation was interrupted or is still in progress)",
            "(pending — step review was interrupted or is still in progress)",
            success,
        );
        if success {
            ctx.finalize_success_trace(&placeholder).await;
        } else {
            ctx.finalize_failure_trace(&placeholder).await;
        }
        let model = self
            .orchestrator_config
            .get("failure_analysis_model")
            .and_then(|v| v.as_str())
            .or_else(|| self.worker.get_str("model").ok())
            .unwrap_or("qwen2.5-coder:14b")
            .to_string();
        let body = ctx.trace_body();
        let result = if success {
            run_summary::generate_success_summary(self.chat.as_ref(), &model, &ctx.goal, &body)
                .await
        } else {
            run_summary::generate_failure_summary(self.chat.as_ref(), &model, &ctx.goal, &body)
                .await
        };
        let summary = result.unwrap_or_else(|e| {
            tracing::warn!(error = ?e, success, "run summary generation failed");
            format!("(automatic summary generation failed: {e})")
        });
        let review =
            run_summary::generate_step_review(self.chat.as_ref(), &model, &ctx.goal, &body)
                .await
                .unwrap_or_else(|e| {
                    tracing::warn!(error = ?e, success, "step review generation failed");
                    format!("(automatic step review generation failed: {e})")
                });
        let body = ctx.summary_body(&summary, &review, success);
        if success {
            ctx.finalize_success_trace(&body).await;
        } else {
            ctx.finalize_failure_trace(&body).await;
        }
        let prefix = if success {
            "Run succeeded"
        } else {
            "Run did not succeed"
        };
        // Only the outcome summary goes to the terminal — the step review is a page of
        // per-round lines, which belongs in a file to read afterwards, not scrolling past at
        // the end of a run.
        let _ = tx
            .send(Ok(StreamItem::Event(AgentEvent::Trace(format!(
                "{prefix}: {summary}\nStep-by-step review written to {}",
                ctx.archive_path(success).display()
            )))))
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{base_config, build_test_orchestrator};
    use super::{Context, is_approval_yes, progress_pct};
    use crate::agent::event::{AgentEvent, StreamItem};
    use serde_json::json;
    use tokio::sync::mpsc;

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
        assert!(summary.contains("ruchat_traces/"));
    }

    #[test]
    fn progress_pct_is_zero_before_the_first_round() {
        assert_eq!(progress_pct(0, 3), 0.0);
    }

    #[test]
    fn progress_pct_scales_linearly_with_round_over_max_iterations() {
        // f32 division order affects the last bit or two, so compare with a small epsilon
        // rather than exact equality (100.0/3.0 computed independently here vs. inside
        // progress_pct doesn't round identically).
        assert!((progress_pct(1, 3) - 100.0 / 3.0).abs() < 0.001);
        assert!((progress_pct(2, 3) - 200.0 / 3.0).abs() < 0.001);
        assert_eq!(progress_pct(3, 3), 100.0);
    }

    #[test]
    fn progress_pct_clamps_at_100_when_round_exceeds_max_iterations() {
        // `Stage::Plan`'s escalate branch increments `ctx.round` past `max_iterations`
        // before checking the budget — progress must still read as a sane percentage,
        // not something like 133%.
        assert_eq!(progress_pct(4, 3), 100.0);
    }

    #[test]
    fn progress_pct_does_not_divide_by_zero_when_max_iterations_is_zero() {
        assert_eq!(progress_pct(0, 0), 100.0);
        assert_eq!(progress_pct(5, 0), 100.0);
    }

    // Regression: the old `.ruchat_trace.md` gave no indication of *why* an unsuccessful run
    // (escalated, or the iteration budget exhausted without ever reaching `Stage::Commit`)
    // failed — a maintainer had to read every round of a possibly long trace to reconstruct
    // that themselves. `finalize_trace` (called from `run_stage_machine` at the very end, with
    // the `success` flag — set only by `Stage::Commit` succeeding) makes one direct LLM call
    // over the trace and reports the result as a `Trace` event; the actual archival (real file
    // I/O under `ruchat_traces/failures/`, via `Context::finalize_failure_trace`) is covered
    // separately by that method's own doc comment / by inspection, same as the old
    // `prepend_failure_summary` was.
    #[tokio::test]
    async fn finalize_trace_sends_a_failure_trace_event_with_the_analysis() {
        // Two scripted responses, in call order: the outcome summary, then the step review.
        let orchestrator = build_test_orchestrator(
            base_config(),
            vec![
                "Worker kept repeating itself and never produced a valid patch",
                "round 1 | Worker | resubmitted the rejected diff | BAD: same rejection reason",
            ],
            None,
        )
        .await;
        let ctx = Context::new("fix the bug".to_string());
        let (tx, mut rx) = mpsc::channel(100);

        orchestrator.finalize_trace(&ctx, &tx, false).await;

        let event = rx.recv().await.expect("expected a trace event");
        match event.expect("expected Ok") {
            StreamItem::Event(AgentEvent::Trace(msg)) => {
                assert!(msg.starts_with("Run did not succeed:"));
                assert!(
                    msg.contains("Worker kept repeating itself and never produced a valid patch")
                );
                // The review is a page of per-round lines; it belongs in the summary file, not
                // scrolling past in the terminal — only its path is reported here.
                assert!(!msg.contains("BAD: same rejection reason"));
                assert!(msg.contains("ruchat_traces/failures/ruchat_trace_0.md"));
            }
            other => panic!("expected a Trace event, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn finalize_trace_sends_a_success_trace_event_with_the_analysis() {
        let orchestrator = build_test_orchestrator(
            base_config(),
            vec![
                "Renamed the helper and updated every call site.",
                "round 1 | Worker | read the file before editing | GOOD: grounded the diff",
            ],
            None,
        )
        .await;
        let ctx = Context::new("rename a function".to_string());
        let (tx, mut rx) = mpsc::channel(100);

        orchestrator.finalize_trace(&ctx, &tx, true).await;

        let event = rx.recv().await.expect("expected a trace event");
        match event.expect("expected Ok") {
            StreamItem::Event(AgentEvent::Trace(msg)) => {
                assert!(msg.starts_with("Run succeeded:"));
                assert!(msg.contains("Renamed the helper and updated every call site."));
                assert!(msg.contains("ruchat_traces/successes/ruchat_trace_0.md"));
            }
            other => panic!("expected a Trace event, got {other:?}"),
        }
    }

    // A step review failing must not cost the run its outcome summary — the review is the
    // newer, slower, more failure-prone of the two calls (much longer output, much longer
    // timeout), and it sits between the outcome summary and the archival step.
    #[tokio::test]
    async fn finalize_trace_still_reports_the_outcome_when_the_step_review_fails() {
        // One scripted response only: the outcome summary. The review call gets an empty
        // response, which `generate_step_review` treats as an error.
        let orchestrator = build_test_orchestrator(
            base_config(),
            vec!["Worker never produced a valid patch", "   "],
            None,
        )
        .await;
        let ctx = Context::new("fix the bug".to_string());
        let (tx, mut rx) = mpsc::channel(100);

        orchestrator.finalize_trace(&ctx, &tx, false).await;

        let event = rx.recv().await.expect("expected a trace event");
        match event.expect("expected Ok") {
            StreamItem::Event(AgentEvent::Trace(msg)) => {
                assert!(msg.contains("Worker never produced a valid patch"));
            }
            other => panic!("expected a Trace event, got {other:?}"),
        }
    }

    // Regression canary for the HITL approval gate (maintainer: "keep on working on roadmap
    // entries, overnight"). `is_approval_yes` is the one piece of this feature with real
    // branching logic that doesn't need a live terminal — the actual pause (plan/diff shown via
    // `ctx.trace`, then a blocking stdin read via the same `Io` type breakpoints already use)
    // was instead verified live: `--approve` against a real agentic run, confirming the plan
    // and a real `git diff` render before the prompt, that typing 'y' proceeds to a real commit,
    // and that any other answer stops the run via `Stage::Escalate` without committing.
    #[test]
    fn is_approval_yes_accepts_only_an_exact_y_or_yes() {
        for accepted in ["y", "Y", "yes", "Yes", " y ", "y\n"] {
            assert!(
                is_approval_yes(accepted),
                "expected {accepted:?} to be approval"
            );
        }
    }

    #[test]
    fn is_approval_yes_rejects_everything_else_including_blank() {
        for rejected in ["n", "N", "no", "", "  ", "YES", "sure", "yep"] {
            assert!(
                !is_approval_yes(rejected),
                "expected {rejected:?} to be rejection"
            );
        }
    }
}
