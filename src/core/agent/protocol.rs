use super::types::{Context, TurnKind};
use crate::{Result, RuChatError};
use std::sync::OnceLock;
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

/// Repairs the single most common diff mistake coder-tuned local models make:
/// dropping the mandatory leading space on an unchanged (context) line inside
/// a hunk. Unified diff requires every hunk-body line to start with ' '
/// (context), '+' (added), '-' (removed), or '\' (no-newline marker) —
/// `diffy::Patch::from_str` rejects anything else outright ("unexpected line
/// in hunk body"), which is easy for a model to trigger since the leading
/// space is invisible whitespace. Any hunk-body line missing one of those
/// prefixes is treated as an implicit context line and given one; this does
/// not change what the diff says to add/remove, only makes an
/// otherwise-valid diff parseable. A genuinely wrong diff (bad content,
/// mismatched context, wrong file) still fails at parse or apply time same
/// as before.
fn normalize_diff_hunk_lines(diff: &str) -> String {
    let mut out = String::with_capacity(diff.len());
    let mut in_hunk = false;
    for line in diff.split_inclusive('\n') {
        let trimmed = line.trim_end_matches('\n');
        if trimmed.starts_with("--- ") || trimmed.starts_with("+++ ") {
            in_hunk = false;
        } else if trimmed.starts_with("@@") {
            in_hunk = true;
        } else if in_hunk && !trimmed.is_empty() && !trimmed.starts_with([' ', '+', '-', '\\']) {
            out.push(' ');
        }
        out.push_str(line);
    }
    out
}

/// Recomputes each hunk's `@@ -old_start,old_count +new_start,new_count @@` count fields from
/// the hunk body itself. Coder-tuned local models reliably get this line-count bookkeeping
/// wrong even when the actual `+`/`-`/context content is perfectly correct — `diffy` rejects
/// the whole patch outright ("hunk header does not match hunk") rather than tolerating it, the
/// same class of easy-to-trigger, easy-to-repair mistake `normalize_diff_hunk_lines` already
/// handles for missing leading spaces. Safe to recompute unconditionally: the counts are
/// redundant metadata fully determined by the body's own line prefixes, so this can't change
/// what the diff says to add/remove, only fix the header to match what's actually there. Must
/// run *after* `normalize_diff_hunk_lines` so context/added/removed classification (which line
/// counts as which) is already correct by the time line prefixes are read here.
fn fix_hunk_header_counts(diff: &str) -> String {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        regex::Regex::new(r"^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@(.*)$").unwrap()
    });

    let lines: Vec<&str> = diff.split_inclusive('\n').collect();
    let mut out = String::with_capacity(diff.len());
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim_end_matches('\n');
        let Some(caps) = re.captures(trimmed) else {
            out.push_str(line);
            i += 1;
            continue;
        };
        let old_start = &caps[1];
        let new_start = &caps[2];
        let trailer = &caps[3]; // e.g. a function-context hint some diffs append; preserved as-is

        let mut j = i + 1;
        while j < lines.len() {
            let t = lines[j].trim_end_matches('\n');
            if t.starts_with("@@") || t.starts_with("--- ") || t.starts_with("+++ ") {
                break;
            }
            j += 1;
        }
        let body = &lines[i + 1..j];
        let (old_count, new_count) = count_hunk_body_lines(body);
        out.push_str(&format!(
            "@@ -{old_start},{old_count} +{new_start},{new_count} @@{trailer}\n"
        ));
        for b in body {
            out.push_str(b);
        }
        i = j;
    }
    out
}

/// Counts (old-side, new-side) lines in an already-normalized hunk body: context lines (` `)
/// count toward both, `-` only the old side, `+` only the new side. Anything else (a `\ No
/// newline at end of file` marker, or a stray unrecognized line) counts toward neither.
fn count_hunk_body_lines(body: &[&str]) -> (usize, usize) {
    let mut old = 0usize;
    let mut new = 0usize;
    for line in body {
        match line.as_bytes().first() {
            Some(b' ') => {
                old += 1;
                new += 1;
            }
            Some(b'-') => old += 1,
            Some(b'+') => new += 1,
            _ => {}
        }
    }
    (old, new)
}

