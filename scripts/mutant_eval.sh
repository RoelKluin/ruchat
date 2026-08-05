#!/usr/bin/env bash
# Compile-error mutation eval for ruchat's reliability pipeline (see TODO.md section 0 and
# fixtures/mutant-repo/README.md).
#
# Distinct from refactoring_examples.sh's gate, and deliberately not a mode of it:
#   - The gate asks a loose question - "did the pipeline land any change to the named file at
#     all." Live testing showed the cost of that: 3 separate runs all "landed" a commit that
#     left the actual named bug completely unfixed, and Validator/Critic approved every one.
#   - This eval asks a strict one instead - given a real, unnamed compile error with a known
#     correct fix, does the agent produce *that* fix (or a plausible different one), not just
#     some edit that happens to touch the right file. Verified by diffing the agent's landed
#     commit against the known-correct content, not by asking an LLM whether it looks right.
#
# Runs against fixtures/mutant-repo (a separate fixture submodule from fixtures/gate-repo - see
# that crate's README for why the gate's own fixture, which deliberately carries a few clippy
# warnings, would give an agent something else to wander off "fixing" instead of the one
# compile error this eval cares about).
#
# Usage:
#   bash scripts/mutant_eval.sh list
#   bash scripts/mutant_eval.sh run <mutation-id>
#   bash scripts/mutant_eval.sh measure [N]   (default: one pass through every mutation once)

set -euo pipefail

RUCHAT_BIN="$(realpath "${RUCHAT_BIN:-./ruchat}")"
# See refactoring_examples.sh's own AGENT_ROLE_DIR comment: agent_role/*.md is a ruchat asset
# resolved relative to CWD, and this script's runs cd into $MUTANT_DIR before invoking ruchat.
AGENT_ROLE_DIR="$(realpath "${AGENT_ROLE_DIR:-agent_role}")"
MUTANT_DIR="$(realpath "${MUTANT_DIR:-fixtures/mutant-repo}")"
MUTATIONS_JSON="$MUTANT_DIR/mutations/mutations.json"
RESULTS_DIR="$(realpath "${MUTANT_RESULTS_DIR:-mutant_eval_results}")"

# No --collection (this is self-contained, no RAG needed) and deliberately no
# --validator-model/--critic either: unlike the gate, this eval's pass/fail comes from diffing
# the landed commit against the known-correct content, not from an LLM's opinion of it - adding
# Validator/Critic would only cost time here. Worth revisiting later as a way to cross-check
# their judgment against ground truth (do they approve the exact-match cases and reject the
# no-land ones?), just not needed for the eval itself.
MUTANT_FLAGS=(
  --team-model qwen2.5-coder:14b
  --iterations 3
)

# Deliberately does not name the file, the bug, or even the kind of error - unlike the gate's
# goal (which names the exact trait), this makes the agent actually use cargo_check's own
# output to diagnose the problem, closer to a real "the build is red, find out why" task.
GOAL="You are a rust specialist. You work on a small Rust crate that currently does not \
compile. Task: run cargo_check to see the compiler's error, then make the minimal change \
needed to fix it. Do not modify any other file, and do not add or change any functionality \
beyond what's needed to make the crate compile again."

mutation_ids() {
  jq -r '.[].id' "$MUTATIONS_JSON"
}

mutation_field() {
  # $1 = mutation id, $2 = field name
  jq -r --arg id "$1" --arg field "$2" '.[] | select(.id == $id) | .[$field]' "$MUTATIONS_JSON"
}

