//! Removes lookup-tool directives from the Architect's plan before the Worker reads it.
//!
//! The Architect has no tools and its template says so plainly, yet in real runs it opens
//! nearly every plan with a step like "1. Run `cargo clippy --lib -p ruchat` to see the current
//! warnings". `worker.md` interpolates `PLAN: {{PLAN}}` verbatim, so the Worker obeys — spending
//! the round's single information-lookup on a tool whose output is already sitting in its
//! DOCUMENTS section, then getting refused for having nothing left to spend. In trace 531 two of
//! five rounds died this way without the Worker ever attempting the edit it was there to make.
//!
//! This is deliberately a structural fix rather than more prompt text: `architect.md` already
//! spends five paragraphs forbidding the behaviour and the orchestrator already injects a
//! "this plan is identical to the previous round's" note, and the model repeated the plan four
//! times regardless. See `TODO.md` item 25.
//!
//! Only *read-only* directives are stripped (`ToolName::is_read_only_lookup`). A plan step telling
//! the Worker to apply a patch or memorize something is the Worker's actual job and must survive
//! untouched — stripping those would break the run rather than help it.

use crate::agent::tools::ToolName;

/// Verbs that make a line an *instruction to invoke* a tool rather than a mention of one.
/// "Run cargo clippy" is a directive; "the cargo clippy output above shows..." is a reference to
/// data already retrieved, and must be kept — it is often the only place the plan states which
/// warning it picked.
const INVOCATION_VERBS: [&str; 6] = ["run", "execute", "call", "invoke", "use", "rerun"];

/// Markers whose line must never be dropped whatever else it contains: the Worker's patch is
/// refused outright if the `FILES:` scope is missing (`protocol.rs`), and `CHOICE:` carries the
/// concrete file/line/symbol the whole plan exists to communicate.
const PROTECTED_MARKERS: [&str; 3] = ["FILES:", "CHOICE:", "PLAN:"];

/// Every spelling a plan might use for a read-only tool: the canonical snake_case name plus the
/// spaced form models actually write ("cargo clippy", "read file").
fn read_only_tool_aliases() -> Vec<String> {
    let mut aliases = Vec::new();
    for tool in ToolName::ALL {
        if !tool.is_read_only_lookup() {
            continue;
        }
        let canonical = tool.as_str().to_string();
        aliases.push(canonical.replace('_', " "));
        aliases.push(canonical);
    }
    aliases
}

/// True when `line` reads as an instruction to invoke a read-only lookup tool.
fn is_lookup_directive(line: &str, aliases: &[String]) -> bool {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return false;
    }
    if PROTECTED_MARKERS
        .iter()
        .any(|m| trimmed.to_uppercase().contains(m))
    {
        return false;
    }
    let lower = trimmed.to_lowercase();
    if !aliases.iter().any(|a| lower.contains(a.as_str())) {
        return false;
    }
    // The verb has to lead the step, not merely appear somewhere in it — "...so that we can run
    // the tests later" should not qualify. Strip any list marker ("1.", "-", "*") first, since
    // plans are written as numbered steps.
    let body = lower
        .trim_start_matches(|c: char| c.is_ascii_digit() || c == '.' || c == ')' || c == '-')
        .trim_start_matches(['*', '#', ' '])
        .trim_start();
    INVOCATION_VERBS
        .iter()
        .any(|v| body.starts_with(v) && body[v.len()..].starts_with(' '))
}

