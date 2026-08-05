# Ruchat TODO

Last updated: 2026-08-05

## 0. CRITICAL - Agentic run reliability (pinned, DO NOT REMOVE until a live run lands a committed change)

Maintainer's top priority. `ruchat pipe --team-model ... --iterations N "<goal>"` runs mostly
fail (~99/100 by maintainer count; 2026-08-04 baseline: 19/20 archived traces failed). Stay
pinned until a real live Ollama/Chroma run actually commits a change - green `cargo test --lib`
alone does not close this.

Contributors found via real trace evidence (`qwen2.5-coder:14b`), 2026-08-04:

Items no longer listed here were moved to Done.md.

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

30. [ ] **Validator/Critic approve a diff that never made the requested change at all.**
    Live-verified 2026-08-05, 3 consecutive gate runs against `fixtures/gate-repo`: every one
    "landed" a commit that only trimmed lines out of `Evictor`'s doc comment - the trait
    declaration itself was untouched in all three, still triggering the exact dead-code warning
    the task asked to fix - yet Validator said `VALIDATED` and Critic said `APPROVED` every
    time. Neither checks that the concrete thing the goal named is actually gone; both only see
    "a diff applied, tests still pass," which is true of an empty edit to the same file too.
    Root cause on the fixture's side (worked around, not the pipeline bug): the trait's old
    11-line doc comment was self-referential meta-commentary about the gate itself, and the
    model treated it as separately editable content, trimming only the comment across repeated
    attempts instead of recognizing "delete the trait, including its doc comment" as one block.
    Simplified to an ordinary one-line doc comment (`ruchat-gate-fixture` commit
    `b71ca79`) - not yet re-measured whether that alone fixes it. Separately, `gate_measure`'s
    own "landed" criterion (`_commit_touches_source`: any change to a non-changelog file) would
    have counted all 3 of these as passes too - it was deliberately loose ("measures the loop,
    not the model's thoroughness"), but this shows that looseness can hide a completely wrong
    diff, not just a rougher-than-ideal correct one. Worth a stricter check specifically for the
    gate (e.g. grep the resulting file for the named symbol) even if the general-purpose
    examples keep the loose criterion.

31. [ ] **New eval added 2026-08-05: compile-error mutation testing, a strict counterpart to
    the gate's loose criterion above.** `fixtures/mutant-repo` (a second, separate fixture
    submodule - deliberately not folded into `gate-repo`, which carries clippy warnings/dead
    code on purpose and would give an agent something else to wander off "fixing" instead of
    the one compile error this eval cares about) has 6 tiny modules, each with a hand-authored,
    single-location compile error and a known-correct fix (the mutation's own reverse - see
    `fixtures/mutant-repo/mutations/mutations.json`). `scripts/mutant_eval.sh` applies one,
    runs the pipeline against it with a goal that doesn't name the file or the bug (unlike the
    gate, forcing real diagnosis from `cargo_check`'s own output), then diffs the landed
    commit's target file against the known-correct content: byte-identical = exact match; a
    commit landed but differs = "alternate fix" (Stage::Test already guarantees it compiles, so
    this isn't a failure, just not *the* expected one) - saved to `mutant_eval_results/
    alternates/` for later review rather than auto-scored, per maintainer request ("some
    alternates are better than others... valuable to collect and re-evaluate later, with more
    intelligence" - not built yet, just the collection point). No commit reaching
    `Stage::Commit` at all = no land. NEEDS A LIVE RUN - mechanics (mutation apply/restore,
    branch diffing, all three verdict paths) verified without one by hand-simulating each
    outcome in the fixture repo directly.
    Deliberately deferred, per maintainer ("nice to have, but for later"): a second mutation
    category where the crate still compiles but a test fails (a logic bug, not a syntax one) -
    would need `cargo test` as the pass/fail signal instead of `cargo_check`, closer to a real
    bug-fixing task than any eval here so far. Not built - only `cargo_check`-detectable
    mutations exist right now.

32. [ ] **Rejections are raw compiler output, never interpreted.** Tester returns
    `src/core/agent.rs:115:17: error: struct Agent has no field named options` - a complete
    diagnosis - but nothing turns it into "the patch is incomplete: line 115 still uses this
    field, a correct fix removes both sites." The Architect must infer it and, over 4 rounds,
    never did. Synthesize an explicit incomplete-deletion message when a compile error names a
    symbol the just-applied patch deleted; the compiler already supplies the line number.

33. [ ] **`history_view` never collapses superseded content, so the wrong answer outvotes the
    correction.** It filters only `TurnKind::Retrieval`; everything else accumulates verbatim.
    By round 5 of trace 531 the Architect's HISTORY held its own failed plan 6x and the failed
    diff 8x against 3 copies of the rejection - the same wrong answer shown twice as often as
    its correction, in identical formatting. Collapse repeats to one instance plus a
    "(repeated Nx)" marker. This is the root cause behind the maintainer's "too much
    irrelevant information" and "information not rephrased after failure" (2026-08-05).
    NOTE: architect.md already contains five paragraphs forbidding a repeated plan, including
    "will be treated as a stall". It repeated anyway. More prompt text is not the fix.
    Partial progress 2026-08-05: the Architect/Scoper "this round repeated the previous one"
    notes (separate from `history_view` itself, this item's actual scope) only fired on
    byte-identical output, so a repeat with a changed self-referential detail (e.g. an
    incrementing "Round N") evaded detection entirely. Both now use
    `stall_mitigation::is_near_duplicate` (word-set Jaccard similarity, >=0.9, digit-normalized)
    instead of `==`. `history_view` collapsing itself is still unaddressed.

34. [ ] `ruchat index` cannot correctly re-index a changed file: chunk IDs derive from chunk
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

37. [ ] Timing measurements must not run concurrently with local-model delegation: ruchat and
    the `ollama-heavy` MCP share :11434 and there is one usable GPU, so they queue against
    each other. Turn delegation off before the section-0 reliability gate.

39. [ ] `LlmClient::chat_stream` sets no `num_ctx`, so Ollama silently truncates any prompt over
    the model default - agent prompts, not just summaries. Item 18 clamps its own input as a
    local mitigation; the general case is unhandled and would explain context-blind agent
    behavior in long runs. Not theoretical: it demonstrably destroyed the run summary on
    trace 472 (see item 18), and the same mechanism drops HISTORY/DOCUMENTS from a Worker
    prompt in a long run - which looks exactly like the model ignoring its own context.

### NEXT ACTION (decided 2026-08-04, roadmap review)

Stop writing new mitigations until there is a number that can move. In order:

2. [ ] **Measure the real success rate on it.** `bash scripts/refactoring_examples.sh gate 5`.
   Runner corrected 2026-08-05: it counted new `ai/feature-*` branches, which would have scored
   all 5 changelog-only commits (#15) as lands, and stops incrementing at all now that branches
   are continued (#16). It now diffs the commit set before/after and requires a new commit
   touching a file other than `featured_changes.md`. Still aborts if a run leaves the tree
   dirty (contributor #7 recurring). Replaces "~99/100 fail" with a baseline. Gate for Phase 2's
   milestone, softened 2026-08-05 (maintainer call, see ROADMAP.md): >=60% of a 5-run batch
   lands (was "5 consecutive" - a streak metric that stayed at effectively 0% regardless of
   real progress, since one failure reset it). `gate_measure` now prints MET/not met against
   this bar itself. NEEDS A LIVE RUN.

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

### 4. Security & CI (found 2026-08-04, specialist review round — see below)

### 5. TUI Chat — closed 2026-08-04
**Decision: ruchat is a non-interactive CLI.**

## Medium Priority

### Code Quality & Maintainability
- [ ] Add integration tests for full agentic flows (using test Ollama/Chroma) — `agent_debug/*.json` already contain ready-made stage sequences (`architect_only`, `worker_and_validator_rejection`, `multiple_critics`, etc.); wire these into `cargo test` against a mocked `LlmClient`/`VectorStore` instead of writing fixtures from scratch
- [ ] Consistent error handling across Chroma subcommands
- [ ] Refactor duplicated JSON update logic (`update_from_json` methods) — an 8th instance now exists (`SqliteVecClientConfigArgs::update_from_json`, found 2026-08-04), though it's a single-field stub too thin to justify a standalone shared-helper item on its own; folds into this existing one.

### Chroma / RAG
- [ ] Add progress bar for large embedding jobs
- [ ] Implement caching layer for repeated file embeddings — **investigated in depth 2026-08-04, turned out to have a real correctness trap, not implemented:** `embed_chunks` (`embed.rs`) computes embeddings for *every* chunk via a live Ollama call before checking `existing_ids` — looks like a free win to reorder (skip re-embedding when the content-hashed ID already exists, since `Md5(model:id_prefix:content)` means same ID ⟹ same content ⟹ same embedding, deterministically). The trap: the ID hash does **not** include metadata, only `model`/`id_prefix`/`content` — so the exact same text chunk can legitimately need a metadata-only update on a re-run (e.g. `ruchat index` after ctags starts reporting a different `signature`/`scope`/`references` for an otherwise-unchanged code chunk). Skipping the write whenever the ID exists would silently drop that metadata update, a real data-loss bug, not just a missed optimization. Doing this safely needs either fetching the stored embedding to pair with fresh metadata (a different round trip, might cost as much as just re-embedding for small chunks) or a new metadata-only update path on `VectorCollection` (`add`/`update`/`upsert` all currently require sending embeddings). Real API-surface question, not a quick reorder — not attempted.
- [ ] Better metadata normalization and type safety

### Architecture & Providers (found 2026-08-04, specialist review round)
- [ ] `VectorStore`/`VectorCollection` (`agent/llm_client.rs`) take `chroma::types::{Where, Metadata, QueryResponse, IncludeList, UpdateMetadata}` directly in their signatures — the "pluggable vector-store" seam is really "pluggable, as long as you speak Chroma's wire types." The SQLite-vec backend already has to `use chroma::types::{...}` just to satisfy the trait it implements. Fine with two implementors; a third (or ever dropping the `chroma` crate as a dependency) forces either a permanent type dependency or a breaking trait redesign. **Maintainer decision 2026-08-04: defer** — no functional cost with only two backends, don't spend time on a newtype pass speculatively; revisit if/when a third backend (e.g. LanceDB) is actually being considered.
- [ ] `Orchestrator::new`'s Librarian client construction (`orchestrator.rs:196-235`) re-implements "pick Chroma vs SQLite-vec, build the right client" by hand on stringly-typed JSON (`remove_str` → `.parse::<Value>()` → `update_from_json`), duplicating `EmbedArgs::client`/`resolve_collection`'s already-typed version of the same branch. Two independent copies that must be kept in sync by hand — a new field on `SqliteVecClientConfigArgs` silently won't reach the Orchestrator's path unless someone remembers to mirror it. Share the logic (e.g. give the Librarian config the same typed struct `EmbedArgs` uses).
- [ ] Neither `--chat-provider` nor `--vector-provider` reach `Team`/`ruchat manager` presets at all (`grep` confirms zero hits in `core/agent/team.rs`) — concretely worsens the already-tracked config-file/CLI-merge gap (`cli/serde.rs::load_merged_config`, see High Priority #1): a saved Team can never select Anthropic or SQLite-vec, only a live `ask`/`pipe` invocation can.
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
- [ ] Optimize history limit calculation and token counting — **investigated 2026-08-04**: token counting (`core/agent/tokens.rs`) is already a real BPE tokenizer (`tiktoken_rs::cl100k_base`), not the old `len()/4` heuristic — reasonably optimized already, and the remaining imprecision (not the exact per-model tokenizer) is a fundamental Ollama API limitation, not a code quality gap. `get_dynamic_history_limit` (`ollama/model.rs`) is a hardcoded model-name-substring table (`qwen2.5`→128k, `llama3`→8k, else 4k) — `ollama_rs::Ollama::show_model_info` could query a model's *real* context length via `/api/show`, but its `model_info` field is a raw `serde_json::Map` with an architecture-prefixed key name (`"llama.context_length"`, `"qwen2.context_length"`, etc. — not a fixed key), and querying it means turning a synchronous hot-path function async across every call site (`agent.rs`, `orchestrator.rs`'s summarizer check). Real improvement, but a multi-call-site signature change needing testing against several real Ollama models to get the key-name matching right — not a quick fix, not attempted here.

### Security & Production Readiness
- [~] Never log sensitive data (tokens, prompts with secrets) — audited `src/` for tracing/println calls that interpolate raw config strings or `{:?}`-dump config values; found and fixed the two real instances (see Done section: `orchestrator.rs`'s Librarian `chroma_client` parse-failure log and `ask.rs`'s `--agentic` parse-failure log both used to echo the raw config string, which can legitimately embed `chroma_token`). No other call site found doing this. Left `[~]` rather than `[x]` since this is an ongoing practice for new code, not a one-time fix — no dedicated lint enforces it.
- [ ] Rate limiting / retry backoff configuration — **investigated 2026-08-04**: exponential-backoff retry infrastructure already exists and is used (`retry_transient!`, `utils/retry.rs`, applied to every Chroma call). The real gap is narrower than the item title suggests: neither chat provider's `chat_stream` (`Ollama`'s own impl in `llm_client.rs`, nor `AnthropicClient`'s) uses it at all — but that's not obviously an oversight to just fix mechanically, since retrying a stream that's already emitted partial content to the live UI would duplicate/corrupt output. A real fix needs to distinguish "failed before any bytes arrived" (safe to retry) from "failed mid-stream" (not safe) — a genuine design question, not a quick wrap-in-the-macro change, so not attempted here. `is_transient` also doesn't recognize `RuChatError::AnthropicError` as retriable at all yet (only `OllamaError`/`ChromaHttpClientError`), worth adding once the streaming question above is resolved.

## Low Priority / Nice-to-have

- [ ] API versioning for future breaking changes (`/v1/`)
- [ ] Plugin system for custom tools and agents
- [ ] Web UI / server mode
- [ ] Export conversation as Markdown / JSON
- [ ] Voice input / output support
- [ ] Multi-modal support (images via `qwen2.5vl`, etc.)
- [ ] Agentic evals (`core/agent/evals.rs`, found 2026-08-04) cover only 3 of 7 roles — Architect, Worker, Validator; zero coverage for Librarian, Critic, Scoper, Summarizer. Matches this project's own accepted framing that eval gaps are lower urgency than deterministic test gaps (`CLAUDE.md`'s Testing Strategy section), so Low rather than Medium — but Librarian prompt-reliability regressions (RAG query construction) specifically would currently only be caught by a full live run.

---

**Next milestone:** v0.3.0 (ROADMAP.md Phase 2) shipped - RAG improvements, automatic Chroma collection management. v0.4.0 (ROADMAP.md Phase 3) in progress: resumable/crash-resilient runs, HITL approval gate, SQLite-vec backend, and Anthropic chat provider are done; see High Priority for what's left.

Help welcome on any item - especially testing and configuration refactoring.
