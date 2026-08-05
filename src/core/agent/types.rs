use crate::Result;
use crate::agent::tools::{self, ToolName};
use crate::agent::{AgentEvent, StreamItem};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;

/// Every run's trace lives only in memory (`Context.turns`) while it's in progress — nothing
/// is written to disk until the run ends. `finalize_success_trace`/`finalize_failure_trace`
/// then write exactly one file, the outcome summary plus a round-by-round review of the
/// agents' decisions (see `run_summary::generate_step_review`), into whichever of these two
/// directories matches the run's outcome. Every file is the same shape — none of them is a
/// raw trace — so recurring failure patterns can be found across runs by grepping the fixed
/// `GOOD:`/`BAD:`/`UNCLEAR:`/`LESSON:` verdict prefixes in either directory.
const TRACE_SUCCESS_DIR: &str = "ruchat_traces/successes";
const TRACE_FAILURE_DIR: &str = "ruchat_traces/failures";

/// Parses `N` out of a `ruchat_trace_<N>.md` filename; `None` for anything else found sitting
/// in one of the trace directories.
fn parse_trace_index(name: &str) -> Option<u64> {
    name.strip_prefix("ruchat_trace_")?
        .strip_suffix(".md")?
        .parse()
        .ok()
}

/// Renders one turn's content for the human-facing trace file. Only `TurnKind::Implementation`
/// turns whose content contains a parseable `apply_patch` tool call get special treatment —
/// everything else (including a non-apply_patch tool call, or content that doesn't parse as a
/// tool call at all) passes through unchanged.
///
/// The fix this exists for: a model's `apply_patch` diff rides inside a JSON string field, so
/// its newlines are the two literal characters `\`+`n`, not real line breaks — printed raw, an
/// entire multi-hunk diff renders as one unreadable line. `tools::parse_tool_call` already does
/// real JSON parsing (a JSON string's `\n` escapes decode to actual `\n` bytes during that
/// parse), so extracting the `diff` field and re-emitting it as its own fenced block is enough
/// to get real line breaks back — no manual unescaping needed beyond parsing the JSON honestly.
fn render_turn_content_for_trace(kind: TurnKind, content: &str) -> String {
    if kind != TurnKind::Implementation {
        return content.to_string();
    }
    let Ok(call) = tools::parse_tool_call(content) else {
        return content.to_string();
    };
    if call.tool != ToolName::ApplyPatch {
        return content.to_string();
    }
    let Some(diff) = call.args.get("diff").and_then(|d| d.as_str()) else {
        return content.to_string();
    };
    format!("[apply_patch]\n```diff\n{diff}\n```")
}

// Serialize/Deserialize (in addition to the derives every other type here already has): needed
// so a whole `Context` can round-trip through `core/orchestrator/checkpoint.rs`'s resumable-run
// checkpoint file — see that module for why (ROADMAP.md Phase 3 "Resumable/crash-resilient
// runs"). Plain data, no invariants a naive round-trip could violate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum TurnKind {
    Plan,           // Architect output
    Implementation, // Worker output
    Retrieval,      // Librarian / on-demand Retrieve tool output
    Rejection,      // Validator / Tester / Critic feedback
    Summary,        // Summarizer output, replaces collapsed turns
    System,         // system-level confirmations (e.g. MEMORIZE ack) - visible in history_view
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Turn {
    pub(crate) round: u64,
    pub(crate) kind: TurnKind,
    pub(crate) source: String,
    pub(crate) content: String,
    /// Wall-clock time the operation that produced this turn actually took — an LLM call
    /// (`push_turn_timed`) or a tool execution, `None` for turns the orchestrator synthesizes
    /// itself (System/Rejection notes, which are instant). `#[serde(default)]` so an
    /// already-checkpointed run (`ruchat_checkpoint.json`, written before this field existed)
    /// still deserializes cleanly on `--resume` instead of erroring on a missing field.
    #[serde(default)]
    pub(crate) duration_ms: Option<u64>,
}

/// Pre-image of a file `apply_patch` just wrote to disk, kept so a rejected
/// round can be rolled back to a clean baseline before the next Worker
/// attempt instead of compounding an unreviewed mutation. Cleared once the
/// round is either reverted (`Stage::Retry` looping back to `Plan`) or the
/// patch survives to `Stage::Accept`/`Stage::Done`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct PendingPatch {
    pub(crate) path: String,
    pub(crate) original: String,
}

