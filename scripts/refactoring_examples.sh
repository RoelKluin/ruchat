#!/usr/bin/env bash
# Example `ruchat pipe` invocations for small, low-risk Rust code-quality tasks
# against ruchat's own repository.
#
# These are modeled on a command the maintainer confirmed works:
#
#   ./ruchat pipe --team-model qwen2.5-coder:14b --validator-model qwen2.5-coder:14b \
#     --critic Security --critic Performance --collection repo_src-all-minilm_l6-v2 \
#     --iterations 5 "You are a rust specialist. You work on the ruchat git \
#     repository. Task: rephrase one single line of comment in any rust file."
#
# Every example below is scoped to stay within what the pipeline can *reliably*
# do today:
#   - One file per accepted patch (`apply_patch` applies a single unified diff;
#     the Worker gets exactly one write-tool call per round — see
#     `Stage::Implement` in src/core/orchestrator.rs). Tasks that would require
#     touching multiple files in one commit are NOT here — see
#     refactoring_examples_todo.sh for those and what they're waiting on.
#   - No public API / behavior change, so `cargo check`/`cargo test` (the
#     mandatory Tester stage) is very likely to still pass.
#   - Under apply_patch's 8,000-byte diff cap.
#   - Explicit about *which single file* isn't required — the Scoper/Librarian
#     (via --collection) narrow that down — but each prompt below is more
#     specific than the reference command above about *what kind* of change
#     is wanted and what NOT to change, which noticeably improves how targeted
#     the Architect's resulting plan is.
#
# Prerequisites:
#   - A local Ollama server running with the model(s) referenced below pulled
#     (`ollama pull qwen2.5-coder:14b`).
#   - A local Chroma server with the `repo_src-all-minilm_l6-v2` collection
#     populated from this repo's source (see embed_script.sh / db_config.json).
#   - Run from the repository root, with a clean `git status` — each
#     successful run creates a new `ai/feature-<timestamp>` branch and commits
#     to it, then returns you to your original branch (see
#     `orchestrator/git.rs::commit_feature_branch`).
#
# Usage:
#   bash scripts/refactoring_examples.sh list
#   bash scripts/refactoring_examples.sh run <name>
#
# Nothing runs just by executing this file with no arguments (or `list`) —
# each example is a real agentic run that talks to Ollama/Chroma and, on
# success, creates a real git branch and commit. Pick one explicitly.

set -euo pipefail

RUCHAT_BIN="${RUCHAT_BIN:-./ruchat}"

# Shared, proven-working flags from the maintainer's confirmed command.
# shellcheck disable=SC2034
COMMON_FLAGS=(
  --team-model qwen2.5-coder:14b
  --validator-model qwen2.5-coder:14b
  --collection repo_src-all-minilm_l6-v2
  --iterations 5
)

run_ruchat() {
  # $1 = space-separated extra flags (e.g. critics), $2 = goal text
  local extra_flags="$1"
  local goal="$2"
  # shellcheck disable=SC2086
  "$RUCHAT_BIN" pipe "${COMMON_FLAGS[@]}" $extra_flags "$goal"
}

# --- Examples --------------------------------------------------------------

rephrase_comment() {
  # The maintainer's own confirmed-working baseline, with a slightly tighter
  # prompt: naming what "improve" means and what must NOT change reliably
  # narrows the Architect's plan to a real wording problem instead of
  # rephrasing an already-clear comment for no reason.
  run_ruchat "--critic Security --critic Performance" \
    "You are a rust specialist. You work on the ruchat git repository. Task: \
find one line comment (// or ///) anywhere in src/ that is awkwardly \
worded, redundant, or grammatically off, and rephrase just that one line \
for clarity. Do not change what it technically claims — only the wording. \
If every comment you find is already clear, pick the least clear one \
rather than inventing a problem."
}

fix_typo_in_comment() {
  run_ruchat "--critic Clarity" \
    "You are a rust specialist. You work on the ruchat git repository. Task: \
find one spelling or grammar mistake in a comment or doc comment (// or \
///) anywhere in src/, and fix only that mistake. Do not reword anything \
else in the same comment or touch any code."
}

clarify_doc_comment() {
  run_ruchat "--critic Clarity" \
    "You are a rust specialist. You work on the ruchat git repository. Task: \
pick one pub(crate) function whose /// doc comment is technically correct \
but hard to parse (run-on, ambiguous pronoun, buried the actual behavior), \
and rewrite just that doc comment to be clearer. Preserve every fact it \
states — do not add or remove claims about behavior, only improve the \
wording."
}

