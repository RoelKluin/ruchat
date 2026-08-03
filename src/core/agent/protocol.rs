use super::types::{Context, TurnKind};
use crate::{Result, RuChatError};
use std::time::Duration;
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

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
        } else if in_hunk
            && !trimmed.is_empty()
            && !trimmed.starts_with([' ', '+', '-', '\\'])
        {
            out.push(' ');
        }
        out.push_str(line);
    }
    out
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
        let normalized = normalize_diff_hunk_lines(diff_text);
        let patch = match diffy::Patch::from_str(&normalized) {
            Ok(p) => p,
            Err(e) => {
                let content = format!("Patch parse error: {e}");
                ctx.push_turn(TurnKind::Rejection, "Validator", content);
                return Ok(Validation::Failure(e.to_string()));
            }
        };
        // Resolve target file from the patch header rather than trusting free text elsewhere.
        let target = patch
            .original()
            .unwrap_or("unknown")
            .trim_start_matches("a/");
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
                let content = format!("Patch apply failed on {target}: {e}");
                ctx.push_turn(TurnKind::Rejection, "Validator", content);
                Ok(Validation::Failure(e.to_string()))
            }
        }
    }
    pub(crate) async fn run_cargo_check() -> Result<Self> {
        let mut cmd = Command::new("cargo");
        cmd.args(["check"]);
        crate::orchestrator::cargo::limit_resources(&mut cmd, 30);
        let output = tokio::time::timeout(Duration::from_secs(30), cmd.output()).await;
        match output {
            Ok(Ok(output)) if output.status.success() => Ok(Validation::Success),
            Ok(Ok(output)) => {
                let err = String::from_utf8_lossy(&output.stderr).to_string();
                Ok(Validation::Failure(err))
            }
            Ok(Err(e)) => Ok(Validation::Failure(format!(
                "Failed to execute cargo check: {e}"
            ))),
            Err(_) => Ok(Validation::Failure(
                "Cargo check timed out after 30s".into(),
            )),
        }
    }

    pub(crate) async fn run_build_and_test(cancel: &CancellationToken) -> Result<BuildReport> {
        let mut check_cmd = Command::new("cargo");
        check_cmd.args(["check", "--message-format=json"]);
        crate::orchestrator::cargo::limit_resources(&mut check_cmd, 60);
        let check = tokio::time::timeout(
            Duration::from_secs(60),
            async {
                tokio::select! {
                    out = check_cmd.output() => Ok(out),
                    _ = cancel.cancelled() => Err(()),
                }
            },
        )
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
            let test = tokio::time::timeout(
                Duration::from_secs(120),
                async {
                    tokio::select! {
                        out = test_cmd.output() => Ok(out),
                        _ = cancel.cancelled() => Err(()),
                    }
                },
            )
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
            parsed_diagnostics: vec![diag("warning", "unused variable: `x`", Some("src/bar.rs"), Some(7))],
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
        assert_eq!(report.rejection_message(), "cargo check timed out after 60s");
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
    fn file_in_scope_matches_exact_and_suffix() {
        let planned = vec!["src/foo.rs".to_string()];
        assert!(file_in_scope("src/foo.rs", &planned));
        // Plan named just the basename, target resolved with a directory prefix.
        assert!(file_in_scope(
            "src/foo.rs",
            &["foo.rs".to_string()]
        ));
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
}
