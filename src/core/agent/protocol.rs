use super::diff_repair::{
    ensure_diff_has_file_header, fix_hunk_header_counts, normalize_diff_hunk_lines,
};
use super::types::{Context, TurnKind};
use crate::{Result, RuChatError};
use std::time::Duration;
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

#[derive(Debug)]
pub(crate) enum Validation {
    Success,
    Failure(String),
    Skip,
}

/// A single compiler diagnostic parsed from `cargo ... --message-format=json`.
#[derive(Debug, Clone)]
pub(crate) struct Diagnostic {
    pub(crate) level: String, // "error", "warning"
    pub(crate) message: String,
    pub(crate) file: Option<String>,
    pub(crate) line: Option<usize>,
    pub(crate) column: Option<usize>,
}

impl std::fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (&self.file, self.line, self.column) {
            (Some(file), Some(line), Some(col)) => {
                write!(f, "{file}:{line}:{col}: {}: {}", self.level, self.message)
            }
            _ => write!(f, "{}: {}", self.level, self.message),
        }
    }
}

/// Parses `cargo ... --message-format=json` stdout (one JSON object per line)
/// into `error`/`warning` diagnostics. Lines that aren't JSON, or JSON messages
/// that aren't `reason: "compiler-message"`, are ignored — cargo's json output
/// also emits `build-finished`/`artifact` lines interleaved on the same stream.
fn parse_cargo_json_diagnostics(stdout: &str) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        if value.get("reason").and_then(|r| r.as_str()) != Some("compiler-message") {
            continue;
        }
        let Some(message) = value.get("message") else {
            continue;
        };
        let level = message
            .get("level")
            .and_then(|v| v.as_str())
            .unwrap_or("note")
            .to_string();
        // "note" often just restates an already-reported error/warning; skip to
        // keep the Worker/Validator prompt focused on actionable items.
        if level != "error" && level != "warning" {
            continue;
        }
        let text = message
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let spans = message.get("spans").and_then(|s| s.as_array());
        let primary = spans.and_then(|arr| {
            arr.iter()
                .find(|s| s.get("is_primary").and_then(|p| p.as_bool()) == Some(true))
                .or_else(|| arr.first())
        });

        out.push(Diagnostic {
            level,
            message: text,
            file: primary
                .and_then(|s| s.get("file_name"))
                .and_then(|v| v.as_str())
                .map(str::to_string),
            line: primary
                .and_then(|s| s.get("line_start"))
                .and_then(|v| v.as_u64())
                .map(|v| v as usize),
            column: primary
                .and_then(|s| s.get("column_start"))
                .and_then(|v| v.as_u64())
                .map(|v| v as usize),
        });
    }
    out
}

pub(crate) struct BuildReport {
    pub(crate) compiled: bool,
    pub(crate) tests_passed: bool,
    /// Text that isn't (or can't be) expressed as structured `Diagnostic`s: `cargo test`'s raw
    /// stdout on a test failure, or a raw stderr/timeout/exec-failure fallback for a `cargo
    /// check` run whose output couldn't be parsed as compiler-message JSON at all.
    pub(crate) diagnostics: String,
    /// Compiler errors/warnings parsed from `cargo check --message-format=json`, each with its
    /// file/line/col when known. See `rejection_message`, the formatter that combines this with
    /// `diagnostics` for a Worker-facing rejection turn.
    pub(crate) parsed_diagnostics: Vec<Diagnostic>,
}

