# Ruchat TODO

Last updated: 2026-08-04

## 0. CRITICAL - Agentic run reliability (pinned, DO NOT REMOVE until a live run lands a committed change)

Maintainer's top priority. `ruchat pipe --team-model ... --iterations N "<goal>"` runs mostly
fail (~99/100 by maintainer count; 2026-08-04 baseline: 19/20 archived traces failed). Stay
pinned until a real live Ollama/Chroma run actually commits a change - green `cargo test --lib`
alone does not close this.

Contributors found via real trace evidence (`qwen2.5-coder:14b`) and fixed, 2026-08-04 unless noted:

1. [x] Live-verified. Hard `Stage::Escalate` on an Architect/Worker output identical to the prior
   round killed runs almost instantly (18/20 died at round 2-3). Removed both escalates;
   `ctx.round > max_iterations` is now the sole backstop, matching the Scoper's existing pattern.
   orchestrator.rs.
2. [x] Live-verified (trace 475). Worker calling a read-only tool twice in one round burned the
   whole round. Added one bounded nudge-and-reask before the existing rejection.
   `is_read_only_worker_tool`, orchestrator.rs.
3. [x] Not live-verified. Worker substituted `memorize` for a real fix while real
   cargo_clippy/check diagnostics were already in context; Validator caught it inconsistently
   (trace 475 r2 vs r3). New `round_has_actionable_diagnostics` deterministically rejects a
   no-op memorize before the Validator sees it. orchestrator.rs.
4. [x] Not live-verified. apply_patch diffs missing `--- a/`/`+++ b/` headers were refused
   outright. `ensure_diff_has_file_header` synthesizes them when the plan's `FILES:` names
   exactly one file. protocol.rs.
5. [x] Live-verified (traces 480/481, 489). Architect illegally embedded a full apply_patch
   tool_call in its plan (forbidden by architect.md); Worker copied it verbatim instead of
   reading the real file. Two fixes: `strip_architect_tool_call_hallucination` truncates the
   Architect's output at the tool_call fence or `IMPLEMENTATION:` heading (generalized after
   trace 489 showed a ```json-fenced variant slipping past the original bare-fence check);
   `auto_ground_planned_file` proactively injects the real, line-numbered target file content
   every round from `FILES:`. orchestrator.rs.
6. [x] Not live-verified. A blank line between diff hunks broke `diffy` parsing
   (`OrphanedHunkHeader`). `normalize_diff_hunk_lines` no longer excludes blank lines from its
   missing-context-prefix repair. protocol.rs.
7. [x] Root cause confirmed live (an unvalidated patch left applied on disk after budget
   exhaustion silently broke two subsequent runs - traces 486/487/488). Iteration-exhausted
   `Stage::Retry` branch now calls `revert_pending_patches` before `Stage::Done`, matching the
   still-has-budget branch. orchestrator.rs.
8. [x] Fully unit-tested, no live-infra caveat. `--summarizer-model` did not exist as a CLI flag
   (blocked testing whether a Summarizer curbs unbounded context growth). Added, mirrors
   `--validator-model`. ask.rs.
9. [x] Not live-verified. `auto_ground_planned_file` re-injected a fresh ~4000-char dump every
   round with no dedup, compounding the unbounded-context problem above (no repro script
   configures a Summarizer). Now drops any prior grounding turn before pushing the fresh one.
   orchestrator.rs.
10. [x] `--trace-timings` flag: per-turn `duration_ms`, shown in trace files only when passed.
    Not yet live-verified. types.rs, orchestrator.rs.
11. [x] Not live-verified. Maintainer report: an Architect generation once streamed continuously
    with no pause (all one color, never stopped) - a runaway decoding loop, not caught by any
    existing round-level check since it never got to a round boundary. New
    `has_runaway_repetition` in `query_stream` (agent.rs) checks the streamed tail after every
    chunk and breaks the stream early once a 6+ word phrase repeats 3+ times in a row, instead of
    waiting for the model to hit its generation limit. Applies to every role, not just Architect.
12. [x] Not live-verified. Root-caused live (trace 492, maintainer's own `fix_one_clippy_lint` run,
    same day as #11): contributor #5's `IMPLEMENTATION:` heading match only tolerated a bare line,
    but the Architect formatted it as a markdown heading (`### IMPLEMENTATION:`) every round -
    `.trim()` doesn't strip `#`, so the match never fired and the hallucinated tool_call leaked
    through for all 5 rounds. Worse, the leaked example used a wrong JSON shape
    (`{"patch":{"path":...,"diff":...}}` instead of the documented flat `{"diff":...}`), which the
    Worker copied verbatim every round - `parse_tool_call` never even recognized it as a tool
    call, so every round died as "no recognized tool_call" before ever reaching apply_patch. Two
    fixes: `strip_architect_tool_call_hallucination` now strips leading `#`/`*`/space and trailing
    `*`/space before comparing to `IMPLEMENTATION:` (stall_mitigation.rs); `structured_call_from_value`
    now promotes a `patch.diff` field to top-level `diff` when the flat field is missing, as a
    defense-in-depth tolerance independent of the hallucination path (tools.rs). Also noted:
    round-1-4's Architect wrote `FILES: None` despite CHOICE correctly naming the target file -
    the same already-logged pattern from trace 484, not fixed here.
