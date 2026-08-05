use super::{Orchestrator, OrchestratorResult};
use crate::Result;
use crate::agent::tools::{self, ToolName};
use crate::agent::types::{Context, TurnKind};
use serde_json::Value;
use tokio::sync::mpsc;

/// Treats an explicit empty string the same as an omitted optional field.
/// Models reliably emit `"path": ""` instead of leaving an optional arg out
/// entirely, and downstream commands (e.g. `git log -- ""`) reject an empty
/// pathspec outright rather than treating it as "no restriction" — this
/// normalizes that before it ever reaches them.
fn opt_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
}

impl Orchestrator {
    /// Dispatches a validated structured tool call from `Stage::Implement`.
    /// Only the read-only tools reach here; `Memorize`/`ApplyPatch` are
    /// handled later by `Agent::execute_and_verify` since they mutate state
    /// tied to the agent's own config, not the orchestrator's.
    pub(super) async fn handle_structured_tool(
        &mut self,
        call: &tools::StructuredToolCall,
        ctx: &mut Context,
        tx: &mpsc::Sender<OrchestratorResult>,
    ) -> Result<()> {
        // Exactly one arm below actually runs per call, so timing the whole match covers
        // whichever tool this dispatch is — for --trace-timings, "how long did this specific
        // tool call take" (`Retrieve` excepted: `handle_retrieve` times itself, since it's a
        // whole RAG round trip, not a single subprocess/filesystem call like the rest here).
        let tool_start = std::time::Instant::now();
        match call.tool {
            ToolName::Retrieve => {
                let query = call.args["query"].as_str().unwrap_or_default();
                self.handle_retrieve(query, ctx, tx).await
            }
            ToolName::GitLog => {
                let path = opt_str(&call.args, "path");
                let max_count = call
                    .args
                    .get("max_count")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as u32);
                let out = super::git::git_log(path, max_count).await?;
                ctx.push_turn_timed(TurnKind::Retrieval, "GitLog", out, tool_start);
                Ok(())
            }
            ToolName::GitBlame => {
                let path = call
                    .args
                    .get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let out = super::git::git_blame(path).await?;
                ctx.push_turn_timed(TurnKind::Retrieval, "GitBlame", out, tool_start);
                Ok(())
            }
            ToolName::GitDiff => {
                let path = opt_str(&call.args, "path");
                let staged = call
                    .args
                    .get("staged")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let out = super::git::git_diff(path, staged).await?;
                ctx.push_turn_timed(TurnKind::Retrieval, "GitDiff", out, tool_start);
                Ok(())
            }
            ToolName::GitSearchHistory => {
                let pattern = call.args["pattern"].as_str().unwrap_or_default();
                let mode = call.args["mode"].as_str().unwrap_or("message");
                let path = opt_str(&call.args, "path");
                let max_count = call
                    .args
                    .get("max_count")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as u32);
                let out = super::git::git_search_history(pattern, mode, path, max_count).await?;
                ctx.push_turn_timed(TurnKind::Retrieval, "GitSearchHistory", out, tool_start);
                Ok(())
            }
            ToolName::ReadFile => {
                let path = call.args["path"].as_str().unwrap_or_default();
                let start = call
                    .args
                    .get("start")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as u32);
                let end = call
                    .args
                    .get("end")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as u32);
                let out = crate::orchestrator::fs::read_file(path, start, end).await?;
                ctx.push_turn_timed(TurnKind::Retrieval, "ReadFile", out, tool_start);
                Ok(())
            }
            ToolName::ListDir => {
                let path = call.args["path"].as_str().unwrap_or_default();
                let out = crate::orchestrator::fs::list_dir(path).await?;
                ctx.push_turn_timed(TurnKind::Retrieval, "ListDir", out, tool_start);
                Ok(())
            }
            ToolName::Ripgrep => {
                let pattern = call.args["pattern"].as_str().unwrap_or_default();
                let path = opt_str(&call.args, "path");
                let glob = opt_str(&call.args, "glob");
                let max_count = call
                    .args
                    .get("max_count")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as u32);
                let context = call
                    .args
                    .get("context")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as u32);
                let out =
                    crate::orchestrator::search::ripgrep(pattern, path, glob, max_count, context)
                        .await?;
                ctx.push_turn_timed(TurnKind::Retrieval, "Ripgrep", out, tool_start);
                Ok(())
            }
            ToolName::ReadTags => {
                let symbol = opt_str(&call.args, "symbol");
                let out = crate::orchestrator::search::read_tags(symbol).await?;
                ctx.push_turn_timed(TurnKind::Retrieval, "ReadTags", out, tool_start);
                Ok(())
            }
            ToolName::CargoCheck => {
                let out = crate::orchestrator::cargo::cargo_check().await?;
                ctx.push_turn_timed(TurnKind::Retrieval, "CargoCheck", out, tool_start);
                Ok(())
            }
            ToolName::CargoClippy => {
                let out = crate::orchestrator::cargo::cargo_clippy().await?;
                ctx.push_turn_timed(TurnKind::Retrieval, "CargoClippy", out, tool_start);
                Ok(())
            }
            ToolName::CargoDupes => {
                let out = crate::orchestrator::cargo::cargo_dupes().await?;
                ctx.push_turn_timed(TurnKind::Retrieval, "CargoDupes", out, tool_start);
                Ok(())
            }
            ToolName::Memorize | ToolName::ApplyPatch => Ok(()),
        }
    }
}
