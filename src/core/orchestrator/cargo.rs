use crate::{Result, RuChatError};
use std::time::Duration;
use tokio::process::Command;

/// Read-only compile check, reusing the same 30s-timeout pattern as
/// `protocol::Validation::run_cargo_check`. Distinct call site: this is a
/// Worker-invoked, on-demand inspection (no rejection/turn semantics),
/// whereas `Validation::run_build_and_test` is the automatic `Stage::Test`
/// gate. Kept as two functions rather than merged to avoid coupling the
/// Tester's rejection flow to what the Worker sees mid-Implement.
pub(crate) async fn cargo_check() -> Result<String> {
    let output = tokio::time::timeout(
        Duration::from_secs(30),
        Command::new("cargo").args(["check", "--message-format=short"]).output(),
    )
    .await
    .map_err(|_| RuChatError::InternalError("cargo check timed out after 30s".into()))?
    .map_err(|e| RuChatError::InternalError(format!("cargo check failed to run: {e}")))?;
    Ok(format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    ))
}

/// `cargo tree --duplicates` — lists dependency versions duplicated in the
/// resolved graph. Fast, read-only, no timeout needed beyond a generous cap.
pub(crate) async fn cargo_dupes() -> Result<String> {
    let output = tokio::time::timeout(
        Duration::from_secs(20),
        Command::new("cargo").args(["tree", "--duplicates"]).output(),
    )
    .await
    .map_err(|_| RuChatError::InternalError("cargo tree timed out after 20s".into()))?
    .map_err(|e| RuChatError::InternalError(format!("cargo tree failed to run: {e}")))?;
    if !output.status.success() {
        return Err(RuChatError::InternalError(
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    if stdout.trim().is_empty() {
        Ok("No duplicate dependency versions found.".into())
    } else {
        Ok(stdout.into_owned())
    }
}
