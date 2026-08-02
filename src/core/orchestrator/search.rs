use crate::{Result, RuChatError};

/// Shells to `rg` (ripgrep). Requires ripgrep on PATH — same "tool missing"
/// posture as `core::index::run_ctags_json`'s universal-ctags check.
pub(crate) async fn ripgrep(
    pattern: &str,
    path: Option<&str>,
    glob: Option<&str>,
    max_count: Option<u32>,
) -> Result<String> {
    let mut args = vec!["--line-number".to_string(), "--no-heading".to_string()];
    if let Some(g) = glob {
        args.push("--glob".into());
        args.push(g.to_string());
    }
    let count = max_count.unwrap_or(50).to_string();
    args.push("--max-count".into());
    args.push(count);
    args.push(pattern.to_string());
    if let Some(p) = path {
        args.push(p.to_string());
    }

    let output = tokio::process::Command::new("rg")
        .args(&args)
        .output()
        .await
        .map_err(|e| {
            RuChatError::InternalError(format!(
                "failed to spawn rg: {e} (is ripgrep installed and on PATH?)"
            ))
        })?;

    // rg exits 1 for "no matches" — not an error condition here.
    if !output.status.success() && output.status.code() != Some(1) {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(RuChatError::InternalError(format!("rg failed: {err}")));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Reads the repo-root `tags` file produced by `universal-ctags` (see
/// `core::index`), optionally filtered to lines mentioning `symbol`.
/// Does NOT regenerate the tags file — returns an actionable error if it's
/// missing rather than silently shelling out to ctags mid-agent-run.
pub(crate) async fn read_tags(symbol: Option<&str>) -> Result<String> {
    let path = "tags";
    if !std::path::Path::new(path).exists() {
        return Err(RuChatError::InternalError(
            "no 'tags' file at repo root — run `ctags -R .` first".into(),
        ));
    }
    let content = tokio::fs::read_to_string(path)
        .await
        .map_err(|e| RuChatError::InternalError(format!("read tags failed: {e}")))?;
    match symbol {
        None => Ok(content),
        Some(s) => Ok(content
            .lines()
            .filter(|l| l.contains(s))
            .collect::<Vec<_>>()
            .join("\n")),
    }
}
