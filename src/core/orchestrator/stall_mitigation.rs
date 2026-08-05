//! Deterministic mitigations for the mechanical ways a local model stalls a round instead of
//! making progress — repeating a read-only lookup, substituting `memorize` for a real fix,
//! hallucinating a tool call inside the Architect's plan, or writing a diff against guessed file
//! content. Pulled out of `orchestrator.rs` (2026-08-04) once that file had accumulated enough of
//! these small, similarly-shaped fixes from the reliability-fix session that they were crowding
//! out the actual stage machine — a pure extraction, no behavior change. See `TODO.md`'s pinned
//! reliability item for the real-run evidence behind each one.

use crate::agent::tools::ToolName;
use crate::agent::types::{Context, TurnKind};

/// Read-only tools the Worker may call once per run as a budgeted information-lookup
/// (`retrieve_budget`) — every other Worker tool call must be `apply_patch`/`memorize`. Pulled
/// out as its own predicate so `Stage::Implement`'s first-call check and its second-chance check
/// (see the nudge-and-reask loop below) can't drift apart into two different tool lists.
pub(super) fn is_read_only_worker_tool(tool: &ToolName) -> bool {
    // Delegates to the enum so this and `plan_sanitize::strip_lookup_directives` can never
    // disagree about which tools cost a lookup. Kept as a named function because the call sites
    // in `run_implement_patch_loop` read better with it and its tests below pin the behavior.
    tool.is_read_only_lookup()
}

/// True if this round already retrieved real, actionable `cargo_clippy`/`cargo_check` output —
/// i.e. it actually contains a compiler/clippy diagnostic, not just a clean "nothing to report"
/// run. Used by `run_implement_patch_loop` to catch a `memorize` call substituting for an actual
/// fix: a real live-verified run (see TODO.md's pinned reliability item) showed the Worker
/// calling `cargo_clippy`, seeing genuine warnings, then `memorize`-ing a note about them instead
/// of ever calling `apply_patch` — and the Validator (an LLM call, not deterministic) approved
/// that exact substitution on one round after correctly rejecting the identical thing the round
/// before. This is a deterministic backstop specifically for that failure shape, not a general
/// replacement for the Validator's broader judgment elsewhere.
pub(super) fn round_has_actionable_diagnostics(ctx: &Context) -> bool {
    ctx.turns.iter().any(|t| {
        t.round == ctx.round
            && t.kind == TurnKind::Retrieval
            && matches!(t.source.as_str(), "CargoClippy" | "CargoCheck")
            && (t.content.contains("warning:") || t.content.contains("error:"))
    })
}

/// `agent_role/architect.md` explicitly forbids the Architect from ever emitting a `tool_call` —
/// it's plan-only, no tools, only the Worker calls tools — and never asks for an
/// "IMPLEMENTATION:" section either (only PLAN/CHOICE/FILES) — but real runs show the local
/// model writing one anyway, embedding a full (often hallucinated) `apply_patch` diff inside
/// what's nominally its plan. The fence label under that heading varies across real traces
/// (`\`\`\`tool_call`, `\`\`\`json`, `\`\`\`sh` all seen — see TODO.md's pinned reliability item),
/// so matching on the fence text alone doesn't generalize — a `\`\`\`json`-fenced hallucination
/// slipped through the first version of this function entirely. The "IMPLEMENTATION:" heading
/// itself is the reliable signal instead, since the Architect should never produce that section
/// under any label at all. Truncates at the first line that trims to exactly "IMPLEMENTATION:"
/// (case-insensitive, matching `Context::planned_files`'s own "FILES:" convention) and drops
/// everything from there on. Tolerates the label being wrapped as a markdown heading or bold text
/// (`### IMPLEMENTATION:`, `**IMPLEMENTATION:**`) — a real live-verified run (see TODO.md's
/// pinned reliability item) showed a bare-line match alone still missing it: the model formatted
/// every section label as a `### `-prefixed heading, so `"### IMPLEMENTATION:".trim()` never
/// equals the bare string this used to compare against, and the hallucination (including a wrong
/// JSON shape the Worker then copied verbatim for 5 rounds straight) sailed through untouched.
/// Falls back to truncating at a bare `\`\`\`tool_call` fence with no preceding label, for the
/// rarer case seen without one. Leaves the plan untouched if neither appears.
pub(super) fn strip_architect_tool_call_hallucination(plan: &str) -> String {
    let mut offset = 0;
    for line in plan.split_inclusive('\n') {
        let label = line
            .trim()
            .trim_start_matches(['#', '*', ' '])
            .trim_end_matches(['*', ' ']);
        if label.eq_ignore_ascii_case("IMPLEMENTATION:") {
            return plan[..offset].trim_end().to_string();
        }
        offset += line.len();
    }
    let Some(idx) = plan.find("```tool_call") else {
        return plan.to_string();
    };
    plan[..idx].trim_end().to_string()
}

