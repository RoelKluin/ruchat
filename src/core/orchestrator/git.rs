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
    let output = tokio::process::Command::new("git")
        .args(&args)
        .output()
        .await
        .map_err(|e| RuChatError::InternalError(format!("Git exec failed: {e}")))?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(RuChatError::InternalError(format!("Git error: {err}")));
    }
    Ok(())
}
