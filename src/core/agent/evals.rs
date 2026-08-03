//! "Agentic evals": tests that a REAL model's response to a specific role's ACTUAL prompt
//! template behaves the way that role is supposed to, given a specific input scenario — as
//! opposed to the rest of this crate's tests, which either test pure logic or drive the stage
//! machine with `FakeLlmClient`'s scripted responses. These need a live Ollama server with the
//! model pulled, cost real inference time (seconds, not milliseconds), and their outcome
//! depends on live model behavior that can drift across model versions/quantizations — none of
//! that belongs in the fast, deterministic `cargo test --lib` pass everything else here runs
//! in, so every test in this file is `#[ignore]`d and must be run explicitly:
//!
//! ```sh
//! cargo test --lib -- --ignored agent_eval
//! ```
//!
//! Override the model with the `RUCHAT_EVAL_MODEL` env var (defaults to `qwen2.5-coder:14b`,
//! matching what this repo's own example scripts use); the server is always
//! `Ollama::default()` (`http://127.0.0.1:11434`) — edit `eval_ollama()` below directly if you
//! need a different one, not worth an env var for how rarely that changes.
//!
//! A failing eval here means the *prompt* (or the model) isn't producing the behavior the rest
//! of the pipeline assumes — not a logic bug in this crate's own code. Treat a red eval as a
//! signal to look at the relevant `agent_role/*.md` template first, same as any other real-run
//! bug report in this project's history.

use super::json_extract::strip_json_fences;
use super::types::{Context, TurnKind};
use super::Agent;
use ollama_rs::Ollama;
use serde_json::{json, Value};
use tokio::sync::mpsc;

fn eval_model() -> String {
    std::env::var("RUCHAT_EVAL_MODEL").unwrap_or_else(|_| "qwen2.5-coder:14b".to_string())
}

fn eval_ollama() -> Ollama {
    Ollama::default()
}

async fn build_eval_agent(role: &str) -> Agent {
    let mut config = json!({ role: { "model": eval_model() } });
    Agent::new(&mut config, role, true, None, json!({}))
        .await
        .unwrap_or_else(|e| panic!("failed to construct eval agent for role {role:?}: {e}"))
}

/// Runs `role`'s real prompt template against a live model with `ctx` as the scenario, and
/// returns the raw response (`ctx.output` after the call). Panics (failing the test, not
/// silently skipping) on any connection/protocol error — an eval that can't reach Ollama at all
/// should be loud about it, not quietly report as if the model behaved correctly.
async fn run_eval(role: &str, ctx: &mut Context) -> String {
    let mut agent = build_eval_agent(role).await;
    let ollama = eval_ollama();
    let (tx, mut rx) = mpsc::channel(100);
    // Drain the channel concurrently — `query_stream` blocks on a full buffer, and nobody else
    // is listening in this harness.
    let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
    agent
        .query_stream(&ollama, ctx, &tx)
        .await
        .unwrap_or_else(|e| {
            panic!(
                "query_stream failed for role {role:?} — is Ollama running at 127.0.0.1:11434 \
                with '{}' pulled? error: {e}",
                eval_model()
            )
        });
    drop(tx);
    let _ = drain.await;
    ctx.output.clone()
}

// Regression canary for the exact bug fixed this session: the Validator used to only ever
// check "does WORKER_OUTPUT ask a question" and never explicitly evaluated narrative/
// walkthrough prose (numbered steps, "### Identified..." headers) as a non-answer — see
// validator.md's rewrite. This eval feeds it exactly that shape of input and checks the real
// model, using the real template, actually rejects it.
#[tokio::test]
#[ignore = "agentic eval — requires a live Ollama server with the model pulled; run explicitly \
    with `cargo test --lib -- --ignored agent_eval`"]
async fn agent_eval_validator_rejects_a_narrative_walkthrough() {
    let mut ctx = Context::new("rename the confusingly-named function `foo` to `bar`".to_string());
    ctx.output = "### Identified the function\n\n\
        I would rename `foo` to `bar` in src/lib.rs. Assuming this resolves the issue, proceed \
        with the next steps."
        .to_string();

    let raw = run_eval("validator", &mut ctx).await;
    let stripped = strip_json_fences(&raw);
    let verdict: Value = serde_json::from_str(stripped).unwrap_or_else(|e| {
        panic!("Validator did not return valid JSON: {e}\nraw response:\n{raw}")
    });
    let verdict_str = verdict["verdict"].as_str().unwrap_or_default().to_uppercase();
    assert_eq!(
        verdict_str, "REJECTED",
        "expected the Validator to reject a narrative walkthrough with no tool call, got: {raw}"
    );
}

