use super::stall_mitigation::round_has_actionable_diagnostics;
use super::{Orchestrator, OrchestratorResult, Stage};
use crate::Result;
use crate::agent::protocol::Validation;
use crate::agent::tools::{self, ToolName};
use crate::agent::types::{Context, TurnKind};
use crate::retry_transient;
use tokio::sync::mpsc;

/// Rejection reason for a Worker turn with no recognized tool call anywhere in it — see
/// `Orchestrator::run_implement_patch_loop`.
const NO_TOOL_CALL_REJECTION: &str = "refused: no recognized tool_call found anywhere in your \
    output — every round you must emit exactly one tool_call (e.g. apply_patch, memorize). A \
    narrative walkthrough, an explanation of what you would do, or any other prose without an \
    actual tool_call accomplishes nothing on its own.";

/// Whether `Stage::Implement`'s multi-file patch loop should re-ask the Worker for another
/// `apply_patch` call after a successful one, instead of finalizing the round. Deliberately
/// count-based rather than matching planned paths against patched ones exactly (that would
/// duplicate `protocol.rs::file_in_scope`'s suffix-matching for little practical benefit here):
/// a plan with no `FILES:` line (or exactly one file) always returns `false` once one patch has
/// landed, so a single-file round behaves identically to before this loop existed.
fn should_continue_patch_loop(ctx: &Context, remaining_patch_budget: u32) -> bool {
    remaining_patch_budget > 0 && ctx.planned_files().len() > ctx.pending_patches.len()
}

impl Orchestrator {
    /// unlike `retrieve_budget`, which is a conservative cap for the whole run) so a plan naming
    /// multiple files in its `FILES:` line can land as one commit instead of only ever touching
    /// the first of them (see `should_continue_patch_loop`). Ends the same way a single-patch
    /// round always did on `Failure` (reject/retry) or a successful `Memorize`/no-op reask after
    /// at least one patch already landed (proceed to Test) — the one new case is a Worker turn
    /// that produced no recognized tool_call *at all* on its first attempt this round (e.g. a
    /// narrative walkthrough instead of an actual tool call): rejected immediately with a
    /// precise, deterministic reason rather than silently proceeding to a Test cycle that will
    /// trivially pass (nothing changed) and hoping the Validator LLM happens to notice.
    pub(super) async fn run_implement_patch_loop(
        &mut self,
        ctx: &mut Context,
        tx: &mpsc::Sender<OrchestratorResult>,
    ) -> Result<Stage> {
        let mut patch_budget: u32 = 3;
        let mut any_patch_applied = false;
        loop {
            let parsed_tool = tools::parse_tool_call(&ctx.output).ok().map(|c| c.tool);
            let is_apply_patch = matches!(parsed_tool, Some(ToolName::ApplyPatch));
            // Times the actual tool execution (apply_patch's parse+apply, or memorize's embed
            // call) — separate from the Worker's own LLM call time above, so --trace-timings can
            // tell the two costs apart.
            let exec_start = std::time::Instant::now();
            match self.worker.execute_and_verify(ctx).await? {
                Validation::Failure(err) => {
                    ctx.push_turn_timed(TurnKind::Rejection, "ApplyPatch", err, exec_start);
                    return Ok(Stage::Retry);
                }
                // A real failure mode, not hypothetical: a live run showed the Worker calling
                // `cargo_clippy`, seeing genuine warnings, then `memorize`-ing a note about them
                // instead of ever calling `apply_patch` — and the (non-deterministic) Validator
                // approved that exact substitution on one round after correctly rejecting the
                // identical thing the round before (see TODO.md's pinned reliability item).
                // Caught deterministically here, before it ever reaches the Validator, rather
                // than trusting an LLM verdict to catch it reliably every time.
                Validation::Success
                    if !any_patch_applied
                        && matches!(parsed_tool, Some(ToolName::Memorize))
                        && round_has_actionable_diagnostics(ctx) =>
                {
                    let content = "refused: you memorized information instead of applying a \
                        fix. This round's cargo_clippy/cargo_check output above shows real, \
                        actionable warnings or errors — memorize does not change any code, so \
                        the reported issue is still unresolved. Call apply_patch to actually \
                        fix it now."
                        .to_string();
                    ctx.push_turn_timed(TurnKind::Rejection, "Worker", content, exec_start);
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
                    let worker_reask_start = std::time::Instant::now();
                    retry_transient!(self.worker.query_stream(&self.chat, ctx, tx))?;
                    ctx.push_turn_timed(
                        TurnKind::Implementation,
                        "Worker",
                        ctx.output.clone(),
                        worker_reask_start,
                    );
                }
                // `Validation::Skip` means `parse_tool_call` found nothing it recognized
                // anywhere in the Worker's output — no tool_call fence, no bare-diff fallback
                // either. On a *follow-up* reask within this same round (after at least one
                // apply_patch already succeeded) that's the Worker's normal way of signaling
                // "I'm done" — fine, proceed to Test. On the *first* attempt this round it means
                // the Worker did nothing actionable at all.
                Validation::Skip if !any_patch_applied => {
                    ctx.push_turn_timed(
                        TurnKind::Rejection,
                        "Worker",
                        NO_TOOL_CALL_REJECTION.to_string(),
                        exec_start,
                    );
                    return Ok(Stage::Retry);
                }
                _ => return Ok(Stage::Test),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{base_config, build_test_orchestrator};
    use super::{Stage, should_continue_patch_loop};
    use crate::agent::types::{Context, TurnKind};
    use tokio::sync::mpsc;

    #[test]
    fn patch_loop_does_not_continue_when_the_plan_named_no_files() {
        // No FILES: line at all (or a plan the parser found none in) — the common/legacy case.
        // Must behave exactly like the single-patch-per-round flow this loop replaced.
        let mut ctx = Context::new("goal".to_string());
        ctx.push_turn(
            TurnKind::Plan,
            "Architect",
            "just do it, no files line".to_string(),
        );
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
    async fn implement_patch_loop_rejects_a_second_read_only_tool_call_with_actionable_guidance() {
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
        assert!(
            rejection
                .content
                .contains("already used this round's one information-lookup")
        );
        assert!(rejection.content.contains("apply_patch"));
    }
}
