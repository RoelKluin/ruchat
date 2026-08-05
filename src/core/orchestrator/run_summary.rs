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

/// Budget for the outcome summary (`generate_failure_summary`/`generate_success_summary`) —
/// a few sentences, so a slow local model still has plenty of room.
const SUMMARY_TIMEOUT: Duration = Duration::from_secs(30);

/// Budget for the step review, which is a different size of job: one line per round of a run
/// that can be a dozen rounds long, so hundreds of output tokens rather than a few dozen. At
/// the 30s used for the outcome summary this timed out on any non-trivial run and the review
/// silently degraded to a placeholder, so it gets its own, much larger budget.
const REVIEW_TIMEOUT: Duration = Duration::from_secs(240);

/// How much of a trace is handed to the summarizing model. Real traces average ~50 KB and the
/// largest seen is ~450 KB — well past what fits in the model's context, and `chat_stream` sets
/// no `num_ctx`, so Ollama would silently drop part of the prompt (including, potentially, the
/// system instruction) with no way to tell what it kept. Clamping here instead keeps that
/// decision ours and marks the gap explicitly in the text the model reads.
const MAX_TRACE_CHARS: usize = 24_000;

/// Fraction of the budget kept from the start of an over-long trace; the rest comes from the
/// end. Both ends matter for a step review — the head has the goal and the first rounds' plans,
/// the tail has how the run actually ended — so the middle rounds are what gets dropped.
const TRACE_HEAD_FRACTION: usize = 2;

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
    generate_run_summary(
        ollama,
        model,
        system,
        goal,
        trace,
        "failure",
        SUMMARY_TIMEOUT,
    )
    .await
}

/// Generates a round-by-round review of the decisions the agents made during a run, saying for
/// each one whether it was a good call — the part of a run summary worth learning from later,
/// as opposed to the outcome summaries above, which only say how the run ended.
///
/// The prompt deliberately distinguishes *stated* reasoning from *observed* action. The
/// Architect writes an explicit `CHOICE:` paragraph and rejections carry their own reason, so
/// those really are in the trace and can be quoted; a Worker's tool call is just a tool call.
/// Asking a model "what was this agent thinking" for the latter gets a confident invention,
/// which is worse than nothing in a file kept specifically to learn from — so the prompt asks
/// it to judge the decision against what was already in context at that point instead.
///
/// Verdicts are fixed, greppable prefixes (`GOOD:`/`BAD:`/`UNCLEAR:`/`LESSON:`) so recurring
/// failure patterns can be found across runs with one `grep` over `ruchat_traces/summaries/`.
pub(crate) async fn generate_step_review(
    ollama: &dyn LlmClient,
    model: &str,
    goal: &str,
    trace: &str,
) -> Result<String> {
    let system = "You are reviewing a finished run of an autonomous multi-agent coding pipeline \
        (Architect plans, Worker implements with tools, Tester/Validator/Critics review), so \
        its maintainer can see where the agents decided well and where they went wrong. You \
        are given the run's goal and its trace in chronological order; each entry is headed \
        '### <Role> [<Kind>, round <N>]:'. The Architect's entries state its plan and often an \
        explicit 'CHOICE:' paragraph — that is its own stated reasoning, so use it. Other \
        roles show only what they did: a tool call, a diff, a rejection message with its \
        reason. For those, describe the decision that was made and judge it — never invent a \
        motive the agent did not state.\n\n\
        Walk the run in order and output ONE LINE per significant step, in exactly this form:\n\
        round <N> | <Role> | <what it decided or did, plus its stated reason if the trace gives \
        one> | <VERDICT>: <short remark>\n\n\
        <N> is the round number copied from that entry's own heading — NOT a running count of \
        the lines you output. One round contains several entries, so the same <N> repeats on \
        consecutive lines; that is correct and expected. Never invent a round number the trace \
        does not contain.\n\n\
        <VERDICT> must be exactly one of GOOD, BAD or UNCLEAR. Judge each step against what was \
        already visible in the trace above it: a step is BAD if it ignored information already \
        in context (repeating a tool call whose result is already shown, editing a file it \
        never read, resubmitting a diff that was just rejected for a stated reason, planning \
        against a file it has not looked at yet), and GOOD if it used that information well. \
        Use UNCLEAR only when the trace genuinely does not show enough to judge. Skip \
        uneventful entries rather than padding the list.\n\n\
        Then output up to three final lines, each starting with 'LESSON:', naming a recurring \
        pattern across the run that would be worth fixing in the pipeline or its prompts. If \
        the run went cleanly, say so in one LESSON line. No preamble, no headers, no fences, \
        no bullet characters.";
    generate_run_summary(
        ollama,
        model,
        system,
        goal,
        trace,
        "step review",
        REVIEW_TIMEOUT,
    )
    .await
}