pub(crate) struct Context {
    pub(crate) goal: String,
    pub(crate) turns: Vec<Turn>,
    pub(crate) output: String, // last agent's raw output — transient scratch, unchanged
    pub(crate) context_config: Value,
    pub(crate) round: u64, // current round number, incremented after each agent's turn
    /// One entry per distinct file `apply_patch` has touched so far *this round* — a round can
    /// now apply up to `Stage::Implement`'s per-round patch budget of sequential `apply_patch`
    /// calls (see the multi-file loop there), each to a different file. Order doesn't matter;
    /// membership does (`record_patch`/`revert_pending_patches`).
    pub(crate) pending_patches: Vec<PendingPatch>,
    /// This run's archive-file number — set once via `init_trace_index()` right after
    /// construction. Left at 0 (colliding with the first real run's file, harmlessly, since
    /// nothing reads it) for `Context::new` callers — mostly tests — that never archive a run
    /// at all.
    pub(crate) trace_index: u64,
    /// `--trace-timings` — whether `full_history_view()` (the trace-file renderer) should show
    /// each timed turn's `duration_ms` inline. Set once by the orchestrator right after
    /// construction, from the CLI flag; not itself persisted through `Checkpoint` (a `--resume`
    /// re-specifies it fresh each invocation, same as `--team-model`/etc. already must). Default
    /// `false` — durations are still captured either way (`push_turn_timed` is unconditional, the
    /// cost of an `Instant::now()` is negligible), this only gates whether they're *shown*, so
    /// turning the flag on for a `--resume`d run can still see timings recorded before the flag
    /// was ever set. Deliberately does NOT affect `history_view`/`context_view` (the prompt-
    /// facing renderers other agents actually read) — timing data is for a human/the trace file,
    /// not something to inject into another agent's own context.
    pub(crate) trace_timings: bool,
}

impl Context {
    pub(crate) fn new(goal: String) -> Self {
        Self {
            goal,
            turns: Vec::new(),
            output: String::new(),
            context_config: Value::Null,
            round: 0,
            pending_patches: Vec::new(),
            trace_index: 0,
            trace_timings: false,
        }
    }

    /// Picks this run's trace-file slot by scanning `TRACE_SUCCESS_DIR`/`TRACE_FAILURE_DIR` for
    /// existing `ruchat_trace_<N>.md` files and using one past the highest `N` found — so every
    /// run gets its own file instead of every run overwriting the same path. Call once, right
    /// after `Context::new`, before the first `trace()` call.
    pub(crate) async fn init_trace_index(&mut self) {
        // See `trace()`'s doc comment for why test builds skip real `ruchat_traces/` I/O
        // entirely — scanning the directory here would be harmless (read-only) on its own, but
        // there's no point doing it when nothing is ever archived under `cfg!(test)`.
        if cfg!(test) {
            return;
        }
        let mut max_seen = 0u64;
        for dir in [TRACE_SUCCESS_DIR, TRACE_FAILURE_DIR] {
            let Ok(mut entries) = tokio::fs::read_dir(dir).await else {
                continue;
            };
            while let Ok(Some(entry)) = entries.next_entry().await {
                if let Some(n) = entry.file_name().to_str().and_then(parse_trace_index) {
                    max_seen = max_seen.max(n);
                }
            }
        }
        self.trace_index = max_seen + 1;
    }

    fn trace_filename(&self) -> String {
        format!("ruchat_trace_{}.md", self.trace_index)
    }

    pub(crate) fn push_turn(&mut self, kind: TurnKind, source: &str, content: String) {
        self.turns.push(Turn {
            round: self.round,
            kind,
            source: source.to_string(),
            content,
            duration_ms: None,
        });
    }

    /// Same as `push_turn`, but records how long the operation that produced `content` actually
    /// took — `start` is the `Instant` captured right before that operation began (an LLM call
    /// or a tool execution). See `--trace-timings` (`Context::trace_timings`) for where this
    /// becomes visible: captured unconditionally here (cheap), only *shown* when that flag is on.
    pub(crate) fn push_turn_timed(
        &mut self,
        kind: TurnKind,
        source: &str,
        content: String,
        start: std::time::Instant,
    ) {
        self.turns.push(Turn {
            round: self.round,
            kind,
            source: source.to_string(),
            content,
            duration_ms: Some(start.elapsed().as_millis() as u64),
        });
    }

    /// Records the pre-patch content of a file `apply_patch` is about to overwrite, unless this
    /// path was already recorded earlier in the same round — the *first* patch to touch a given
    /// file this round is the one whose original content matters for revert; a second
    /// `apply_patch` call to the same file (allowed within one round's patch budget, see
    /// `Stage::Implement`) reads its "original" fresh off disk, which by then is the
    /// already-patched content, not the true pre-round baseline.
    pub(crate) fn record_patch(&mut self, path: String, original: String) {
        if !self.pending_patches.iter().any(|p| p.path == path) {
            self.pending_patches.push(PendingPatch { path, original });
        }
    }

