use super::{Orchestrator, OrchestratorResult, Stage, scope};
use crate::agent::tools;
use crate::agent::types::{Context, TurnKind};
use crate::retry_transient;
use crate::{Result, RuChatError};
use serde_json::Value;
use tokio::sync::mpsc;

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

impl Orchestrator {
    pub(super) async fn run_scope_stage(
        &mut self,
        ctx: &mut Context,
        tx: &mpsc::Sender<OrchestratorResult>,
    ) -> Result<Stage> {
        let scoper = self
            .scoper
            .as_mut()
            .ok_or_else(|| RuChatError::Is("Scoper not enabled".into()))?;

        let scoper_start = std::time::Instant::now();
        retry_transient!(scoper.query_stream(&self.chat, ctx, tx))?;
        // The Scoper's own raw output used to only ever reach `ctx.turns` in fragments — the
        // `notes` field below if non-empty, a rejected-lookup reason, a failed-lookup message —
        // never the actual action it took this round. A round where the Scoper found nothing
        // notable to say (empty notes, goal already READY) left no trace of it having run at
        // all, even though its output was streamed live to the console. Record it unconditionally.
        ctx.push_turn_timed(TurnKind::System, "Scoper", ctx.output.clone(), scoper_start);

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
                    if let Err(e) = self.handle_structured_tool(&call, ctx, tx).await {
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

#[cfg(test)]
mod tests {
    use super::super::test_support::{base_config, build_test_orchestrator};
    use super::Stage;
    use crate::agent::types::{Context, TurnKind};
    use serde_json::json;
    use tokio::sync::mpsc;

    // Regression: same class of bug as the critic-approval fix above — a Scoper round that
    // found nothing notable to say (goal already READY, empty notes) used to leave `ctx.turns`
    // completely untouched, even though the Scoper's own output was streamed live to the
    // console. `run_scope_stage` now records the raw output unconditionally, before any of the
    // selective notes/lookup-rejection turns that only fire in specific cases.
    #[tokio::test]
    async fn run_scope_stage_records_its_raw_output_even_with_empty_notes() {
        let mut config = base_config();
        config["Scoper"] = json!({ "model": "fake" });
        let mut orchestrator =
            build_test_orchestrator(config, vec![r#"{"verdict": "READY", "notes": ""}"#], None)
                .await;
        let mut ctx = Context::new("goal".to_string());
        let (tx, _rx) = mpsc::channel(100);

        let stage = orchestrator.run_scope_stage(&mut ctx, &tx).await.unwrap();

        assert_eq!(stage, Stage::Plan);
        let recorded = ctx.turns.iter().find(|t| {
            t.kind == TurnKind::System && t.source == "Scoper" && t.content.contains("READY")
        });
        assert!(
            recorded.is_some(),
            "expected the Scoper's raw output recorded even with empty notes, got: {:?}",
            ctx.turns
        );
    }
}
