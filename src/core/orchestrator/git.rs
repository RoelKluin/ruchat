use crate::agent::llm_client::LlmClient;
use crate::agent::types::Context;
use crate::{Result, RuChatError};
use log::info;
use ollama_rs::generation::chat::ChatMessage;
use std::time::Duration;
use tokio_stream::StreamExt;

pub(crate) async fn commit_feature_branch(
    ctx: &Context,
    ollama: &dyn LlmClient,
    model: &str,
) -> Result<()> {
    let timestamp = chrono::Utc::now().timestamp();
    let branch_name = format!("ai/feature-{}", timestamp);

    // 1. Get current branch name to return to it later
    let current_branch_output = tokio::process::Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .await?;
    let original_branch = String::from_utf8_lossy(&current_branch_output.stdout)
        .trim()
        .to_string();

    // 2. Execution with rollback
    let result = async {
        run_git_command(vec!["checkout", "-b", &branch_name]).await?;

        // Append to featured_changes.md
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open("featured_changes.md")
            .await?;

        // 2. Prepare the Summary Entry
        let summary_entry = format!(
            "\n--- \n### 🤖 AI Update: {}\n**Date:** {}\n**Goal:** {}\n**Changes:** \n{}\n",
            branch_name,
            chrono::Utc::now().to_rfc2822(),
            ctx.goal,
            ctx.output.lines().take(5).collect::<Vec<_>>().join("\n") // Take first 5 lines of worker output as summary
        );

        tokio::io::AsyncWriteExt::write_all(&mut file, summary_entry.as_bytes()).await?;

        // Stage only what this run actually changed. Previously this was `git add .`, which
        // staged every dirty/untracked file in the whole working tree onto the AI's feature
        // branch, not just its own change.
        let add_targets = commit_add_targets(ctx);
        let add_args: Vec<&str> = std::iter::once("add")
            .chain(add_targets.iter().map(String::as_str))
            .collect();
        run_git_command(add_args).await?;

        let diff = git_diff(None, true).await.unwrap_or_default();
        let message = generate_commit_message(ollama, model, &ctx.goal, &diff)
            .await
            .unwrap_or_else(|e| {
                tracing::warn!(error = ?e, "commit message generation failed, using fallback");
                fallback_commit_message(ctx)
            });
        run_git_command(vec!["commit", "-m", &message]).await?;
        Ok::<(), RuChatError>(())
    }
    .await;

    // 3. Always attempt to return to the original branch
    let _ = run_git_command(vec!["checkout", &original_branch]).await;

    if let Err(e) = result {
        // If we failed after creating the branch, maybe delete the failed branch
        let _ = run_git_command(vec!["branch", "-D", &branch_name]).await;
        return Err(e);
    }

    info!(
        "🚀 Changes committed to {} and returned to {}",
        branch_name, original_branch
    );
    Ok(())
}

/// Which paths get staged for the auto-commit: `featured_changes.md` (the changelog entry just
/// written) plus the single file `apply_patch` touched this run, if any (`Context` currently
/// tracks at most one accepted patch per run — see `PendingPatch`'s doc comment). Deliberately
/// NOT `git add .`/`-A`: this must never stage anything ruchat itself didn't produce, whether
/// that's the user's own unrelated in-progress work or stray files already sitting in the tree.
fn commit_add_targets(ctx: &Context) -> Vec<String> {
    let mut targets = vec!["featured_changes.md".to_string()];
    if let Some(pending) = ctx.pending_patch.as_ref() {
        targets.push(pending.path.clone());
    }
    targets
}

/// Deterministic fallback used when LLM-based commit message generation fails (Ollama
/// unreachable, timeout, empty response) — a validated, accepted change must never fail to
/// commit just because this nicety failed.
fn fallback_commit_message(ctx: &Context) -> String {
    let message = match ctx.pending_patch.as_ref() {
        Some(pending) => format!("AI: {}\n\nFile changed: {}", ctx.goal, pending.path),
        None => format!("AI: {}", ctx.goal),
    };
    wrap_commit_message_body(&message)
}

/// Generates a conventional commit message (imperative summary + short body) from the run's
/// goal and the actual staged diff, via a single direct LLM call — deliberately bypassing the
/// Agent/Role/Context turn-log machinery (`agent.rs::query_stream`) since this is a one-shot,
/// non-conversational utility call with nothing worth recording as a Turn, unlike the fixed
/// pipeline roles.
async fn generate_commit_message(
    ollama: &dyn LlmClient,
    model: &str,
    goal: &str,
    diff: &str,
) -> Result<String> {
    if diff.trim().is_empty() {
        return Err(RuChatError::Is("empty diff, nothing to summarize".into()));
    }
    let system = "You write git commit messages. Given a goal and a diff, output ONLY the \
        commit message: an imperative-mood summary line under 72 characters, then a blank \
        line, then 1-3 short sentences explaining why the change was made, wrapped so no body \
        line exceeds 80 characters. No fences, no preamble, no trailing commentary.";
    let user = format!("GOAL: {goal}\n\nDIFF:\n{diff}");
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
    .map_err(|_| RuChatError::Is("commit message generation timed out after 30s".into()))??;

    let generated = generated.trim().to_string();
    if generated.is_empty() {
        Err(RuChatError::Is("LLM returned an empty commit message".into()))
    } else {
        Ok(wrap_commit_message_body(&generated))
    }
}

