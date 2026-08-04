use super::Stage;
use crate::agent::types::{Context, PendingPatch, Turn};
use crate::{Result, RuChatError};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Where the current run's checkpoint lives — one fixed path, not scoped per-goal/per-run-id:
/// ruchat's CLI usage model is one invocation running to completion (or crashing) at a time, so
/// there's never more than one resumable run to track. Matches `ruchat_manager.json`'s naming
/// convention (no leading dot — the older, deprecated `.ruchat_trace.md` used one).
pub(super) const CHECKPOINT_PATH: &str = "ruchat_checkpoint.json";

/// Everything `run_stage_machine` needs to pick a run back up after a crash — deliberately a
/// small, explicit subset of `Context`, not the whole struct: `output` (transient scratch,
/// overwritten by the next role call regardless) and `context_config` (re-derived every run,
/// fresh or resumed, by the existing `db_config.json` read at the top of `run_stage_machine`)
/// don't need to survive a round trip. See ROADMAP.md Phase 3 "Resumable/crash-resilient runs"
/// for the scoping this implements: "persist Context (turns, round, pending patch) to a local
/// file after each stage transition, and add a `--resume` flag... instead of a
/// Temporal/LangGraph-style durable-execution engine" — this is exactly that and nothing more:
/// one plain JSON file, no distributed coordination, no partial-stage recovery (a stage that was
/// only half-run when the process died is simply re-run in full from its start on resume, same
/// as any other stage transition already works).
#[derive(Debug, Serialize, Deserialize)]
pub(super) struct Checkpoint {
    goal: String,
    turns: Vec<Turn>,
    round: u64,
    pending_patches: Vec<PendingPatch>,
    trace_index: u64,
    stage: Stage,
}

impl Checkpoint {
    /// Captures the state right after a stage transition — `stage` is the value the stage
    /// machine's `match` block just computed as the *next* stage, i.e. "the last completed
    /// transition," matching the roadmap wording above exactly.
    fn capture(ctx: &Context, stage: &Stage) -> Self {
        Self {
            goal: ctx.goal.clone(),
            turns: ctx.turns.clone(),
            round: ctx.round,
            pending_patches: ctx
                .pending_patches
                .iter()
                .map(|p| PendingPatch {
                    path: p.path.clone(),
                    original: p.original.clone(),
                })
                .collect(),
            trace_index: ctx.trace_index,
            stage: stage.clone(),
        }
    }

    /// Writes (overwrites) the checkpoint file at `path` after a stage transition. Best-effort:
    /// a failure to persist a checkpoint must never abort an otherwise-healthy run over a
    /// diagnostic nicety — logged, not propagated, same posture as this codebase's other
    /// non-critical side-effect writes (e.g. `Context::trace`'s live trace-file refresh).
    /// `path` is a parameter (not always the bare `CHECKPOINT_PATH` constant internally) so
    /// tests can point this at a tempdir file instead of mutating the process's real, global
    /// current directory — `std::env::set_current_dir` races with any other test running
    /// concurrently in a different thread, which is exactly the kind of flakiness this avoids.
    pub(super) async fn save(ctx: &Context, stage: &Stage, path: &Path) {
        let checkpoint = Self::capture(ctx, stage);
        let json = match serde_json::to_string_pretty(&checkpoint) {
            Ok(j) => j,
            Err(e) => {
                tracing::warn!(error = %e, "failed to serialize run checkpoint");
                return;
            }
        };
        if let Err(e) = tokio::fs::write(path, json).await {
            tracing::warn!(error = %e, "failed to write run checkpoint");
        }
    }

