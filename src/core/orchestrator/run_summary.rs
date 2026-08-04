use crate::agent::llm_client::LlmClient;
use crate::utils::text::wrap_line;
use crate::{Result, RuChatError};
use ollama_rs::generation::chat::ChatMessage;
use std::time::Duration;
use tokio_stream::StreamExt;

/// Line length the generated summary gets wrapped to before it's written to the trace file —
/// same word-boundary wrapping as commit messages (`git::MAX_COMMIT_LINE_LEN`), just a
/// different width: a run summary is read in a trace file, not `git log --oneline`, so there's
/// no first-line convention to protect and every line gets wrapped the same way.
const SUMMARY_LINE_LEN: usize = 120;

/// Generates a short, human-readable explanation of why an unsuccessful run (escalated, or
/// the iteration budget exhausted without ever reaching `Stage::Commit`) failed to reach an
/// accepted result, from the run's own trace.
///
/// Deliberately asks for *every* distinct contributing issue, not just the one that ultimately
/// ended the run — maintainer feedback that "several things may have gone wrong" during a run,
/// and a summary naming only the single final cause was throwing away useful information about
/// the others (e.g. an earlier stall that got worked around, followed by a genuinely fatal
/// technical error) that would help understand the whole run, not just its last moment.
pub(crate) async fn generate_failure_summary(
    ollama: &dyn LlmClient,
    model: &str,
    goal: &str,
    trace: &str,
) -> Result<String> {
    let system = "You are analyzing a finished, unsuccessful run of an autonomous multi-agent \
        coding pipeline (Architect plans, Worker implements, Tester/Validator/Critics review). \
        Given the run's goal and its full trace — every round's plan, implementation, tool \
        output, and rejection, in order — identify EVERY distinct thing that went wrong over \
        the course of the run, not just the one that ultimately ended it: a recurring \
        rejection reason, an agent repeating identical output and stalling, a technical error \
        like a failing test or an unparseable patch, a wrong assumption an earlier round made \
        and later abandoned, running out of iterations, etc. Several issues can contribute \
        even when only one of them was the final, decisive cause. Output ONLY a concise list, \
        one distinct issue per line, most significant/decisive first — a line per issue, not a \
        single paragraph. If there's genuinely only one issue, one line is fine. No preamble, \
        no headers, no fences, no bullet characters (line breaks alone separate the issues).";
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
        Err(RuChatError::Is(format!(
            "LLM returned an empty {label} summary"
        )))
    } else {
        // A backstop for the prompt's own formatting instructions, same reasoning as
        // `git::wrap_commit_message_body`: models don't reliably honor exact line-length
        // instructions on their own. Each line (the failure summary can be several, one per
        // distinct issue) is wrapped independently rather than the whole thing as one
        // paragraph, so the one-issue-per-line structure survives wrapping.
        Ok(generated
            .lines()
            .map(|l| wrap_line(l, SUMMARY_LINE_LEN))
            .collect::<Vec<_>>()
            .join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::llm_client::FakeLlmClient;

    #[tokio::test]
    async fn generate_failure_summary_returns_the_trimmed_llm_response() {
        let ollama = FakeLlmClient::new(vec![
            "  Worker kept hallucinating a function \
            signature; every patch failed to apply.  ",
        ]);
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
        let result =
            generate_failure_summary(&ollama, "any-model", "fix a bug", "some trace").await;
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

    // Regression: maintainer feedback that the run summary should be wrapped, and that a run
    // can have several distinct contributing issues, not just one. This test covers the
    // wrapping backstop (models don't reliably honor the prompt's own line-length instruction);
    // the "identify every issue" instruction itself lives in the prompt text and isn't
    // separately testable without a live model.
    #[tokio::test]
    async fn generate_failure_summary_wraps_each_line_independently_at_120_chars() {
        let scripted = "The Worker repeated an identical apply_patch attempt against a \
            hallucinated function signature across three consecutive rounds without ever \
            calling read_file first, so every attempt failed the exact same way.\n\
            Short second issue.";
        let ollama = FakeLlmClient::new(vec![scripted]);
        let summary = generate_failure_summary(&ollama, "any-model", "fix a bug", "some trace")
            .await
            .unwrap();
        for line in summary.lines() {
            assert!(
                line.chars().count() <= SUMMARY_LINE_LEN,
                "line exceeds {SUMMARY_LINE_LEN} chars: {line:?} ({} chars)",
                line.chars().count()
            );
        }
        // Wrapping must not lose either issue or merge them into one.
        assert!(summary.contains("Short second issue."));
        assert!(summary.contains("hallucinated function signature"));
    }
}
