use crate::agent::{AgentEvent, StreamItem};
use crate::Result;
use serde_json::Value;
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub(crate) struct Issue {
    pub(crate) source: String,
    pub(crate) text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TurnKind {
    Plan,           // Architect output
    Implementation, // Worker output
    Retrieval,      // Librarian / on-demand Retrieve tool output
    Rejection,      // Validator / Tester / Critic feedback
    Summary,        // Summarizer output, replaces collapsed turns
    System,         // system-level confirmations (e.g. MEMORIZE ack) - visible in history_view
}

#[derive(Debug, Clone)]
pub(crate) struct Turn {
    pub(crate) round: u64,
    pub(crate) kind: TurnKind,
    pub(crate) source: String,
    pub(crate) content: String,
}

/// Pre-image of a file `apply_patch` just wrote to disk, kept so a rejected
/// round can be rolled back to a clean baseline before the next Worker
/// attempt instead of compounding an unreviewed mutation. Cleared once the
/// round is either reverted (`Stage::Retry` looping back to `Plan`) or the
/// patch survives to `Stage::Accept`/`Stage::Done`.
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
    pub(crate) pending_patch: Option<PendingPatch>,
}

impl Context {
    pub(crate) fn new(goal: String) -> Self {
        Self {
            goal,
            turns: Vec::new(),
            output: String::new(),
            context_config: Value::Null,
            round: 0,
            pending_patch: None,
        }
    }

    pub(crate) fn push_turn(&mut self, kind: TurnKind, source: &str, content: String) {
        self.turns.push(Turn {
            round: self.round,
            kind,
            source: source.to_string(),
            content,
        });
    }

    /// Records the pre-patch content of a file `apply_patch` is about to
    /// overwrite. Only one round's worth of pending patch is ever tracked —
    /// a new record replaces any prior one, since a round only ever applies
    /// one patch (see `Stage::Implement`).
    pub(crate) fn record_patch(&mut self, path: String, original: String) {
        self.pending_patch = Some(PendingPatch { path, original });
    }

    /// Writes the tracked file back to its pre-patch content and clears the
    /// pending record. Called when a round's patch is rejected and the
    /// orchestrator is about to loop back to `Stage::Plan` — restores a
    /// clean baseline for the next attempt instead of stacking an unreviewed
    /// mutation. No-ops if no patch is pending (e.g. `apply_patch` was never
    /// reached, or failed before writing).
    pub(crate) async fn revert_pending_patch(
        &mut self,
        tx: &mpsc::Sender<Result<StreamItem>>,
    ) {
        if let Some(pending) = self.pending_patch.take() {
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
    pub(crate) fn is_approved(&self) -> bool {
        self.turns.iter().all(|t| t.kind != TurnKind::Rejection)
    }
    pub(crate) async fn trace(&mut self, tx: &mpsc::Sender<Result<StreamItem>>, msg: String) {
        if !msg.is_empty() {
            let _ = tx.send(Ok(StreamItem::Event(AgentEvent::Trace(msg)))).await;
        }
        let trace_output = format!(
            "# Orchestration Trace\n\n## Goal\n{}\n\n## Context\n{}\n\n## History\n{}\n",
            self.goal,
            self.context_view(),
            self.history_view(u64::MAX)
        );
        let _ = tokio::fs::write(".ruchat_trace.md", trace_output).await;
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
            });
        }
        if let Some(c) = imputations.get("context").and_then(|v| v.as_str()) {
            self.turns.push(Turn {
                round: 0,
                kind: TurnKind::Plan,
                source: "DebugImputation".to_string(),
                content: c.to_string(),
            });
        }
        if let Some(h) = imputations.get("history").and_then(|v| v.as_str()) {
            self.turns.push(Turn {
                round: 0,
                kind: TurnKind::Summary,
                source: "DebugImputation".to_string(),
                content: h.to_string(),
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
            self.goal, context, self.history_view(u64::MAX), self.documents_view(u64::MAX));
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

    /// Replaces the old `ctx.context` string: latest Plan + latest Implementation only.
    pub(crate) fn context_view(&self) -> String {
        let plan = self.turns.iter().rev().find(|t| t.kind == TurnKind::Plan);
        let implementation = self
            .turns
            .iter()
            .rev()
            .find(|t| t.kind == TurnKind::Implementation);
        match (plan, implementation) {
            (Some(p), Some(i)) => format!("PLAN:\n{}\n\nIMPLEMENTATION:\n{}", p.content, i.content),
            (Some(p), None) => format!("PLAN:\n{}", p.content),
            (None, Some(i)) => format!("IMPLEMENTATION:\n{}", i.content),
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
    pub(crate) fn collapse_to_summary(&mut self, summary_text: String) {
        self.turns.retain(|t| t.round > self.round);
        self.turns.insert(
            0,
            Turn {
                round: self.round,
                kind: TurnKind::Summary,
                source: "Summarizer".to_string(),
                content: summary_text,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planned_files_empty_when_no_plan_turn() {
        let ctx = Context::new("goal".to_string());
        assert!(ctx.planned_files().is_empty());
    }

    #[test]
    fn planned_files_empty_when_plan_has_no_files_line() {
        let mut ctx = Context::new("goal".to_string());
        ctx.push_turn(TurnKind::Plan, "Architect", "just think about it".to_string());
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
}