/// Drops lookup-tool directives from `plan`, leaving everything else byte-identical.
///
/// Returns the plan unchanged when nothing matched, so the overwhelmingly common case costs
/// only the scan. When something *was* removed the Worker gets one short note in place of the
/// removed steps — silently deleting a numbered step would leave a plan whose numbering skips,
/// which reads as truncation and invites the Worker to "recover" the missing step by guessing.
pub(crate) fn strip_lookup_directives(plan: &str) -> String {
    let aliases = read_only_tool_aliases();
    if !plan.lines().any(|l| is_lookup_directive(l, &aliases)) {
        return plan.to_string();
    }
    let kept: Vec<&str> = plan
        .lines()
        .filter(|l| !is_lookup_directive(l, &aliases))
        .collect();
    format!(
        "{}\n\n(One or more plan steps instructing an information-lookup tool were removed: the \
         Architect has no tools, and any output it refers to is already in DOCUMENTS above. Do \
         not re-run a lookup to recover them — spend this round's action on the actual change.)",
        kept.join("\n").trim_end()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // The verbatim opening step from trace 531's plan, which every round repeated and the Worker
    // obeyed every round.
    const TRACE_531_PLAN: &str = "PLAN:\n\
        1. Run `cargo clippy --lib -p ruchat` to see the current Clippy warnings in the `src/` directory.\n\
        2. Identify and review the first warning reported by Clippy that can be fixed with a single edit.\n\
        \n\
        CHOICE: Remove the unused field `options` from the `Agent` struct in `src/core/agent.rs`.\n\
        \n\
        FILES: src/core/agent.rs";

    #[test]
    fn strips_the_verbatim_trace_531_lookup_step() {
        let out = strip_lookup_directives(TRACE_531_PLAN);
        assert!(!out.contains("1. Run `cargo clippy"));
        // Everything load-bearing survives.
        assert!(out.contains("CHOICE: Remove the unused field"));
        assert!(out.contains("FILES: src/core/agent.rs"));
        assert!(out.contains("2. Identify and review the first warning"));
    }

    #[test]
    fn keeps_a_plan_with_no_directives_byte_identical() {
        let plan =
            "PLAN:\n1. Remove the field at src/core/agent.rs:82.\n\nFILES: src/core/agent.rs";
        assert_eq!(strip_lookup_directives(plan), plan);
    }

    // apply_patch/memorize are the Worker's job, not a budgeted lookup — stripping these would
    // remove the instruction the round exists to carry out.
    #[test]
    fn never_strips_a_write_tool_directive() {
        let plan = "1. Run apply_patch to remove the field.\n2. Use memorize to record the change.";
        assert_eq!(strip_lookup_directives(plan), plan);
    }

    // A plan that *refers* to already-retrieved output is how it states which warning it picked;
    // dropping that line would throw away the choice itself.
    #[test]
    fn keeps_a_reference_to_already_retrieved_tool_output() {
        let plan = "The cargo clippy output above shows `options` is never read, so fix that one.";
        assert_eq!(strip_lookup_directives(plan), plan);
    }

    #[test]
    fn keeps_a_trailing_mention_of_running_something_later() {
        let plan = "1. Delete the field so that we can run cargo check afterwards.";
        assert_eq!(strip_lookup_directives(plan), plan);
    }

    #[test]
    fn strips_bulleted_and_bare_directives_too() {
        let plan = "- Execute ripgrep for the symbol\n* Call read_file on src/main.rs\nInvoke git_log for context";
        let out = strip_lookup_directives(plan);
        assert!(!out.contains("ripgrep"));
        assert!(!out.contains("read_file"));
        assert!(!out.contains("git_log"));
    }

    #[test]
    fn tells_the_worker_why_a_step_is_missing() {
        let out = strip_lookup_directives(TRACE_531_PLAN);
        assert!(out.contains("already in DOCUMENTS"));
    }

    // FILES: is load-bearing — `Validation::apply_patch` refuses a patch outside the declared
    // scope — so it must survive even if a tool name somehow lands on that line.
    #[test]
    fn never_drops_the_files_scope_line() {
        let plan = "1. Run ripgrep first.\nFILES: src/core/agent.rs, src/ripgrep_helper.rs";
        let out = strip_lookup_directives(plan);
        assert!(out.contains("FILES: src/core/agent.rs, src/ripgrep_helper.rs"));
    }
}
