use crate::{Result, RuChatError};
use std::path::Path;

const MAX_READ_BYTES: usize = 32_000; // keep a single read from blowing the prompt budget

/// Read-only file read, optionally sliced by 1-indexed inclusive line range.
/// Rejects paths that escape the current working directory (repo root) —
/// the Worker's tool_call arguments are model output, same untrusted-input
/// posture as retrieved document content elsewhere in this crate.
pub(crate) async fn read_file(path: &str, start: Option<u32>, end: Option<u32>) -> Result<String> {
    let cwd = std::env::current_dir().map_err(|e| RuChatError::InternalError(e.to_string()))?;
    let target = cwd.join(path);
    let canonical = target
        .canonicalize()
        .map_err(|e| RuChatError::InternalError(format!("cannot resolve {path}: {e}")))?;
    if !canonical.starts_with(&cwd) {
        return Err(RuChatError::InternalError(format!(
            "path {path} escapes the repository root — refused"
        )));
    }
    let content = tokio::fs::read_to_string(&canonical)
        .await
        .map_err(|e| RuChatError::InternalError(format!("read {path} failed: {e}")))?;

    let sliced = match (start, end) {
        (None, None) => content,
        (s, e) => {
            let lines: Vec<&str> = content.lines().collect();
            let s = s.unwrap_or(1).max(1) as usize - 1;
            let e = (e.unwrap_or(lines.len() as u32) as usize).min(lines.len());
            lines.get(s..e).unwrap_or(&[]).join("\n")
        }
    };
    if sliced.len() > MAX_READ_BYTES {
        Ok(format!(
            "{}\n\n[TRUNCATED at {MAX_READ_BYTES} bytes — request a narrower start/end range]",
            &sliced[..MAX_READ_BYTES]
        ))
    } else {
        Ok(sliced)
    }
}

/// Non-recursive directory listing (files and subdirs, one level).
pub(crate) async fn list_dir(path: &str) -> Result<String> {
    let cwd = std::env::current_dir().map_err(|e| RuChatError::InternalError(e.to_string()))?;
    let target = cwd.join(path);
    let canonical = target
        .canonicalize()
        .map_err(|e| RuChatError::InternalError(format!("cannot resolve {path}: {e}")))?;
    if !canonical.starts_with(&cwd) {
        return Err(RuChatError::InternalError(format!(
            "path {path} escapes the repository root — refused"
        )));
    }
    let mut entries = tokio::fs::read_dir(&canonical)
        .await
        .map_err(|e| RuChatError::InternalError(format!("list {path} failed: {e}")))?;
    let mut out = Vec::new();
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| RuChatError::InternalError(e.to_string()))?
    {
        let marker = if entry.path().is_dir() { "/" } else { "" };
        out.push(format!("{}{marker}", entry.file_name().to_string_lossy()));
    }
    out.sort();
    Ok(out.join("\n"))
}
