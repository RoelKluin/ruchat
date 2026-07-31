use crate::{Result, RuChatError};
use ollama_rs::generation::completion::GenerationResponse;
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

pub(crate) struct Context {
    pub(crate) goal: String,
    pub(crate) turns: Vec<Turn>,
    pub(crate) output: String, // last agent's raw output — transient scratch, unchanged
    pub(crate) context_config: Value,
    pub(crate) round: u64, // current round number, incremented after each agent's turn
}

impl Context {
    pub(crate) fn new(goal: String) -> Self {
        Self {
            goal,
            turns: Vec::new(),
            output: String::new(),
            context_config: Value::Null,
            round: 0,
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
    pub(crate) fn read_config_file(&mut self, path: &str) -> Result<()> {
        let config_str = std::fs::read_to_string(path)?;
        self.context_config = serde_json::from_str(&config_str)?;
        Ok(())
    }
    pub(crate) fn is_approved(&self) -> bool {
        self.turns.iter().all(|t| t.kind != TurnKind::Rejection)
    }
    pub(crate) async fn trace(
        &mut self,
        tx: &mpsc::Sender<Result<Vec<GenerationResponse>>>,
        err: String,
    ) {
        if !err.is_empty() {
            self.push_turn(TurnKind::Rejection, "Validator", err.clone());
            tx.send(Err(RuChatError::Trace(err))).await.ok();
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
        tx: &mpsc::Sender<Result<Vec<GenerationResponse>>>,
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
