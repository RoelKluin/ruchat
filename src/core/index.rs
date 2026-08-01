use crate::core::embed::{EmbedArgs, UpsertMode};
use crate::{Result, RuChatError};
use chroma::types::UpdateMetadataValue;
use clap::Parser;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::process::Command;

/// Recursively walks `root`, collecting files whose extension is in `exts`.
/// Skips common non-source noise directories outright. Not async — directory
/// walking of typical repo sizes is fast enough that blocking briefly inside
/// an async fn is an acceptable trade-off here; move to `spawn_blocking` if
/// profiling shows otherwise on very large trees.
fn walk_files(root: &Path, exts: &[&str], out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if !root.is_dir() {
        return Ok(());
    }
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if path.is_dir() {
            if matches!(
                file_name,
                ".git" | "target" | "node_modules" | ".venv" | "dist" | "build"
            ) {
                continue;
            }
            walk_files(&path, exts, out)?;
        } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if exts.contains(&ext) {
                out.push(path);
            }
        }
    }
    Ok(())
}

fn language_for_ext(ext: &str) -> &'static str {
    match ext {
        "rs" => "rust",
        "py" => "python",
        "js" => "javascript",
        "ts" => "typescript",
        "go" => "go",
        "c" => "c",
        "cpp" | "cc" | "cxx" => "cpp",
        "h" | "hpp" => "c-header",
        _ => "unknown",
    }
}

/// Invokes `universal-ctags` in line-delimited JSON mode and returns the
/// parsed `"tag"`-kind lines (pseudo-tag/header lines are discarded).
/// Requires `ctags --version` to report "Universal Ctags" — legacy
/// exuberant-ctags does not support `--output-format=json`.
async fn run_ctags_json(path: &Path) -> Result<Vec<Value>> {
    let path_str = path
        .to_str()
        .ok_or_else(|| RuChatError::InternalError(format!("non-utf8 path: {path:?}")))?;

    let output = Command::new("ctags")
        .args([
            "--output-format=json",
            "--fields=+n+e+S+z",
            "-f",
            "-",
            path_str,
        ])
        .output()
        .await
        .map_err(|e| {
            RuChatError::InternalError(format!(
                "failed to spawn ctags: {e} (is universal-ctags installed and on PATH?)"
            ))
        })?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(RuChatError::InternalError(format!(
            "ctags failed on {path:?}: {err}"
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut tags = Vec::new();
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<Value>(line) {
            Ok(v) if v.get("_type").and_then(|t| t.as_str()) == Some("tag") => tags.push(v),
            Ok(_) => {} // pseudo-tag / header line — not a symbol entry
            Err(e) => {
                tracing::warn!(?path, error = %e, "skipping unparseable ctags output line");
            }
        }
    }
    Ok(tags)
}

/// Builds per-symbol metadata items matching `db_config.json`'s `repo_src`
/// schema (`file`, `language`, `name`, `kind`, `start`, `end`, plus
/// pass-through of `signature`/`scope`/`access` when ctags reports them).
///
/// `end` uses ctags' own field when present; otherwise it's approximated as
/// "up to the line before the next symbol starts" (or end-of-file for the
/// last symbol). This is a heuristic boundary, not a real AST scope — it
/// will misattribute trailing doc-comments/attributes that belong to the
/// *next* item, and ctags flattens nested items (e.g. methods inside an
/// `impl` block) rather than nesting them, so overlapping ranges are
/// possible. Good enough for RAG-style chunking; not a substitute for a real
/// parser if precise scoping matters later.
fn build_symbol_metadata(
    mut tags: Vec<Value>,
    file_rel: &str,
    language: &str,
    total_lines: usize,
) -> Vec<HashMap<String, UpdateMetadataValue>> {
    tags.sort_by_key(|t| t.get("line").and_then(|v| v.as_u64()).unwrap_or(0));
    let starts: Vec<u64> = tags
        .iter()
        .map(|t| t.get("line").and_then(|v| v.as_u64()).unwrap_or(1))
        .collect();

    tags.iter()
        .enumerate()
        .map(|(i, t)| {
            let name = t
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let kind = t
                .get("kind")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let start = starts[i].max(1);
            let end = t.get("end").and_then(|v| v.as_u64()).unwrap_or_else(|| {
                starts
                    .get(i + 1)
                    .map(|n| n.saturating_sub(1).max(start))
                    .unwrap_or(total_lines as u64)
            });

            let mut m: HashMap<String, UpdateMetadataValue> = HashMap::new();
            m.insert(
                "file".into(),
                UpdateMetadataValue::Str(file_rel.to_string()),
            );
            m.insert(
                "language".into(),
                UpdateMetadataValue::Str(language.to_string()),
            );
            m.insert("name".into(), UpdateMetadataValue::Str(name));
            m.insert("kind".into(), UpdateMetadataValue::Str(kind));
            m.insert("start".into(), UpdateMetadataValue::Int(start as i64));
            m.insert(
                "end".into(),
                UpdateMetadataValue::Int(end.max(start) as i64),
            );
            if let Some(sig) = t.get("signature").and_then(|v| v.as_str()) {
                m.insert(
                    "signature".into(),
                    UpdateMetadataValue::Str(sig.to_string()),
                );
            }
            if let Some(scope) = t.get("scope").and_then(|v| v.as_str()) {
                m.insert("scope".into(), UpdateMetadataValue::Str(scope.to_string()));
            }
            if let Some(access) = t.get("access").and_then(|v| v.as_str()) {
                m.insert(
                    "access".into(),
                    UpdateMetadataValue::Str(access.to_string()),
                );
            }
            m
        })
        .collect()
}

fn word_occurrences(haystack: &str, word: &str) -> usize {
    haystack
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|tok| *tok == word)
        .count()
}

/// Naive, non-scalable reference counting: whole-word occurrence of each
/// symbol's name across every indexed file (excluding nothing — it will
/// also match the definition line itself). This is NOT a call graph: it
/// over-counts unrelated identifiers sharing a name and under-counts
/// aliased/re-exported symbols. It exists purely to populate the
/// `references` metadata field described in `db_config.json` with
/// *something* usable for `references CONTAINS 'x'`-style queries.
/// O(symbols × total_file_bytes) — capped per-symbol at 50 matching files;
/// only enable (`--with-references`) on small-to-medium repos. Replace with
/// real cross-referencing (rust-analyzer/LSP or a language-specific
/// call-graph tool) when available.
///
/// NEEDS VERIFICATION: assumes `UpdateMetadataValue` has an array/list
/// variant analogous to `MetadataValue::StringArray` (seen in
/// `chroma/where.rs`). If `chroma-types` doesn't expose one for
/// `UpdateMetadataValue`, join into a delimited string instead:
/// `UpdateMetadataValue::Str(refs.join(","))`.
fn attach_reference_counts(
    items: &mut [HashMap<String, UpdateMetadataValue>],
    all_file_texts: &HashMap<String, String>,
) {
    for item in items.iter_mut() {
        let name = match item.get("name") {
            Some(UpdateMetadataValue::Str(n)) if !n.is_empty() => n.clone(),
            _ => continue,
        };
        let mut refs: Vec<String> = Vec::new();
        for (file, text) in all_file_texts {
            if word_occurrences(text, &name) > 0 {
                refs.push(file.clone());
            }
            if refs.len() >= 50 {
                break;
            }
        }
        item.insert("references".into(), UpdateMetadataValue::StringArray(refs));
    }
}

#[derive(Parser, Debug, Clone, PartialEq)]
pub(crate) struct IndexArgs {
    /// Root directory to recursively index.
    path: PathBuf,

    /// File extensions to index (comma-separated, no dot). Defaults to a
    /// conservative set of languages universal-ctags reliably supports.
    #[arg(
        long,
        value_delimiter = ',',
        default_value = "rs,py,js,ts,go,c,cpp,h,hpp"
    )]
    ext: Vec<String>,

    /// Also compute the naive cross-file reference counts described above.
    /// Off by default — expensive on large repos.
    #[arg(long, default_value_t = false)]
    with_references: bool,

    #[command(flatten)]
    embed_args: EmbedArgs,

    #[arg(long, default_value = "upsert")]
    mode: UpsertMode,
}