    /// Writes every tracked file in this round back to its pre-patch content and clears the
    /// pending list. Called when a round's patch(es) are rejected and the orchestrator is about
    /// to loop back to `Stage::Plan` — restores a clean baseline for the next attempt instead of
    /// stacking an unreviewed mutation. No-ops if nothing is pending (e.g. `apply_patch` was
    /// never reached, or failed before writing).
    pub(crate) async fn revert_pending_patches(&mut self, tx: &mpsc::Sender<Result<StreamItem>>) {
        // Collected into an owned Vec first: `self.trace` below needs `&mut self`, which would
        // conflict with an in-progress `drain` iterator still borrowing `self.pending_patches`.
        let patches: Vec<PendingPatch> = self.pending_patches.drain(..).collect();
        for pending in patches {
            match tokio::fs::write(&pending.path, &pending.original).await {
                Ok(()) => {
                    self.trace(
                        tx,
                        format!(
                            "Rejected patch to '{}' rolled back to its pre-patch content.",
                            pending.path
                        ),
                    )
                    .await;
                }
                Err(e) => {
                    self.trace(
                        tx,
                        format!(
                            "WARNING: failed to roll back rejected patch to '{}': {e} — \
                            file may be left in a mutated, unreviewed state",
                            pending.path
                        ),
                    )
                    .await;
                }
            }
        }
    }
    pub(crate) fn read_config_file(&mut self, path: &str) -> Result<()> {
        let config_str = std::fs::read_to_string(path)?;
        self.context_config = serde_json::from_str(&config_str)?;
        Ok(())
    }
    pub(crate) async fn trace(&mut self, tx: &mpsc::Sender<Result<StreamItem>>, msg: String) {
        if !msg.is_empty() {
            let _ = tx.send(Ok(StreamItem::Event(AgentEvent::Trace(msg)))).await;
        }
    }

    /// Renders the full trace body from the in-memory `turns` log: goal, latest
    /// plan/implementation, and every turn in chronological order, including retrieval turns
    /// (see `full_history_view` — this deliberately doesn't build on `history_view`, which
    /// excludes retrievals since those are rendered as a separate, round-scoped `DOCUMENTS`
    /// section in prompts; that meant any round whose only content was a tool call or RAG
    /// lookup — `read_file`, `ripgrep`, `cargo_clippy`, Librarian retrieval, etc. — never
    /// appeared here at all, not even collapsed, even though `print_debug_info` already got
    /// this right for debug-sequence runs). Never written to disk — only fed to the LLM calls
    /// in `finalize_trace` that produce the outcome summary and step review that do get
    /// archived.
    pub(crate) fn trace_body(&self) -> String {
        format!(
            "# Orchestration Trace\n\n## Goal\n{}\n\n## Context\n{}\n\n## History\n{}\n",
            self.goal,
            self.context_view_for_trace(),
            self.full_history_view()
        )
    }

    /// Where this run's analysis file lands, for reporting the path to the maintainer once the
    /// run ends — `TRACE_SUCCESS_DIR` or `TRACE_FAILURE_DIR` depending on outcome.
    pub(crate) fn archive_path(&self, success: bool) -> PathBuf {
        let dir = if success {
            TRACE_SUCCESS_DIR
        } else {
            TRACE_FAILURE_DIR
        };
        Path::new(dir).join(self.trace_filename())
    }

    /// Renders the standalone run-analysis document written by `finalize_success_trace`/
    /// `finalize_failure_trace`: the goal, how the run ended, and the round-by-round review of
    /// the agents' decisions. Deliberately repeats the goal and the outcome rather than pointing
    /// at a trace, since no raw trace is ever kept on disk to point at.
    ///
    /// Pure and separately tested: the writes below are skipped under `cfg!(test)` along with
    /// every other `ruchat_traces/` write, so this is the part that can actually be asserted on.
    pub(crate) fn summary_body(&self, outcome: &str, review: &str, success: bool) -> String {
        let verdict = if success {
            "succeeded"
        } else {
            "did not succeed"
        };
        format!(
            "# Run summary — trace {} ({verdict})\n\n## Goal\n{}\n\n## Outcome\n{outcome}\n\n\
            ## Step review\n{review}\n",
            self.trace_index, self.goal
        )
    }

