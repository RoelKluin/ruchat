use super::fs::canonicalize_within_repo;
use crate::{Result, RuChatError};

/// Shells to `rg` (ripgrep). Requires ripgrep on PATH — same "tool missing"
/// posture as `core::index::run_ctags_json`'s universal-ctags check.
///
/// `pattern`/`path` are Worker (model) tool-call arguments — untrusted input, same posture as
/// every other fs-reading tool. Before 2026-08-04 this function had two real gaps: `path` was
/// never checked against the repo root (a Worker call could read arbitrary files like
/// `/etc/passwd` — verified live), and `pattern`/`path` were passed to `rg` with no `--` flag
/// terminator, so a pattern like `--pre=/bin/sh` would be parsed as ripgrep's own `--pre` flag
/// (runs an arbitrary program against every scanned file's contents) instead of being searched
/// for literally — combined with `apply_patch` already being able to write arbitrary content to
/// a tracked file, that's a real code-execution chain through a tool meant to be safe by
/// construction (see CLAUDE.md's "no generic shell-execution tool" invariant). Both are closed
/// here: `path` goes through the same `canonicalize_within_repo` every other fs tool uses, and
/// `build_rg_args` always inserts `--` before the untrusted positionals.
pub(crate) async fn ripgrep(
    pattern: &str,
    path: Option<&str>,
    glob: Option<&str>,
    max_count: Option<u32>,
) -> Result<String> {
    let canonical_path = path
        .map(canonicalize_within_repo)
        .transpose()?
        .map(|p| p.to_string_lossy().into_owned());

    let args = build_rg_args(pattern, canonical_path.as_deref(), glob, max_count);

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

/// Builds `rg`'s argv. Kept pure/synchronous (no process spawn) so the flag-terminator
/// invariant can be tested directly without a live `rg` binary. `--` always separates the
/// flags this function controls from `pattern`/`path`, which are untrusted — see `ripgrep`'s
/// doc comment for why that matters.
fn build_rg_args(
    pattern: &str,
    path: Option<&str>,
    glob: Option<&str>,
    max_count: Option<u32>,
) -> Vec<String> {
    let mut args = vec!["--line-number".to_string(), "--no-heading".to_string()];
    if let Some(g) = glob {
        args.push("--glob".into());
        args.push(g.to_string());
    }
    let count = max_count.unwrap_or(50).to_string();
    args.push("--max-count".into());
    args.push(count);
    args.push("--".to_string());
    args.push(pattern.to_string());
    if let Some(p) = path {
        args.push(p.to_string());
    }
    args
}

/// Repo-root path of the `universal-ctags` symbol index `read_tags` serves lookups from —
/// distinct from `core::index`'s per-file JSON ctags invocations (those feed Chroma chunk
/// metadata; this one is a single flat index for text lookups by symbol name).
const TAGS_FILE: &str = "tags";

/// Every git-tracked `*.rs` file — the exact scope both the staleness check and regeneration
/// use. Deliberately never a raw recursive directory walk (`ctags -R .`): this repo has a large
/// (37 MB+), gitignored-but-physically-present `docs/` directory of saved reference webpages
/// that a recursive walk would happily sweep in, which is exactly how the `tags` file this
/// replaces ballooned to ~494 MB / 2.5M lines of unusable minified-JS noise in practice — `git
/// ls-files` only ever sees what's actually tracked.
async fn tracked_rust_files() -> Result<Vec<String>> {
    let output = tokio::process::Command::new("git")
        .args(["ls-files", "--", "*.rs"])
        .output()
        .await
        .map_err(|e| RuChatError::InternalError(format!("failed to run git ls-files: {e}")))?;
    if !output.status.success() {
        return Err(RuChatError::InternalError(format!(
            "git ls-files failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_string)
        .collect())
}

/// True if `tags_path` doesn't exist yet, or any of `files` has been modified more recently
/// than it was last generated. Unreadable/vanished source files are skipped rather than treated
/// as "newer" — a file that disappeared doesn't need re-tagging to prove it disappeared. Takes
/// `tags_path` as a parameter (rather than hardcoding `TAGS_FILE`) purely for testability —
/// tests point it at a tempdir instead of the real repo-root `tags` file.
async fn tags_are_stale(tags_path: &str, files: &[String]) -> bool {
    let tags_mtime = match tokio::fs::metadata(tags_path)
        .await
        .and_then(|m| m.modified())
    {
        Ok(t) => t,
        Err(_) => return true, // missing (or mtime unavailable) — treat as stale
    };
    for f in files {
        if let Ok(meta) = tokio::fs::metadata(f).await
            && let Ok(src_mtime) = meta.modified()
            && src_mtime > tags_mtime
        {
            return true;
        }
    }
    false
}

/// Regenerates `tags_path` from exactly `files` via `universal-ctags`, fed over stdin (`-L -`)
/// so the argument list stays well under OS limits regardless of repo size — same mechanism
/// `gen_tags.sh` already used, just with the file list computed the same safe way the staleness
/// check above uses it, instead of a separate, potentially-divergent invocation.
async fn regenerate_tags(tags_path: &str, files: &[String]) -> Result<()> {
    if files.is_empty() {
        return Err(RuChatError::InternalError(
            "no tracked *.rs files found — nothing to tag".into(),
        ));
    }
    let file_list = files.join("\n");

    let mut child = tokio::process::Command::new("ctags")
        .args(["--rust-kinds=+fP", "-f", tags_path, "-L", "-"])
        .stdin(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| {
            RuChatError::InternalError(format!(
                "failed to spawn ctags: {e} (is universal-ctags installed and on PATH?)"
            ))
        })?;

    // Written and dropped before `wait()` so ctags sees EOF on stdin instead of hanging.
    {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| RuChatError::InternalError("failed to open ctags stdin".to_string()))?;
        tokio::io::AsyncWriteExt::write_all(&mut stdin, file_list.as_bytes())
            .await
            .map_err(|e| {
                RuChatError::InternalError(format!("failed to write to ctags stdin: {e}"))
            })?;
    }

    let status = tokio::time::timeout(std::time::Duration::from_secs(30), child.wait())
        .await
        .map_err(|_| RuChatError::InternalError("ctags timed out after 30s".into()))?
        .map_err(|e| RuChatError::InternalError(format!("failed to run ctags: {e}")))?;
    if !status.success() {
        return Err(RuChatError::InternalError(format!(
            "ctags exited with {status}"
        )));
    }
    Ok(())
}

/// Reads the repo-root `tags` file produced by `universal-ctags`, optionally filtered to lines
/// mentioning `symbol`. Transparently checks whether it's missing or stale (any tracked `*.rs`
/// file modified more recently than `tags` itself) and regenerates it first if so — a single
/// tool call always returns fresh results, rather than requiring the caller to separately
/// check/update/read across its limited per-round tool budget (see `agent_role/worker.md`'s
/// one-lookup-per-round rule).
pub(crate) async fn read_tags(symbol: Option<&str>) -> Result<String> {
    let files = tracked_rust_files().await?;
    if tags_are_stale(TAGS_FILE, &files).await {
        regenerate_tags(TAGS_FILE, &files).await?;
    }
    let content = tokio::fs::read_to_string(TAGS_FILE)
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

#[cfg(test)]
mod tests {
    use super::*;

    // Regression (found 2026-08-04, security review): without a `--` flag terminator, a
    // Worker-supplied `pattern` like `--pre=/bin/sh` is parsed by `rg` as the `--pre` flag
    // (runs an arbitrary program against every scanned file's contents) instead of being
    // searched for literally — a real code-execution path. Verified live against the actual
    // `rg` binary before this fix: `rg` accepted `--pre=...` as a flag with no complaint when
    // passed as a bare positional, silently reinterpreting the next positional as the pattern
    // instead. This test would have failed against the pre-fix `build_rg_args` (no `--` was
    // ever inserted).
    #[test]
    fn build_rg_args_inserts_a_flag_terminator_before_the_untrusted_pattern() {
        let args = build_rg_args("--pre=/bin/sh", None, None, None);
        let sep_pos = args
            .iter()
            .position(|a| a == "--")
            .expect("expected a `--` flag terminator in rg's argv");
        let pattern_pos = args
            .iter()
            .position(|a| a == "--pre=/bin/sh")
            .expect("the untrusted pattern should be present verbatim");
        assert!(
            sep_pos < pattern_pos,
            "the `--` terminator must come before the untrusted pattern, got: {args:?}"
        );
    }

    #[test]
    fn build_rg_args_terminator_also_precedes_the_untrusted_path() {
        let args = build_rg_args("needle", Some("--pre=/bin/sh"), None, None);
        let sep_pos = args.iter().position(|a| a == "--").unwrap();
        let path_pos = args
            .iter()
            .position(|a| a == "--pre=/bin/sh")
            .expect("the untrusted path should be present verbatim");
        assert!(sep_pos < path_pos, "got: {args:?}");
    }

    // Regression (found 2026-08-04, security review): `ripgrep`'s `path` argument was never
    // checked against the repo root before this fix — verified live before fixing: `rg` happily
    // read `/etc/passwd` when given an absolute path outside the repo. This test would have
    // failed (returned `Ok` instead of `Err`) against the pre-fix `ripgrep`.
    #[tokio::test]
    async fn ripgrep_rejects_a_path_that_escapes_the_repo_root() {
        let err = ripgrep("root", Some("/etc"), None, None).await.unwrap_err();
        assert!(
            format!("{err}").contains("escapes"),
            "expected an escape-rejection error, got: {err}"
        );
    }

    #[tokio::test]
    async fn ripgrep_still_finds_real_matches_within_the_repo() {
        // Confirms the containment fix didn't break the ordinary in-repo case — this file
        // itself contains its own function name.
        let result = ripgrep(
            "fn ripgrep",
            Some("src/core/orchestrator/search.rs"),
            None,
            None,
        )
        .await
        .unwrap();
        assert!(
            result.contains("ripgrep"),
            "expected a real match, got: {result:?}"
        );
    }

    #[tokio::test]
    async fn tags_are_stale_when_the_tags_file_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("tags").to_str().unwrap().to_string();
        assert!(tags_are_stale(&missing, &[]).await);
    }

    #[tokio::test]
    async fn tags_are_stale_when_a_source_file_is_newer() {
        let dir = tempfile::tempdir().unwrap();
        let tags_path = dir.path().join("tags");
        let src_path = dir.path().join("foo.rs");
        std::fs::write(&tags_path, "old tags content").unwrap();
        // Ensure a real, observable mtime gap — same-millisecond writes on fast filesystems can
        // otherwise land on an identical mtime and make this test flaky.
        std::thread::sleep(std::time::Duration::from_millis(10));
        std::fs::write(&src_path, "fn foo() {}").unwrap();

        let tags_path = tags_path.to_str().unwrap().to_string();
        let src_path = src_path.to_str().unwrap().to_string();
        assert!(tags_are_stale(&tags_path, &[src_path]).await);
    }

    #[tokio::test]
    async fn tags_are_not_stale_when_tags_is_newer_than_every_source_file() {
        let dir = tempfile::tempdir().unwrap();
        let src_path = dir.path().join("foo.rs");
        let tags_path = dir.path().join("tags");
        std::fs::write(&src_path, "fn foo() {}").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
        std::fs::write(&tags_path, "tags content, generated after foo.rs").unwrap();

        let tags_path = tags_path.to_str().unwrap().to_string();
        let src_path = src_path.to_str().unwrap().to_string();
        assert!(!tags_are_stale(&tags_path, &[src_path]).await);
    }

    #[tokio::test]
    async fn tags_are_stale_ignores_a_vanished_source_file() {
        let dir = tempfile::tempdir().unwrap();
        let tags_path = dir.path().join("tags");
        std::fs::write(&tags_path, "tags content").unwrap();
        let tags_path = tags_path.to_str().unwrap().to_string();
        let never_existed = dir.path().join("gone.rs").to_str().unwrap().to_string();
        assert!(!tags_are_stale(&tags_path, &[never_existed]).await);
    }

    // Read-only, safe to run unconditionally: this repo IS a git checkout, so `git ls-files`
    // always works here. Asserts the scope that matters most given how the real `tags` file
    // ballooned to ~494 MB in practice — every result must be a real `.rs` path, nothing from
    // the large, gitignored-but-physically-present `docs/` directory of saved webpages that a
    // recursive `ctags -R .` walk (what this replaces) would otherwise sweep in.
    #[tokio::test]
    async fn tracked_rust_files_only_returns_rs_paths_never_docs() {
        let files = tracked_rust_files().await.unwrap();
        assert!(!files.is_empty(), "this repo has real .rs files to find");
        for f in &files {
            assert!(f.ends_with(".rs"), "non-.rs path returned: {f}");
            assert!(!f.starts_with("docs/"), "docs/ path leaked in: {f}");
        }
    }

    // Requires `universal-ctags` on PATH — not run by default so `cargo test --lib` doesn't
    // depend on an external tool being installed. Run explicitly with
    // `cargo test --lib -- --ignored tags_regeneration` to verify against a real ctags binary.
    #[tokio::test]
    #[ignore = "requires universal-ctags on PATH"]
    async fn regenerate_tags_produces_a_readable_tags_file_scoped_to_rust_sources() {
        let dir = tempfile::tempdir().unwrap();
        let tags_path = dir.path().join("tags").to_str().unwrap().to_string();
        let files = tracked_rust_files().await.unwrap();

        regenerate_tags(&tags_path, &files).await.unwrap();

        let content = tokio::fs::read_to_string(&tags_path).await.unwrap();
        assert!(!content.is_empty());
        // A known symbol from this very file should be present.
        assert!(content.contains("tracked_rust_files"));
    }
}