add_missing_doc_comment() {
  # Adding a /// summary to an undocumented item is standard rustdoc practice
  # (a WHAT-level summary), distinct from this repo's stricter policy on
  # inline // comments (WHY only, see CLAUDE.md) — the prompt says so
  # explicitly so the model doesn't over-apply the wrong rule.
  run_ruchat "--critic Clarity" \
    "You are a rust specialist. You work on the ruchat git repository. Task: \
find one pub(crate) function or struct in src/ that has no /// doc \
comment, and add a single-line /// summary describing what it does. This \
is a standard rustdoc summary (a brief WHAT), not the inline-comment style \
this repo otherwise prefers (WHY-only) — a short factual summary is \
correct here. Do not change any code."
}

simplify_boolean_expression() {
  run_ruchat "--critic Idiomatic-Rust" \
    "You are a rust specialist. You work on the ruchat git repository. Task: \
search src/ for one redundant boolean comparison (e.g. \`x == true\`, \
\`x == false\`, \`!(!x)\`, or an if/else that just returns true/false) and \
simplify it to the idiomatic form. Only touch that one expression. If \
none exist anywhere in src/, say so instead of inventing a change."
}

remove_redundant_clone() {
  run_ruchat "--critic Performance" \
    "You are a rust specialist. You work on the ruchat git repository. Task: \
find one function in src/ with a \`.clone()\` call that is provably \
unnecessary (the original value is never used again after the clone, or a \
reference would work instead), and remove it. Only make this change if \
you're confident \`cargo check\` will still pass — if you're not sure a \
clone is removable, pick a different, more obviously-redundant one \
instead."
}

convert_loop_to_iterator() {
  run_ruchat "--critic Idiomatic-Rust" \
    "You are a rust specialist. You work on the ruchat git repository. Task: \
find one small \`for\` loop in src/ that only pushes computed values into a \
Vec (no early return, no side effects, no complex control flow), and \
rewrite it as an iterator chain ending in \`.collect()\`. Keep the exact \
same output for the same input — this is a style change, not a behavior \
change."
}

add_unit_test_for_pure_function() {
  run_ruchat "--critic Testing" \
    "You are a rust specialist. You work on the ruchat git repository. Task: \
find one small, pure, already-implemented function in src/ (no I/O, no \
network, no filesystem access) that has no direct unit test covering it \
yet, and add one focused #[test] for it in that same file's existing \
#[cfg(test)] mod tests block (or add that block if the file has none). \
Do not modify the function itself."
}

replace_unwrap_with_question_mark() {
  run_ruchat "--critic Correctness" \
    "You are a rust specialist. You work on the ruchat git repository. Task: \
find one \`.unwrap()\` call in src/core or src/providers where the \
enclosing function already returns a Result, and the value being unwrapped \
comes from a call that could legitimately fail at runtime (not one the \
compiler or an earlier check already guarantees is safe). Replace that one \
\`.unwrap()\` with \`?\` (adding a \`.map_err(...)\` only if the error types \
don't already convert). Change nothing else."
}

extract_magic_number() {
  run_ruchat "--critic Clarity" \
    "You are a rust specialist. You work on the ruchat git repository. Task: \
find one unexplained numeric literal (a 'magic number', not 0, 1, or a \
loop bound) used in src/ without a comment saying what it represents, \
extract it into a named \`const\`, and add a one-line comment on the \
const explaining what the number means. Only touch that one literal and \
its new const."
}

# --- Dispatch ----------------------------------------------------------------

list() {
  cat <<'EOF'
Available examples (each is one small, single-file, low-compile-risk task):

  rephrase_comment              - improve one awkward line comment's wording
  fix_typo_in_comment           - fix one spelling/grammar mistake in a comment
  clarify_doc_comment           - rewrite one unclear /// doc comment
  add_missing_doc_comment       - add a /// summary to one undocumented item
  simplify_boolean_expression   - simplify one redundant boolean comparison
  remove_redundant_clone        - remove one provably-unnecessary .clone()
  convert_loop_to_iterator      - rewrite one push-only for loop as .collect()
  add_unit_test_for_pure_function - add one #[test] for an untested pure fn
  replace_unwrap_with_question_mark - replace one risky .unwrap() with ?
  extract_magic_number          - name one magic number as a const

Run one with:
  bash scripts/refactoring_examples.sh run <name>
EOF
}

main() {
  local cmd="${1:-list}"
  case "$cmd" in
    list) list ;;
    run)
      local name="${2:?usage: $0 run <name> — see: $0 list}"
      if ! declare -F "$name" > /dev/null; then
        echo "Unknown example: $name" >&2
        list >&2
        exit 1
      fi
      "$name"
      ;;
    *)
      echo "Usage: $0 {list|run <name>}" >&2
      exit 1
      ;;
  esac
}

main "$@"
