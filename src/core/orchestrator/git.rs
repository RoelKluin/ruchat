use crate::agent::types::Context;
use crate::{Result, RuChatError};
use log::info;

pub(crate) async fn commit_feature_branch(ctx: &Context) -> Result<()> {
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

        run_git_command(vec!["add", "."]).await?;
        run_git_command(vec!["commit", "-m", &format!("AI Success: {}", ctx.goal)]).await?;
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