/// Shortens an over-long trace to `MAX_TRACE_CHARS`, keeping both ends and replacing the
/// middle with a marker naming how much was dropped. Character-based (not byte-based) so a
/// multi-byte character can never be split; pure and separately tested, since the whole point
/// is that the model gets a known input rather than whatever Ollama happened not to truncate.
fn clamp_trace(trace: &str) -> String {
    let total = trace.chars().count();
    if total <= MAX_TRACE_CHARS {
        return trace.to_string();
    }
    let head_len = MAX_TRACE_CHARS / TRACE_HEAD_FRACTION;
    let tail_len = MAX_TRACE_CHARS - head_len;
    let head: String = trace.chars().take(head_len).collect();
    let tail: String = trace.chars().skip(total - tail_len).collect();
    let dropped = total - head_len - tail_len;
    format!("{head}\n\n[... {dropped} characters of middle rounds omitted ...]\n\n{tail}")
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
    generate_run_summary(
        ollama,
        model,
        system,
        goal,
        trace,
        "success",
        SUMMARY_TIMEOUT,
    )
    .await
}

/// Shared one-shot LLM call behind both summary functions above, same pattern as
/// `git::generate_commit_message` — deliberately bypassing the Agent/Role/Context turn-log
/// machinery, since this analyzes a finished trace rather than participating in the run.
/// `label` is only used to make timeout/empty-response errors identify which summary failed;
/// `budget` is how long that one call gets, which differs a lot between a few-sentence outcome
/// summary and a per-round step review (see `SUMMARY_TIMEOUT`/`REVIEW_TIMEOUT`).
async fn generate_run_summary(
    ollama: &dyn LlmClient,
    model: &str,
    system: &str,
    goal: &str,
    trace: &str,
    label: &str,
    budget: Duration,
) -> Result<String> {
    if trace.trim().is_empty() {
        return Err(RuChatError::Is("empty trace, nothing to analyze".into()));
    }
    let user = format!("GOAL: {goal}\n\nFULL TRACE:\n{}", clamp_trace(trace));
    let messages = vec![
        ChatMessage::system(system.to_string()),
        ChatMessage::user(user),
    ];
    let generated = tokio::time::timeout(budget, async {
        let mut stream = ollama.chat_stream(model, messages).await?;
        let mut message = String::new();
        while let Some(chunk) = stream.next().await {
            message.push_str(&chunk?.message.content);
        }
        Ok::<String, RuChatError>(message)
    })
    .await
    .map_err(|_| {
        RuChatError::Is(format!(
            "{label} generation timed out after {}s",
            budget.as_secs()
        ))
    })??;

    let generated = generated.trim().to_string();
    if generated.is_empty() {
        Err(RuChatError::Is(format!("LLM returned an empty {label}")))
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

    #[tokio::test]
    async fn generate_step_review_returns_the_per_step_verdict_lines() {
        let scripted = "round 1 | Worker | called cargo_clippy a third time after being told \
            twice its result was already in context | BAD: ignored the orchestrator's own \
            instruction\n\
            LESSON: the Worker re-runs read-only tools instead of acting on their output.";
        let ollama = FakeLlmClient::new(vec![scripted]);
        let review = generate_step_review(&ollama, "any-model", "fix a lint", "some trace")
            .await
            .unwrap();
        assert!(review.contains("BAD:"));
        assert!(review.contains("LESSON:"));
    }

    #[tokio::test]
    async fn generate_step_review_rejects_an_empty_trace_without_calling_the_llm() {
        let ollama = FakeLlmClient::new(vec![]); // would panic if chat_stream were called
        let result = generate_step_review(&ollama, "any-model", "fix a lint", "  ").await;
        assert!(result.is_err());
    }

    #[test]
    fn clamp_trace_leaves_a_trace_within_budget_untouched() {
        let trace = "a short trace\nwith two lines\n";
        assert_eq!(clamp_trace(trace), trace);
    }

    #[test]
    fn clamp_trace_keeps_both_ends_of_an_over_long_trace() {
        // The head and tail are what a step review needs (goal and first plans; how the run
        // ended); only the middle may be dropped.
        let trace = format!(
            "HEAD MARKER\n{}\nTAIL MARKER",
            "x".repeat(MAX_TRACE_CHARS * 2)
        );
        let clamped = clamp_trace(&trace);
        assert!(clamped.starts_with("HEAD MARKER"));
        assert!(clamped.ends_with("TAIL MARKER"));
        assert!(clamped.contains("characters of middle rounds omitted"));
        // The marker itself adds a little, so this bounds the payload, not the exact result.
        assert!(clamped.chars().count() < MAX_TRACE_CHARS + 200);
    }

    #[test]
    fn clamp_trace_never_splits_a_multi_byte_character() {
        // Traces routinely contain em dashes and other non-ASCII from agent output; a
        // byte-sliced clamp would panic or produce invalid UTF-8 on exactly this input.
        let trace = "—".repeat(MAX_TRACE_CHARS * 2);
        let clamped = clamp_trace(&trace);
        assert!(clamped.starts_with('—'));
        assert!(clamped.ends_with('—'));
    }
}