impl IndexArgs {
    pub(crate) async fn run(&self, cfg: &Value) -> Result<()> {
        let mut files = Vec::new();
        let exts: Vec<&str> = self.ext.iter().map(String::as_str).collect();
        walk_files(&self.path, &exts, &mut files)
            .map_err(|e| RuChatError::InternalError(format!("walk failed: {e}")))?;

        if files.is_empty() {
            return Err(RuChatError::Is(format!(
                "no files with extensions {exts:?} found under {:?}",
                self.path
            )));
        }

        let mut all_texts = HashMap::new();
        let mut per_file: Vec<(String, String, Vec<HashMap<String, UpdateMetadataValue>>)> =
            Vec::new();

        for path in &files {
            let rel = path
                .strip_prefix(&self.path)
                .unwrap_or(path)
                .to_string_lossy()
                .into_owned();

            let text = match tokio::fs::read_to_string(path).await {
                Ok(t) => t,
                Err(e) => {
                    tracing::warn!(?path, error = %e, "skipping unreadable file (likely binary)");
                    continue;
                }
            };

            let language =
                language_for_ext(path.extension().and_then(|e| e.to_str()).unwrap_or(""));
            let total_lines = text.lines().count();

            let tags = match run_ctags_json(path).await {
                Ok(t) => t,
                Err(e) => {
                    tracing::warn!(?path, error = %e, "ctags failed, embedding whole-file only");
                    Vec::new()
                }
            };

            // Empty metadata_items falls back to whole-file embedding —
            // `EmbedArgs::embed`'s existing `metadata_items.len() < 2` branch
            // already handles this without any special-casing here.
            let items = if tags.is_empty() {
                Vec::new()
            } else {
                build_symbol_metadata(tags, &rel, language, total_lines)
            };

            all_texts.insert(rel.clone(), text.clone());
            per_file.push((rel, text, items));
        }

        for (rel, text, mut items) in per_file {
            if self.with_references {
                attach_reference_counts(&mut items, &all_texts);
            }
            let mut args = self.embed_args.clone();
            args.set_id_prefix(rel);
            args.embed_with_metadata_items(&text, self.mode, cfg, items)
                .await?;
        }

        Ok(())
    }
}