/// Hard-wraps every line of `message` after the first at `MAX_COMMIT_LINE_LEN` characters,
/// breaking on word boundaries — a backstop for the prompt's own "wrapped at 80 characters"
/// instruction, since models don't reliably honor exact character limits. The first line (the
/// summary) is left untouched: git treats it specially (e.g. `git log --oneline`), so wrapping
/// it mid-line would corrupt that convention rather than just look untidy.
const MAX_COMMIT_LINE_LEN: usize = 80;

fn wrap_commit_message_body(message: &str) -> String {
    let Some((summary, body)) = message.split_once('\n') else {
        return message.to_string();
    };
    let wrapped_body = body
        .lines()
        .map(wrap_line)
        .collect::<Vec<_>>()
        .join("\n");
    format!("{summary}\n{wrapped_body}")
}

fn wrap_line(line: &str) -> String {
    if line.chars().count() <= MAX_COMMIT_LINE_LEN {
        return line.to_string();
    }
    let mut wrapped = String::new();
    let mut current_len = 0;
    for word in line.split(' ') {
        let word_len = word.chars().count();
        if current_len == 0 {
            wrapped.push_str(word);
            current_len = word_len;
        } else if current_len + 1 + word_len <= MAX_COMMIT_LINE_LEN {
            wrapped.push(' ');
            wrapped.push_str(word);
            current_len += 1 + word_len;
        } else {
            wrapped.push('\n');
            wrapped.push_str(word);
            current_len = word_len;
        }
    }
    wrapped
}

async fn run_git_command(args: Vec<&str>) -> Result<()> {
    run_git_command_capture(args).await.map(|_| ())
}

/// Read-only `git log`, capped at `max_count` (default 20), optionally scoped
/// to `path`. Shared verbatim by the orchestrator's structured tool dispatch
/// and the native `#[ollama_rs::function]` wrapper in `providers::llm::ollama::func`.
pub(crate) async fn git_log(path: Option<&str>, max_count: Option<u32>) -> Result<String> {
    let count_flag = format!("-n{}", max_count.unwrap_or(20));
    let mut args: Vec<&str> = vec!["log", "--oneline", count_flag.as_str()];
    if let Some(p) = path {
        args.push("--");
        args.push(p);
    }
    run_git_command_capture(args).await
}

/// Read-only `git blame --line-porcelain` for a single file.
pub(crate) async fn git_blame(path: &str) -> Result<String> {
    run_git_command_capture(vec!["blame", "--line-porcelain", path]).await
}

/// Read-only `git diff`, optionally `--staged` and/or scoped to `path`.
pub(crate) async fn git_diff(path: Option<&str>, staged: bool) -> Result<String> {
    let mut args: Vec<&str> = vec!["diff"];
    if staged {
        args.push("--staged");
    }
    if let Some(p) = path {
        args.push("--");
        args.push(p);
    }
    run_git_command_capture(args).await
}

