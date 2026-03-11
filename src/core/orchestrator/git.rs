use crate::agent::types::Context;
use crate::{Result, RuChatError};
use log::info;

pub(crate) async fn commit_feature_branch(ctx: &Context) -> Result<()> {
    // 1. Sanitize Branch Name
    let timestamp = chrono::Utc::now().timestamp();
    let branch_name = format!("ai/feature-{}", timestamp);
    let goal = ctx.get_goal();

    // 2. Prepare the Summary Entry
    let summary_entry = format!(
        "\n--- \n### 🤖 AI Update: {}\n**Date:** {}\n**Goal:** {}\n**Changes:** \n{}\n",
        branch_name,
        chrono::Utc::now().to_rfc2822(),
        goal,
        ctx.output.lines().take(5).collect::<Vec<_>>().join("\n") // Take first 5 lines of worker output as summary
    );

    // 3. Execution Sequence
    // We use a helper to run commands and check status
    // Create branch and switch
    run_git_command(vec!["checkout", "-b", &branch_name]).await?;

    // Append to featured_changes.md
    let mut file = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("featured_changes.md")
        .await?;

    tokio::io::AsyncWriteExt::write_all(&mut file, summary_entry.as_bytes()).await?;

    // Finalize Git sequence
    run_git_command(vec!["add", "."]).await?;
    run_git_command(vec!["commit", "-m", &format!("AI Success: {}", goal)]).await?;
    run_git_command(vec!["checkout", "-"]).await?; // Return to main

    info!(
        "🚀 Changes logged in featured_changes.md and committed to {}",
        branch_name
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