/// True if `target` matches one of the plan's declared paths. Matches exactly or by suffix in
/// either direction (`p.ends_with(target)`/`target.ends_with(p)`) so a plan that names just
/// `foo.rs` still covers a target resolved as `src/foo.rs`, and vice versa.
fn file_in_scope(target: &str, planned: &[String]) -> bool {
    planned
        .iter()
        .any(|p| p == target || target.ends_with(p.as_str()) || p.ends_with(target))
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
        // every patch, since local models don't reliably follow the convention yet.
        let planned = ctx.planned_files();
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
                let content = format!(
                    "Patch apply failed on {target}: {e}\n\nThis means the diff's context \
                    lines don't match {target}'s actual current content. Here is the file's \
                    real current content, with line numbers (N:content) — write your next \
                    diff's context lines AND its @@ -a,b +c,d @@ hunk header's starting line \
                    number to match this exactly, don't guess:\n\n{shown}{truncated_note}"
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

    #[test]
    fn normalize_adds_missing_leading_space_on_context_lines() {
        // Regression: qwen2.5-coder:14b reliably drops the mandatory leading
        // space on unchanged hunk lines, which diffy::Patch::from_str used to
        // reject outright ("unexpected line in hunk body") even though the
        // diff's actual add/remove intent was perfectly clear.
        let diff = "--- a/src/foo.rs\n+++ b/src/foo.rs\n@@ -1,3 +1,3 @@\nfn foo() {\n-    old();\n+    new();\n}\n";
        let normalized = normalize_diff_hunk_lines(diff);
        assert_eq!(
            normalized,
            "--- a/src/foo.rs\n+++ b/src/foo.rs\n@@ -1,3 +1,3 @@\n fn foo() {\n-    old();\n+    new();\n }\n"
        );
        // And the repaired diff must now actually parse.
        diffy::Patch::from_str(&normalized).expect("repaired diff should parse");
    }

    #[test]
    fn normalize_leaves_well_formed_diff_unchanged() {
        let diff = "--- a/src/foo.rs\n+++ b/src/foo.rs\n@@ -1,3 +1,3 @@\n fn foo() {\n-    old();\n+    new();\n }\n";
        assert_eq!(normalize_diff_hunk_lines(diff), diff);
    }

    #[test]
    fn normalize_does_not_touch_file_header_lines() {
        // "--- "/"+++ " lines are real filenames, not hunk content — must
        // never gain a spurious leading space even though they don't start
        // with one of the hunk-body prefixes either.
        let diff = "--- a/src/foo.rs\n+++ b/src/foo.rs\n@@ -1,1 +1,1 @@\n-old\n+new\n";
        assert_eq!(normalize_diff_hunk_lines(diff), diff);
    }

    #[test]
    fn fix_hunk_header_counts_corrects_a_wrong_line_count() {
        // Regression: a model wrote `@@ -1,4 +1,4 @@` (4/4) but the actual hunk body only has
        // 3 old-side and 3 new-side lines — diffy rejects this outright ("hunk header does not
        // match hunk") even though the +/- content itself is perfectly clear.
        let diff = "--- a/src/foo.rs\n+++ b/src/foo.rs\n@@ -1,4 +1,4 @@\n fn foo() {\n-    old();\n+    new();\n }\n";
        let fixed = fix_hunk_header_counts(diff);
        assert!(fixed.contains("@@ -1,3 +1,3 @@\n"), "got: {fixed:?}");
        diffy::Patch::from_str(&fixed).expect("corrected header should now parse");
    }

    #[test]
    fn fix_hunk_header_counts_leaves_a_correct_count_unchanged() {
        let diff = "--- a/src/foo.rs\n+++ b/src/foo.rs\n@@ -1,3 +1,3 @@\n fn foo() {\n-    old();\n+    new();\n }\n";
        assert_eq!(fix_hunk_header_counts(diff), diff);
    }

    #[test]
    fn fix_hunk_header_counts_handles_multiple_hunks_independently() {
        let diff = "--- a/src/foo.rs\n+++ b/src/foo.rs\n\
            @@ -1,9 +1,9 @@\n fn a() {\n-    old_a();\n+    new_a();\n }\n\
            @@ -20,9 +20,9 @@\n fn b() {\n-    old_b();\n+    new_b();\n }\n";
        let fixed = fix_hunk_header_counts(diff);
        assert!(fixed.contains("@@ -1,3 +1,3 @@\n"), "got: {fixed:?}");
        assert!(fixed.contains("@@ -20,3 +20,3 @@\n"), "got: {fixed:?}");
        diffy::Patch::from_str(&fixed).expect("both corrected hunks should now parse");
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
}