    /// Archives this run's analysis under `TRACE_FAILURE_DIR`. Called once, after an
    /// unsuccessful run (escalated, or the iteration budget exhausted without ever reaching
    /// `Stage::Commit`).
    pub(crate) async fn finalize_failure_trace(&self, body: &str) {
        // Fixture-driven tests (`run_fixture`/`debug_stage_machine`) exercise the real stage
        // machine against `agent_debug/*.json` sequences, with nothing to distinguish "a real
        // CLI invocation" from "a unit test" at this layer — skip real `ruchat_traces/` I/O
        // under `cfg!(test)` so `cargo test --lib` never writes real archive files.
        if cfg!(test) {
            return;
        }
        let _ = tokio::fs::create_dir_all(TRACE_FAILURE_DIR).await;
        let _ = tokio::fs::write(self.archive_path(false), body).await;
    }

    /// Archives this run's analysis under `TRACE_SUCCESS_DIR`. Called once, after
    /// `Stage::Commit` succeeds.
    pub(crate) async fn finalize_success_trace(&self, body: &str) {
        // See `finalize_failure_trace`'s comment on why tests skip this.
        if cfg!(test) {
            return;
        }
        let _ = tokio::fs::create_dir_all(TRACE_SUCCESS_DIR).await;
        let _ = tokio::fs::write(self.archive_path(true), body).await;
    }
    pub(crate) fn build_collections_summary(&self) -> String {
        let mut summary = String::from("AVAILABLE COLLECTIONS (loaded from config):\n");

        if let Some(collections) = self
            .context_config
            .get("collections")
            .and_then(|v| v.as_array())
        {
            for coll in collections {
                let name = coll["name"].as_str().unwrap_or("unknown");
                let desc = coll["description"].as_str().unwrap_or("");
                let model = coll["embedding_model"].as_str().unwrap_or("unknown");
                let metadata: Vec<String> = coll["metadata_keys"]
                    .as_array()
                    .unwrap_or(&vec![])
                    .iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect();

                let examples =
                    if let Some(exs) = coll.get("example_queries").and_then(|v| v.as_array()) {
                        exs.iter()
                            .map(|e| {
                                let q = e["query"].as_str().unwrap_or("");
                                let w = e.get("where").and_then(|v| v.as_str()).unwrap_or("none");
                                format!("    • query: \"{q}\"  where: \"{w}\"")
                            })
                            .collect::<Vec<_>>()
                            .join("\n")
                    } else {
                        String::from("    (no examples provided)")
                    };

                summary.push_str(&format!(
                    "- **{name}**\n  Description: {desc}\n  Embedding model: {model}\n  Available metadata keys: {}\n  Collection-specific examples:\n{examples}\n\n",
                    metadata.join(", ")
                ));
            }
        } else {
            summary.push_str("(No collections defined in config — falling back to defaults)\n");
        }

        // Global settings
        if let Some(includes) = self
            .context_config
            .get("allowed_include_fields")
            .and_then(|v| v.as_array())
        {
            let inc_list: Vec<&str> = includes.iter().filter_map(|v| v.as_str()).collect();
            summary.push_str(&format!(
                "GLOBAL OPTIONS:\n- Allowed \"include\" fields (any combination): {}\n- Default n_results: {}\n",
                inc_list.join(", "),
                self.context_config.get("default_n_results").and_then(|v| v.as_u64()).unwrap_or(5)
            ));
        }

        summary
    }
    /// Apply debug imputations from a JSON file (only for the **first** agent in a debug sequence).
    /// Called exactly once per debug run.
    pub(crate) fn apply_debug_imputations(&mut self, imputations: &Value) {
        if let Some(d) = imputations.get("documents").and_then(|v| v.as_str()) {
            self.turns.push(Turn {
                round: 0,
                kind: TurnKind::Retrieval,
                source: "DebugImputation".to_string(),
                content: d.to_string(),
                duration_ms: None,
            });
        }
        if let Some(c) = imputations.get("context").and_then(|v| v.as_str()) {
            self.turns.push(Turn {
                round: 0,
                kind: TurnKind::Plan,
                source: "DebugImputation".to_string(),
                content: c.to_string(),
                duration_ms: None,
            });
        }
        if let Some(h) = imputations.get("history").and_then(|v| v.as_str()) {
            self.turns.push(Turn {
                round: 0,
                kind: TurnKind::Summary,
                source: "DebugImputation".to_string(),
                content: h.to_string(),
                duration_ms: None,
            });
        }
    }
    pub(crate) async fn print_debug_info(
        &mut self,
        tx: &mpsc::Sender<Result<StreamItem>>,
        role: &str,
    ) {
        let context = self.context_view();
        let debug_info = format!(
            "DEBUG INFO FOR ROLE: {role}\n\nGOAL:\n{}\n\nCONTEXT:\n{}\n\nHISTORY:\n{}\n\nDOCUMENTS:\n{}",
            self.goal,
            context,
            self.history_view(u64::MAX),
            self.documents_view(u64::MAX)
        );
        self.trace(tx, debug_info).await;
    }