/// Cap on how much of a planned target file's real content gets auto-injected per round — same
/// size as `MAX_SHOWN_ORIGINAL_CHARS`'s post-rejection grounding dump (`agent/protocol.rs`), just
/// proactive instead of reactive.
const MAX_AUTO_GROUNDED_FILE_CHARS: usize = 4_000;

/// Proactively shows the plan's target file's real, line-numbered content to the Worker before it
/// ever writes a diff, instead of only doing this reactively after a failed `apply_patch` (see
/// `Validation::apply_patch`'s existing grounding-on-mismatch rejection). Real live-verified runs
/// (see TODO.md's pinned reliability item) repeatedly showed the Worker writing a diff against a
/// file's *guessed* content — sometimes without ever calling `read_file` on it at all, sometimes
/// after calling it once but the content apparently not being attended to by a later round of a
/// multi-round run — fabricating fields or comments that don't exist in the real file. Only acts
/// when the plan names exactly one file (same "unambiguous" bar `ensure_diff_has_file_header`
/// uses) — with zero or multiple planned files there's no single safe target to show.
/// Deliberately re-injected every round, not just once: the same recency reasoning behind
/// everything else in this section — local models attend far better to content near the end of
/// their context than to something shown several rounds ago. Any *earlier* grounding turn is
/// removed before the fresh one is pushed, rather than left in place alongside it — without this,
/// a multi-round run repeatedly targeting the same file (the common case) would accumulate one
/// near-duplicate ~4000-char dump per round with nothing to ever compress them (these test runs
/// have no Summarizer configured), a real, self-inflicted contributor to context bloat found
/// while investigating why later rounds of a run seemed to ignore instructions that were
/// technically already in context — see TODO.md's pinned reliability item. Keeping exactly one,
/// always freshly re-inserted, gets the recency benefit without the duplication cost. Best-effort
/// like every other diagnostic-nicety path in this codebase (`Checkpoint::save`): a
/// missing/unreadable file (about to be created, or a real I/O error) is silently skipped rather
/// than surfaced as an error, since this is a proactive nicety, not a required step — and it
/// never costs `retrieve_budget`, since it's orchestrator-driven, not a Worker-initiated lookup.
pub(super) async fn auto_ground_planned_file(ctx: &mut Context) {
    let planned = ctx.planned_files();
    let [target] = planned.as_slice() else {
        return;
    };
    let Ok(content) = tokio::fs::read_to_string(target).await else {
        return;
    };
    ctx.turns.retain(|t| t.source != "AutoGroundedFile");
    let numbered: String = content
        .lines()
        .enumerate()
        .map(|(i, line)| format!("{}:{line}", i + 1))
        .collect::<Vec<_>>()
        .join("\n");
    let shown: String = numbered
        .chars()
        .take(MAX_AUTO_GROUNDED_FILE_CHARS)
        .collect();
    let truncated_note = if numbered.chars().count() > MAX_AUTO_GROUNDED_FILE_CHARS {
        format!(
            "\n... (truncated, {} bytes total — request a narrower range with read_file if you \
            need more)",
            content.len()
        )
    } else {
        String::new()
    };
    ctx.push_turn(
        TurnKind::Retrieval,
        "AutoGroundedFile",
        format!(
            "The plan's FILES: line names '{target}' — here is its real current content, with \
            line numbers (N:content), so your diff's context lines and its @@ -a,b +c,d @@ hunk \
            header match it exactly. Do not guess or assume content not shown here:\n\n\
            {shown}{truncated_note}"
        ),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_read_only_worker_tool_covers_every_budgeted_lookup_tool() {
        for tool in [
            ToolName::Retrieve,
            ToolName::GitLog,
            ToolName::GitBlame,
            ToolName::GitDiff,
            ToolName::GitSearchHistory,
            ToolName::ReadFile,
            ToolName::ListDir,
            ToolName::Ripgrep,
            ToolName::ReadTags,
            ToolName::CargoCheck,
            ToolName::CargoClippy,
            ToolName::CargoDupes,
        ] {
            assert!(
                is_read_only_worker_tool(&tool),
                "{tool:?} should be a budgeted read-only lookup tool"
            );
        }
    }

    #[test]
    fn is_read_only_worker_tool_excludes_the_write_tools() {
        assert!(!is_read_only_worker_tool(&ToolName::ApplyPatch));
        assert!(!is_read_only_worker_tool(&ToolName::Memorize));
    }

    // Regression: a live-verified run (see TODO.md's pinned reliability item) showed the Worker
    // calling cargo_clippy, seeing real warnings, then memorize-ing a note about them instead of
    // ever calling apply_patch — and the Validator (non-deterministic) approved that exact
    // substitution on one round after correctly rejecting the identical thing the round before.
    // `round_has_actionable_diagnostics` is the deterministic backstop for that; not testable
    // through `run_implement_patch_loop` itself (Memorize's `execute_and_verify` branch calls
    // `Agent::embed`, which builds a real Ollama/Chroma client — same "needs live infra" category
    // as `Stage::Test`'s real `cargo test` invocation, out of scope for `--lib` unit tests per
    // this file's own established precedent), so tested directly as the pure predicate it is.
    #[test]
    fn round_has_actionable_diagnostics_true_for_a_real_clippy_warning_this_round() {
        let mut ctx = Context::new("goal".to_string());
        ctx.round = 1;
        ctx.push_turn(
            TurnKind::Retrieval,
            "CargoClippy",
            "src/foo.rs:1:1: warning: field `x` is never read".to_string(),
        );
        assert!(round_has_actionable_diagnostics(&ctx));
    }

    #[test]
    fn round_has_actionable_diagnostics_true_for_a_real_check_error_this_round() {
        let mut ctx = Context::new("goal".to_string());
        ctx.round = 1;
        ctx.push_turn(
            TurnKind::Retrieval,
            "CargoCheck",
            "src/foo.rs:1:1: error: mismatched types".to_string(),
        );
        assert!(round_has_actionable_diagnostics(&ctx));
    }

    #[test]
    fn round_has_actionable_diagnostics_false_for_a_clean_clippy_run() {
        let mut ctx = Context::new("goal".to_string());
        ctx.round = 1;
        ctx.push_turn(
            TurnKind::Retrieval,
            "CargoClippy",
            "    Finished `dev` profile [unoptimized + debuginfo] target(s) in 3.11s".to_string(),
        );
        assert!(!round_has_actionable_diagnostics(&ctx));
    }

    #[test]
    fn round_has_actionable_diagnostics_false_when_the_diagnostics_are_from_an_earlier_round() {
        let mut ctx = Context::new("goal".to_string());
        ctx.round = 1;
        ctx.push_turn(
            TurnKind::Retrieval,
            "CargoClippy",
            "src/foo.rs:1:1: warning: field `x` is never read".to_string(),
        );
        ctx.round = 2;
        assert!(!round_has_actionable_diagnostics(&ctx));
    }

    #[test]
    fn round_has_actionable_diagnostics_false_with_no_diagnostic_turn_at_all() {
        let ctx = Context::new("goal".to_string());
        assert!(!round_has_actionable_diagnostics(&ctx));
    }

    // Regression: a live-verified run (see TODO.md's pinned reliability item) showed the
    // Architect — plan-only, explicitly forbidden by its own prompt from ever emitting a
    // tool_call — hallucinating a full apply_patch diff inside its plan anyway, which the Worker
    // then copied verbatim, unchanged, across 3 separate rounds. Fixture shaped exactly like the
    // real trace: PLAN/CHOICE/FILES text, an earlier unrelated fenced block (the clippy warning
    // quoted back), then an "IMPLEMENTATION:" label and a ```tool_call fence.
    #[test]
    fn strip_architect_tool_call_hallucination_removes_a_hallucinated_implementation_block() {
        let plan = "**PLAN:**\nFix the lint.\n\n**FILES:**\n- src/core/agent.rs\n\n\
            The first warning reported is:\n```\nsrc/core/agent.rs:36:5: warning: field \
            `options` is never read\n```\n\nRemove the unused field.\n\nIMPLEMENTATION:\n\
            ```tool_call\n{\n  \"tool\": \"apply_patch\",\n  \"diff\": \"...\"\n}\n```";
        let stripped = strip_architect_tool_call_hallucination(plan);
        assert!(
            !stripped.contains("tool_call"),
            "hallucinated tool_call should be removed, got: {stripped:?}"
        );
        assert!(
            !stripped.contains("IMPLEMENTATION:"),
            "the dangling label should be removed too, got: {stripped:?}"
        );
        assert!(
            stripped.contains("Remove the unused field."),
            "legitimate plan text before the hallucination should survive, got: {stripped:?}"
        );
        // The earlier, unrelated fenced block (quoting the clippy warning back) is real plan
        // content, not part of the hallucination — must not be touched.
        assert!(stripped.contains("field `options` is never read"));
    }

    #[test]
    fn strip_architect_tool_call_hallucination_leaves_a_real_plan_untouched() {
        let plan = "**PLAN:**\nFix the lint.\n\n**FILES:**\n- src/core/agent.rs";
        assert_eq!(strip_architect_tool_call_hallucination(plan), plan);
    }

    #[test]
    fn strip_architect_tool_call_hallucination_truncates_even_without_a_label_before_it() {
        let plan = "**PLAN:**\nFix the lint.\n\n```tool_call\n{\"tool\": \"apply_patch\"}\n```";
        let stripped = strip_architect_tool_call_hallucination(plan);
        assert_eq!(stripped, "**PLAN:**\nFix the lint.");
    }

    // Regression: the exact case that slipped through the first version of this function, live
    // (see TODO.md's pinned reliability item, ruchat_trace_489.md round 5) — the fence label was
    // ```json, not ```tool_call, so the old substring search never matched at all and the
    // hallucinated diff reached the Worker untouched. Confirms the "IMPLEMENTATION:" heading
    // itself, not any particular fence label, is what actually gets caught now.
    #[test]
    fn strip_architect_tool_call_hallucination_catches_a_json_fenced_variant_too() {
        let plan = "PLAN:\nFix the lint.\n\nCHOICE: some reasoning.\n\nFILES: src/foo.rs\n\n\
            REASON: some more reasoning.\n\nIMPLEMENTATION:\n\
            ```json\n{\n  \"tool\": \"apply_patch\",\n  \"diff\": \"...\"\n}\n```";
        let stripped = strip_architect_tool_call_hallucination(plan);
        assert!(
            !stripped.contains("apply_patch") && !stripped.contains("IMPLEMENTATION:"),
            "the json-fenced hallucination should be removed too, got: {stripped:?}"
        );
        assert!(stripped.contains("FILES: src/foo.rs"));
    }

    // Regression: a live-verified run (see TODO.md's pinned reliability item, ruchat_trace_492.md)
    // showed the previous fix's bare-line match still missing a real case — the Architect
    // formatted the label as a markdown heading (`### IMPLEMENTATION:`), which
    // `"### IMPLEMENTATION:".trim()` never equals the bare "IMPLEMENTATION:" string the old check
    // compared against. The leaked hallucination used a wrong JSON shape
    // (`{"patch": {"path":..., "diff":...}}` instead of the documented flat `{"diff":...}`), which
    // the Worker then copied verbatim for 5 rounds straight, never producing a recognizable
    // tool_call at all.
    #[test]
    fn strip_architect_tool_call_hallucination_catches_a_markdown_heading_label_too() {
        let plan = "PLAN:\nFix the lint.\n\nFILES: src/foo.rs\n\n### CHOICE:\n\nSome reasoning.\n\n\
            ### IMPLEMENTATION:\n\n\
            ```json\n{\"tool\": \"apply_patch\", \"patch\": {\"path\": \"src/foo.rs\", \
            \"diff\": \"...\"}}\n```";
        let stripped = strip_architect_tool_call_hallucination(plan);
        assert!(
            !stripped.contains("apply_patch")
                && !stripped.to_uppercase().contains("IMPLEMENTATION"),
            "the markdown-heading-labeled hallucination should be removed too, got: {stripped:?}"
        );
        assert!(stripped.contains("FILES: src/foo.rs"));
    }

    // Regression: two live-verified runs (see TODO.md's pinned reliability item) showed the
    // Worker writing a diff against a file it never actually read — in one case fabricating a
    // struct field (`pub client: LlmClient,`) that doesn't exist anywhere in the real file.
    // Absolute paths used throughout (not a real repo-relative path + `set_current_dir`) — this
    // codebase has a documented, previously-real flakiness lesson about `set_current_dir` being
    // process-global, not per-test-isolated (see `checkpoint.rs`'s tests) — `planned_files`'s own
    // `a/`/`./`-stripping only touches those specific prefixes, so an absolute tempdir path
    // passes through untouched and needs no CWD change at all.
    #[tokio::test]
    async fn auto_ground_planned_file_shows_real_line_numbered_content_when_unambiguous() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("target.rs");
        std::fs::write(&file, "fn a() {}\nfn b() {}\n").unwrap();

        let mut ctx = Context::new("goal".to_string());
        ctx.push_turn(
            TurnKind::Plan,
            "Architect",
            format!("PLAN: fix it\nFILES: {}", file.display()),
        );

        auto_ground_planned_file(&mut ctx).await;

        let turn = ctx
            .turns
            .iter()
            .find(|t| t.source == "AutoGroundedFile")
            .expect("should have pushed a grounding turn");
        assert!(turn.kind == TurnKind::Retrieval);
        assert!(
            turn.content.contains("1:fn a() {}"),
            "got: {}",
            turn.content
        );
        assert!(
            turn.content.contains("2:fn b() {}"),
            "got: {}",
            turn.content
        );
    }

    #[tokio::test]
    async fn auto_ground_planned_file_does_nothing_with_zero_planned_files() {
        let mut ctx = Context::new("goal".to_string());
        ctx.push_turn(TurnKind::Plan, "Architect", "PLAN: fix it".to_string());
        auto_ground_planned_file(&mut ctx).await;
        assert!(!ctx.turns.iter().any(|t| t.source == "AutoGroundedFile"));
    }

    #[tokio::test]
    async fn auto_ground_planned_file_does_nothing_with_multiple_planned_files_ambiguous() {
        let mut ctx = Context::new("goal".to_string());
        ctx.push_turn(
            TurnKind::Plan,
            "Architect",
            "PLAN: fix it\nFILES: a.rs, b.rs".to_string(),
        );
        auto_ground_planned_file(&mut ctx).await;
        assert!(!ctx.turns.iter().any(|t| t.source == "AutoGroundedFile"));
    }

    #[tokio::test]
    async fn auto_ground_planned_file_silently_skips_a_missing_file() {
        let mut ctx = Context::new("goal".to_string());
        ctx.push_turn(
            TurnKind::Plan,
            "Architect",
            "PLAN: fix it\nFILES: /nonexistent/path/that/does/not/exist_ruchat_test.rs".to_string(),
        );
        auto_ground_planned_file(&mut ctx).await;
        assert!(!ctx.turns.iter().any(|t| t.source == "AutoGroundedFile"));
    }

    // Regression: found while investigating whether later-round failures were really a model-
    // capability limit or something this codebase was contributing itself (see TODO.md's pinned
    // reliability item) — with no Summarizer configured (true of the real repro scripts) and
    // this function re-injecting a fresh grounding dump every round, a multi-round run repeatedly
    // targeting the same file used to accumulate one near-duplicate ~4000-char copy per round,
    // all still sitting in DOCUMENTS with nothing to ever compress them. Exactly one
    // AutoGroundedFile turn must exist at a time, no matter how many rounds re-ground the same
    // (or a different) file.
    #[tokio::test]
    async fn auto_ground_planned_file_replaces_the_previous_grounding_instead_of_accumulating() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("target.rs");
        std::fs::write(&file, "fn a() {}\n").unwrap();

        let mut ctx = Context::new("goal".to_string());
        ctx.push_turn(
            TurnKind::Plan,
            "Architect",
            format!("PLAN: fix it\nFILES: {}", file.display()),
        );

        // Simulate three rounds each re-grounding the same file.
        for round in 1..=3u64 {
            ctx.round = round;
            auto_ground_planned_file(&mut ctx).await;
        }

        let grounding_turns: Vec<_> = ctx
            .turns
            .iter()
            .filter(|t| t.source == "AutoGroundedFile")
            .collect();
        assert_eq!(
            grounding_turns.len(),
            1,
            "expected exactly one grounding turn after 3 rounds, got: {grounding_turns:?}"
        );
        assert_eq!(
            grounding_turns[0].round, 3,
            "the surviving turn should be the freshest one"
        );
    }
}