13. [x] Not live-verified. Root-caused from trace 497 (another `fix_one_clippy_lint` run, same
    shape as #12 but a different bug): clippy flagged `options` as dead, the Architect's plan said
    to remove `options`, but the Worker's diff removed a different, still-used field (`cfg`) from
    the same struct three rounds running - a syntactically valid deletion of a real line, so
    nothing upstream could tell it was wrong and it cost a full Tester round-trip each time to
    surface as a confusing "no field named cfg" error. Guard in `Validation::apply_patch`: for a
    pure-deletion diff answering a dead-code warning, at least one removed line must mention the
    symbol clippy named. Fails open otherwise. protocol.rs (`clippy_dead_code_symbols_for`,
    `removed_line_texts`).

14. [x] Not live-verified. Regression introduced by #13's first attempt and caught in traces
    499/500: that guard compared *line numbers* - the diff's computed removed-line offsets vs the
    `file:line` clippy reported - and so rejected correct edits. The Worker deleted exactly the
    right line (`options`) but listed the struct's other fields in the wrong order, which shifted
    the computed offset to 84; the run then looped on "wrong line, re-check the field" (a message
    that was simply false) for every remaining round. Root cause: model-written `@@` offsets are
    unreliable by construction - this repo already has to recompute them
    (`diff_repair::fix_hunk_header_counts`) - so nothing that *rejects* an edit may be derived
    from them. Guard rewritten to be position-independent (#13 above), plus the actual fix for
    the reordered-context shape: `diff_repair::realign_pure_deletion_hunks` re-anchors a
    pure-deletion hunk onto the real location when its removed lines exist in the file exactly
    once, rebuilding the hunk from the file's own bytes. Only runs after `diffy::apply` has
    already failed, only for unambiguous single-match pure deletions - never changes *what* is
    deleted, only *where* it is anchored. Regression test uses the verbatim trace-500 diff.

    Also observed, not fixed: (a) traces 494-500 all ended without archival into successes/ or
    failures/ - unconfirmed whether a real finalize_trace gap or the maintainer Ctrl-C'ing a
    visibly stuck run; needs a live repro. (b) "you called CargoClippy again instead of applying
    a change" fires in round 1 of all three of traces 498/499/500 (and again in 500 round 3),
    burning roughly half the round budget before any diff problem matters - a distinct bug from
    #13/#14. (c) every apply_patch rejection is pushed twice into the context, once as `Validator`
    by protocol.rs and again as `ApplyPatch` by orchestrator.rs:1577 - pre-existing on all
    rejection paths, pure context bloat.

    Known still-blocking, so #14 alone will NOT make `fix_one_clippy_lint` land: the one-line
    deletion the Worker keeps writing does not compile on its own. `options` is a struct field
    that is also constructed at src/core/agent.rs:116, so removing only the declaration gives
    "struct `Agent` has no field named `options`" - the task needs a 2-hunk deletion. apply_patch
    now applies the correct diff; Test still rejects it. That plus (b) are the next two blockers
    ahead of a landed run.

Two patterns seen live (traces 484/485), not fixed - look like a model-capability limit, not a
further orchestration bug: Architect suggesting a raw shell command instead of `FILES:` (484);
Worker calling cargo_clippy every round for a full 5-round run despite worker.md's explicit
instruction not to (485). Next step if these persist after a live re-run: try
`qwen2.5-coder:32b` (already pulled) before writing more mitigations for 14b specifically.

15. [x] Not live-verified. Found 2026-08-05 by inspecting the 8 archived `ai/feature-*` commits
    from the maintainer's own runs: **5 of 8 changed no source at all** - only
    `featured_changes.md`, ruchat's changelog, which `commit_add_targets` stages
    unconditionally. So a run that applied zero patches still reached Commit and produced a
    branch + commit describing work that never happened (incl. `c0e37d4 "Fix clippy warning:
    redundant type annotation"`, which fixed nothing). `Stage::Commit` now escalates instead
    when `ctx.pending_patches` is empty. This also silently corrupted any success measurement -
    see the gate-runner fix below. Answers the maintainer's open question about where the bogus
    extra commit came from: ruchat's own changelog append, not a `feature_changes` memorize.

16. [x] Commits now continue the most recent `ai/feature-*` branch instead of starting a new one
    per run (maintainer request 2026-08-05 - successive commits on one branch read better than
    eight one-commit branches). `--feature-branch <name>` overrides and is created on demand.
    A continued branch is never `branch -D`'d on a late failure, only one this run created -
    otherwise a failure would destroy earlier runs' commits. git.rs
    (`resolve_feature_branch`, `branch_exists`, `latest_ai_feature_branch`), 5 read-only tests.

17. [x] Example tasks may now abandon an intractable target and pick another
    (`PICK_ANOTHER` clause, scripts/refactoring_examples.sh). Maintainer observation: clippy
    reports one location per warning, but a dead-code fix often needs the initializer/
    construction site too, and those are neither shown nor adjacent - so "fix the first warning"
    could hand the model a multi-site edit with no way to decline, and it would re-emit the same
    single-site diff every round. `fix_one_clippy_lint` now asks for a warning fixable in one
    edit. The reliability gate deliberately does NOT get this clause (`run_ruchat_raw`) - its
    whole point is that the target never varies.

18. [x] Every finished run now writes `ruchat_traces/summaries/ruchat_trace_<N>.md`: goal,
    outcome, and a round-by-round review of the agents' decisions - one line per step with a
    `GOOD:`/`BAD:`/`UNCLEAR:` verdict, then `LESSON:` lines (maintainer request 2026-08-05, to
    have something learnable per run rather than only a cause-of-death line). Fixed prefixes so
    recurring patterns are greppable across runs. `run_summary::generate_step_review` +
    `Context::summary_body`/`finalize_summary_trace`. Judges observed decisions, not claimed
    reasoning - a Worker tool call states no motive and a model asked for one invents it.
    Verified against real trace 529 on qwen2.5-coder:14b: correct verdicts, correct round
    numbers, 95s cold. Own 240s timeout (30s timed out) and a 24k-char head+tail clamp, since
    `chat_stream` sets no `num_ctx` and traces average ~50 KB. The clamp also fixed the existing
    outcome summary, which shares the same trace: `failures/ruchat_trace_472.md` (535 KB) had
    recorded the summary "I'm sorry, but I can't assist with that request." - Ollama had
    truncated away the system instruction. Same trace clamped now summarizes correctly in 4s.
    Any pre-clamp summary on a large trace should be treated as unreliable.

21. [x] GPU/Ollama facts corrected by measurement 2026-08-05, replacing two wrong claims:
    (a) both instances ARE pinned correctly (`/proc/<pid>/environ`: :11434 has
    `CUDA_VISIBLE_DEVICES=0`, :11431 has `1,2,3,4`) - the "unsubstituted `N` placeholder" note
    was wrong; (b) the Teslas do NOT work - `/api/ps` after a real generation reports
    `size_vram: 0` on :11431 vs 100% on :11434, so ollama-light is CPU inference and the
    "frees the 3090" premise is false. Delegation policy now routes everything to
    ollama-heavy; build-log-summarizer switched off ollama-light. Detail moved out of
    CLAUDE.md into the ruchat-dev skill's `references/gpu-and-ollama.md`.
    Note `ollama.service` never starts (duplicate `ExecStart=` in override.conf) - the real
    launcher is `~/ollama_serve.sh` in tmux, so `systemctl edit`/`journalctl -u` are dead ends.

25. [ ] **Architect emits tool instructions the Worker then obeys, burning the round's one
    lookup.** Every plan in trace 531 opens "1. Run `cargo clippy --lib -p ruchat`" although
    architect.md tells it plainly it has no tools. worker.md interpolates `PLAN: {{PLAN}}`
    verbatim, so the Worker calls cargo_clippy - whose output is already in DOCUMENTS - then
    gets refused for using its lookup. Rounds 1 and 4 of 5 died entirely to this. Fix is
    structural: strip tool-invocation steps from a plan before it reaches the Worker, or
    reject a plan containing them, rather than adding more prompt text.

26. [ ] **Rejections are raw compiler output, never interpreted.** Tester returns
    `src/core/agent.rs:115:17: error: struct Agent has no field named options` - a complete
    diagnosis - but nothing turns it into "the patch is incomplete: line 115 still uses this
    field, a correct fix removes both sites." The Architect must infer it and, over 4 rounds,
    never did. Synthesize an explicit incomplete-deletion message when a compile error names a
    symbol the just-applied patch deleted; the compiler already supplies the line number.

27. [ ] **`history_view` never collapses superseded content, so the wrong answer outvotes the
    correction.** It filters only `TurnKind::Retrieval`; everything else accumulates verbatim.
    By round 5 of trace 531 the Architect's HISTORY held its own failed plan 6x and the failed
    diff 8x against 3 copies of the rejection - the same wrong answer shown twice as often as
    its correction, in identical formatting. Collapse repeats to one instance plus a
    "(repeated Nx)" marker. This is the root cause behind the maintainer's "too much
    irrelevant information" and "information not rephrased after failure" (2026-08-05).
    NOTE: architect.md already contains five paragraphs forbidding a repeated plan, including
    "will be treated as a stall". It repeated anyway. More prompt text is not the fix.

23. [ ] `ruchat index` cannot correctly re-index a changed file: chunk IDs derive from chunk
    content, so an edited chunk is inserted as a new record and the old one stays forever.
    Measured 2026-08-05 on a 2-section test doc - editing one section's body took the
    collection 3 -> 5 -> 7 chunks, with both old and new text retrievable and nothing marking
    which is current. The incremental marker makes it worse: only changed files are re-indexed,
    i.e. exactly the ones that duplicate. Affects the Librarian at runtime, not just tooling.
    Fix is an ID scheme keyed on (file, symbol, occurrence) plus deleting a file's existing
    records before re-inserting. `scripts/index_rag.sh` currently works around it by deleting
    and recreating volatile collections. Note: dedup-by-(file,symbol) is NOT a valid detector
    here - Rust legitimately repeats a method name across impl blocks (`generate_embeddings`
    appears 7x in llm_client.rs), so a count of duplicate keys proves nothing either way.

24. [x] Rebuilt `repo_src-all-minilm_l6-v2` 2026-08-05: it held 1230 chunks where a clean index
    of the same tree gives 1535, so the Librarian had been retrieving against an index missing
    ~305 chunks of current code. `scripts/index_rag.sh src` regenerates it; not in the
    post-commit path (too slow) - rerun after a refactor that moves or renames much of src/.

22. [ ] Timing measurements must not run concurrently with local-model delegation: ruchat and
    the `ollama-heavy` MCP share :11434 and there is one usable GPU, so they queue against
    each other. Turn delegation off before the section-0 reliability gate.

19. [ ] ~58 traces sit unarchived in `ruchat_traces/` (471-529), so those runs never reached
    `finalize_trace` - no outcome summary and now no step review either. Find out how a run
    exits without it (cancel? panic? `?` early-return in `run_stage_machine`?). Each one is a
    lost data point on exactly the reliability question section 0 exists to answer.

20. [ ] `LlmClient::chat_stream` sets no `num_ctx`, so Ollama silently truncates any prompt over
    the model default - agent prompts, not just summaries. Item 18 clamps its own input as a
    local mitigation; the general case is unhandled and would explain context-blind agent
    behavior in long runs. Not theoretical: it demonstrably destroyed the run summary on
    trace 472 (see item 18), and the same mechanism drops HISTORY/DOCUMENTS from a Worker
    prompt in a long run - which looks exactly like the model ignoring its own context.

### NEXT ACTION (decided 2026-08-04, roadmap review)

Stop writing new mitigations until there is a number that can move. In order:

1. [x] **Gate task retargeted, 2026-08-04.** `fix_one_clippy_lint` needs a 2-hunk edit
   (declaration at agent.rs:82 + construction at agent.rs:116), so it conflates "pipeline works"
   with "model can decompose a multi-site edit" - it cannot answer either question.
   New gate: `gate_remove_dead_trait` in scripts/refactoring_examples.sh - delete the dead
   `LlmProvider` trait in src/providers/llm.rs (nothing implements or calls it). Verified
   one-hunk by hand both ways before adopting it: trait alone (3 lines) compiles, leaving an
   unused-import warning; `use` + trait (5 lines) compiles clean. Both are one contiguous hunk,
   so either lands - a deliberately forgiving criterion, since this measures the loop, not the
   model's thoroughness. It is a control, not a challenge: it names the file so that
   target-selection isn't a variable. Naturally repeatable (a success commits to an
   `ai/feature-*` branch and returns to the working branch, leaving llm.rs intact).
2. [ ] **Measure the real success rate on it.** `bash scripts/refactoring_examples.sh gate 5`.
   Runner corrected 2026-08-05: it counted new `ai/feature-*` branches, which would have scored
   all 5 changelog-only commits (#15) as lands, and stops incrementing at all now that branches
   are continued (#16). It now diffs the commit set before/after and requires a new commit
   touching a file other than `featured_changes.md`. Still aborts if a run leaves the tree
   dirty (contributor #7 recurring). Replaces "~99/100 fail" with a baseline. Gate for Phase 2's
   milestone: 5 consecutive unaided lands. NEEDS A LIVE RUN.

   Known real rate so far, from the maintainer's own archived runs: **3 genuine lands out of 8
   commits** - and those 8 came from many more runs than 8, so the true per-run rate is lower.
3. [ ] **Then** hold that task constant and run it with `--chat-provider anthropic`. Only with
   the task fixed does Claude-vs-qwen discriminate orchestration from model capability - on a
   2-hunk task it would only show that Claude writes better diffs, which proves nothing.
   Claude lands it + qwen doesn't => capability limit, stop hardening for 14b (see reference
   model below). Both fail => a real orchestration bug remains.

Reference model is now `qwen2.5-coder:32b`; 14b is best-effort, not the bar (see ROADMAP.md).

Mitigation policy, from #14's regression: model-agnostic repairs only. `diff_repair.rs` passes
this test - it repairs malformed diffs, not one model's malformed diffs. Nothing that *rejects*
an edit may derive from model-written `@@` offsets.

Done (2026-08-04): extracted protocol.rs's diff-repair functions into agent/diff_repair.rs and
orchestrator.rs's stall-mitigation functions into orchestrator/stall_mitigation.rs. Pure move,
no behavior change - same 318 tests pass, clippy/fmt clean. (A nom-parser rewrite was considered
and rejected earlier: diffy already parses unified diffs; rewriting would duplicate that work
for an organizational, not behavioral, win.)

Logged, not acted on: maintainer's "add a reason field to every tool_call" idea - ties into
ROADMAP.md's chain-of-thought Phase 3 item, deferred pending the agentic-evals harness having
enough scenarios to judge it.

Net: meaningfully improved, not resolved. #1, #2, #5 are live-confirmed; #3/4/6/7/9/10/11/12/13/14 are
code-fixed and unit-tested but not yet live-confirmed. The two capability-suspected patterns are
still open. Whether a real run can land a committed change is still unanswered - keep this
section until one does.

## High Priority

### 1. Configuration & CLI Improvements
- [~] Environment variable support for Chroma/Ollama settings — added `OLLAMA_SERVER` (parity with the existing `CHROMA_SERVER`/`CHROMA_TOKEN`) and `CHROMA_TENANT`/`CHROMA_DATABASE`. Deliberately did NOT env-var every flag (e.g. `--temperature`/`--top-k`/`--seed`/model selection) — those are per-invocation tuning, not deployment config, and `--options <JSON|file>` already exists as the mechanism for persisting generation-parameter presets; env-var-ifying them would be scope creep with no real usage pattern behind it.
- [ ] Deprecate/phase out scattered JSON string hacks in favor of structured sub-configs — includes the generic per-flag CLI/file merge noted in `cli/serde.rs::load_merged_config`'s comment: today each subcommand's `*Args` struct applies its own CLI flags over the config-file `Value` individually (`update_from_json` per struct); a fully generic merge would need every `*Args` struct to serialize itself to `Value`, which doesn't exist yet. Left as the documented, deliberate deferral it already was — not attempted here to avoid exactly the "implemented speculatively" risk that comment warns against.

### 2. Error Handling & Logging
- [~] **Partially done 2026-08-04**: the SQLite-vec batch (30 call sites, the single most concentrated instance) was converted to a dedicated `RuChatError::SqliteVecError(String)` variant, mirroring `AnthropicError`'s pattern — mechanical swap, same message text, all sqlite_vec tests still pass. Baseline before this fix was **138** total `Is`/`InternalError` call sites (up from the previously-documented ~85); now ~108 remain across the rest of `src/core`/`src/providers`, still flattening distinct failure causes into two generic variants. Auditing the rest (deciding a dedicated variant or `#[source]`-carrying wrapper per remaining call site) is still a large, higher-risk pass, not attempted here.
- [~] Implement graceful degradation when Ollama/Chroma are unavailable — Chroma being unreachable during the Librarian's on-demand retrieval (`Stage::Retrieve`) no longer kills the run (see Done section). Still open: Ollama being unreachable from the very start of a run (Architect/Worker's first call) still surfaces whatever raw error `retry_transient!` exhausts to, not necessarily a clear "Ollama isn't running at <address>" message — not attempted, since Architect/Worker genuinely can't proceed without Ollama regardless of message clarity, this was judged lower-impact than the Chroma case.

### 3. Agent Orchestration
- [ ] **Reasoning/advisory roles — unblocked 2026-08-04, now the recommended next feature work.**
  These were previously marked blocked on section 0's reliability item, applied uniformly to all
  six new use cases without checking which touch the failing path. The advisory ones don't:
  every contributor in section 0 is a diff-writing failure (`apply_patch`, Worker tool
  discipline, the Tester round-trip), and an advisory role never calls `apply_patch`, never
  commits, and never reaches `Stage::Implement`/`Test`. Scope: answer a question directly
  (RAG-informed), work through a hard multi-step question, produce a non-code plan. Exercises
  Scoper → Librarian → Architect only. Two reasons to do this *before* the coding loop is
  fixed: likely the shortest route to ruchat completing some agentic run reliably end-to-end,
  and it isolates whether the retrieval/planning half is sound — which the coding loop cannot
  tell us today, since a failure there is indistinguishable from a diff-writing failure. Also
  gives `core/agent/evals.rs` its first non-coding scenarios (it covers 3 of 7 roles today).
  The prompt-engineering assistant stays blocked, but on RAG-collection scoping, not reliability.
- [ ] Make agent pipeline fully configurable via JSON (the `Stage` sequence in `orchestrator.rs` is still fixed in code, not data — see `ROADMAP.md` Phase 3)
- [x] Stale, corrected 2026-08-04: this whole item (per-document summarization, multi-collection queries, reranking) already shipped — see `ROADMAP.md`'s "Improved RAG" Phase 2 entry (`doc_summary.rs`, `Query`'s `collection` now a list, `chroma/rerank.rs`).

### 5. Security & CI (found 2026-08-04, specialist review round — see below)
- [x] **CRITICAL, fixed same-day**: `ripgrep` Worker tool had no repo-root containment check (a tool call could read arbitrary files like `/etc/passwd` — verified live) and no `--` flag terminator before the untrusted `pattern`/`path` args, so a pattern like `--pre=/bin/sh` would be parsed as ripgrep's own `--pre` flag (runs an arbitrary program against scanned file contents) — combined with `apply_patch` already being able to write arbitrary tracked-file content, a real code-execution chain through a tool meant to be safe by construction. See Done section for the fix writeup.
- [x] Fixed 2026-08-04: `--verbose` printed the raw command line including any `--chroma-token`/`--anthropic-api-key` secret. `cli/args.rs::redact_args` now redacts both flag forms before printing.
- [x] Fixed 2026-08-04: `.github/workflows/ci.yml`'s `push` trigger targeted `main`, repo's actual branch is `master` — CI silently never ran on direct pushes. One-line fix.
- [x] Fixed 2026-08-04: `cargo_dupes` now calls `limit_resources` like `cargo_check`/`cargo_clippy` do.
- [x] Fixed 2026-08-04: `ripgrep` now has a 20s wall-clock timeout, matching every other subprocess tool.
- [x] Fixed 2026-08-04: `git_blame` now inserts `--` before `path`, matching `git_log`/`git_diff`.
- [x] Fixed 2026-08-04: `AnthropicArgs` now has a manual redacting `Debug` impl mirroring `AnthropicClient`'s.

### 4. TUI Chat — closed 2026-08-04
**Decision: ruchat is a non-interactive CLI.** The five TUI bug items here described a
subsystem deleted 2026-07-31 (the crossterm interactive layer — cursor movement, selection,
history/undo-redo editing, ~1,260 lines, `ad0708d` and neighbours); they were re-triaged twice
and removed rather than carried a third time. `src/tui/` today is `io.rs` + `render.rs` (175
lines, no cursor/selection/editing code). `--step`/`--breakpoint`/`--approve` already cover
interactivity where it matters, over plain stdin. A rebuild would be new work, not a bug fix —
see ROADMAP.md Long-Term Vision; git history has the deleted implementation.
- [x] Removed 2026-08-04: `crossterm` was unused (no `src/` references). Decision: drop rather than keep for a hypothetical TUI rebuild — trivial to re-add if/when that's actually planned.
- [x] ~~Wire up an actual producer for `AgentEvent::Progress`~~ — done, see Done section below.

## Medium Priority

### Code Quality & Maintainability
- [ ] Add integration tests for full agentic flows (using test Ollama/Chroma) — `agent_debug/*.json` already contain ready-made stage sequences (`architect_only`, `worker_and_validator_rejection`, `multiple_critics`, etc.); wire these into `cargo test` against a mocked `LlmClient`/`VectorStore` instead of writing fixtures from scratch
- [ ] Consistent error handling across Chroma subcommands
- [ ] Refactor duplicated JSON update logic (`update_from_json` methods) — an 8th instance now exists (`SqliteVecClientConfigArgs::update_from_json`, found 2026-08-04), though it's a single-field stub too thin to justify a standalone shared-helper item on its own; folds into this existing one.
- [x] Fixed 2026-08-04 (no userbase, so the "intended behavior" call could just be made): all three were the test's own expectation being wrong, not a real logic bug. `test_get_metadata_valid` now uses real JSON input, matching `parse_metadata`'s actual documented JSON-only behavior. `test_create_table` now asserts the short field aliases ("DOC"/"META") every sibling test already used, not the full words. `test_json_output` fixture now uses a valid `Include` variant, and its JSON assertions check for substrings present rather than an exact compact-JSON match (the code renders pretty-printed JSON, which never produces that).
- [x] **`cargo clippy --lib --tests` baseline re-verified and mostly cleaned up 2026-08-04:** was **86** total warnings (docs previously said ~16). Fixed: boxed `RuChatError::ChannelError`'s oversized payload (manual `From` impl replacing `#[from]`, since the field type no longer matches the source type) — this alone dropped lib warnings from 86 to **12**. Also removed 3 genuinely-dead items (`Issue` struct, `Context::is_approved`, `Validation::run_cargo_check`) and marked a 4th (`cli::utils::parse_key_val`) `#[allow(dead_code)]` since it's an intentional fixture `core/agent/evals.rs` tests the Architect against by name. Also fixed: `redundant_closure` (clippy auto-fix), `missing_transmute_annotations` (added the explicit turbofish clippy suggested, verified all sqlite-vec tests still pass — same real extension registration), and both `assertions_on_constants` (`cli/args.rs`'s `assert!(true)`/`assert!(false, ...)` replaced with a real `matches!` assertion). Down to **10** lib warnings. Remaining: the rest of the dead-code list (`EmbeddingsClient`/`LlmProvider` traits, `OllamaArgs::init_server`, two `build_generation_request` methods, `ChromaCollectionConfigArgs::get_or_create_collection`, `Agent::options`, `EmbedArgs::embed_raw_items` — see the `chroma-import` item below), 2 `large_enum_variant`, 1 `type_complexity`.

### Chroma / RAG
- [x] Fixed 2026-08-04: `ruchat embed`/`ruchat index` (Chroma path) previously called the create-free `get_collection`, so embedding into a brand-new collection name failed unless `chroma-init`/`chroma-create` had already been run separately — the exact opposite of the SQLite-vec backend, which auto-creates lazily. `EmbedArgs::resolve_collection` now calls `get_or_create_collection` (which had zero callers anywhere — a real, previously-unwired method, not new code) instead. Live-verified against a real running Chroma server: embedded into a brand-new collection name, confirmed via `chroma-ls` that it was auto-created with the record present. (Not the `db_config.json`-driven variant the title describes — that's still `chroma-init`'s job, run separately; this closes the narrower "embed shouldn't fail on a new name" gap, which was the part actually blocking real use.)
- [ ] Add progress bar for large embedding jobs
- [ ] Implement caching layer for repeated file embeddings — **investigated in depth 2026-08-04, turned out to have a real correctness trap, not implemented:** `embed_chunks` (`embed.rs`) computes embeddings for *every* chunk via a live Ollama call before checking `existing_ids` — looks like a free win to reorder (skip re-embedding when the content-hashed ID already exists, since `Md5(model:id_prefix:content)` means same ID ⟹ same content ⟹ same embedding, deterministically). The trap: the ID hash does **not** include metadata, only `model`/`id_prefix`/`content` — so the exact same text chunk can legitimately need a metadata-only update on a re-run (e.g. `ruchat index` after ctags starts reporting a different `signature`/`scope`/`references` for an otherwise-unchanged code chunk). Skipping the write whenever the ID exists would silently drop that metadata update, a real data-loss bug, not just a missed optimization. Doing this safely needs either fetching the stored embedding to pair with fresh metadata (a different round trip, might cost as much as just re-embedding for small chunks) or a new metadata-only update path on `VectorCollection` (`add`/`update`/`upsert` all currently require sending embeddings). Real API-surface question, not a quick reorder — not attempted.
- [x] Revived 2026-08-04 (maintainer confirmed worth doing): `ruchat chroma-import` now wired into the CLI. Fixed the three missing pieces: added `git::git_log_hashes` (just the hashes, one per line — `git_log`'s `--oneline` form isn't reliably parseable back into hashes alone) and `git::git_show` (a `--format=%an%x1f%ad%n%B%x1e` dump matching `parse_show_output`'s expected shape exactly — deliberately no `--` separator before the hash, since unlike a path argument that would make git treat it as a pathspec instead of a revision) to `orchestrator/git.rs`; added `EmbedArgs::new_for_ingestion` (builds an `EmbedArgs` from already-resolved sub-configs, Chroma-only, for callers with their own flattened config surface); fixed the `Some(msg_meta)`/`UpdateMetadata` type mismatch. Added the module's first tests ever (`parse_show_output`/`parse_diff_hunks`, previously zero coverage since the file wasn't even compiled in). Live-verified against this repo's real git history: `ruchat chroma-import --path README.md --max-count 3 -c <scratch>` produced 20 real records with correct commit hashes/authors/messages and correctly-split diff hunks, confirmed via `chroma-get`, then cleaned up.
- [ ] Better metadata normalization and type safety
- [x] Retired 2026-08-04 (maintainer confirmed: dead weight, no longer used directly): `embed_script.sh` removed outright rather than patching its incomplete per-language chunk-boundary FIXMEs — `ruchat index`/`core/index.rs` already superseded it with a real, tested implementation (ctags JSON `end` field / `build_symbol_metadata`'s next-symbol heuristic, not brace-counting). One stale doc reference (`scripts/refactoring_examples.sh`) updated to point at `ruchat index` instead.

### Architecture & Providers (found 2026-08-04, specialist review round)
- [ ] `VectorStore`/`VectorCollection` (`agent/llm_client.rs`) take `chroma::types::{Where, Metadata, QueryResponse, IncludeList, UpdateMetadata}` directly in their signatures — the "pluggable vector-store" seam is really "pluggable, as long as you speak Chroma's wire types." The SQLite-vec backend already has to `use chroma::types::{...}` just to satisfy the trait it implements. Fine with two implementors; a third (or ever dropping the `chroma` crate as a dependency) forces either a permanent type dependency or a breaking trait redesign. **Maintainer decision 2026-08-04: defer** — no functional cost with only two backends, don't spend time on a newtype pass speculatively; revisit if/when a third backend (e.g. LanceDB) is actually being considered.
- [x] Fixed 2026-08-04: `ask`/`pipe` now have their own `--vector-provider`/`--sqlite-vec-path` flags (mirroring `--chat-provider`), wired through the `--collection` shortcut's Librarian-config injection in `ask.rs::into_config` (same two keys `Orchestrator::new` already reads — `"vector_provider"`/`"sqlite_vec_client"`, previously only reachable by hand-writing `--agentic` JSON). Two new tests confirm the right keys land for each provider and that they don't leak into each other (sqlite-vec doesn't also inject `chroma_client`, and vice versa for `vector_provider`).
- [ ] `Orchestrator::new`'s Librarian client construction (`orchestrator.rs:196-235`) re-implements "pick Chroma vs SQLite-vec, build the right client" by hand on stringly-typed JSON (`remove_str` → `.parse::<Value>()` → `update_from_json`), duplicating `EmbedArgs::client`/`resolve_collection`'s already-typed version of the same branch. Two independent copies that must be kept in sync by hand — a new field on `SqliteVecClientConfigArgs` silently won't reach the Orchestrator's path unless someone remembers to mirror it. Share the logic (e.g. give the Librarian config the same typed struct `EmbedArgs` uses).
- [x] Fixed 2026-08-04: added a Features bullet + Installation note to `README.md` and a requirements note to `INSTALL.md` for SQLite-vec.
- [ ] Neither `--chat-provider` nor `--vector-provider` reach `Team`/`ruchat manager` presets at all (`grep` confirms zero hits in `core/agent/team.rs`) — concretely worsens the already-tracked config-file/CLI-merge gap (`cli/serde.rs::load_merged_config`, see High Priority #1): a saved Team can never select Anthropic or SQLite-vec, only a live `ask`/`pipe` invocation can.
- [x] Partially fixed 2026-08-04: added a direct test proving `--vector-provider sqlite-vec` reaches a real `SqliteVecCollection` (writes through the resolved collection, reads it back with an independent client). The Chroma-default branch still has no equivalent direct test (would need a live server or a stub Chroma client this module doesn't have) — lower risk given how extensively the Chroma path is already exercised elsewhere, left as-is.
- [x] Fixed 2026-08-04: added `add_with_a_dimension_mismatch_returns_a_legible_error` for the SQLite-vec backend.
- [ ] `--approve` HITL gate's `Stage::Commit` interactive branch remains never live-verified. **Investigated 2026-08-04, turned out bigger than "small, cheap":** `tui::io::Io` wraps `std::io::Stdin` directly, not a trait — making it injectable for a fake-stdin test means either introducing an `Io` trait (real new abstraction) or adding another parameter to `run_stage_machine` (which already has a growing bool-parameter list flagged elsewhere in this file as its own tech-debt item — adding a 6th/7th parameter for this would compound exactly that problem). A real architecture call, not a quick win; not attempted here.
- [ ] `manager run` has now fallen behind `ask`/`pipe` on three consecutive Phase-3 features in a row (`--resume`, `--approve`, `--chat-provider`, each individually deferred "Team presets don't get this yet"). **Maintainer decision 2026-08-04: let it lag** — `ask`/`pipe` is the primary interface being actively developed against; keep deferring Team support feature-by-feature rather than doubling every future change. Revisit only if Team/`manager run` becomes an actively-used path again.
- [ ] `run_task_stream`/`run_stage_machine`/`AgentPipeline::Orchestrator` now all carry the same 5-6-field parameter list (`debug_sequence, breakpoints, resume, approve_commit, ...`), added independently over several features — cheap now, but the classic precursor to a wrong-bool-in-wrong-slot bug that compiles silently. A `RunOptions` struct would cost little today.
- [ ] `-c`/`--collection` has no help text at all on `embed`/`index` (`[default: ""]` under a bare "Collection" heading) — contrast `pipe --collection`, which has a full descriptive sentence for the same flag.
- [ ] Provider-selector flags split their own on/off switch and sub-args across two different `--help` headings each (internally consistent between the two providers, but a user has to scan two sections to fully configure either one) — cosmetic, low priority.

### Documentation
- [ ] **Pipe-composition recipes (decided 2026-08-04, answers a maintainer request).** The ask
  was whether multi-role composition needs a first-class declarative multi-stage config file.
  Decision: no — shell piping already composes `ruchat pipe`/`ruchat ask --agentic` invocations,
  costs no engine surface, and keeps the stage machine one predictable unit. Remaining work is a
  doc pass promoting the working patterns already in `examples_thuis_ses.sh` into real,
  explained recipes (README.md or a dedicated doc; keep the existing doc split). A declarative
  format is revisited only if recipes prove insufficient — and if so it must be recognized as
  the same decision as ROADMAP.md's parked graph items, not built under another name.

### Performance
- [x] Stale, corrected 2026-08-04: this already works and isn't buffered. `Agent::query_stream` (`core/agent.rs`) forwards each `StreamItem::ChatChunk` over the channel the instant it arrives from the model; `tui/render.rs` writes each chunk to the terminal immediately on receipt (`cio.write_line`). `ctx.output.push_str` alongside it is a separate concern (accumulating the full text for tool-call parsing after the round completes), not a buffering-before-display step.
- [ ] Optimize history limit calculation and token counting — **investigated 2026-08-04**: token counting (`core/agent/tokens.rs`) is already a real BPE tokenizer (`tiktoken_rs::cl100k_base`), not the old `len()/4` heuristic — reasonably optimized already, and the remaining imprecision (not the exact per-model tokenizer) is a fundamental Ollama API limitation, not a code quality gap. `get_dynamic_history_limit` (`ollama/model.rs`) is a hardcoded model-name-substring table (`qwen2.5`→128k, `llama3`→8k, else 4k) — `ollama_rs::Ollama::show_model_info` could query a model's *real* context length via `/api/show`, but its `model_info` field is a raw `serde_json::Map` with an architecture-prefixed key name (`"llama.context_length"`, `"qwen2.context_length"`, etc. — not a fixed key), and querying it means turning a synchronous hot-path function async across every call site (`agent.rs`, `orchestrator.rs`'s summarizer check). Real improvement, but a multi-call-site signature change needing testing against several real Ollama models to get the key-name matching right — not a quick fix, not attempted here.
- [x] Reviewed 2026-08-04: `reqwest`'s default features (`default-tls` (rustls), `charset`, `http2`, `system-proxy`) are all genuinely used — TLS for the Anthropic API, `system-proxy` for corporate/dev network setups, `http2` likely relevant to Anthropic's API. No trimming warranted; explicit `["json", "stream"]` are the only additions beyond defaults, both used.

### Security & Production Readiness
- [~] Never log sensitive data (tokens, prompts with secrets) — audited `src/` for tracing/println calls that interpolate raw config strings or `{:?}`-dump config values; found and fixed the two real instances (see Done section: `orchestrator.rs`'s Librarian `chroma_client` parse-failure log and `ask.rs`'s `--agentic` parse-failure log both used to echo the raw config string, which can legitimately embed `chroma_token`). No other call site found doing this. Left `[~]` rather than `[x]` since this is an ongoing practice for new code, not a one-time fix — no dedicated lint enforces it.
- [x] Closed 2026-08-04 (no userbase, so a speculative feature isn't worth carrying): Ollama has no standard built-in auth mechanism to integrate against (unlike Chroma's token auth or Anthropic's API key) — any real deployment needing auth would front it with a reverse proxy, outside this CLI's scope. Revisit only if a concrete need shows up.
- [ ] Rate limiting / retry backoff configuration — **investigated 2026-08-04**: exponential-backoff retry infrastructure already exists and is used (`retry_transient!`, `utils/retry.rs`, applied to every Chroma call). The real gap is narrower than the item title suggests: neither chat provider's `chat_stream` (`Ollama`'s own impl in `llm_client.rs`, nor `AnthropicClient`'s) uses it at all — but that's not obviously an oversight to just fix mechanically, since retrying a stream that's already emitted partial content to the live UI would duplicate/corrupt output. A real fix needs to distinguish "failed before any bytes arrived" (safe to retry) from "failed mid-stream" (not safe) — a genuine design question, not a quick wrap-in-the-macro change, so not attempted here. `is_transient` also doesn't recognize `RuChatError::AnthropicError` as retriable at all yet (only `OllamaError`/`ChromaHttpClientError`), worth adding once the streaming question above is resolved.

## Low Priority / Nice-to-have

- [ ] API versioning for future breaking changes (`/v1/`)
- [ ] Plugin system for custom tools and agents
- [ ] Web UI / server mode
- [ ] Export conversation as Markdown / JSON
- [ ] Voice input / output support
- [ ] Multi-modal support (images via `qwen2.5vl`, etc.)
- [ ] Agentic evals (`core/agent/evals.rs`, found 2026-08-04) cover only 3 of 7 roles — Architect, Worker, Validator; zero coverage for Librarian, Critic, Scoper, Summarizer. Matches this project's own accepted framing that eval gaps are lower urgency than deterministic test gaps (`CLAUDE.md`'s Testing Strategy section), so Low rather than Medium — but Librarian prompt-reliability regressions (RAG query construction) specifically would currently only be caught by a full live run.

## Done / Recently Completed

- [x] --trace-timings: per-turn duration_ms in trace files (2026-08-04). See section 0 item 10.
- [x] git apply --check second opinion on apply_patch diffy failures (2026-08-04). check_with_git_apply, protocol.rs. Diagnostic only, doesn't change the apply engine.
- [x] Grounded apply_patch diffs in real file content: numbered rejection dump + ripgrep --context flag (2026-08-04). protocol.rs.
- [x] cargo test --lib no longer pollutes real ruchat_traces/ (2026-08-04). cfg!(test) early-return in trace()/init_trace_index()/finalize_*_trace(). Cleaned up 467 stray files.
- [x] ruchat index skips files unchanged since the last successful run; --force to bypass (2026-08-04). .ruchat_index_state/<collection>.marker.
- [x] chroma-delete --force whole-collection deletion: help text clarified, was already a feature (2026-08-04).
- [x] Repo-wide cargo fmt pass, 45 files, no behavior change (2026-08-04). fmt --check clean since.
- [x] CRITICAL: ripgrep tool had no repo-root containment and no `--` flag terminator (live-verified exploit: read /etc/shadow-, RCE via --pre=) (2026-08-04). fs.rs canonicalize_within_repo, build_rg_args. See fb54763.
- [x] Resumable/crash-resilient runs: ruchat_checkpoint.json after each stage transition, --resume flag (2026-08-04, ROADMAP Phase 3). checkpoint.rs. Live-verified (SIGKILL mid-round, --resume continued correctly).
- [x] Interactive --approve gate on Stage::Commit (2026-08-04, ROADMAP Phase 3). is_approval_yes. Gate logic unit-tested; never live-reached Stage::Commit in 4 attempts (models stalled earlier - not a defect in this code).
- [x] SQLite-vec vector-store backend: real create/write/query, second VectorStore impl (2026-08-04, ROADMAP Phase 3). providers/vector/sqlite_vec. --vector-provider sqlite-vec / --sqlite-vec-path. Chroma admin-subcommand parity (get/delete/modify/...) deliberately not built.
- [x] --debug-sequence breakpoints: --step, --breakpoint <role> (2026-08-04, ROADMAP Phase 2). DebugBreakpoints; debug_stage_machine only, never wired into real runs.
- [x] Paragraph chunking for non-code files with no ctags symbols (2026-08-04, ROADMAP Phase 2). chunk_by_paragraph, core/index.rs. Also fixed md/txt language detection.
- [x] Document summarization before the Worker on large RAG retrievals (2026-08-04, ROADMAP Phase 2). maybe_summarize_retrieved_docs, doc_summary.rs. No-op without a Summarizer configured.
- [x] Multi-collection queries: Query.collection is Vec<String>, each queried/reranked independently (2026-08-04, ROADMAP Phase 2). Side effect: found/fixed clap short-flag collisions across 4 files (every --help panicked in debug builds).
- [x] Closed a debug-mode fixture gap: 2 of 11 agent_debug/*.json fixtures were never actually run by a test (2026-08-04, ROADMAP Phase 2).
- [x] ruchat chroma-init: reads db_config.json, get_or_create_collection per entry (2026-08-04, ROADMAP Phase 2). providers/vector/chroma/init.rs.
- [x] Anthropic (Claude) opt-in chat provider - chat only, no embeddings API, RAG/memorize stay Ollama-only (2026-08-04). providers/llm/anthropic/. --chat-provider anthropic. Orchestrator's ollama field split into chat/embed.
- [x] Agentic evals: live-model behavioral tests, #[ignore]d, run via `--ignored agent_eval` (2026-08-03). agent/evals.rs. 3 starter evals; the Architect one is genuinely flaky by design (real model judgment variance, not a bug).
- [x] Added then reverted replace_in_file as an apply_patch alternative (2026-08-03). No real-run improvement over diffs; diff syntax wasn't the real failure mode. apply_patch stays the sole write tool.
- [x] Sped up cargo build (lld linker) and ruchat index (bounded-concurrency ctags/embed phases) (2026-08-03). .cargo/config.toml, core/index.rs.
- [x] Fixed memory recall always querying collection "default" instead of --collection; fixed the Architect repeating an identical plan after real file content had disproved its assumption (2026-08-03). recall_prior_memories; architect.md now treats shown file content as ground truth.
- [x] Run summary now lists every contributing issue, not just one, wrapped at 120 chars (2026-08-03). run_summary.rs, shared utils::text::wrap_line.
- [x] Trace readability: Validator VALIDATED verdicts, approving critic reviews, and the Scoper's raw output previously left no trace turn at all (2026-08-03). Now pushed unconditionally.
- [x] Trace readability: Worker's read-only tool-call actions weren't recorded (only their output); apply_patch diffs rendered as one unreadable \n-escaped line (2026-08-03). render_turn_content_for_trace.
- [x] Fixed CONTAINS on scalar metadata fields always returning zero rows, across query.rs, get.rs/retrieve.rs, delete.rs (2026-08-03). Chroma's filter language has no scalar-substring op; added a client-side metadata_matches evaluator in where.rs.
- [x] Memory recall now works without a Librarian configured, so memorize-only runs can recall what they wrote (2026-08-03). EmbedArgs gained collection_name/embed_model_name/client accessors.
- [x] Wired a real producer for AgentEvent::Progress (2026-08-03). progress_pct, sent from Stage::Plan each round.
- [x] Chroma unreachable during Librarian retrieval no longer kills the run (2026-08-03). run_librarian_retrieval degrades gracefully; Ollama-unreachable-at-start left alone (can't proceed regardless).
- [x] Fixed a regression from the default-model fix: `pipe` failed instantly with "No model specified" (2026-08-03). resolve_model_slot_count - OllamaArgs::init's .max(1) had forced resolution even for callers passing an empty default.
- [x] apply_patch: clear rejection for a diff spanning two files instead of a cryptic parse crash (2026-08-03). Also cleaned Librarian retrieval noise (raw Debug-formatted metadata, uncapped references list) and switched ruchat index's file walk to `git ls-files` instead of an unscoped recursive walk.
- [x] Trace file overhaul: one file per run (ruchat_traces/ruchat_trace_<N>.md, never clobbered), full unfiltered trace body, LLM-generated outcome summary on every run, archived to successes/ or failures/ (2026-08-03). run_summary.rs (renamed from postmortem.rs).
- [x] Prompt-engineering pass over every agent_role/*.md template: moved the "no human available" rule into the shared system message, strengthened validator.md/summarizer.md/critic.md (2026-08-03, quality pass, no bug reported).
- [x] read_tags auto-regenerates a missing/stale tags file, always scoped to `git ls-files -- '*.rs'` via stdin, never a raw recursive walk (2026-08-03). Root-caused a real incident: tags had grown to 494MB/2.5M lines after a recursive ctags run swept in a gitignored docs/ dir; regenerated at 108KB.
- [x] apply_patch rejection now shows the file's real current content (numbered), not just diffy's raw error, after a run showed the Worker fabricating a plausible but nonexistent function signature (2026-08-03). MAX_SHOWN_ORIGINAL_CHARS.
- [x] Worker calling a read-only tool twice in a round (instead of switching to apply_patch) now gets a clearer rejection plus a proactive reminder right after the first tool result (2026-08-03).
- [x] Worker replying with a narrative walkthrough instead of a real tool_call now gets rejected deterministically on the first no-tool-call response, not just via the LLM Validator (2026-08-03). run_implement_patch_loop, Validation::Skip. architect.md/worker.md now name and reject this pattern explicitly.
- [x] Two apply_patch diff-parsing fixes from a real failed run: wrong hunk-header line counts, and no --- a/+++ b/ headers at all (2026-08-03). fix_hunk_header_counts, protocol.rs.
- [x] Multi-file patches per round: Stage::Implement loops up to a 3-call patch_budget instead of finalizing after one apply_patch (2026-08-03). should_continue_patch_loop. pending_patch became pending_patches: Vec<PendingPatch>.
- [x] cargo_clippy typed Worker tool, mirrors cargo_check's plain-text shape (2026-08-03).
- [x] Commit message body lines now hard-wrapped at 80 chars (models didn't reliably honor the prompt instruction); fixed a missing newline gluing role banners onto the previous turn's output (2026-08-03). wrap_commit_message_body, render.rs.
- [x] Removed a "querying 'model'" trace line sent on every single turn - pure noise once each role has its own banner (2026-08-03). model_summary() prints configured models once at run start instead.
- [x] Three bugs from a live multi-critic run (2026-08-03): commit_feature_branch staged the whole working tree (`git add .`) instead of just the AI's change; commit messages were a fixed uninformative string, now LLM-generated from the real staged diff; concurrent critics streamed onto the same channel and interleaved into garbled text (each critic now gets its own local channel).
- [x] Fixed two secret-leaking log lines: Librarian setup and --agentic parsing both echoed the raw config string (can embed chroma_token) on a parse failure (2026-08-03).
- [x] Refreshed comparisons/*_COMPARISON.md against the current codebase - all four still described a removed generic SHELL tool and the old flat-string Context (2026-08-03). Added a Safety/Sandboxing row to each.
- [x] Found and fixed a bug making Librarian RAG retrieval silently render as empty text on every real run (2026-08-03): OutputArgs derived Default, but its clap default_value only applies via Parser::parse_from, so every non-CLI construction (incl. the real Librarian path) got an empty fields list. Added a manual impl Default for OutputArgs.
- [x] Automatic memory recall at session start, before Stage::Scope (2026-08-03). recall_prior_memories. Deterministic query from ctx.goal; no-op before anything's ever been memorized.
- [x] Resource-limited sandboxing for cargo subprocesses: RLIMIT_AS (4GiB) + RLIMIT_CPU via pre_exec, inherited by every child rustc/build-script process (2026-08-03). orchestrator::cargo::limit_resources. Unix-only.
- [x] BuildReport::rejection_message() surfaces parsed compile errors (file:line:col) and warnings to the Worker, not just a raw diagnostics string; fixed warnings-only compiles rendering as an empty string (2026-08-03).
- [x] Fixed model_options file/config merge being a silent no-op - the gate checked serialized ModelOptions::default(), always `{}` since every field skips serializing when None (2026-08-03). Replaced with an explicit MODEL_OPTION_KEYS allowlist, cli/options.rs.
- [x] apply_patch scope check against the Architect's plan: a plan's FILES: line now bounds which files apply_patch accepts (2026-08-03). Context::planned_files, protocol.rs. Fails open when FILES: is absent - local models don't reliably follow new prompt conventions.
- [x] v0.2.0 released (2026-08-03).
- [x] Fixed a flaky test: two option-file tests raced on the same relative path under the multi-threaded runner (2026-08-03). Switched to tempfile::tempdir().
- [x] Removed a double JSON round-trip in ModelArgs::build_generation_request (2026-08-03). options::merge_options_json. Surfaced the model_options no-op bug above.
- [x] Global config file with profile support - turned out to already exist and work (~/.config/ruchat/config.json, --profile) (2026-08-03). Deleted a dead duplicate reader, cli/serde.rs::read_config_file.
- [x] Migrated genuine diagnostic println!/eprintln! in src/core, src/providers to tracing; left each subcommand's actual designed stdout output (chroma-ls, ollama ls, etc.) untouched (2026-08-03).
- [x] Fixed error handlers that discarded useful diagnostic info via map_err(|_| ...): model-not-found vs. Ollama-unreachable, ToolParseError::UnknownTool now carries the actual bad name (2026-08-03). Also replaced an unwrap() in func_struct's chat loop and ~8 is_string()/unwrap() pairs with if-let.
- [x] Added unit tests for include.rs/where.rs's parse()/update_from_json() wrappers and cli/prompt.rs, previously zero coverage (2026-08-03).
- [x] Fixed cargo test --lib being uncompilable (33 errors, stale test code after a prior refactor) and a `-h` test call that made clap exit(0) mid-suite, silently killing other tests (2026-08-03).
- [x] Multi-critic consensus review was completely non-functional: Agent::new's config lookup could never find a flat critic config, and Role::from_str didn't recognize "Critic_0"/"Critic_1" naming (2026-08-03). orchestrator.rs, agent/role.rs.
- [x] Wired 9 of 10 agent_debug/*.json fixtures into cargo test --lib via a new FakeLlmClient (2026-08-03). Also fixed a fixture naming bug ("Critic0" vs "Critic_0") that had made multi-critic dispatch silently no-op.
- [x] Added .github/workflows/ci.yml: build + clippy + test on push/PR (2026-08-03). No -D warnings/fmt --check yet.
- [x] Investigated connection pooling for Ollama/Chroma clients - already satisfied, one shared Arc<Client> per run, reqwest's default pooling applies (2026-08-03).
- [x] Consolidated TODO files into single `TODO.md`
- [x] Improved model option merging with CLI flags
- [x] env_logger / tracing integration
- [x] Basic multi-agent orchestration with RAG support
- [x] Git auto-commit feature branch on approval
- [x] Robust Chroma CLI with where/include parsing
- [x] TUI chat with history, undo/redo, selection - **later removed** (2026-07-31, `ad0708d` and surrounding commits deleted the ~1,260-line `providers/llm/ollama/chat/{conversation_tree,history,pos,event_result}.rs` this was built on); no longer accurate as a "done" claim, see the "TUI Chat" section above
- [x] Structured tool calling framework (`agent/tools.rs::ToolName`, schema-validated, replaces regex-only parsing) - 13 typed tools including `apply_patch`, `git_*`, `read_file`, `ripgrep`, `read_tags`, `cargo_check`/`cargo_dupes`
- [x] Parallel critic execution (`Orchestrator::run_critics_parallel`, `futures_util::future::join_all`)
- [x] RAG relevance scoring / reranking (`providers/vector/chroma/rerank.rs`, distance+lexical blend)
- [x] Token-aware history management with automatic Summarizer trigger (`Stage::Retry`, `get_dynamic_history_limit`)
- [x] Pre-planning repo-grounding stage (`Scoper` role - not in the original TODO/ROADMAP list at all)
- [x] Structured `Context` event log (`Vec<Turn>` + `TurnKind`) replacing the old flat-string `history`/`context`/`documents`/`rejections` fields
- [x] Reconciled the legacy `Team`/`Manager` pipeline - `ruchat manager` now runs a saved `Team` preset through the real `Orchestrator` stage machine instead of a separate, unvalidated linear engine
- [x] `apply_patch` diff-size cap (`MAX_PATCH_DIFF_BYTES`, `agent/protocol.rs`) and automatic rollback of a rejected round's patch before looping back to `Plan` (`Context::{record_patch,revert_pending_patch}`)
- [x] Confirmed the "remove dead code" item once flagged above for `conversation_tree.rs`/legacy `Team`/`Manager` is fully resolved - removed the stale duplicate bullet
- [x] Removed an unused `OrchestratorRun` struct (`orchestrator.rs`) - stale leftover, `AgentPipeline` is an enum, not a trait; `ask.rs`/`manager.rs` already construct `AgentPipeline::Orchestrator` directly

---

**Next milestone:** v0.3.0 (ROADMAP.md Phase 2) shipped - RAG improvements, automatic Chroma collection management. v0.4.0 (ROADMAP.md Phase 3) in progress: resumable/crash-resilient runs, HITL approval gate, SQLite-vec backend, and Anthropic chat provider are done; see High Priority for what's left.

Help welcome on any item - especially testing and configuration refactoring.