    /// Replaces the old `ctx.history` string: chronological transcript up to `round`,
    /// excluding retrieval payloads (those are rendered separately via `documents_view`).
    pub(crate) fn history_view(&self, upto_round: u64) -> String {
        self.turns
            .iter()
            .filter(|t| t.round <= upto_round && t.kind != TurnKind::Retrieval)
            .map(|t| {
                format!(
                    "### {} [{:?}, round {}]:\n{}\n",
                    t.source, t.kind, t.round, t.content
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Every turn in chronological order, unfiltered — unlike `history_view`, this includes
    /// `TurnKind::Retrieval` turns (tool output, RAG documents, git log/diff, memory recall).
    /// Used only by `trace_body()` for a complete human-readable snapshot of the run; prompt
    /// building keeps using `history_view`/`documents_view` as separate sections, since that
    /// split is meaningful to the model (data vs. narrative), not just a formatting choice.
    ///
    /// When `--trace-timings` (`self.trace_timings`) is on, each timed turn's header gets an
    /// inline `(N.Ns)` — how long the LLM call or tool execution that produced it actually took.
    /// Deliberately only here, not in `history_view`/`context_view`: those feed other agents'
    /// own prompts, and timing data is for a human reading the trace file, not something to hand
    /// another agent as if it were task-relevant context.
    pub(crate) fn full_history_view(&self) -> String {
        self.turns
            .iter()
            .map(|t| {
                let timing = if self.trace_timings {
                    t.duration_ms
                        .map(|ms| format!(" ({:.1}s)", ms as f64 / 1000.0))
                        .unwrap_or_default()
                } else {
                    String::new()
                };
                format!(
                    "### {} [{:?}, round {}]{timing}:\n{}\n",
                    t.source,
                    t.kind,
                    t.round,
                    render_turn_content_for_trace(t.kind, &t.content)
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Replaces the old `ctx.context` string: latest Plan + latest Implementation only.
    pub(crate) fn context_view(&self) -> String {
        Self::render_context(self.latest_plan_and_implementation(), |t| t.content.clone())
    }

    /// Same content as `context_view`, but with each turn passed through
    /// `render_turn_content_for_trace` first — used only by `trace_body()`, so an
    /// `apply_patch` diff sitting in the latest Implementation turn reads as an actual diff
    /// there too, not just in the History section below it. `context_view` itself stays
    /// untouched since it also feeds prompt-facing PLAN/IMPLEMENTATION variables, where the
    /// model needs to see its own prior output exactly as it was, not reformatted.
    fn context_view_for_trace(&self) -> String {
        Self::render_context(self.latest_plan_and_implementation(), |t| {
            render_turn_content_for_trace(t.kind, &t.content)
        })
    }

    fn latest_plan_and_implementation(&self) -> (Option<&Turn>, Option<&Turn>) {
        let plan = self.turns.iter().rev().find(|t| t.kind == TurnKind::Plan);
        let implementation = self
            .turns
            .iter()
            .rev()
            .find(|t| t.kind == TurnKind::Implementation);
        (plan, implementation)
    }

    fn render_context(
        (plan, implementation): (Option<&Turn>, Option<&Turn>),
        format: impl Fn(&Turn) -> String,
    ) -> String {
        match (plan, implementation) {
            (Some(p), Some(i)) => format!("PLAN:\n{}\n\nIMPLEMENTATION:\n{}", format(p), format(i)),
            (Some(p), None) => format!("PLAN:\n{}", format(p)),
            (None, Some(i)) => format!("IMPLEMENTATION:\n{}", format(i)),
            (None, None) => String::new(),
        }
    }

    /// Replaces the old `ctx.documents` string: all retrieval turns for the given round,
    /// most recent first so newly-retrieved (on-demand) context isn't buried.
    pub(crate) fn documents_view(&self, upto_round: u64) -> String {
        self.turns
            .iter()
            .filter(|t| t.round <= upto_round && t.kind == TurnKind::Retrieval)
            .rev()
            .map(|t| t.content.clone())
            .collect::<Vec<_>>()
            .join("\n---\n")
    }

    /// Parses the file paths the latest Architect plan declared with a `FILES:` line (see
    /// `agent_role/architect.md`). Returns an empty vec if no such line is present — that means
    /// "the plan didn't declare scope," not "the plan declared zero files": `apply_patch`
    /// (`agent/protocol.rs`) only enforces the scope check when this is non-empty, so a plan
    /// that forgets the line doesn't retroactively block every patch.
    pub(crate) fn planned_files(&self) -> Vec<String> {
        let Some(plan) = self.turns.iter().rev().find(|t| t.kind == TurnKind::Plan) else {
            return Vec::new();
        };
        plan.content
            .lines()
            .find_map(|line| {
                let trimmed = line.trim();
                if trimmed.len() >= 6 && trimmed[..6].eq_ignore_ascii_case("files:") {
                    Some(trimmed[6..].to_string())
                } else {
                    None
                }
            })
            .map(|rest| {
                rest.split(',')
                    .map(|s| s.trim().trim_start_matches("a/").trim_start_matches("./"))
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Dedup rejection turns for the current round in place; returns true if any remain.
    pub(crate) fn reconcile_rejections(&mut self) -> bool {
        let mut seen = std::collections::HashSet::new();
        self.turns.retain(|t| {
            if t.kind == TurnKind::Rejection && t.round == self.round {
                seen.insert(t.content.trim().to_string())
            } else {
                true
            }
        });
        self.turns
            .iter()
            .any(|t| t.kind == TurnKind::Rejection && t.round == self.round)
    }

    /// Collapses all turns up to `round` into a single Summary turn — this is what
    /// the Summarizer role's output now does instead of overwriting `ctx.history`.
    /// `start` is when the Summarizer's own query began, for `--trace-timings`.
    pub(crate) fn collapse_to_summary(&mut self, summary_text: String, start: std::time::Instant) {
        self.turns.retain(|t| t.round > self.round);
        self.turns.insert(
            0,
            Turn {
                round: self.round,
                kind: TurnKind::Summary,
                source: "Summarizer".to_string(),
                content: summary_text,
                duration_ms: Some(start.elapsed().as_millis() as u64),
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_patch_keeps_the_first_original_for_a_repeated_path() {
        // A round can now apply_patch the same file twice (within its patch budget — see
        // Stage::Implement). The second call's "original" (read fresh off disk by
        // `Validation::apply_patch`) would be the already-patched content, not the true
        // pre-round baseline — `record_patch` must ignore that second, wrong "original".
        let mut ctx = Context::new("goal".to_string());
        ctx.record_patch("src/foo.rs".to_string(), "true original".to_string());
        ctx.record_patch(
            "src/foo.rs".to_string(),
            "intermediate patched content".to_string(),
        );
        assert_eq!(ctx.pending_patches.len(), 1);
        assert_eq!(ctx.pending_patches[0].original, "true original");
    }

    #[tokio::test]
    async fn revert_pending_patches_restores_every_recorded_file_and_clears_the_list() {
        let dir = tempfile::tempdir().unwrap();
        let file_a = dir.path().join("a.txt");
        let file_b = dir.path().join("b.txt");
        std::fs::write(&file_a, "patched a").unwrap();
        std::fs::write(&file_b, "patched b").unwrap();

        let mut ctx = Context::new("goal".to_string());
        ctx.record_patch(
            file_a.to_str().unwrap().to_string(),
            "original a".to_string(),
        );
        ctx.record_patch(
            file_b.to_str().unwrap().to_string(),
            "original b".to_string(),
        );

        let (tx, mut rx) = mpsc::channel(100);
        ctx.revert_pending_patches(&tx).await;
        drop(tx);
        while rx.recv().await.is_some() {}

        assert!(ctx.pending_patches.is_empty());
        assert_eq!(std::fs::read_to_string(&file_a).unwrap(), "original a");
        assert_eq!(std::fs::read_to_string(&file_b).unwrap(), "original b");
    }

    #[test]
    fn planned_files_empty_when_no_plan_turn() {
        let ctx = Context::new("goal".to_string());
        assert!(ctx.planned_files().is_empty());
    }

    #[test]
    fn planned_files_empty_when_plan_has_no_files_line() {
        let mut ctx = Context::new("goal".to_string());
        ctx.push_turn(
            TurnKind::Plan,
            "Architect",
            "just think about it".to_string(),
        );
        assert!(ctx.planned_files().is_empty());
    }

    #[test]
    fn planned_files_parses_comma_separated_list() {
        let mut ctx = Context::new("goal".to_string());
        ctx.push_turn(
            TurnKind::Plan,
            "Architect",
            "Do the thing.\nFILES: src/foo.rs, a/src/bar.rs , ./src/baz.rs\n".to_string(),
        );
        assert_eq!(
            ctx.planned_files(),
            vec!["src/foo.rs", "src/bar.rs", "src/baz.rs"]
        );
    }

    #[test]
    fn planned_files_uses_latest_plan_turn_only() {
        let mut ctx = Context::new("goal".to_string());
        ctx.push_turn(TurnKind::Plan, "Architect", "FILES: old.rs".to_string());
        ctx.round += 1;
        ctx.push_turn(TurnKind::Plan, "Architect", "FILES: new.rs".to_string());
        assert_eq!(ctx.planned_files(), vec!["new.rs"]);
    }

    #[test]
    fn trace_body_includes_retrieval_turns() {
        // Regression for a real bug: `.ruchat_trace.md` was built from `history_view`, which
        // deliberately excludes TurnKind::Retrieval (tool output, RAG docs, git log/diff,
        // cargo_check/clippy, memory recall) since prompts render those as a separate
        // DOCUMENTS section — but that meant any round whose only content was a tool call or
        // RAG lookup never appeared anywhere in the trace file at all.
        let mut ctx = Context::new("goal".to_string());
        ctx.push_turn(TurnKind::Plan, "Architect", "make a plan".to_string());
        ctx.push_turn(
            TurnKind::Retrieval,
            "ReadFile",
            "fn parse_key_val<T, U>(s: &str) -> ...".to_string(),
        );
        let body = ctx.trace_body();
        assert!(body.contains("make a plan"));
        assert!(body.contains("ReadFile"));
        assert!(body.contains("fn parse_key_val<T, U>(s: &str) -> ..."));
    }

    #[test]
    fn push_turn_timed_records_the_elapsed_duration() {
        let mut ctx = Context::new("goal".to_string());
        let start = std::time::Instant::now();
        std::thread::sleep(std::time::Duration::from_millis(5));
        ctx.push_turn_timed(TurnKind::Plan, "Architect", "a plan".to_string(), start);
        assert!(
            ctx.turns[0].duration_ms.unwrap() >= 5,
            "expected at least 5ms recorded, got: {:?}",
            ctx.turns[0].duration_ms
        );
    }

    #[test]
    fn push_turn_records_no_duration() {
        let mut ctx = Context::new("goal".to_string());
        ctx.push_turn(TurnKind::System, "Orchestrator", "a note".to_string());
        assert_eq!(ctx.turns[0].duration_ms, None);
    }

    // --trace-timings: `full_history_view` (the trace-file renderer `trace_body` uses) must show
    // each timed turn's duration inline when the flag is on...
    #[test]
    fn full_history_view_shows_duration_when_trace_timings_is_on() {
        let mut ctx = Context::new("goal".to_string());
        ctx.trace_timings = true;
        let start = std::time::Instant::now() - std::time::Duration::from_millis(2500);
        ctx.push_turn_timed(TurnKind::Plan, "Architect", "a plan".to_string(), start);
        let view = ctx.full_history_view();
        assert!(
            view.contains("(2.5s)"),
            "expected a duration annotation, got: {view}"
        );
    }

    // ...but not when it's off (the default) — durations are still recorded either way (see
    // `push_turn_timed_records_the_elapsed_duration` above), this only gates whether they show.
    #[test]
    fn full_history_view_hides_duration_when_trace_timings_is_off() {
        let mut ctx = Context::new("goal".to_string());
        assert!(!ctx.trace_timings, "trace_timings should default to false");
        let start = std::time::Instant::now() - std::time::Duration::from_millis(2500);
        ctx.push_turn_timed(TurnKind::Plan, "Architect", "a plan".to_string(), start);
        let view = ctx.full_history_view();
        assert!(
            !view.contains("2.5s"),
            "duration should not appear when the flag is off, got: {view}"
        );
    }

    // A turn the orchestrator synthesizes itself (a System reminder, say) has no duration to
    // show even with the flag on — must not print a bogus "(0.0s)" for something that was never
    // actually timed.
    #[test]
    fn full_history_view_omits_timing_for_an_untimed_turn_even_when_the_flag_is_on() {
        let mut ctx = Context::new("goal".to_string());
        ctx.trace_timings = true;
        ctx.push_turn(TurnKind::System, "Orchestrator", "a note".to_string());
        let view = ctx.full_history_view();
        assert!(
            view.contains("### Orchestrator [System, round 0]:"),
            "expected no duration suffix on an untimed turn, got: {view}"
        );
    }

    #[test]
    fn collapse_to_summary_records_the_summarizers_own_duration() {
        let mut ctx = Context::new("goal".to_string());
        let start = std::time::Instant::now() - std::time::Duration::from_millis(1200);
        ctx.collapse_to_summary("condensed history".to_string(), start);
        assert!(ctx.turns[0].duration_ms.unwrap() >= 1200);
    }

    // Regression: a real apply_patch diff rides inside a JSON string field, so its newlines
    // are the two literal characters `\`+`n`, not real line breaks — printed raw in the trace,
    // an entire multi-hunk diff rendered as one unreadable line, making it hard to tell what a
    // patch actually changed from the trace file alone.
    #[test]
    fn trace_body_renders_an_apply_patch_diff_with_real_newlines() {
        let mut ctx = Context::new("goal".to_string());
        let tool_call = r#"```tool_call
{"tool": "apply_patch", "diff": "--- a/src/foo.rs\n+++ b/src/foo.rs\n@@ -1,1 +1,1 @@\n-old\n+new\n"}
```"#;
        ctx.push_turn(TurnKind::Implementation, "Worker", tool_call.to_string());
        let body = ctx.trace_body();
        // The escaped form must be gone — this is the actual bug: `\n` as two literal chars.
        assert!(
            !body.contains(r"\n-old"),
            "diff should not contain literal \\n escapes: {body}"
        );
        // And the real newline-separated diff lines must be present instead.
        assert!(
            body.contains("-old\n+new"),
            "expected real newlines in the diff, got: {body}"
        );
    }

    // A non-apply_patch turn (or content that doesn't parse as a tool call at all, e.g. a
    // narrative rejection reason) must pass through completely unchanged — this rendering only
    // special-cases apply_patch's diff field.
    #[test]
    fn trace_body_leaves_non_apply_patch_content_unchanged() {
        let mut ctx = Context::new("goal".to_string());
        ctx.push_turn(
            TurnKind::Implementation,
            "Worker",
            "```tool_call\n{\"tool\": \"memorize\", \"content\": \"note\"}\n```".to_string(),
        );
        ctx.push_turn(
            TurnKind::Rejection,
            "Validator",
            "plain rejection text".to_string(),
        );
        let body = ctx.trace_body();
        assert!(body.contains(r#"{"tool": "memorize", "content": "note"}"#));
        assert!(body.contains("plain rejection text"));
    }

    #[test]
    fn parse_trace_index_extracts_the_number_from_a_well_formed_filename() {
        // The scan `init_trace_index` runs over TRACE_SUCCESS_DIR/TRACE_FAILURE_DIR relies on
        // this to find the highest existing run number and pick one past it — every run must
        // get its own file instead of every run overwriting the same path (the old
        // `.ruchat_trace.md` behavior).
        assert_eq!(parse_trace_index("ruchat_trace_42.md"), Some(42));
        assert_eq!(parse_trace_index("ruchat_trace_0.md"), Some(0));
    }

    #[test]
    fn parse_trace_index_rejects_anything_that_is_not_that_exact_shape() {
        assert_eq!(parse_trace_index("ruchat_trace.md"), None);
        assert_eq!(parse_trace_index("notes.md"), None);
        assert_eq!(parse_trace_index("ruchat_trace_abc.md"), None);
        assert_eq!(parse_trace_index("ruchat_trace_3.txt"), None);
    }

    // The summary file is meant to be readable — and feedable to another model — on its own,
    // without a trace beside it (none is ever kept on disk), so it has to carry the goal as
    // well as the two analyses. `finalize_success_trace`/`finalize_failure_trace` skip their
    // write under `cfg!(test)`, which is why the composition is a separate, pure function.
    #[test]
    fn summary_body_is_self_contained_and_names_the_outcome() {
        let ctx = Context::new("fix the clippy warning".to_string());
        let body = ctx.summary_body(
            "The Worker never produced an applicable patch.",
            "round 1 | Worker | re-ran cargo_clippy | BAD: result was already in context",
            false,
        );
        assert!(body.contains("did not succeed"));
        assert!(body.contains("fix the clippy warning"));
        assert!(body.contains("The Worker never produced an applicable patch."));
        assert!(body.contains("BAD: result was already in context"));
    }

    #[test]
    fn summary_body_marks_a_successful_run_as_succeeded() {
        let ctx = Context::new("rename a function".to_string());
        let body = ctx.summary_body(
            "Renamed it.",
            "round 1 | Worker | edited | GOOD: fine",
            true,
        );
        assert!(body.contains("succeeded"));
        assert!(!body.contains("did not succeed"));
    }
}
