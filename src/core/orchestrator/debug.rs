use super::{DebugBreakpoints, Orchestrator, OrchestratorResult};
use crate::Result;
use crate::RuChatError;
use crate::agent::json_extract::strip_json_fences;
use crate::agent::types::{Context, TurnKind};
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

impl Orchestrator {
    pub(super) async fn debug_stage_machine(
        &mut self,
        goal: String,
        path: String,
        tx: mpsc::Sender<OrchestratorResult>,
        cancel: CancellationToken,
        mut breakpoints: DebugBreakpoints,
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
        ctx.init_trace_index().await;
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
                            .query_stream(&self.chat, &mut ctx, &tx)
                            .await?;
                        TurnKind::Plan
                    }
                    "Worker" => {
                        self.worker.query_stream(&self.chat, &mut ctx, &tx).await?;
                        TurnKind::Implementation
                    }
                    "Validator" => {
                        self.validator
                            .as_mut()
                            .ok_or(RuChatError::Is("Validator not enabled".into()))?
                            .query_stream(&self.chat, &mut ctx, &tx)
                            .await?;
                        let reason = strip_json_fences(&ctx.output).to_string();
                        ctx.trace(&tx, format!("[REJECTED] {reason}")).await;
                        TurnKind::Rejection
                    }
                    "Summarizer" => {
                        self.summarizer
                            .as_mut()
                            .ok_or(RuChatError::Is("Summarizer not enabled".into()))?
                            .query_stream(&self.chat, &mut ctx, &tx)
                            .await?;
                        TurnKind::Summary
                    }
                    "Scoper" => {
                        self.scoper
                            .as_mut()
                            .ok_or(RuChatError::Is("Scoper not enabled".into()))?
                            .query_stream(&self.chat, &mut ctx, &tx)
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
                            .query_stream(&self.chat, &mut ctx, &tx)
                            .await?;
                        let reason = strip_json_fences(&ctx.output).to_string();
                        ctx.trace(&tx, format!("[REJECTED] {reason}")).await;
                        TurnKind::Rejection
                    }
                    _ => return Err(RuChatError::Is(format!("Unknown agent: {role}"))),
                };
                ctx.push_turn(kind, &role, ctx.output.clone());
            }

            ctx.print_debug_info(&tx, &role).await;

            // Sent via `ctx.trace` (the same channel `print_debug_info`'s state dump just went
            // through), not a direct stdout write, so the two can't race — the renderer sees
            // this prompt strictly after the state it's a prompt *about*. Only the actual
            // blocking read needs real stdin, via the same `Io` type used elsewhere in this
            // codebase for interactive prompts (`func`'s REPL, `AskArgs::ask`'s stdin fallback).
            if breakpoints.should_pause_after(&role) {
                ctx.trace(
                    &tx,
                    format!(
                        "[BREAKPOINT] Paused after round {} ({role}). Press Enter to \
                         continue, 'c' to continue without further pauses, 'q' to abort.",
                        ctx.round,
                    ),
                )
                .await;
                let mut io = crate::io::Io::new();
                match io.read_line().await.unwrap_or_default().trim() {
                    "q" | "Q" => return Err(RuChatError::Cancelled),
                    "c" | "C" => breakpoints = DebugBreakpoints::default(),
                    _ => {}
                }
            }
        }

        ctx.trace(
            &tx,
            "DEBUG SEQUENCE COMPLETE — real Librarian query used when present".to_string(),
        )
        .await;
        Ok(())
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
    use super::super::test_support::{base_config, fake_query_response, run_fixture};
    use crate::agent::event::{AgentEvent, StreamItem};
    use serde_json::json;

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

    // The `agent_debug/*.json` fixture directory had two combinations (`architect_librarian_
    // and_worker.json`, `architect_librarian_worker_validator.json`) that existed on disk but
    // were never actually driven through `run_fixture`/`debug_stage_machine` by any test —
    // `cargo test --lib` compiling and passing gave no signal about whether these two specific
    // role sequences worked, since nothing exercised them. ROADMAP.md previously (incorrectly)
    // described the fixed-sequence debug mode as "not wired into cargo test" at all, which was
    // stale — 9 of the then-11 fixtures already were; these two were the only real gap.
    #[tokio::test]
    async fn architect_librarian_and_worker_completes() {
        let mut config = base_config();
        config["Librarian"] = json!({ "model": "fake", "embed_model": "fake-embed" });
        let items = run_fixture(
            "architect_librarian_and_worker.json",
            config,
            vec![
                "Plan: refactor error handling to use `?` and anyhow::Context.",
                "{\"query\": \"error handling\", \"n_results\": 5, \"collection\": \"repo\"}",
                "Replaced unwrap() with `?` and anyhow::Context.",
            ],
            Some(fake_query_response()),
        )
        .await;
        assert!(!items.is_empty());
    }

    #[tokio::test]
    async fn architect_librarian_worker_validator_completes() {
        let mut config = base_config();
        config["Librarian"] = json!({ "model": "fake", "embed_model": "fake-embed" });
        config["Validator"] = json!({ "model": "fake" });
        let items = run_fixture(
            "architect_librarian_worker_validator.json",
            config,
            vec![
                "Plan: refactor error handling to use `?` and anyhow::Context.",
                "{\"query\": \"error handling\", \"n_results\": 5, \"collection\": \"repo\"}",
                "Replaced unwrap() with `?` and anyhow::Context.",
                "{\"verdict\": \"VALIDATED\", \"reason\": \"\"}",
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
