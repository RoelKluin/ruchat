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
#     populated from this repo's source (`ruchat index src` / `db_config.json`).
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
  --summarizer-model qwen2.5-coder:14b
  --collection repo_src-all-minilm_l6-v2
  --iterations 5
)

run_ruchat_raw() {
  # $1 = space-separated extra flags (e.g. critics), $2 = goal text
  local extra_flags="$1"
  local goal="$2"
  # shellcheck disable=SC2086
  "$RUCHAT_BIN" pipe "${COMMON_FLAGS[@]}" $extra_flags "$goal"
}

# Appended to every *example* goal (added 2026-08-05). Each example names a kind of change, not a
# specific one, so which instance gets fixed was always the model's choice — but nothing said it
# could revise that choice after finding the first candidate intractable. Real runs showed the
# cost: handed a target whose fix spanned two places, the model re-sent the same one-place diff
# every round until the budget ran out, because "pick the first one" left no way to back out.
# Not applied to the reliability gate, which must stay a fixed control.
PICK_ANOTHER="If the specific instance you first pick turns out to need changes in more than \
one place, or your edit is rejected twice for the same reason, abandon it and pick a different \
instance of the same kind of change rather than resubmitting. Only report that no change is \
possible if you have genuinely run out of candidates."

run_ruchat() {
  run_ruchat_raw "$1" "$2 $PICK_ANOTHER"
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

fix_one_clippy_lint() {
  # Needs the `cargo_clippy` typed tool (agent/tools.rs::ToolName::CargoClippy,
  # orchestrator::cargo::cargo_clippy) — added 2026-08-03, previously listed in
  # refactoring_examples_todo.sh as blocked.
  #
  # Deliberately NOT "fix the first warning" any more (changed 2026-08-05). Clippy reports one
  # location per warning, but the fix is not always confined to it: a dead-code warning on a
  # field or variable points at the declaration while the *other* references to it — an
  # initializer, a struct construction — also have to go, and those are neither shown in the
  # warning nor adjacent to it. Pinning the model to the first warning meant a run could be
  # handed a multi-site edit with no way to decline, and it would then re-emit the same
  # single-site diff every round until the budget ran out. Letting it skip to a warning it can
  # actually finish trades a fixed target for a completable one.
  run_ruchat "--critic Idiomatic-Rust" \
    "You are a rust specialist. You work on the ruchat git repository. Task: \
use the cargo_clippy tool to see the crate's current clippy warnings, then pick \
ONE warning reported in src/ that you can fix completely in a single edit to a \
single file, and fix just that one. Prefer the first such warning, but you are \
free to skip any warning whose fix would require touching more than one place \
— for example a dead-code warning where the item is also constructed, \
initialized or referenced elsewhere, since removing only the declaration will \
not compile. If an attempt is rejected, do not resubmit the same diff: either \
extend it to cover every place that must change, or abandon that warning and \
pick a different one. If clippy reports nothing, say so instead of inventing a \
change."
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

rename_helper_and_call_sites() {
  # Needs multi-file patches per round (Stage::Implement's per-round patch
  # budget, up to 3 apply_patch calls) — added 2026-08-03, previously listed
  # in refactoring_examples_todo.sh as blocked. Stays within budget: one file
  # for the definition, up to two more for call sites.
  run_ruchat "--critic Correctness" \
    "You are a rust specialist. You work on the ruchat git repository. Task: \
find one pub(crate) function whose name doesn't clearly describe what it \
does, used from no more than two other files. Rename it and update every \
call site (its definition plus each of those files, at most 3 files \
total)."
}

format_a_few_drifted_files() {
  # Same capability this needs as rename_helper_and_call_sites above.
  run_ruchat "--critic Style" \
    "You are a rust specialist. You work on the ruchat git repository. Task: \
pick 3 files under src/ with cargo fmt formatting drift and reformat just \
those 3 to match cargo fmt's default style, with no behavior change."
}

# --- Reliability gate --------------------------------------------------------
#
# The measurement task for TODO.md section 0's Phase 2 milestone gate. Unlike
# every example above, this one is a *control*, not a challenge: it names the
# exact file and change so that target-selection is not a variable. What it
# measures is only whether the loop can plan -> write a valid diff -> apply ->
# pass the Tester -> commit. That is the question section 0 has never been able
# to answer, because every previous attempt used `fix_one_clippy_lint`, whose
# correct fix needs two hunks (`options` is declared at src/core/agent.rs:82 and
# constructed at :116) — so a failure there could always be either the pipeline
# or the model's inability to decompose a multi-site edit.
#
# Verified one-hunk 2026-08-04, both ways, by applying each by hand and running
# `cargo check --lib`:
#   - delete the trait only (3 lines)      -> compiles; leaves an unused-import warning
#   - delete the `use` + trait (5 lines)   -> compiles clean
# Both are a single contiguous hunk, so a correct run lands either way. That
# forgiving success criterion is deliberate: this measures the loop, not the
# model's thoroughness.
#
# Naturally repeatable: a successful run commits to a new `ai/feature-<ts>`
# branch and returns to the original branch, so `src/providers/llm.rs` on the
# working branch still contains the trait for the next run. No manual reset.

gate_remove_dead_trait() {
  # run_ruchat_raw, not run_ruchat: the gate must NOT get the "pick a different instance" clause
  # every example gets. Its whole purpose is that the target never varies between runs.
  run_ruchat_raw "--critic Idiomatic-Rust" \
    "You are a rust specialist. You work on the ruchat git repository. Task: \
the trait \`LlmProvider\` in src/providers/llm.rs is dead code — nothing \
implements it and nothing calls it. Delete that trait. The \`use \
crate::Result;\` line just above it exists only to support that trait, so you \
may delete that line in the same change too. Do not modify any other file."
}

# Runs the gate N times (default 5) and reports how many landed a REAL commit.
#
# Success is deliberately NOT "a new ai/feature-* branch appeared". Two reasons that measure is
# wrong, both found 2026-08-05 by inspecting the 8 archived agent commits:
#   - 5 of those 8 commits touched only `featured_changes.md`, ruchat's own changelog, which
#     `commit_add_targets` stages unconditionally — so a run that applied no patch at all still
#     produced a branch and a commit. Counting branches would have scored those as lands.
#   - Commits now continue the latest ai/feature-* branch by default rather than creating a new
#     one per run, so branch count stops incrementing at all.
# Instead: record the branch tip before and after, and require a new commit that touches at least
# one file other than featured_changes.md.
# Every commit sitting on an ai/feature-* branch but not on the current branch, sorted so `comm`
# can diff two snapshots of this set.
_agent_commits() {
  git rev-list --branches='ai/feature-*' --not master 2>/dev/null | sort -u
}

# True when commit $1 changes at least one file that isn't ruchat's own changelog. This is what
# separates a real land from a run that applied no patch but still committed featured_changes.md.
_commit_touches_source() {
  git show --format= --name-only "$1" \
    | grep -v '^featured_changes\.md$' \
    | grep -q '[^[:space:]]'
}

gate_measure() {
  local runs="${1:-5}"
  if [ -n "$(git status --porcelain --untracked-files=no)" ]; then
    echo "refusing to measure: working tree has uncommitted tracked changes." >&2
    echo "A run that fails mid-way can leave a patch applied; start clean so" >&2
    echo "that a dirty tree afterward is unambiguously this run's doing." >&2
    exit 1
  fi
  local landed=0 i before after new h
  for i in $(seq 1 "$runs"); do
    before=$(_agent_commits)
    echo "=== gate run $i/$runs ==="
    gate_remove_dead_trait || echo "(run $i exited non-zero)"
    after=$(_agent_commits)
    new=$(comm -13 <(echo "$before") <(echo "$after"))

    if [ -z "$new" ]; then
      echo "--- run $i: no commit"
    else
      local real=0
      for h in $new; do
        if _commit_touches_source "$h"; then
          real=1
          echo "--- run $i: LANDED $(git log -1 --format=%h\ %s "$h")"
        else
          echo "--- run $i: changelog-only commit $(git log -1 --format=%h "$h") — NOT a land"
        fi
      done
      [ "$real" -eq 1 ] && landed=$((landed + 1))
    fi
    if [ -n "$(git status --porcelain --untracked-files=no)" ]; then
      echo "!!! run $i left the tree dirty — a rejected patch was not reverted." >&2
      echo "!!! That is itself a section-0 bug (cf. contributor #7). Stopping." >&2
      git status --short >&2
      exit 1
    fi
  done
  echo
  echo "gate result: $landed/$runs landed a commit"
  echo "Phase 2 milestone gate needs 5 consecutive lands (see TODO.md section 0)."
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
  fix_one_clippy_lint           - fix the first clippy warning cargo_clippy reports
  remove_redundant_clone        - remove one provably-unnecessary .clone()
  convert_loop_to_iterator      - rewrite one push-only for loop as .collect()
  add_unit_test_for_pure_function - add one #[test] for an untested pure fn
  replace_unwrap_with_question_mark - replace one risky .unwrap() with ?
  extract_magic_number          - name one magic number as a const
  rename_helper_and_call_sites  - rename a function and its call sites (up to 3 files)
  format_a_few_drifted_files    - cargo-fmt 3 files with formatting drift

Reliability gate (a control task, not an example — see TODO.md section 0):

  gate_remove_dead_trait        - delete the dead LlmProvider trait (verified 1 hunk)

Run one with:
  bash scripts/refactoring_examples.sh run <name>

Measure the success rate over N runs (default 5):
  bash scripts/refactoring_examples.sh gate [N]
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
    gate) gate_measure "${2:-5}" ;;
    *)
      echo "Usage: $0 {list|run <name>|gate [N]}" >&2
      exit 1
      ;;
  esac
}

main "$@"