# Runs one mutation end to end. Exit code carries the verdict (mutant_measure below relies on
# this instead of parsing captured output, so ruchat's own streamed output still goes straight
# to the terminal in real time, same as refactoring_examples.sh's gate):
#   0 = exact match (the landed commit's target file is byte-identical to the correct version)
#   1 = alternate fix (a commit landed, differs from the expected fix - saved for later review;
#       Stage::Test already guarantees this compiles, since the pipeline can't reach Commit
#       without cargo check/test passing first)
#   2 = no land (the run never reached Stage::Commit - escalated, exhausted its budget, or
#       errored)
#   3 = refused to run at all (fixture repo wasn't clean beforehand - a prior run's mess, not
#       this run's own failure; mutant_measure treats this as fatal, not just one bad run)
run_one_mutation() {
  local id="$1"
  local target broken branch
  target="$(mutation_field "$id" target_file)"
  broken="$(mutation_field "$id" broken_variant)"
  branch="mutant-${id}-$(date +%s)-$$"

  (
    cd "$MUTANT_DIR"
    git checkout --quiet master
    if [ -n "$(git status --porcelain --untracked-files=no)" ]; then
      echo "refusing to run $id: $MUTANT_DIR has uncommitted tracked changes." >&2
      echo "A prior run's mess, not this run's own failure - investigate before continuing." >&2
      git status --short >&2
      exit 3
    fi
    cp "$broken" "$target"
    export RUCHAT_TEMPLATES_DIR="$AGENT_ROLE_DIR"
    # shellcheck disable=SC2068
    "$RUCHAT_BIN" pipe ${MUTANT_FLAGS[@]} --feature-branch "$branch" "$GOAL" \
      || echo "(run for $id exited non-zero)"
    # Always restore master's working tree, whether or not a commit landed - if it never
    # reached Commit, the raw mutation (or a rejected half-applied patch) is otherwise left
    # sitting uncommitted here for the next run to trip over.
    git checkout --quiet -- "$target" 2>/dev/null || true
    if ! git rev-parse --verify --quiet "refs/heads/$branch" > /dev/null; then
      echo "--- $id: NO LAND (never reached Stage::Commit)"
      exit 2
    fi
    if git diff --quiet master "$branch" -- "$target"; then
      echo "--- $id: EXACT MATCH"
      exit 0
    else
      mkdir -p "$RESULTS_DIR/alternates"
      out="$RESULTS_DIR/alternates/${id}-$(date +%s).diff"
      git diff master "$branch" -- "$target" > "$out"
      echo "--- $id: ALTERNATE FIX (compiles, differs from the expected fix - saved to $out)"
      exit 1
    fi
  )
}

mutant_measure() {
  local runs="${1:-}"
  mkdir -p "$RESULTS_DIR"
  mapfile -t ids < <(mutation_ids)
  local n=${#ids[@]}
  [ -z "$runs" ] && runs="$n"
  local exact=0 alt=0 fail=0 code i id
  for i in $(seq 0 $((runs - 1))); do
    id="${ids[$((i % n))]}"
    echo "=== mutation run $((i + 1))/$runs: $id ==="
    if run_one_mutation "$id"; then
      exact=$((exact + 1))
    else
      code=$?
      if [ "$code" -eq 3 ]; then
        echo "Stopping: fixture repo was not clean before a run." >&2
        exit 1
      elif [ "$code" -eq 1 ]; then
        alt=$((alt + 1))
      else
        fail=$((fail + 1))
      fi
    fi
  done
  echo
  echo "mutation eval result: $exact/$runs exact match, $alt/$runs alternate fix (collected" \
    "for review in $RESULTS_DIR/alternates/), $fail/$runs no land"
}

list() {
  echo "Mutations (each a different rustc error class, one localized compile error, an" \
    "unambiguous correct fix):"
  echo
  jq -r '.[] | "  \(.id)\t- \(.error_class)"' "$MUTATIONS_JSON" | column -t -s $'\t'
  echo
  echo "Usage:"
  echo "  bash scripts/mutant_eval.sh run <id>"
  echo "  bash scripts/mutant_eval.sh measure [N]   (default: one pass through every mutation)"
}

main() {
  local cmd="${1:-list}"
  case "$cmd" in
    list) list ;;
    run)
      local id="${2:?usage: $0 run <id> — see: $0 list}"
      run_one_mutation "$id"
      ;;
    measure) mutant_measure "${2:-}" ;;
    *)
      echo "Usage: $0 {list|run <id>|measure [N]}" >&2
      exit 1
      ;;
  esac
}

main "$@"
