use super::{Orchestrator, OrchestratorResult};
use crate::Result;
use crate::agent::types::{Context, TurnKind};
use crate::retry_transient;
use tokio::sync::mpsc;

impl Orchestrator {
    pub(super) async fn run_critics_parallel(
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
            let ollama = &self.chat;
            let start = std::time::Instant::now();
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
                (
                    label,
                    result.map(|_| scratch.output),
                    approval_signal,
                    start,
                )
            });
        }
        let results = futures_util::future::join_all(futs).await;
        for (label, result, approval_signal, start) in results {
            match result {
                Ok(text) => {
                    ctx.trace(tx, format!("[Critic '{label}']:\n{text}")).await;
                    let source = format!("Critic '{label}'");
                    if !text.contains(&approval_signal) {
                        ctx.push_turn_timed(TurnKind::Rejection, &source, text, start);
                    } else {
                        // Unlike the rejection arm above, an approving critic's review used to
                        // push no turn at all — only the ephemeral `ctx.trace(...)` call above
                        // saw it, which shows up live on the console/event stream but is never
                        // added to `ctx.turns`, so it's gone from the persisted trace file the
                        // next time it's rewritten. An approving review is still an action this
                        // critic took and should be just as visible as a rejecting one.
                        ctx.push_turn_timed(TurnKind::System, &source, text, start);
                    }
                }
                Err(e) => {
                    // A critic that exhausts retries must count as a
                    // rejection, not a silent no-op — otherwise an
                    // unreachable/erroring critic is indistinguishable from
                    // an approving one, inverting the consensus gate's intent.
                    ctx.push_turn_timed(
                        TurnKind::Rejection,
                        "Critic",
                        format!("critic failed to produce a verdict: {e}"),
                        start,
                    );
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{base_config, build_test_orchestrator};
    use crate::agent::event::{AgentEvent, StreamItem};
    use crate::agent::types::{Context, TurnKind};
    use serde_json::json;
    use tokio::sync::mpsc;

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
        assert!(
            traces
                .iter()
                .any(|t| t.starts_with("[Critic 'Performance']:"))
        );
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

    // Regression: maintainer feedback that the trace "only contains the agent output, not the
    // agent actions" — an approving critic's review used to push no turn at all, only the
    // ephemeral `ctx.trace(...)` call visible live on the event stream at the time, which is
    // never added to `ctx.turns` and so vanishes from the persisted trace file afterward. An
    // approving review is still an action the critic took and must be just as visible as a
    // rejecting one.
    #[tokio::test]
    async fn run_critics_parallel_records_an_approving_review_as_a_system_turn() {
        let mut config = base_config();
        config["Critics"] =
            json!([{ "model": "fake", "name": "Security", "task": "security review" }]);
        let mut orchestrator =
            build_test_orchestrator(config, vec!["No issues found.\nAPPROVED"], None).await;
        let mut ctx = Context::new("goal".to_string());
        ctx.output = "some implementation".to_string();
        let (tx, _rx) = mpsc::channel(100);

        orchestrator
            .run_critics_parallel(&mut ctx, &tx)
            .await
            .unwrap();

        let recorded = ctx.turns.iter().find(|t| {
            t.kind == TurnKind::System
                && t.source == "Critic 'Security'"
                && t.content.contains("No issues found.")
        });
        assert!(
            recorded.is_some(),
            "expected the approving critic's review recorded as a System turn, got: {:?}",
            ctx.turns
        );
    }
}
