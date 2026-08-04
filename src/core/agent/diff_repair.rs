//! Tolerant pre-parse repairs for the diff-syntax mistakes local models reliably make, so
//! `diffy::Patch::from_str` sees something it can actually parse instead of refusing outright.
//! Pulled out of `protocol.rs` (2026-08-04) once that file had accumulated enough of these small,
//! similarly-shaped repair functions from the reliability-fix session to crowd out
//! `Validation::apply_patch` itself — a pure extraction, no behavior change. See `TODO.md`'s
//! pinned reliability item for the real-run evidence behind each one.

use std::sync::OnceLock;

/// Repairs the two most common diff mistakes coder-tuned local models make: (1) dropping the
/// mandatory leading space on an unchanged (context) line inside a hunk, and (2) inserting a
/// genuinely blank separator line *between* two hunks of the same file — real unified diffs never
/// have one; the next hunk's `@@ ... @@` header always immediately follows the previous hunk's
/// last body line. Unified diff requires every hunk-body line to start with ' ' (context), '+'
/// (added), '-' (removed), or '\' (no-newline marker) — `diffy::Patch::from_str` rejects anything
/// else outright, a fully blank line included, which it can't distinguish from the patch having
/// ended (surfacing a cryptic "orphaned hunk header after trailing content" once it then hits the
/// next hunk's `@@` line — a real, live-verified failure, see TODO.md's pinned reliability item).
/// Both cases are repaired the same way: any hunk-body line missing one of the four valid
/// prefixes — blank or not — is treated as an implicit (possibly empty) context line and given a
/// leading space. This does not change what the diff says to add/remove, only makes an
/// otherwise-valid diff parseable; if the file's real content doesn't actually have a blank line
/// at that position, `diffy::apply` still fails with its normal, actionable context-mismatch
/// message afterward — never a silent wrong edit. A genuinely wrong diff (bad content, mismatched
/// context, wrong file) still fails at parse or apply time same as before.
pub(super) fn normalize_diff_hunk_lines(diff: &str) -> String {
    let mut out = String::with_capacity(diff.len());
    let mut in_hunk = false;
    for line in diff.split_inclusive('\n') {
        let trimmed = line.trim_end_matches('\n');
        if trimmed.starts_with("--- ") || trimmed.starts_with("+++ ") {
            in_hunk = false;
        } else if trimmed.starts_with("@@") {
            in_hunk = true;
        } else if in_hunk && !trimmed.starts_with([' ', '+', '-', '\\']) {
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
pub(super) fn fix_hunk_header_counts(diff: &str) -> String {
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

/// Repairs a diff that's missing its `--- a/<file>`/`+++ b/<file>` header entirely — just a bare
/// `@@ ... @@` hunk (or several) with no header line at all. `diffy::Patch::from_str` needs this
/// header to know which file the hunks apply to; without it, `apply_patch` used to only ever
/// refuse and ask the Worker to resubmit with a header added — but real runs showed the Worker
/// not reliably doing that within its remaining round budget (see TODO.md's pinned reliability
/// item; this is the second of the two contributors identified there in the live-verified
/// `fix_one_clippy_lint` run and `ruchat_traces/failures/ruchat_trace_66.md`).
///
/// Only repairs when it's unambiguous: zero existing `--- ` lines (a header that's merely
/// malformed is left alone rather than second-guessed — the multi-file check right after this
/// runs still needs to see the diff as submitted) AND the plan's `FILES:` line names exactly one
/// file. With zero or more than one planned file there's no safe way to guess which file a bare
/// hunk belongs to, so the existing "add a header" refusal below still fires in those cases.
pub(super) fn ensure_diff_has_file_header(diff: &str, planned: &[String]) -> String {
    let already_has_header = diff.lines().any(|l| l.starts_with("--- "));
    let has_hunk = diff.lines().any(|l| l.starts_with("@@"));
    if already_has_header || !has_hunk {
        return diff.to_string();
    }
    let [only_file] = planned else {
        return diff.to_string();
    };
    format!("--- a/{only_file}\n+++ b/{only_file}\n{diff}")
}

/// How many real context lines to put either side of a relocated deletion when rebuilding its
/// hunk — the customary unified-diff default.
const REALIGN_CONTEXT_LINES: usize = 3;

/// Re-anchors a *pure-deletion* hunk whose removed lines really do exist in the file, but whose
/// surrounding context lines (and/or `@@` offsets) don't match it — so `diffy::apply` rejects an
/// edit whose actual intent was correct and unambiguous.
///
/// The live failure this fixes (traces 499/500, `fix_one_clippy_lint`): clippy flagged
/// `src/core/agent.rs:82: field `options` is never read`, and the Worker correctly emitted
/// `-    options: ModelOptions,` — but wrote the struct's *other* fields around it in the wrong
/// order, so no such context block exists in the file and the patch could never apply. The model
/// then re-emitted the identical diff every remaining round, because from its point of view it
/// was already deleting exactly the field it was asked to delete.
///
/// Only acts where the intent is unambiguous, otherwise returns `None` and leaves the existing
/// rejection path (which shows the real line-numbered file content) to handle it:
/// - The hunk must be a pure deletion — at least one `-` line and no `+` lines. A hunk that also
///   adds lines needs an insertion point, which can't be inferred once its context is known-bad.
/// - The removed lines must appear in the file as one contiguous, in-order block, and that block
///   must occur **exactly once**. Ambiguity is left alone rather than guessed at.
/// - Matching compares trimmed text (the model's indentation is frequently off) but the rebuilt
///   hunk always uses the file's own exact bytes, so the result applies cleanly by construction.
///
/// This only ever changes *where* a deletion is anchored, never *what* it deletes — the rebuilt
/// hunk removes lines whose trimmed text the model itself wrote. It is deliberately run only
/// after `diffy::apply` has already failed, so a diff that applies as submitted is never touched.
pub(super) fn realign_pure_deletion_hunks(diff: &str, original: &str) -> Option<String> {
    let file_lines: Vec<&str> = original.lines().collect();
    let lines: Vec<&str> = diff.split_inclusive('\n').collect();
    let mut out = String::with_capacity(diff.len());
    let mut changed = false;
    let mut next_allowed_start = 0usize;
    let mut i = 0;
    while i < lines.len() {
        let trimmed = lines[i].trim_end_matches('\n');
        if !trimmed.starts_with("@@") {
            out.push_str(lines[i]);
            i += 1;
            continue;
        }
        // Collect this hunk's body: everything up to the next hunk or file header.
        let mut j = i + 1;
        while j < lines.len() {
            let t = lines[j].trim_end_matches('\n');
            if t.starts_with("@@") || t.starts_with("--- ") || t.starts_with("+++ ") {
                break;
            }
            j += 1;
        }
        let body: Vec<&str> = lines[i + 1..j]
            .iter()
            .map(|l| l.trim_end_matches('\n'))
            .collect();
        match rebuild_deletion_hunk(&body, &file_lines) {
            Some((rebuilt, start)) => {
                // A relocated hunk can land before an earlier (or unrepaired) one's start.
                // `diffy` requires ascending, non-overlapping hunks, so a diff that lost that
                // ordering would just fail to parse and throw the repair away — bail instead and
                // let the normal rejection path, which shows the real file, handle it.
                if start < next_allowed_start {
                    return None;
                }
                next_allowed_start = start + REALIGN_CONTEXT_LINES;
                out.push_str(&rebuilt);
                changed = true;
            }
            None => {
                for line in &lines[i..j] {
                    out.push_str(line);
                }
            }
        }
        i = j;
    }
    changed.then_some(out)
}

/// Rebuilds one hunk body against the real file, returning the rebuilt hunk text and the 0-based
/// file line it now starts at, or `None` if this isn't an unambiguously relocatable pure
/// deletion. See `realign_pure_deletion_hunks` for the rules.
fn rebuild_deletion_hunk(body: &[&str], file_lines: &[&str]) -> Option<(String, usize)> {
    if body.iter().any(|l| l.starts_with('+')) {
        return None;
    }
    // The `-` lines must be adjacent *in the hunk body* too, not just findable adjacently in the
    // file. Without this, a body like " a / -b / c / -d / e" would search for `b` immediately
    // followed by `d` — a pair the model never claimed was contiguous — and, if that pair happens
    // to occur once elsewhere (easy with short repeated lines like `}`), relocate onto it and
    // delete two lines nothing pointed at. Requiring the diff's own claim to match what gets
    // searched for keeps this from ever being a silent wrong edit.
    let first = body.iter().position(|l| l.starts_with('-'))?;
    let last = body.iter().rposition(|l| l.starts_with('-'))?;
    if body[first..=last].iter().any(|l| !l.starts_with('-')) {
        return None;
    }
    let removed: Vec<&str> = body[first..=last]
        .iter()
        .filter_map(|l| l.strip_prefix('-'))
        .map(str::trim)
        .collect();
    if removed.is_empty() {
        return None;
    }
    // Locate the removed block in the file, requiring exactly one contiguous in-order match.
    let mut matches = file_lines
        .windows(removed.len())
        .enumerate()
        .filter(|(_, w)| w.iter().map(|l| l.trim()).eq(removed.iter().copied()))
        .map(|(idx, _)| idx);
    let start = matches.next()?;
    if matches.next().is_some() {
        return None; // ambiguous — refuse to guess which occurrence was meant
    }
    let ctx_start = start.saturating_sub(REALIGN_CONTEXT_LINES);
    let ctx_end = (start + removed.len() + REALIGN_CONTEXT_LINES).min(file_lines.len());
    let mut rebuilt = String::new();
    let old_count = ctx_end - ctx_start;
    let new_count = old_count - removed.len();
    rebuilt.push_str(&format!(
        "@@ -{},{old_count} +{},{new_count} @@\n",
        ctx_start + 1,
        ctx_start + 1
    ));
    for (idx, line) in file_lines.iter().enumerate().take(ctx_end).skip(ctx_start) {
        let prefix = if (start..start + removed.len()).contains(&idx) {
            '-'
        } else {
            ' '
        };
        rebuilt.push(prefix);
        rebuilt.push_str(line);
        rebuilt.push('\n');
    }
    Some((rebuilt, ctx_start))
}

#[cfg(test)]
mod tests {
    use super::*;

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

    // Regression: a real, live-verified failure (see TODO.md's pinned reliability item,
    // ruchat_trace_483.md round 2) — qwen2.5-coder:14b wrote a multi-hunk diff for
    // one file but put a genuinely blank line *between* the hunks (real unified diffs never do
    // this — the next hunk's `@@` header always immediately follows the previous hunk's last
    // body line). `diffy::Patch::from_str` used to reject this outright ("orphaned hunk header
    // after trailing content"), a cryptic parse error giving the Worker nothing to act on. The
    // blank line is now treated as an implicit empty context line, same as any other
    // missing-prefix hunk-body line.
    #[test]
    fn normalize_repairs_a_blank_separator_line_between_two_hunks() {
        let diff = "--- a/src/foo.rs\n+++ b/src/foo.rs\n\
            @@ -1,3 +1,2 @@\n fn a() {}\n-fn b() {}\n fn c() {}\n\
            \n\
            @@ -10,3 +9,2 @@\n fn x() {}\n-fn y() {}\n fn z() {}\n";
        // Confirms the actual, previously-broken behavior: diffy chokes on the bare blank line
        // and treats the following `@@` as unexpected trailing content, exactly like the real
        // trace's "orphaned hunk header after trailing content" error.
        assert!(diffy::Patch::from_str(diff).is_err());
        // Mirrors the real pipeline (`Validation::apply_patch`): normalize, then recompute hunk
        // header counts (the blank line becoming a real body line shifts them), then parse.
        let normalized = normalize_diff_hunk_lines(diff);
        let fixed = fix_hunk_header_counts(&normalized);
        diffy::Patch::from_str(&fixed)
            .expect("a blank line between hunks should no longer break parsing");
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

    #[test]
    fn ensure_diff_has_file_header_synthesizes_one_when_the_plan_names_exactly_one_file() {
        let diff = "@@ -1,3 +1,3 @@\n line one\n-line two\n+line two changed\n line three\n";
        let repaired = ensure_diff_has_file_header(diff, &["src/foo.rs".to_string()]);
        assert!(
            repaired.starts_with("--- a/src/foo.rs\n+++ b/src/foo.rs\n@@"),
            "got: {repaired:?}"
        );
    }

    #[test]
    fn ensure_diff_has_file_header_leaves_diff_unchanged_with_zero_or_multiple_planned_files() {
        let diff = "@@ -1,3 +1,3 @@\n line one\n-line two\n+line two changed\n line three\n";
        assert_eq!(ensure_diff_has_file_header(diff, &[]), diff);
        assert_eq!(
            ensure_diff_has_file_header(
                diff,
                &["src/foo.rs".to_string(), "src/bar.rs".to_string()]
            ),
            diff
        );
    }

    #[test]
    fn ensure_diff_has_file_header_does_not_second_guess_an_existing_header() {
        let diff = "--- a/src/foo.rs\n+++ b/src/foo.rs\n@@ -1,1 +1,1 @@\n-x\n+y\n";
        assert_eq!(
            ensure_diff_has_file_header(diff, &["src/bar.rs".to_string()]),
            diff
        );
    }

    #[test]
    fn ensure_diff_has_file_header_leaves_a_diff_with_no_hunk_at_all_unchanged() {
        let diff = "not a diff at all";
        assert_eq!(
            ensure_diff_has_file_header(diff, &["src/foo.rs".to_string()]),
            diff
        );
    }

    /// The real struct `fix_one_clippy_lint` kept failing on, byte-for-byte from
    /// `src/core/agent.rs` at the time (lines 81-88).
    const AGENT_RS: &str = "fn unrelated() {\n    false\n}\n\npub(crate) struct Agent {\n    options: ModelOptions,\n    agent_config: HashMap<String, Value>,\n    pub(super) embed_args: Option<EmbedArgs>,\n    cfg: Value,\n}\n\nimpl Agent {\n";

    #[test]
    fn realigns_the_verbatim_trace_500_diff_onto_the_right_line() {
        // Verbatim from ruchat_traces/ruchat_trace_500.md round 2: the removed line is exactly
        // right (`options`, the field clippy flagged), but the model listed the struct's other
        // fields in the wrong order, so this context block appears nowhere in the file and
        // diffy::apply could never match it — the run then looped on this identical diff.
        let submitted = "--- a/src/core/agent.rs\n+++ b/src/core/agent.rs\n@@ -81,7 +81,6 @@ pub struct Agent {\n     agent_config: HashMap<String, Value>,\n     pub(super) embed_args: Option<EmbedArgs>,\n     cfg: Value,\n-    options: ModelOptions,\n }\n";
        // Run the same pre-processing `apply_patch` does before it ever calls diffy, so this
        // exercises the diff exactly as the real pipeline sees it.
        let diff = fix_hunk_header_counts(&normalize_diff_hunk_lines(submitted));
        // Precondition: after that repair it parses, but still genuinely does not apply.
        let as_submitted = diffy::Patch::from_str(&diff).expect("parses");
        assert!(diffy::apply(AGENT_RS, &as_submitted).is_err());

        let repaired = realign_pure_deletion_hunks(&diff, AGENT_RS).expect("should realign");
        let patch = diffy::Patch::from_str(&repaired).expect("repaired diff should parse");
        let patched = diffy::apply(AGENT_RS, &patch).expect("repaired diff should apply");

        // Deletes `options` and nothing else — every other field survives verbatim.
        assert!(!patched.contains("options: ModelOptions,"));
        assert!(patched.contains("    agent_config: HashMap<String, Value>,"));
        assert!(patched.contains("    pub(super) embed_args: Option<EmbedArgs>,"));
        assert!(patched.contains("    cfg: Value,"));
        assert_eq!(patched.lines().count(), AGENT_RS.lines().count() - 1);
    }

    #[test]
    fn realign_refuses_when_the_removed_line_is_ambiguous() {
        // Two identical candidate lines — relocating would be a coin flip, so it must decline
        // and let the normal context-mismatch rejection show the real file instead.
        let file = "a\n    dup();\nb\n    dup();\nc\n";
        let diff = "--- a/f.rs\n+++ b/f.rs\n@@ -1,3 +1,2 @@\n zzz\n-    dup();\n yyy\n";
        assert!(realign_pure_deletion_hunks(diff, file).is_none());
    }

    #[test]
    fn realign_refuses_when_removed_lines_are_split_by_context() {
        // The body claims `b` and `d` are separated by `c`, so searching for them as an adjacent
        // pair would be searching for something the diff never asserted — and here that pair does
        // occur, once, further down. Relocating onto it would silently delete two lines nothing
        // pointed at, so this must decline instead.
        let file = "x\nb\nc\nd\ny\nb\nd\nz\n";
        let diff = "--- a/f.rs\n+++ b/f.rs\n@@ -1,5 +1,3 @@\n a\n-b\n c\n-d\n e\n";
        assert!(realign_pure_deletion_hunks(diff, file).is_none());
    }

    #[test]
    fn realign_refuses_a_hunk_that_also_adds_lines() {
        // A replacement needs an insertion point, which can't be inferred once the hunk's
        // context is known not to match.
        let file = "a\n    old();\nb\n";
        let diff =
            "--- a/f.rs\n+++ b/f.rs\n@@ -1,3 +1,3 @@\n zzz\n-    old();\n+    new();\n yyy\n";
        assert!(realign_pure_deletion_hunks(diff, file).is_none());
    }

    #[test]
    fn realign_refuses_when_the_removed_line_is_absent_entirely() {
        let file = "a\nb\nc\n";
        let diff = "--- a/f.rs\n+++ b/f.rs\n@@ -1,2 +1,1 @@\n a\n-    nonexistent();\n";
        assert!(realign_pure_deletion_hunks(diff, file).is_none());
    }

    #[test]
    fn realign_matches_despite_wrong_indentation() {
        // Models routinely get leading whitespace wrong; the rebuilt hunk still uses the file's
        // own exact bytes, so it applies cleanly.
        let file = "fn a() {\n        deeply_indented();\n}\n";
        let diff = "--- a/f.rs\n+++ b/f.rs\n@@ -1,2 +1,1 @@\n zzz\n-deeply_indented();\n";
        let repaired = realign_pure_deletion_hunks(diff, file).expect("should realign");
        let patch = diffy::Patch::from_str(&repaired).expect("parses");
        let patched = diffy::apply(file, &patch).expect("applies");
        assert_eq!(patched, "fn a() {\n}\n");
    }
}
