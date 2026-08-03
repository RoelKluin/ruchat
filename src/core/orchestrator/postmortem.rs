use crate::agent::llm_client::LlmClient;
use crate::{Result, RuChatError};
use ollama_rs::generation::chat::ChatMessage;
use std::time::Duration;
use tokio_stream::StreamExt;

/// Generates a short, human-readable explanation of why an unsuccessful run (escalated, or
/// the iteration budget exhausted without ever reaching `Stage::Commit`) failed to reach an
/// accepted result, from the run's own trace — a single direct LLM call, same one-shot pattern
/// as `git::generate_commit_message`, deliberately bypassing the Agent/Role/Context turn-log
/// machinery since this analyzes a finished trace rather than participating in the run.
pub(crate) async fn generate_failure_summary(
    ollama: &dyn LlmClient,
    model: &str,
    goal: &str,
    trace: &str,
) -> Result<String> {
    if trace.trim().is_empty() {
        return Err(RuChatError::Is("empty trace, nothing to analyze".into()));
    }
    let system = "You are analyzing a finished, unsuccessful run of an autonomous multi-agent \
        coding pipeline (Architect plans, Worker implements, Tester/Validator/Critics review). \
        Given the run's goal and its full trace — every round's plan, implementation, tool \
        output, and rejection, in order — identify the single main reason the run did not \
        reach an accepted, committed result. Output ONLY a concise explanation, 2-4 sentences: \
        what was attempted, and specifically why it failed (e.g. a recurring rejection reason, \
        an agent repeating identical output and stalling, a technical error like a failing \
        test or an unparseable patch, or simply running out of iterations). No preamble, no \
        headers, no fences.";
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
    .map_err(|_| RuChatError::Is("failure summary generation timed out after 30s".into()))??;

    let generated = generated.trim().to_string();
    if generated.is_empty() {
        Err(RuChatError::Is("LLM returned an empty failure summary".into()))
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
}