/// Like `run_git_command` but returns captured stdout — used by read-only
/// tools where the output itself is the payload fed back to the LLM, unlike
/// `run_git_command`'s write-side commands where only success/failure matters.
async fn run_git_command_capture(args: Vec<&str>) -> Result<String> {
    let output = tokio::process::Command::new("git")
        .args(&args)
        .output()
        .await
        .map_err(|e| RuChatError::InternalError(format!("Git exec failed: {e}")))?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(RuChatError::InternalError(format!("Git error: {err}")));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Search commit history: `mode: "message"` uses `git log --grep`,
/// `mode: "content"` uses pickaxe search (`git log -S<pattern>`, commits
/// that added/removed occurrences of `pattern`). Read-only, same
/// `run_git_command_capture` plumbing as `git_log`/`git_diff`.
pub(crate) async fn git_search_history(
    pattern: &str,
    mode: &str,
    path: Option<&str>,
    max_count: Option<u32>,
) -> Result<String> {
    let count_flag = format!("-n{}", max_count.unwrap_or(20));
    let mut args: Vec<String> = vec!["log".into(), "--oneline".into(), count_flag];
    match mode {
        "content" => args.push(format!("-S{pattern}")),
        _ => args.push(format!("--grep={pattern}")),
    }
    if let Some(p) = path {
        args.push("--".into());
        args.push(p.into());
    }
    let args_ref: Vec<&str> = args.iter().map(String::as_str).collect();
    run_git_command_capture(args_ref).await
}

/// Returns the set of paths tracked by git in the current repo (i.e.
/// `git ls-files`). Used to gate any tool that writes to disk — apply_patch
/// must never touch a file outside version control (build artifacts,
/// .git internals, ignored files, paths outside the repo entirely).
pub(crate) async fn tracked_files() -> Result<std::collections::HashSet<String>> {
    let out = run_git_command_capture(vec!["ls-files"]).await?;
    Ok(out.lines().map(str::to_string).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::llm_client::FakeLlmClient;

    // Regression: `commit_add_targets` is the fix for `git add .` staging every dirty/
    // untracked file in the working tree onto the AI's feature branch, not just its own
    // change. `commit_feature_branch` itself isn't unit tested — it runs real `git`
    // commands against the process's actual working directory (this repo), so exercising it
    // directly in `cargo test` would mutate this repo's own git state.
    #[test]
    fn commit_add_targets_is_just_the_changelog_when_no_patch_was_applied() {
        let ctx = Context::new("goal".to_string());
        assert_eq!(commit_add_targets(&ctx), vec!["featured_changes.md"]);
    }

    #[test]
    fn commit_add_targets_includes_only_the_one_patched_file() {
        let mut ctx = Context::new("goal".to_string());
        ctx.record_patch("src/foo.rs".to_string(), "original content".to_string());
        assert_eq!(
            commit_add_targets(&ctx),
            vec!["featured_changes.md", "src/foo.rs"]
        );
    }

    #[test]
    fn fallback_commit_message_includes_the_changed_file_when_present() {
        let mut ctx = Context::new("fix the bug".to_string());
        ctx.record_patch("src/foo.rs".to_string(), "original".to_string());
        let msg = fallback_commit_message(&ctx);
        assert!(msg.contains("fix the bug"));
        assert!(msg.contains("src/foo.rs"));
    }

    #[test]
    fn fallback_commit_message_without_a_patch_still_names_the_goal() {
        let ctx = Context::new("fix the bug".to_string());
        assert_eq!(fallback_commit_message(&ctx), "AI: fix the bug");
    }

    #[tokio::test]
    async fn generate_commit_message_returns_the_trimmed_llm_response() {
        let ollama = FakeLlmClient::new(vec!["  Rephrase a comment for clarity  "]);
        let msg = generate_commit_message(&ollama, "fake", "goal", "some diff")
            .await
            .unwrap();
        assert_eq!(msg, "Rephrase a comment for clarity");
    }

    #[tokio::test]
    async fn generate_commit_message_rejects_an_empty_diff_without_calling_the_llm() {
        let ollama = FakeLlmClient::new(vec![]); // would panic if chat_stream were called
        let result = generate_commit_message(&ollama, "fake", "goal", "   ").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn generate_commit_message_errors_on_an_empty_llm_response() {
        let ollama = FakeLlmClient::new(vec![""]);
        let result = generate_commit_message(&ollama, "fake", "goal", "some diff").await;
        assert!(result.is_err());
    }

    #[test]
    fn wrap_line_leaves_short_lines_alone() {
        let line = "short line";
        assert_eq!(wrap_line(line), line);
    }

    #[test]
    fn wrap_line_breaks_long_lines_at_word_boundaries_under_80_chars() {
        let line = "This explains why the change was made in quite a bit more detail \
            than usual, spanning well past the eighty character line limit we want to enforce.";
        let wrapped = wrap_line(line);
        for l in wrapped.lines() {
            assert!(
                l.chars().count() <= MAX_COMMIT_LINE_LEN,
                "line exceeds {MAX_COMMIT_LINE_LEN} chars: {l:?} ({} chars)",
                l.chars().count()
            );
        }
        // Wrapping must not drop or reorder words.
        assert_eq!(
            wrapped.split_whitespace().collect::<Vec<_>>(),
            line.split_whitespace().collect::<Vec<_>>()
        );
    }

    #[test]
    fn wrap_commit_message_body_never_wraps_the_summary_line() {
        let long_summary = "A".repeat(100);
        let message = format!("{long_summary}\n\nsome body text");
        let wrapped = wrap_commit_message_body(&message);
        assert!(wrapped.starts_with(&long_summary));
    }

    #[test]
    fn wrap_commit_message_body_wraps_long_body_lines() {
        let body_line = "This explains why the change was made in quite a bit more detail \
            than usual, spanning well past the eighty character line limit we want to enforce.";
        let message = format!("Short summary\n\n{body_line}");
        let wrapped = wrap_commit_message_body(&message);
        let mut lines = wrapped.lines();
        assert_eq!(lines.next().unwrap(), "Short summary");
        for l in lines {
            assert!(l.chars().count() <= MAX_COMMIT_LINE_LEN);
        }
    }
}