    /// Removes the checkpoint file at `path` once a run reaches a stage the machine itself
    /// decided to stop at (`Stage::Done` or `Stage::Escalate`) — a deliberate, recorded outcome,
    /// not a crash. `--resume` is for surviving an unexpected interruption; a run that finished
    /// on its own (successfully or not) has nothing left to resume. Best-effort, same reasoning
    /// as `save`: a stale checkpoint left behind by a failed delete is a papercut (the next
    /// `--resume` picks it back up, redoing the tail of an already-finished run), not a
    /// correctness problem worth failing a run over.
    pub(super) async fn clear(path: &Path) {
        if let Err(e) = tokio::fs::remove_file(path).await
            && e.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!(error = %e, "failed to remove run checkpoint");
        }
    }

    /// Loads the checkpoint file at `path` for `--resume`. A missing or unparseable file is a
    /// clear user error (nothing to resume, or a corrupt/hand-edited checkpoint) — surfaced, not
    /// silently treated as "start fresh," since silently discarding what looked like resumable
    /// state would be a worse surprise than a clear error telling the user what's wrong.
    pub(super) async fn load(path: &Path) -> Result<Self> {
        let raw = tokio::fs::read_to_string(path).await.map_err(|e| {
            RuChatError::Is(format!(
                "--resume given but no checkpoint found at {}: {e}",
                path.display()
            ))
        })?;
        serde_json::from_str(&raw).map_err(|e| {
            RuChatError::Is(format!(
                "--resume given but {} is not a valid checkpoint: {e}",
                path.display()
            ))
        })
    }

    /// Reconstructs the `(Context, Stage)` pair `run_stage_machine` resumes the loop with.
    /// `trace_index` is preserved (not re-allocated via `Context::init_trace_index`) so a
    /// resumed run keeps writing to the same trace file it was using before the interruption,
    /// rather than starting a new one that loses the pre-crash history.
    pub(super) fn into_context_and_stage(self) -> (Context, Stage) {
        let mut ctx = Context::new(self.goal);
        ctx.turns = self.turns;
        ctx.round = self.round;
        ctx.pending_patches = self.pending_patches;
        ctx.trace_index = self.trace_index;
        (ctx, self.stage)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::types::TurnKind;

    fn sample_context() -> Context {
        let mut ctx = Context::new("fix the flaky test".to_string());
        ctx.round = 3;
        ctx.trace_index = 7;
        ctx.push_turn(
            TurnKind::Plan,
            "Architect",
            "Plan: do the thing.".to_string(),
        );
        ctx.pending_patches.push(PendingPatch {
            path: "src/lib.rs".to_string(),
            original: "original content".to_string(),
        });
        ctx
    }

    #[tokio::test]
    async fn save_then_load_round_trips_every_persisted_field() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CHECKPOINT_PATH);

        let ctx = sample_context();
        Checkpoint::save(&ctx, &Stage::Plan, &path).await;
        let loaded = Checkpoint::load(&path).await.unwrap();
        let (restored, stage) = loaded.into_context_and_stage();

        assert_eq!(restored.goal, "fix the flaky test");
        assert_eq!(restored.round, 3);
        assert_eq!(restored.trace_index, 7);
        assert_eq!(restored.turns.len(), 1);
        assert_eq!(restored.turns[0].content, "Plan: do the thing.");
        assert_eq!(restored.pending_patches.len(), 1);
        assert_eq!(restored.pending_patches[0].path, "src/lib.rs");
        assert_eq!(stage, Stage::Plan);
    }

    // Regression: `Turn` gained a `duration_ms` field for `--trace-timings` after this
    // checkpoint format already shipped — a checkpoint written by an older binary (no such
    // field in its JSON at all) must still load cleanly on `--resume`, not fail with a missing-
    // field deserialize error. `#[serde(default)]` on `Turn::duration_ms` is what makes this
    // work; this test locks in that a checkpoint file shaped like the pre-field format actually
    // still parses, not just that the derive macro compiles.
    #[tokio::test]
    async fn load_accepts_a_checkpoint_written_before_duration_ms_existed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CHECKPOINT_PATH);
        let pre_field_json = r#"{
            "goal": "fix the flaky test",
            "turns": [
                {"round": 1, "kind": "Plan", "source": "Architect", "content": "Plan: do it."}
            ],
            "round": 1,
            "pending_patches": [],
            "trace_index": 1,
            "stage": "Plan"
        }"#;
        tokio::fs::write(&path, pre_field_json).await.unwrap();

        let loaded = Checkpoint::load(&path)
            .await
            .expect("a checkpoint missing duration_ms entirely should still parse");
        let (restored, _stage) = loaded.into_context_and_stage();

        assert_eq!(restored.turns[0].duration_ms, None);
    }

    #[tokio::test]
    async fn load_without_a_checkpoint_file_errors_clearly() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CHECKPOINT_PATH);

        let result = Checkpoint::load(&path).await;

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("no checkpoint found")
        );
    }

    #[tokio::test]
    async fn load_rejects_a_corrupt_checkpoint_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CHECKPOINT_PATH);
        std::fs::write(&path, "not valid json at all").unwrap();

        let result = Checkpoint::load(&path).await;

        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("not a valid checkpoint")
        );
    }

    #[tokio::test]
    async fn clear_removes_an_existing_checkpoint_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CHECKPOINT_PATH);
        let ctx = sample_context();
        Checkpoint::save(&ctx, &Stage::Done, &path).await;
        assert!(path.exists());

        Checkpoint::clear(&path).await;

        assert!(
            !path.exists(),
            "checkpoint file should be gone after clear()"
        );
    }

    #[tokio::test]
    async fn clear_without_an_existing_checkpoint_file_does_not_panic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CHECKPOINT_PATH);

        Checkpoint::clear(&path).await; // must not panic even though nothing was ever saved
    }
}
