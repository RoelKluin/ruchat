use crate::agent::llm_client::LlmClient;
use crate::{Result, RuChatError};
use ollama_rs::generation::chat::ChatMessage;
use std::time::Duration;
use tokio_stream::StreamExt;

/// Generates a short, human-readable explanation of why an unsuccessful run (escalated, or
/// the iteration budget exhausted without ever reaching `Stage::Commit`) failed to reach an
/// accepted result, from the run's own trace.
pub(crate) async fn generate_failure_summary(
    ollama: &dyn LlmClient,
    model: &str,
    goal: &str,
    trace: &str,
) -> Result<String> {
    let system = "You are analyzing a finished, unsuccessful run of an autonomous multi-agent \
        coding pipeline (Architect plans, Worker implements, Tester/Validator/Critics review). \
        Given the run's goal and its full trace — every round's plan, implementation, tool \
        output, and rejection, in order — identify the single main reason the run did not \
        reach an accepted, committed result. Output ONLY a concise explanation, 2-4 sentences: \
        what was attempted, and specifically why it failed (e.g. a recurring rejection reason, \
        an agent repeating identical output and stalling, a technical error like a failing \
        test or an unparseable patch, or simply running out of iterations). No preamble, no \
        headers, no fences.";
    generate_run_summary(ollama, model, system, goal, trace, "failure").await
}

/// Generates a short, human-readable explanation of how/why a successful run reached its
/// accepted, committed result, from the run's own trace — used for the one-file-per-run
/// summary kept in `TRACE_SUCCESS_DIR` (deliberately not the full trace; see
/// `Context::finalize_success_trace`).
pub(crate) async fn generate_success_summary(
    ollama: &dyn LlmClient,
    model: &str,
    goal: &str,
    trace: &str,
) -> Result<String> {
    let system = "You are analyzing a finished, successful run of an autonomous multi-agent \
        coding pipeline (Architect plans, Worker implements, Tester/Validator/Critics review, \
        then the result is committed). Given the run's goal and its full trace — every \
        round's plan, implementation, tool output, and any rejections along the way, in order \
        — summarize how the goal was actually accomplished. Output ONLY a concise summary, \
        2-4 sentences: what was changed (files/functions if apparent), and note briefly if any \
        earlier attempt was rejected and corrected before the final accepted result. No \
        preamble, no headers, no fences.";
    generate_run_summary(ollama, model, system, goal, trace, "success").await
}

/// Shared one-shot LLM call behind both summary functions above, same pattern as
/// `git::generate_commit_message` — deliberately bypassing the Agent/Role/Context turn-log
/// machinery, since this analyzes a finished trace rather than participating in the run.
/// `label` is only used to make timeout/empty-response errors identify which summary failed.
async fn generate_run_summary(
    ollama: &dyn LlmClient,
    model: &str,
    system: &str,
    goal: &str,
    trace: &str,
    label: &str,
) -> Result<String> {
    if trace.trim().is_empty() {
        return Err(RuChatError::Is("empty trace, nothing to analyze".into()));
    }
    let user = format!("GOAL: {goal}\n\nFULL TRACE:\n{trace}");
    let messages = vec![
        ChatMessage::system(system.to_string()),
        ChatMessage::user(user),
    ];
    let generated = tokio::time::timeout(Duration::from_secs(30), async {
        let mut stream = ollama.chat_stream(model, messages).await?;
        let mut message = String::new();
        while let Some(chunk) = stream.next().await {
            message.push_str(&chunk?.message.content);
        }
        Ok::<String, RuChatError>(message)
    })
    .await
    .map_err(|_| RuChatError::Is(format!("{label} summary generation timed out after 30s")))??;

    let generated = generated.trim().to_string();
    if generated.is_empty() {
        Err(RuChatError::Is(format!("LLM returned an empty {label} summary")))
    } else {
        Ok(generated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::llm_client::FakeLlmClient;

    #[tokio::test]
    async fn generate_failure_summary_returns_the_trimmed_llm_response() {
        let ollama = FakeLlmClient::new(vec!["  Worker kept hallucinating a function \
            signature; every patch failed to apply.  "]);
        let summary = generate_failure_summary(&ollama, "any-model", "fix a bug", "some trace")
            .await
            .unwrap();
        assert_eq!(
            summary,
            "Worker kept hallucinating a function signature; every patch failed to apply."
        );
    }

    #[tokio::test]
    async fn generate_failure_summary_rejects_an_empty_trace_without_calling_the_llm() {
        let ollama = FakeLlmClient::new(vec![]); // would panic if chat_stream were called
        let result = generate_failure_summary(&ollama, "any-model", "fix a bug", "  ").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn generate_failure_summary_errors_on_an_empty_llm_response() {
        let ollama = FakeLlmClient::new(vec!["   "]);
        let result = generate_failure_summary(&ollama, "any-model", "fix a bug", "some trace")
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn generate_success_summary_returns_the_trimmed_llm_response() {
        let ollama = FakeLlmClient::new(vec!["  Renamed the helper and updated call sites.  "]);
        let summary = generate_success_summary(&ollama, "any-model", "rename a fn", "some trace")
            .await
            .unwrap();
        assert_eq!(summary, "Renamed the helper and updated call sites.");
    }

    #[tokio::test]
    async fn generate_success_summary_rejects_an_empty_trace_without_calling_the_llm() {
        let ollama = FakeLlmClient::new(vec![]); // would panic if chat_stream were called
        let result = generate_success_summary(&ollama, "any-model", "rename a fn", "  ").await;
        assert!(result.is_err());
    }
}