// Regression canary for the architect.md fix from this session: the Architect used to repeat
// an identical plan after a rejection revealed the file's real content contradicted its own
// prior assumption (it assumed a function was dead code; the real file showed it was a
// different, in-use function) — the run stalled and escalated. This eval reproduces exactly
// that HISTORY shape and checks the real model's new plan doesn't just repeat the same CHOICE.
//
// Expect this one to be genuinely more flake-prone than the other evals here — confirmed by a
// real run: qwen2.5-coder:14b sometimes correctly reasons "the old assumption was wrong, target
// something else instead" (passes), and sometimes updates its diff to match the real signature
// shown but still concludes the function is unused and targets it for removal anyway (fails).
// Both are real model behavior, not a bug in this eval — the scenario only shows the file's
// real *signature*, not a fresh clippy result confirming whether it's still flagged as dead
// code, so "is it actually unused" is a judgment call the model can get wrong even after
// correcting the surface-level mistake. A red run here is a genuine signal that architect.md's
// instruction is a partial mitigation (stops verbatim repetition) rather than a guarantee of
// full semantic correction — not something to "fix" by weakening the assertion.
#[tokio::test]
#[ignore = "agentic eval — requires a live Ollama server with the model pulled; run explicitly \
    with `cargo test --lib -- --ignored agent_eval`"]
async fn agent_eval_architect_does_not_repeat_a_choice_the_real_content_disproved() {
    let wrong_plan = "1. Review the clippy warnings.\n\
        2. Identify the first warning: unused function `parse_key_val` in `src/cli/utils.rs`.\n\
        3. Remove the function to fix the lint.\n\nFILES: src/cli/utils.rs";
    let bad_diff = "```tool_call\n{\"tool\": \"apply_patch\", \"diff\": \"--- a/src/cli/utils.rs\\n+++ b/src/cli/utils.rs\\n@@ -1,3 +1,1 @@\\n use anyhow::Result;\\n-fn parse_key_val(s: &str) -> Option<(&str, &str)> {\\n-    s.split_once('=')\\n-}\\n\"}\n```";
    let real_content_rejection = "Patch apply failed on src/cli/utils.rs: error applying hunk #1\n\n\
        This means the diff's context lines don't match src/cli/utils.rs's actual current \
        content. Here is the file's real current content — write your next diff's context \
        lines to match this exactly, don't guess:\n\n\
        use anyhow::Result;\nuse std::error::Error;\n\n\
        pub(super) fn parse_key_val<T, U>(s: &str) -> Result<(T, U), Box<dyn Error + Send + \
        Sync + 'static>>\nwhere\n    T: std::str::FromStr,\n    T::Err: Error + Send + Sync + \
        'static,\n    U: std::str::FromStr,\n    U::Err: Error + Send + Sync + 'static,\n{\n    \
        match s.split_once(':') {\n        Some((key, value)) => Ok((key.parse()?, \
        value.parse()?)),\n        None => Err(format!(\"invalid KEY:VALUE, no `:` found in \
        `{}`\", s).into()),\n    }\n}\n";

    let mut ctx = Context::new(
        "fix one clippy warning; use the cargo_clippy tool, pick the first one reported in \
        src/, and fix just that one lint"
            .to_string(),
    );
    ctx.round = 1;
    ctx.push_turn(TurnKind::Plan, "Architect", wrong_plan.to_string());
    ctx.push_turn(TurnKind::Implementation, "Worker", bad_diff.to_string());
    ctx.push_turn(TurnKind::Rejection, "ApplyPatch", real_content_rejection.to_string());
    ctx.round = 2;

    let new_plan = run_eval("architect", &mut ctx).await;

    assert_ne!(
        new_plan.trim(),
        wrong_plan.trim(),
        "expected the Architect's new plan to change after real file content disproved its \
        assumption, but it repeated the exact same plan verbatim"
    );
    // Checking for an actual diff *removal* line naming parse_key_val, not just any mention of
    // the phrase — a real run showed the model correctly explaining *why* the old assumption
    // was wrong ("the real content shows the function is actually used... instead we should
    // remove X") while still mentioning parse_key_val by name in that explanation. That's the
    // desired behavior, not a repeat of the mistake; only still trying to remove it is.
    let still_targets_removal = new_plan.lines().any(|l| {
        let trimmed = l.trim_start();
        trimmed.starts_with('-') && !trimmed.starts_with("---") && l.contains("parse_key_val")
    });
    assert!(
        !still_targets_removal,
        "expected the Architect to abandon the disproven \"parse_key_val is unused\" \
        assumption, but its new plan still targets removing it:\n{new_plan}"
    );
}

// Regression canary for the pre-existing "never narrate instead of acting" instruction in
// worker.md: given a plan that clearly specifies what to do, the Worker's entire response
// should be a tool_call, not a prose walkthrough of what it would do.
#[tokio::test]
#[ignore = "agentic eval — requires a live Ollama server with the model pulled; run explicitly \
    with `cargo test --lib -- --ignored agent_eval`"]
async fn agent_eval_worker_emits_a_tool_call_not_narration() {
    let mut ctx = Context::new(
        "add a doc comment above the `main` function in src/main.rs explaining what it does"
            .to_string(),
    );
    ctx.round = 1;
    ctx.push_turn(
        TurnKind::Plan,
        "Architect",
        "1. Open src/main.rs.\n2. Add a one-line doc comment above `fn main`.\n\nFILES: src/main.rs"
            .to_string(),
    );

    let raw = run_eval("worker", &mut ctx).await;
    assert!(
        super::tools::parse_tool_call(&raw).is_ok(),
        "expected the Worker's response to contain a recognizable tool_call, got:\n{raw}"
    );
}