impl BuildReport {
    /// Renders a Worker-facing rejection message: compile errors first (each citing an exact
    /// file/line/col when the compiler gave one), then any non-blocking warnings so they aren't
    /// silently lost even though they didn't cause the rejection, then whatever couldn't be
    /// expressed structurally (`diagnostics` — raw test stdout, or a stderr/timeout fallback).
    pub(crate) fn rejection_message(&self) -> String {
        let errors: Vec<_> = self
            .parsed_diagnostics
            .iter()
            .filter(|d| d.level == "error")
            .collect();
        let warnings: Vec<_> = self
            .parsed_diagnostics
            .iter()
            .filter(|d| d.level == "warning")
            .collect();
        let mut sections = Vec::new();
        if !errors.is_empty() {
            sections.push(
                errors
                    .iter()
                    .map(|d| d.to_string())
                    .collect::<Vec<_>>()
                    .join("\n"),
            );
        }
        if !warnings.is_empty() {
            sections.push(format!(
                "Non-blocking warnings (compiled fine, didn't cause this rejection):\n{}",
                warnings
                    .iter()
                    .map(|d| d.to_string())
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }
        if !self.diagnostics.is_empty() {
            sections.push(self.diagnostics.clone());
        }
        sections.join("\n\n")
    }
}

/// Diff bodies beyond this size are refused before ever touching disk —
/// large enough for a genuine focused change, small enough to catch a
/// hallucinated patch that tries to rewrite most of a file (or paste
/// unrelated content) in one autonomous, unreviewed step. Comparable in
/// spirit to `orchestrator::fs::MAX_READ_BYTES`, just on the write side.
const MAX_PATCH_DIFF_BYTES: usize = 8_000;

/// Cap on how much of a file's real content gets echoed back in a patch-apply-failure
/// rejection (see `Validation::apply_patch`'s `Err` arm) — large enough to show a genuinely
/// small-to-medium file in full, small enough not to blow the Worker's context budget on a
/// single rejection turn.
const MAX_SHOWN_ORIGINAL_CHARS: usize = 4_000;

/// Cap on how much of `git apply --check`'s own stderr gets folded into a rejection — its
/// output is normally a few lines per failed hunk, this is defensive headroom, not a size this
/// is expected to actually hit.
const MAX_GIT_APPLY_DIAGNOSIS_CHARS: usize = 2_000;

/// Second opinion on a `diffy::apply` failure, via `git apply --check` — dry-run only, never
/// touches the working tree either way. `diffy`'s own source describes itself as following GNU
/// patch's algorithm "minus fuzzy-matching context lines," so it's strictly stricter than either
/// external tool; git's own patch engine can often name the exact hunk and what it searched for
/// vs. what's actually there, a more specific diagnosis than diffy's own error. Also handles the
/// case where git reports the diff WOULD apply cleanly — informative on its own (the mismatch is
/// this tool's own strictness, not necessarily a wrong diff), even though nothing changes here in
/// that case yet (see `TODO.md` for the bigger, not-yet-done option of using `git apply` as the
/// actual apply engine, not just a diagnostic).
///
/// Best-effort like every other diagnostic-nicety path in this codebase (`Checkpoint::save`,
/// `finalize_trace`): if `git` itself can't be spawned or times out, this is silently omitted —
/// the primary diffy-based rejection message must never be blocked on a secondary opinion.
async fn check_with_git_apply(diff_text: &str) -> String {
    let mut child = match Command::new("git")
        .args(["apply", "--check", "-v"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => return String::new(),
    };
    // Written and dropped before `wait_with_output()` so git sees EOF on stdin instead of
    // hanging — same pattern `orchestrator::search::regenerate_tags` already uses for ctags.
    {
        let Some(mut stdin) = child.stdin.take() else {
            return String::new();
        };
        if tokio::io::AsyncWriteExt::write_all(&mut stdin, diff_text.as_bytes())
            .await
            .is_err()
        {
            return String::new();
        }
    }
    let Ok(Ok(output)) =
        tokio::time::timeout(Duration::from_secs(10), child.wait_with_output()).await
    else {
        return String::new();
    };
    if output.status.success() {
        return "(`git apply --check` reports this diff would actually apply cleanly against \
            the file — the mismatch above is specific to this tool's own patch engine, which \
            is stricter than git's, not necessarily a wrong diff.)\n\n"
            .to_string();
    }
    let stderr: String = String::from_utf8_lossy(&output.stderr)
        .trim()
        .chars()
        .take(MAX_GIT_APPLY_DIAGNOSIS_CHARS)
        .collect();
    if stderr.is_empty() {
        String::new()
    } else {
        format!("`git apply --check`'s own diagnosis: {stderr}\n\n")
    }
}

/// True if `target` matches one of the plan's declared paths. Matches exactly or by suffix in
/// either direction (`p.ends_with(target)`/`target.ends_with(p)`) so a plan that names just
/// `foo.rs` still covers a target resolved as `src/foo.rs`, and vice versa.
fn file_in_scope(target: &str, planned: &[String]) -> bool {
    planned
        .iter()
        .any(|p| p == target || target.ends_with(p.as_str()) || p.ends_with(target))
}

/// Old-file line numbers of every `-` (removed) line across all of a unified diff's hunks — the
/// lines this diff actually *changes*, as opposed to lines it merely shows as context. Parsed
/// directly from `@@ -a,b +c,d @@` headers rather than via `diffy::Patch`, so this can run before
/// (and independent of) `diffy::apply` succeeding or failing.
fn removed_line_numbers(diff_text: &str) -> Vec<usize> {
    let mut removed = Vec::new();
    let mut old_line: usize = 0;
    for line in diff_text.lines() {
        if let Some(rest) = line.strip_prefix("@@ ") {
            old_line = rest
                .split_whitespace()
                .next()
                .and_then(|old| old.trim_start_matches('-').split(',').next())
                .and_then(|start| start.parse::<usize>().ok())
                .unwrap_or(0);
            continue;
        }
        if old_line == 0 || line.starts_with("---") || line.starts_with("+++") {
            continue;
        }
        if line.starts_with('-') {
            removed.push(old_line);
            old_line += 1;
        } else if !line.starts_with('+') {
            // Context line — present in both old and new, advances the old-file cursor.
            old_line += 1;
        }
    }
    removed
}

/// Line numbers a `CargoClippy` retrieval turn (`orchestrator.rs`'s `ToolName::CargoClippy`
/// dispatch, output format `cargo clippy --message-format=short`: `path:line:col: level: msg`)
/// flagged in `target` this run, if any. Empty whenever clippy wasn't consulted this run (the
/// overwhelmingly common case) — this makes the caller's check below a no-op for every task that
/// isn't specifically "fix a clippy-flagged line."
fn diagnostic_lines_for(ctx: &Context, target: &str) -> Vec<usize> {
    let file_name = target.rsplit('/').next().unwrap_or(target);
    ctx.turns
        .iter()
        .filter(|t| t.kind == TurnKind::Retrieval && t.source == "CargoClippy")
        .flat_map(|t| t.content.lines())
        .filter_map(|line| {
            let mut parts = line.splitn(3, ':');
            let path = parts.next()?.trim();
            if !(path.ends_with(target) || path.ends_with(file_name)) {
                return None;
            }
            parts.next()?.trim().parse::<usize>().ok()
        })
        .collect()
}

impl Validation {
    pub(crate) async fn apply_patch(diff_text: &str, ctx: &mut Context) -> Result<Self> {
        if diff_text.len() > MAX_PATCH_DIFF_BYTES {
            let content = format!(
                "Patch refused: diff is {} bytes, exceeds the {MAX_PATCH_DIFF_BYTES}-byte limit \
                for a single apply_patch call — split this into smaller, more focused patches.",
                diff_text.len()
            );
            ctx.push_turn(TurnKind::Rejection, "Validator", content.clone());
            return Ok(Validation::Failure(content));
        }
        // Enforced only when the Architect's plan actually declared a `FILES:` scope (see
        // `Context::planned_files`) — a plan that omits the line doesn't retroactively unlock
        // anything here, `ensure_diff_has_file_header` only ever acts when it names exactly one
        // file. Computed once and reused below for the scope check too.
        let planned = ctx.planned_files();
        let repaired = ensure_diff_has_file_header(diff_text, &planned);
        let diff_text = repaired.as_str();
        // `diffy::Patch::from_str` only understands one file's diff (one '--- a/'/'+++ b/'
        // pair, followed by that file's hunks) — a real failure had the Worker concatenate two
        // files' diffs into a single apply_patch call instead of calling apply_patch twice (the
        // multi-file patch loop in `Stage::Implement` supports exactly that, sequentially).
        // diffy doesn't detect this cleanly: it parses the first file's hunks, then chokes on
        // the second '--- a/' header as unparseable trailing content ("orphaned hunk header
        // after trailing content" or similar) — a cryptic message that doesn't tell the Worker
        // what's actually wrong. Caught explicitly, before diffy ever sees it, with a message
        // that does.
        let file_header_count = diff_text.lines().filter(|l| l.starts_with("--- ")).count();
        if file_header_count > 1 {
            let content = format!(
                "refused: this diff contains {file_header_count} separate '--- a/<file>' \
                headers — apply_patch accepts only one file's diff per call. Submit a diff for \
                just one of those files now; you may call apply_patch again afterward for each \
                additional file this round (up to the round's patch budget)."
            );
            ctx.push_turn(TurnKind::Rejection, "Validator", content.clone());
            return Ok(Validation::Failure(content));
        }
        let normalized = normalize_diff_hunk_lines(diff_text);
        let normalized = fix_hunk_header_counts(&normalized);
        let patch = match diffy::Patch::from_str(&normalized) {
            Ok(p) => p,
            Err(e) => {
                let content = format!("Patch parse error: {e}");
                ctx.push_turn(TurnKind::Rejection, "Validator", content);
                return Ok(Validation::Failure(e.to_string()));
            }
        };
        // Resolve target file from the patch header rather than trusting free text elsewhere
        // (e.g. the Architect's plan) — a diff with no `--- a/<file>` header at all gives no
        // safe way to infer one, so this is refused with an actionable message rather than
        // guessed at.
        let Some(original) = patch.original() else {
            let content = "refused: this diff has no '--- a/<file>' header line, so apply_patch \
                can't tell which file to patch. Add '--- a/<path>' and '+++ b/<path>' lines \
                (with the exact path of the file you're editing) immediately before the \
                '@@ ... @@' hunk line, then resubmit the same diff."
                .to_string();
            ctx.push_turn(TurnKind::Rejection, "Validator", content.clone());
            return Ok(Validation::Failure(content));
        };
        let target = original.trim_start_matches("a/");
        let tracked = crate::orchestrator::git::tracked_files().await?;
        if !tracked.contains(target) {
            let content = format!(
                "refused: '{target}' is not tracked by git (not in `git ls-files`) — \
                apply_patch may only modify files already under version control in this repo"
            );
            ctx.push_turn(TurnKind::Rejection, "Validator", content.clone());
            return Ok(Validation::Failure(content));
        }
        // Enforced only when the Architect's plan actually declared a `FILES:` scope (see
        // `Context::planned_files`) — a plan that omits the line doesn't retroactively block
        // every patch, since local models don't reliably follow the convention yet. `planned`
        // was already computed above for `ensure_diff_has_file_header`, reused here.
        if !planned.is_empty() && !file_in_scope(target, &planned) {
            let content = format!(
                "refused: '{target}' is not one of the files the plan named with its `FILES:` \
                line ({}) — apply_patch may only touch files the Architect's plan declared \
                in scope for this round",
                planned.join(", ")
            );
            ctx.push_turn(TurnKind::Rejection, "Validator", content.clone());
            return Ok(Validation::Failure(content));
        }
        // Real, observed failure mode (see TODO.md): the Worker's plan correctly names the
        // clippy-flagged field (e.g. "remove the unused field `options`", quoting
        // `path:82:5: warning: field \`options\` is never read` verbatim), but the diff it
        // actually writes removes a different, nearby line in the same struct instead — diffy
        // happily applies it (still syntactically valid), and the mistake surfaces only after a
        // full cargo-check round-trip via a confusing "no field named ..." compile error. Caught
        // here, deterministically and before any file I/O, whenever this run's own CargoClippy
        // tool already pointed at a specific line in this exact file.
        let diag_lines = diagnostic_lines_for(ctx, target);
        if !diag_lines.is_empty() {
            let touched = removed_line_numbers(diff_text);
            if !touched.is_empty() && !diag_lines.iter().any(|d| touched.contains(d)) {
                let flagged = diag_lines
                    .iter()
                    .map(|n| n.to_string())
                    .collect::<Vec<_>>()
                    .join(", ");
                let content = format!(
                    "refused: this run's own cargo_clippy result flagged {target}:{flagged} — \
                    but this diff's removed/changed line(s) are {touched:?} instead. You likely \
                    have the right file but the wrong line: re-check the field/line the warning \
                    actually named before writing the diff again."
                );
                ctx.push_turn(TurnKind::Rejection, "Validator", content.clone());
                return Ok(Validation::Failure(content));
            }
        }
        let original = tokio::fs::read_to_string(target).await.unwrap_or_default();
        match diffy::apply(&original, &patch) {
            Ok(patched) => {
                tokio::fs::write(target, patched).await?;
                // Recorded so a later Test/Validate/Critique rejection this round can
                // restore the pre-patch content instead of leaving it applied — see
                // `Context::revert_pending_patches`.
                ctx.record_patch(target.to_string(), original);
                Ok(Validation::Success)
            }
            Err(e) => {
                // `diffy::apply` fails here for essentially one reason: the diff's context
                // lines don't match `target`'s actual current content — almost always because
                // the Worker guessed/hallucinated what the file looks like instead of reading
                // it. Telling it to go call `read_file` and retry would cost another
                // round-trip that may not even be available (`retrieve_budget` could already be
                // exhausted) — showing the real content directly in this same rejection lets it
                // write a correct diff on the very next attempt instead.
                // Line-numbered (`grep -n`/`ripgrep` style: "N:content") rather than plain text
                // — the model needs to write not just matching context lines but an accurate
                // `@@ -a,b +c,d @@` hunk header, and a mismatched hunk header is exactly as
                // fatal to `diffy::apply` as mismatched context text. Numbering the shown
                // content lets it read the correct starting line number directly instead of
                // guessing that too.
                let numbered: String = original
                    .lines()
                    .enumerate()
                    .map(|(i, line)| format!("{}:{line}", i + 1))
                    .collect::<Vec<_>>()
                    .join("\n");
                let shown: String = numbered.chars().take(MAX_SHOWN_ORIGINAL_CHARS).collect();
                let truncated_note = if numbered.chars().count() > MAX_SHOWN_ORIGINAL_CHARS {
                    format!(
                        "\n... (truncated, {} bytes total — request a narrower range with \
                        read_file if you need more)",
                        original.len()
                    )
                } else {
                    String::new()
                };
                let git_diagnosis = check_with_git_apply(diff_text).await;
                let content = format!(
                    "Patch apply failed on {target}: {e}\n\n{git_diagnosis}This means the \
                    diff's context lines don't match {target}'s actual current content. Here \
                    is the file's real current content, with line numbers (N:content) — write \
                    your next diff's context lines AND its @@ -a,b +c,d @@ hunk header's \
                    starting line number to match this exactly, don't guess:\n\n{shown}{truncated_note}"
                );
                ctx.push_turn(TurnKind::Rejection, "Validator", content);
                Ok(Validation::Failure(e.to_string()))
            }
        }
    }
    pub(crate) async fn run_build_and_test(cancel: &CancellationToken) -> Result<BuildReport> {
        let mut check_cmd = Command::new("cargo");
        check_cmd.args(["check", "--message-format=json"]);
        crate::orchestrator::cargo::limit_resources(&mut check_cmd, 60);
        let check = tokio::time::timeout(Duration::from_secs(60), async {
            tokio::select! {
                out = check_cmd.output() => Ok(out),
                _ = cancel.cancelled() => Err(()),
            }
        })
        .await;
        let (compiled, parsed_diagnostics, mut diagnostics) = match check {
            Ok(Ok(Ok(o))) => {
                let stdout = String::from_utf8_lossy(&o.stdout);
                let parsed = parse_cargo_json_diagnostics(&stdout);
                // Only a fallback for output `parse_cargo_json_diagnostics` found nothing
                // structured in — `rejection_message` renders `parsed` itself when non-empty.
                let fallback = if parsed.is_empty() {
                    String::from_utf8_lossy(&o.stderr).into_owned()
                } else {
                    String::new()
                };
                (o.status.success(), parsed, fallback)
            }
            Ok(Ok(Err(e))) => (false, Vec::new(), format!("cargo check failed to run: {e}")),
            Ok(Err(())) => return Err(RuChatError::Cancelled),
            Err(_) => (
                false,
                Vec::new(),
                "cargo check timed out after 60s".to_string(),
            ),
        };
        let mut tests_passed = false;
        if compiled {
            let mut test_cmd = Command::new("cargo");
            test_cmd.args(["test", "--", "--nocapture"]);
            crate::orchestrator::cargo::limit_resources(&mut test_cmd, 120);
            let test = tokio::time::timeout(Duration::from_secs(120), async {
                tokio::select! {
                    out = test_cmd.output() => Ok(out),
                    _ = cancel.cancelled() => Err(()),
                }
            })
            .await;
            match test {
                Ok(Ok(Ok(o))) => {
                    tests_passed = o.status.success();
                    diagnostics.push_str(&String::from_utf8_lossy(&o.stdout));
                }
                Ok(Ok(Err(e))) => diagnostics.push_str(&format!("\ncargo test failed to run: {e}")),
                Ok(Err(())) => return Err(RuChatError::Cancelled),
                Err(_) => diagnostics.push_str("\ncargo test timed out after 120s"),
            }
        }
        Ok(BuildReport {
            compiled,
            tests_passed,
            diagnostics,
            parsed_diagnostics,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diag(level: &str, message: &str, file: Option<&str>, line: Option<usize>) -> Diagnostic {
        Diagnostic {
            level: level.to_string(),
            message: message.to_string(),
            file: file.map(str::to_string),
            line,
            column: line.map(|_| 1),
        }
    }

    #[test]
    fn rejection_message_cites_exact_file_and_line_for_errors() {
        let report = BuildReport {
            compiled: false,
            tests_passed: false,
            diagnostics: String::new(),
            parsed_diagnostics: vec![diag(
                "error",
                "mismatched types",
                Some("src/foo.rs"),
                Some(42),
            )],
        };
        let msg = report.rejection_message();
        assert!(
            msg.contains("src/foo.rs:42:1: error: mismatched types"),
            "expected exact file/line in message, got: {msg}"
        );
    }

    #[test]
    fn rejection_message_surfaces_warnings_even_when_they_did_not_cause_the_rejection() {
        // Regression: a compile that produced only warnings used to render an empty diagnostics
        // string ("keep informational" in the old comment, but the code discarded them), so if
        // `cargo test` then failed for an unrelated reason, the warnings never reached the
        // Worker at all. `rejection_message` must surface them from `parsed_diagnostics`
        // directly, independent of whether `diagnostics` (here: raw test stdout) is also set.
        let report = BuildReport {
            compiled: true,
            tests_passed: false,
            diagnostics: "test assertion_failed panicked".to_string(),
            parsed_diagnostics: vec![diag(
                "warning",
                "unused variable: `x`",
                Some("src/bar.rs"),
                Some(7),
            )],
        };
        let msg = report.rejection_message();
        assert!(msg.contains("src/bar.rs:7:1: warning: unused variable"));
        assert!(msg.contains("test assertion_failed panicked"));
    }

    #[test]
    fn rejection_message_falls_back_to_raw_diagnostics_when_nothing_parsed() {
        let report = BuildReport {
            compiled: false,
            tests_passed: false,
            diagnostics: "cargo check timed out after 60s".to_string(),
            parsed_diagnostics: Vec::new(),
        };
        assert_eq!(
            report.rejection_message(),
            "cargo check timed out after 60s"
        );
    }

    #[tokio::test]
    async fn apply_patch_gives_an_actionable_message_for_a_diff_with_no_file_header() {
        // The exact diff (line-count wrong AND no --- a/ +++ b/ headers) reported by the
        // maintainer from a real `fix_one_clippy_lint` run: qwen2.5-coder:14b emitted a
        // header-less diff with an incorrect hunk count, which used to surface only a cryptic
        // "Patch parse error: ... hunk header does not match hunk" — fix_hunk_header_counts now
        // resolves the count problem, so this reaches (and exercises) the clearer,
        // actionable-for-a-retry message for the still-missing header instead.
        let diff = "@@ -3,12 +3,10 @@\nuse std::collections::HashMap;\n\n\
            /// Parses a key=value pair from a string\n\
            -fn parse_key_val(s: &str) -> Result<(String, String), String> {\n\
            +fn _parse_key_val(s: &str) -> Result<(String, String), String> {\n\
                 let mut parts = s.split('=');\n\
                 match (parts.next(), parts.next()) {\n\
                     (Some(key), Some(value)) => Ok((key.to_string(), value.to_string())),\n\
                     _ => Err(\"Invalid key=value format\".to_string()),\n\
                 }\n\
            -}\n\
            +}";
        let mut ctx = Context::new("goal".to_string());
        match Validation::apply_patch(diff, &mut ctx).await.unwrap() {
            Validation::Failure(msg) => {
                assert!(
                    msg.contains("no '--- a/<file>' header line"),
                    "expected the actionable missing-header message, got: {msg}"
                );
            }
            other => panic!("expected a Failure explaining the missing header, got: {other:?}"),
        }
    }

    // Regression: the second of the two contributors found in the live-verified
    // `fix_one_clippy_lint` run documented in TODO.md's pinned reliability item — the Worker's
    // diff omitted the mandatory header entirely, and used to always get refused outright even
    // when the plan unambiguously named the one file it must be. `ensure_diff_has_file_header`
    // now synthesizes it in that case; this test proves the repair actually reaches this call
    // site (not just correct in isolation) by checking the failure reason changed from "no
    // header" to a real context-mismatch — the content is still fabricated on purpose, so this
    // never reaches `diffy::apply`'s success path (would write to the real tracked file).
    #[tokio::test]
    async fn apply_patch_synthesizes_a_missing_header_when_the_plan_names_exactly_one_file() {
        let diff = "@@ -1,3 +1,3 @@\n totally made up line one\n totally made up line two\n-totally made up line three\n+totally made up line three, changed\n";
        let mut ctx = Context::new("goal".to_string());
        ctx.push_turn(TurnKind::Plan, "Architect", "FILES: Cargo.toml".to_string());
        match Validation::apply_patch(diff, &mut ctx).await.unwrap() {
            Validation::Failure(_) => {
                let rejection = ctx
                    .turns
                    .iter()
                    .find(|t| t.kind == TurnKind::Rejection)
                    .expect("a failed apply should push a rejection");
                assert!(
                    rejection.content.contains("real current content"),
                    "expected the header to have been synthesized, reaching the \
                    context-mismatch stage instead of the missing-header refusal, got: {}",
                    rejection.content
                );
            }
            other => panic!(
                "expected the patch to fail on the (fabricated) content mismatch, got: {other:?}"
            ),
        }
    }

    // The ambiguous case: with more than one planned file, there's no safe way to guess which
    // one a bare hunk belongs to, so the missing-header refusal must still fire exactly as
    // before rather than guessing the wrong file.
    #[tokio::test]
    async fn apply_patch_does_not_guess_a_header_when_multiple_files_are_planned() {
        let diff = "@@ -1,3 +1,3 @@\n totally made up line one\n totally made up line two\n-totally made up line three\n+totally made up line three, changed\n";
        let mut ctx = Context::new("goal".to_string());
        ctx.push_turn(
            TurnKind::Plan,
            "Architect",
            "FILES: Cargo.toml, README.md".to_string(),
        );
        match Validation::apply_patch(diff, &mut ctx).await.unwrap() {
            Validation::Failure(msg) => {
                assert!(
                    msg.contains("no '--- a/<file>' header line"),
                    "expected the missing-header refusal since which of two files is \
                    ambiguous, got: {msg}"
                );
            }
            other => panic!("expected a Failure explaining the missing header, got: {other:?}"),
        }
    }

    #[tokio::test]
    async fn apply_patch_gives_an_actionable_message_for_a_diff_spanning_two_files() {
        // From a real failure report: the Worker concatenated diffs for src/tui/io.rs and
        // src/cli/prompt.rs into a single apply_patch call instead of calling apply_patch
        // twice (the multi-file patch loop supports exactly that, sequentially, one file per
        // call) — diffy only surfaced a cryptic "orphaned hunk header after trailing content"
        // once it choked on the second file's header, which gave the Worker nothing to act on.
        let diff = "--- a/src/tui/io.rs\n\
            +++ b/src/tui/io.rs\n\
            @@ -20,7 +20,7 @@ pub(crate) struct Io {\n\
             }\n\
             \n\
             impl Io {\n\
            -    /// Creates a new `Io` instance.\n\
            +    /// Initializes a new `Io` instance.\n\
             \n\
            --- a/src/cli/prompt.rs\n\
            +++ b/src/cli/prompt.rs\n\
            @@ -172,7 +172,7 @@ impl Prompt {\n\
                 pub(crate) fn get_prompt(&self) -> Result<String> {\n\
            -        let io = Io::new();\n\
            +        let io = Io::initialize();\n\
             }\n";
        let mut ctx = Context::new("goal".to_string());
        match Validation::apply_patch(diff, &mut ctx).await.unwrap() {
            Validation::Failure(msg) => {
                assert!(
                    msg.contains("only one file's diff per call"),
                    "expected the actionable multi-file message, got: {msg}"
                );
            }
            other => panic!("expected a Failure explaining the multi-file diff, got: {other:?}"),
        }
    }

    #[test]
    fn file_in_scope_matches_exact_and_suffix() {
        let planned = vec!["src/foo.rs".to_string()];
        assert!(file_in_scope("src/foo.rs", &planned));
        // Plan named just the basename, target resolved with a directory prefix.
        assert!(file_in_scope("src/foo.rs", &["foo.rs".to_string()]));
        // Plan named a longer path than the diff header's target.
        assert!(file_in_scope("foo.rs", &["src/foo.rs".to_string()]));
        assert!(!file_in_scope("src/bar.rs", &planned));
    }

    #[tokio::test]
    async fn apply_patch_rejects_target_outside_declared_scope() {
        // Cargo.toml is tracked by git in this repo, so it passes the tracked-file check and
        // reaches the scope check — the diff content itself is never applied since the scope
        // rejection happens first.
        let mut ctx = Context::new("goal".to_string());
        ctx.push_turn(
            TurnKind::Plan,
            "Architect",
            "FILES: src/some_other_file.rs".to_string(),
        );
        let diff = "--- a/Cargo.toml\n+++ b/Cargo.toml\n@@ -1,1 +1,1 @@\n-old\n+new\n";
        let result = Validation::apply_patch(diff, &mut ctx)
            .await
            .expect("apply_patch should not error, only reject");
        match result {
            Validation::Failure(msg) => assert!(
                msg.contains("not one of the files"),
                "expected scope rejection message, got: {msg}"
            ),
            _ => panic!("expected the patch to be rejected as out of declared scope"),
        }
    }

    // Regression test for a real failure: the Worker wrote a diff assuming a plausible-looking
    // but entirely hallucinated function signature instead of the file's actual content (it
    // never read the file first), so every attempt failed to apply with only a generic
    // "context mismatch"-shaped error — nothing telling the Worker what the file *actually*
    // looks like, so every retry guessed again instead of correcting.
    #[tokio::test]
    async fn apply_patch_shows_real_file_content_when_context_does_not_match() {
        // Cargo.toml is tracked by git in this repo (passes the tracked-file check) and no
        // FILES: line is set (scope check is skipped) — reaches diffy::apply, which fails since
        // these context lines don't match Cargo.toml's real content, syntactically valid diff
        // otherwise (hunk header count matches its own body, so this isn't a parse failure).
        let mut ctx = Context::new("goal".to_string());
        let diff = "--- a/Cargo.toml\n+++ b/Cargo.toml\n@@ -1,3 +1,3 @@\n totally made up line one\n totally made up line two\n-totally made up line three\n+totally made up line three, changed\n";
        let result = Validation::apply_patch(diff, &mut ctx).await.unwrap();
        match result {
            Validation::Failure(_) => {
                let rejection = ctx
                    .turns
                    .iter()
                    .find(|t| t.kind == TurnKind::Rejection)
                    .expect("a failed apply should push a rejection");
                assert!(rejection.content.contains("real current content"));
                // Cargo.toml's actual [package] header should appear verbatim in the message —
                // confirms the REAL content was shown, not just a generic error.
                assert!(
                    rejection.content.contains("[package]"),
                    "expected Cargo.toml's real content in the message, got: {}",
                    rejection.content
                );
            }
            other => panic!("expected the patch to fail to apply, got: {other:?}"),
        }
    }

    // Regression: the shown-real-content-on-mismatch rejection used to be plain, unnumbered
    // text — the model could match context lines but had no way to read the correct `@@ -a,b
    // +c,d @@` hunk header starting line number off it, since it wasn't shown at all. Now
    // formatted `grep -n`/`ripgrep`-style ("N:content") so both the context text AND the hunk
    // header's line number can be copied directly instead of guessed.
    #[tokio::test]
    async fn apply_patch_shows_line_numbers_in_the_real_content_on_mismatch() {
        let mut ctx = Context::new("goal".to_string());
        let diff = "--- a/Cargo.toml\n+++ b/Cargo.toml\n@@ -1,3 +1,3 @@\n totally made up line one\n totally made up line two\n-totally made up line three\n+totally made up line three, changed\n";
        let result = Validation::apply_patch(diff, &mut ctx).await.unwrap();
        let Validation::Failure(_) = result else {
            panic!("expected the patch to fail to apply, got: {result:?}");
        };
        let rejection = ctx
            .turns
            .iter()
            .find(|t| t.kind == TurnKind::Rejection)
            .expect("a failed apply should push a rejection");
        // Cargo.toml's first real line is "[package]" — must appear as "1:[package]", not bare.
        assert!(
            rejection.content.contains("1:[package]"),
            "expected a line-numbered first line, got: {}",
            rejection.content
        );
    }

    #[tokio::test]
    async fn check_with_git_apply_surfaces_gits_own_diagnosis_for_a_mismatched_diff() {
        let diff = "--- a/Cargo.toml\n+++ b/Cargo.toml\n@@ -1,3 +1,3 @@\n totally made up line one\n totally made up line two\n-totally made up line three\n+totally made up line three, changed\n";
        let diagnosis = check_with_git_apply(diff).await;
        assert!(
            diagnosis.contains("patch failed") || diagnosis.contains("while searching for"),
            "expected git's own failure diagnosis, got: {diagnosis:?}"
        );
    }

    #[tokio::test]
    async fn check_with_git_apply_reports_when_a_diff_would_actually_apply_cleanly() {
        // Real content, 3-line hunk (context/change/context) against Cargo.toml's actual first
        // lines — a diff `git apply --check` genuinely accepts, independent of whether diffy
        // also would (that's exercised separately via `Validation::apply_patch` itself).
        let diff = "--- a/Cargo.toml\n+++ b/Cargo.toml\n@@ -1,3 +1,3 @@\n [package]\n-authors = [\"Roelof J.C. Kluin\"]\n+authors = [\"Someone Else\"]\n description = \"ollama/chroma command-line AI chat tool\"\n";
        let diagnosis = check_with_git_apply(diff).await;
        assert!(
            diagnosis.contains("would actually apply cleanly"),
            "expected the clean-apply message, got: {diagnosis:?}"
        );
    }

    // Regression: confirms the second opinion is actually wired into the real rejection path,
    // not just correct in isolation.
    #[tokio::test]
    async fn apply_patch_rejection_includes_gits_own_diagnosis() {
        let mut ctx = Context::new("goal".to_string());
        let diff = "--- a/Cargo.toml\n+++ b/Cargo.toml\n@@ -1,3 +1,3 @@\n totally made up line one\n totally made up line two\n-totally made up line three\n+totally made up line three, changed\n";
        let result = Validation::apply_patch(diff, &mut ctx).await.unwrap();
        let Validation::Failure(_) = result else {
            panic!("expected the patch to fail to apply, got: {result:?}");
        };
        let rejection = ctx
            .turns
            .iter()
            .find(|t| t.kind == TurnKind::Rejection)
            .expect("a failed apply should push a rejection");
        assert!(
            rejection.content.contains("git apply --check"),
            "expected the rejection to include git's second opinion, got: {}",
            rejection.content
        );
    }

    #[test]
    fn removed_line_numbers_ignores_context_and_added_lines() {
        let diff = "--- a/x\n+++ b/x\n@@ -81,7 +81,6 @@ pub(crate) struct Agent {\n     options: ModelOptions,\n     agent_config: HashMap<String, Value>,\n     pub(super) embed_args: Option<EmbedArgs>,\n-    cfg: Value,\n }\n \n impl Agent {\n";
        // Body line 1 (`options`) sits at the hunk's declared old-start (81); the removed line
        // (`cfg`) is the 4th body line, so 81 + 3 = 84 — never 82 (`options`, what the plan
        // actually named), which is the property this guard depends on.
        assert_eq!(removed_line_numbers(diff), vec![84]);
    }

    #[test]
    fn removed_line_numbers_handles_multiple_hunks() {
        let diff =
            "--- a/x\n+++ b/x\n@@ -1,2 +1,1 @@\n-one\n two\n@@ -10,2 +9,1 @@\n-ten\n eleven\n";
        assert_eq!(removed_line_numbers(diff), vec![1, 10]);
    }

    #[test]
    fn diagnostic_lines_for_reads_a_cargo_clippy_retrieval_turn() {
        let mut ctx = Context::new("goal".to_string());
        ctx.push_turn(
            TurnKind::Retrieval,
            "CargoClippy",
            "src/core/agent.rs:82:5: warning: field `options` is never read\n\
             src/core/index.rs:595:9: warning: this `if` statement can be collapsed"
                .to_string(),
        );
        assert_eq!(diagnostic_lines_for(&ctx, "src/core/agent.rs"), vec![82]);
        assert!(diagnostic_lines_for(&ctx, "src/core/other.rs").is_empty());
    }

    #[test]
    fn diagnostic_lines_for_is_empty_when_clippy_was_never_run_this_turn() {
        let ctx = Context::new("goal".to_string());
        assert!(diagnostic_lines_for(&ctx, "Cargo.toml").is_empty());
    }

    // Regression for the real `fix_one_clippy_lint` failure documented in TODO.md: the Worker's
    // plan correctly quotes the clippy warning at Cargo.toml:2, but the diff it writes removes a
    // different line (4) in the same hunk instead — this must be refused before diffy::apply
    // (and hence before any disk write), with a message pointing at the specific mismatch.
    #[tokio::test]
    async fn apply_patch_rejects_a_diff_that_misses_the_flagged_clippy_line() {
        let mut ctx = Context::new("goal".to_string());
        ctx.push_turn(
            TurnKind::Retrieval,
            "CargoClippy",
            "Cargo.toml:2:1: warning: field `authors` is never read".to_string(),
        );
        let diff = "--- a/Cargo.toml\n+++ b/Cargo.toml\n@@ -1,4 +1,3 @@\n [package]\n authors = [\"Roelof J.C. Kluin\"]\n description = \"ollama/chroma command-line AI chat tool\"\n-edition = \"2024\"\n";
        let result = Validation::apply_patch(diff, &mut ctx).await.unwrap();
        match result {
            Validation::Failure(_) => {
                let rejection = ctx
                    .turns
                    .iter()
                    .rev()
                    .find(|t| t.kind == TurnKind::Rejection)
                    .expect("a mismatched-line diff should be rejected");
                assert!(
                    rejection.content.contains("cargo_clippy result flagged"),
                    "expected the flagged-line mismatch message, got: {}",
                    rejection.content
                );
            }
            other => panic!("expected the patch to be rejected, got: {other:?}"),
        }
        // Never reached diffy::apply, so the real file on disk must be untouched.
        let on_disk = tokio::fs::read_to_string("Cargo.toml").await.unwrap();
        assert!(on_disk.contains("edition = \"2024\""));
    }

    // A diff that DOES touch the flagged line must pass this guard through to the normal
    // diffy::apply path — proven here by using a real line number but fabricated content, which
    // fails for the pre-existing "content mismatch" reason instead, never this new one. Confirms
    // no false positive on a genuinely correct target line.
    #[tokio::test]
    async fn apply_patch_does_not_reject_a_diff_that_touches_the_flagged_line() {
        let mut ctx = Context::new("goal".to_string());
        ctx.push_turn(
            TurnKind::Retrieval,
            "CargoClippy",
            "Cargo.toml:2:1: warning: field `authors` is never read".to_string(),
        );
        let diff = "--- a/Cargo.toml\n+++ b/Cargo.toml\n@@ -1,4 +1,3 @@\n [package]\n-totally made up line two\n description = \"ollama/chroma command-line AI chat tool\"\n edition = \"2024\"\n";
        let result = Validation::apply_patch(diff, &mut ctx).await.unwrap();
        let Validation::Failure(_) = result else {
            panic!("expected the fabricated content to still fail apply, got: {result:?}");
        };
        let rejection = ctx
            .turns
            .iter()
            .rev()
            .find(|t| t.kind == TurnKind::Rejection)
            .expect("a failed apply should push a rejection");
        assert!(
            !rejection.content.contains("cargo_clippy result flagged"),
            "the flagged-line guard must not fire when the diff does target that line, got: {}",
            rejection.content
        );
        assert!(rejection.content.contains("Patch apply failed"));
    }
}
