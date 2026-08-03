use crate::core::embed::{EmbedArgs, UpsertMode};
use crate::{Result, RuChatError};
use chroma::types::UpdateMetadataValue;
use clap::Parser;
use futures_util::stream::{self, StreamExt, TryStreamExt};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::process::Command;

/// Bound on concurrent `ctags` subprocess spawns and file reads during indexing — cheap,
/// independent, CPU-bound work per file, safe to parallelize aggressively; bounded rather than
/// unbounded purely to avoid exhausting file descriptors/process slots on a very large repo.
const CTAGS_CONCURRENCY: usize = 8;

/// Bound on concurrent per-file embed+upsert round trips. Deliberately lower than
/// `CTAGS_CONCURRENCY`: Ollama typically serializes inference per loaded model, so this mostly
/// overlaps HTTP/network round-trip latency across files rather than multiplying raw embedding
/// throughput — a large number here would just queue up requests Ollama processes one at a
/// time anyway, with no benefit and more memory held per in-flight file.
const EMBED_CONCURRENCY: usize = 4;

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
        } else if let Some(ext) = path.extension().and_then(|e| e.to_str())
            && exts.contains(&ext) {
                out.push(path);
            }
    }
    Ok(())
}

/// Prefers `git ls-files` (scoped to `root`, filtered by `exts`) over `walk_files`'s raw
/// recursive walk whenever `root` is inside a git work tree — matches the precedent in
/// `orchestrator::search::tracked_rust_files`: a real, severe bug there came from exactly this
/// class of unscoped recursive walk sweeping in a large, gitignored-but-physically-present
/// directory (`docs/`) that a plain directory walk has no way to know isn't meant to be
/// indexed. `None` means `root` isn't a git work tree (or `git` isn't on PATH) — the caller
/// falls back to `walk_files` in that case, preserving the old behavior for non-repo use.
/// `Some(vec![])` is a legitimate answer (a real git repo with zero matching files), distinct
/// from `None`.
async fn tracked_files_under(root: &Path, exts: &[&str]) -> Option<Vec<PathBuf>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files"])
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(|line| root.join(line))
            .filter(|p| {
                p.extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| exts.contains(&e))
            })
            .collect(),
    )
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
        // Not in the default `--ext` list (still a conservative code-language set), but
        // recognized so an explicit `--ext md,txt,...` gets a correct `language` metadata value
        // instead of "unknown". `md` has real ctags support (heading-based chapter/section/
        // subsection tags), so it chunks via the existing `build_symbol_metadata` path just like
        // code; `txt` has none, so it always falls through to `chunk_by_paragraph` below.
        "md" | "markdown" => "markdown",
        "txt" => "text",
        _ => "unknown",
    }
}

/// Target size (in lines) for each paragraph-based chunk — see `chunk_by_paragraph`. A chunk
/// only ever ends at a paragraph boundary, never mid-paragraph, so this is a target, not a hard
/// cap: a single very long paragraph can still exceed it. Same order of magnitude as a typical
/// ctags symbol chunk, not an arbitrary number.
const PARAGRAPH_CHUNK_TARGET_LINES: usize = 40;

/// Below this many lines, `chunk_by_paragraph` returns no chunks at all, deferring to
/// `EmbedArgs::embed`'s existing "empty metadata_items -> whole-file embedding" fallback — a
/// short file doesn't need splitting, and one embedding for the whole thing is both simpler and
/// more coherent than an arbitrary partial chunk.
const MIN_LINES_FOR_PARAGRAPH_CHUNKING: usize = PARAGRAPH_CHUNK_TARGET_LINES;

/// Real semantic/document-aware chunking for content ctags can't (or didn't) tag with symbols —
/// unsupported languages, or a supported one where ctags legitimately found nothing. Previously
/// any such file fell all the way back to one whole-file embedding regardless of size, which is
/// fine for a short file but hurts retrieval precision for a long one (a large prose document or
/// plain-text file becomes a single unfocused embedding covering everything in it at once).
///
/// Splits on blank-line-separated paragraphs (tracking real 1-based start/end line numbers per
/// paragraph), then greedily merges consecutive paragraphs into chunks of around
/// `PARAGRAPH_CHUNK_TARGET_LINES` lines each. Deliberately never splits *inside* a paragraph —
/// prose/doc structure isn't cut mid-sentence — so an individual very long paragraph can still
/// produce an oversized chunk; that's an accepted trade-off for staying document-structure-aware
/// rather than a fixed-size sliding window that would ignore it entirely.
///
/// Returns metadata items in the same shape `build_symbol_metadata` produces (`file`/`language`/
/// `name`/`kind`/`start`/`end`), just `kind: "paragraph"` instead of a ctags symbol kind, so
/// existing `WHERE` queries (`kind = '...'`, numeric `start`/`end` comparisons) keep working
/// uniformly across both chunking strategies. `EmbedArgs::embed_with_metadata_items` slices the
/// actual chunk text out of the file using these same `start`/`end` fields — no separate text
/// extraction needed here, exactly like the existing ctags-chunk path.
fn chunk_by_paragraph(
    text: &str,
    file_rel: &str,
    language: &str,
) -> Vec<HashMap<String, UpdateMetadataValue>> {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() < MIN_LINES_FOR_PARAGRAPH_CHUNKING {
        return Vec::new();
    }

    let mut paragraphs: Vec<(usize, usize)> = Vec::new(); // (start_line, end_line), 1-based inclusive
    let mut para_start: Option<usize> = None;
    for (i, line) in lines.iter().enumerate() {
        let line_no = i + 1;
        if line.trim().is_empty() {
            if let Some(start) = para_start.take() {
                paragraphs.push((start, line_no - 1));
            }
        } else if para_start.is_none() {
            para_start = Some(line_no);
        }
    }
    if let Some(start) = para_start {
        paragraphs.push((start, lines.len()));
    }

    let mut chunks: Vec<(usize, usize)> = Vec::new();
    let mut cur_start: Option<usize> = None;
    let mut cur_end = 0;
    for (p_start, p_end) in paragraphs {
        if cur_start.is_none() {
            cur_start = Some(p_start);
        }
        cur_end = p_end;
        if cur_end - cur_start.unwrap_or(cur_end) + 1 >= PARAGRAPH_CHUNK_TARGET_LINES {
            chunks.push((cur_start.take().unwrap_or(cur_end), cur_end));
        }
    }
    if let Some(start) = cur_start {
        chunks.push((start, cur_end));
    }

    chunks
        .into_iter()
        .enumerate()
        .map(|(i, (start, end))| {
            let mut m: HashMap<String, UpdateMetadataValue> = HashMap::new();
            m.insert("file".into(), UpdateMetadataValue::Str(file_rel.to_string()));
            m.insert(
                "language".into(),
                UpdateMetadataValue::Str(language.to_string()),
            );
            m.insert(
                "name".into(),
                UpdateMetadataValue::Str(format!("paragraph_{i}")),
            );
            m.insert(
                "kind".into(),
                UpdateMetadataValue::Str("paragraph".to_string()),
            );
            m.insert("start".into(), UpdateMetadataValue::Int(start as i64));
            m.insert("end".into(), UpdateMetadataValue::Int(end.max(start) as i64));
            m
        })
        .collect()
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
        let exts: Vec<&str> = self.ext.iter().map(String::as_str).collect();
        let files = match tracked_files_under(&self.path, &exts).await {
            Some(tracked) => tracked,
            None => {
                let mut walked = Vec::new();
                walk_files(&self.path, &exts, &mut walked)
                    .map_err(|e| RuChatError::InternalError(format!("walk failed: {e}")))?;
                walked
            }
        };

        if files.is_empty() {
            return Err(RuChatError::Is(format!(
                "no files with extensions {exts:?} found under {:?}",
                self.path
            )));
        }

        // Reading + ctags-tagging each file is independent, CPU-bound work — run up to
        // `CTAGS_CONCURRENCY` at once instead of one file at a time. `buffer_unordered` doesn't
        // preserve file order, which is fine here: `all_texts` is a map and the embed phase
        // below doesn't care what order files were tagged in.
        let root = &self.path;
        let tagged: Vec<(String, String, Vec<HashMap<String, UpdateMetadataValue>>)> =
            stream::iter(files.iter())
                .map(|path| async move {
                    let rel = path
                        .strip_prefix(root)
                        .unwrap_or(path)
                        .to_string_lossy()
                        .into_owned();

                    let text = match tokio::fs::read_to_string(path).await {
                        Ok(t) => t,
                        Err(e) => {
                            tracing::warn!(
                                ?path,
                                error = %e,
                                "skipping unreadable file (likely binary)"
                            );
                            return None;
                        }
                    };

                    let language = language_for_ext(
                        path.extension().and_then(|e| e.to_str()).unwrap_or(""),
                    );
                    let total_lines = text.lines().count();

                    let tags = match run_ctags_json(path).await {
                        Ok(t) => t,
                        Err(e) => {
                            tracing::warn!(
                                ?path,
                                error = %e,
                                "ctags failed, embedding whole-file only"
                            );
                            Vec::new()
                        }
                    };

                    // No ctags symbols (unsupported language, or a supported one where ctags
                    // found nothing): fall back to real paragraph-based chunking for a file
                    // large enough to benefit from it, rather than always embedding the whole
                    // file as one chunk. `chunk_by_paragraph` itself returns empty for a short
                    // file, at which point `EmbedArgs::embed`'s existing `metadata_items.len() <
                    // 2` branch takes over and embeds the whole thing as one chunk anyway — no
                    // special-casing needed here either way.
                    let items = if tags.is_empty() {
                        chunk_by_paragraph(&text, &rel, language)
                    } else {
                        build_symbol_metadata(tags, &rel, language, total_lines)
                    };

                    Some((rel, text, items))
                })
                .buffer_unordered(CTAGS_CONCURRENCY)
                .filter_map(|x| async move { x })
                .collect()
                .await;

        let mut all_texts = HashMap::new();
        for (rel, text, _) in &tagged {
            all_texts.insert(rel.clone(), text.clone());
        }

        // Embedding is the network/Ollama-bound half — bounded lower than the ctags phase (see
        // `EMBED_CONCURRENCY`'s doc comment). `try_for_each_concurrent` keeps the existing
        // stop-on-first-error behavior (a single failed file still aborts the whole index run)
        // instead of silently continuing past a failure just because it's now concurrent.
        stream::iter(tagged.into_iter().map(Ok::<_, RuChatError>))
            .try_for_each_concurrent(EMBED_CONCURRENCY, |(rel, text, mut items)| {
                let all_texts = &all_texts;
                async move {
                    if self.with_references {
                        attach_reference_counts(&mut items, all_texts);
                    }
                    let mut args = self.embed_args.clone();
                    args.set_id_prefix(rel);
                    args.embed_with_metadata_items(&text, self.mode, cfg, items)
                        .await
                }
            })
            .await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Regression: `IndexArgs::run` used to always call `walk_files` — a raw recursive
    // directory walk with only a small hardcoded skip-list (.git/target/node_modules/.venv/
    // dist/build) — regardless of whether `path` was a git work tree. That's the exact class
    // of bug already found and fixed for the ctags-freshness path
    // (`orchestrator::search::regenerate_tags`): a recursive walk has no way to know a large,
    // gitignored-but-physically-present directory (this repo's own `docs/`) isn't meant to be
    // indexed. `tracked_files_under` prefers `git ls-files` when available, so pointing
    // `ruchat index` at this repo's root must never pull in `docs/`.
    #[tokio::test]
    async fn tracked_files_under_this_repo_never_includes_docs() {
        let root = Path::new(".");
        let exts = ["rs"];
        let files = tracked_files_under(root, &exts)
            .await
            .expect("this repo is a git work tree");
        assert!(!files.is_empty(), "this repo has real .rs files to find");
        for f in &files {
            let s = f.to_string_lossy();
            assert!(s.ends_with(".rs"), "non-.rs path returned: {s}");
            assert!(!s.contains("docs/"), "docs/ path leaked in: {s}");
        }
    }

    #[tokio::test]
    async fn tracked_files_under_returns_none_outside_a_git_work_tree() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("foo.rs"), "fn main() {}").unwrap();
        assert!(tracked_files_under(dir.path(), &["rs"]).await.is_none());
    }

    #[test]
    fn language_for_ext_recognizes_markdown_and_plain_text() {
        // Not in the default `--ext` list, but must report a real language (not "unknown") for
        // an explicit `--ext md,txt,...` run — see `chunk_by_paragraph`'s doc comment.
        assert_eq!(language_for_ext("md"), "markdown");
        assert_eq!(language_for_ext("txt"), "text");
    }

    // Builds a fixture large enough to exceed `MIN_LINES_FOR_PARAGRAPH_CHUNKING`: 5 paragraphs
    // of 10 lines each, separated by single blank lines (54 lines total). Line numbers are
    // deliberately traced in the doc comment above `chunk_by_paragraph`'s call site so this
    // test's expected boundaries aren't just "whatever the code currently does."
    fn paragraph_fixture() -> String {
        (1..=5)
            .map(|p| {
                (1..=10)
                    .map(|l| format!("p{p}l{l}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    #[test]
    fn chunk_by_paragraph_merges_paragraphs_up_to_the_target_size_without_splitting_one() {
        let text = paragraph_fixture();
        let items = chunk_by_paragraph(&text, "notes.txt", "text");

        // Paragraphs sit at lines 1-10, 12-21, 23-32, 34-43, 45-54 (blank lines at 11/22/33/44).
        // Greedily merging until >= 40 lines: paragraphs 1-4 merge into one 43-line chunk
        // (1..=10 is 10 lines, plus each subsequent paragraph's own span brings the running
        // total to 43 by paragraph 4), then paragraph 5 alone becomes the final chunk — the
        // boundary lands exactly on paragraph 4's end/paragraph 5's start, never mid-paragraph.
        let bounds: Vec<(i64, i64)> = items
            .iter()
            .map(|m| {
                let start = match m.get("start") {
                    Some(UpdateMetadataValue::Int(n)) => *n,
                    other => panic!("expected an Int start, got {other:?}"),
                };
                let end = match m.get("end") {
                    Some(UpdateMetadataValue::Int(n)) => *n,
                    other => panic!("expected an Int end, got {other:?}"),
                };
                (start, end)
            })
            .collect();
        assert_eq!(bounds, vec![(1, 43), (45, 54)]);

        for m in &items {
            assert_eq!(m.get("kind"), Some(&UpdateMetadataValue::Str("paragraph".into())));
            assert_eq!(m.get("file"), Some(&UpdateMetadataValue::Str("notes.txt".into())));
            assert_eq!(m.get("language"), Some(&UpdateMetadataValue::Str("text".into())));
        }
    }

    #[test]
    fn chunk_by_paragraph_returns_nothing_for_a_short_file() {
        // Below MIN_LINES_FOR_PARAGRAPH_CHUNKING — defers to EmbedArgs::embed's own
        // whole-file-as-one-chunk fallback (empty metadata_items) rather than producing a
        // pointless single paragraph chunk identical to what that fallback already does.
        let text = "line one\n\nline two\nline three";
        assert!(chunk_by_paragraph(text, "short.txt", "text").is_empty());
    }

    #[test]
    fn chunk_by_paragraph_chunk_bounds_slice_back_real_paragraph_text() {
        // Sanity check that the 1-based inclusive start/end this function produces actually
        // line up with `EmbedArgs::embed_with_metadata_items`'s own slicing math
        // (`line_pool[start-1..end]`), not just with this function's own internal bookkeeping.
        let text = paragraph_fixture();
        let lines: Vec<&str> = text.lines().collect();
        let items = chunk_by_paragraph(&text, "notes.txt", "text");
        let (start, end) = match (items[1].get("start"), items[1].get("end")) {
            (Some(UpdateMetadataValue::Int(s)), Some(UpdateMetadataValue::Int(e))) => {
                (*s as usize, *e as usize)
            }
            other => panic!("expected Int start/end, got {other:?}"),
        };
        let sliced = lines[(start - 1)..end].join("\n");
        assert_eq!(sliced, "p5l1\np5l2\np5l3\np5l4\np5l5\np5l6\np5l7\np5l8\np5l9\np5l10");
    }
}
