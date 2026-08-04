# Ruchat Roadmap

**Vision**  
Ruchat remains the **fastest, lightest, local-first** AI agent orchestration tool built for software engineering workflows.  
It stays 100% Rust-native, zero Python dependencies, and runs entirely offline by default with **Ollama + Chroma** (or future local vector DBs) — cloud-backed providers are a possible *opt-in* extension for use cases that call for them (see Phase 3), not a reversal of the default.  
We prioritize **predictability**, **performance**, **token efficiency**, and **tight integration** with local tools (Git, file system, terminal) over general-purpose flexibility offered by LangChain, LangGraph, AutoGen, or CrewAI.

**Core Differentiators to Preserve**
- Local-first by default (Ollama + Chroma today; the same `LlmClient`/`VectorStore` trait seam that already backs `FakeLlmClient`/`FakeVectorStore` in tests is the natural extension point for an opt-in cloud provider later — see Phase 3)
- Explicit shared `Context` + fixed-role supervisor-critic pipeline
- Predictable linear flow with approval gates and Git auto-commit
- Minimal overhead and low token usage
- Simple, auditable code path

---

### Phase 1: Stability & Polish (v0.2.0) — Q2 2026

**Goal**: Production-ready foundation

- [~] Consolidate configuration system — the config-file + profile piece already existed and works (`cli/config.rs::ConfigArgs`, JSON not TOML but that was always an allowed format); added `OLLAMA_SERVER`/`CHROMA_TENANT`/`CHROMA_DATABASE` env vars for parity with the existing `CHROMA_SERVER`/`CHROMA_TOKEN`. What's still open is the generic per-flag CLI/file merge — deliberately deferred already (see `cli/serde.rs::load_merged_config`'s comment, and `TODO.md`), not attempted here
- [x] Structured logging (`tracing`) — `main.rs` wires `tracing_subscriber` with `EnvFilter` for configurable levels (`RUST_LOG`) and now also supports `RUCHAT_LOG_FORMAT=json` for newline-delimited JSON output; genuine diagnostic `eprintln!`/`println!` call sites in library code migrated to `tracing`, remaining ones are each command's actual stdout output (see `TODO.md`)
- [~] Comprehensive error handling with actionable messages — fixed the concrete cases found where an error handler discarded the real cause (model-not-found vs. unreachable-Ollama-server, unknown tool names, a crash-on-transient-failure `.unwrap()`) and removed ~8 provably-safe-but-panic-shaped unwraps; the much larger job — auditing ~85 call sites using the generic `InternalError`/`Is` catch-all variants — is still open, see `TODO.md`
- [x] Unit tests for all parsers (`where.rs`, `prompt.rs`, `include.rs`) — `where.rs` already had 19 tests; added coverage for `IncludeArgs`/`WhereArgs`'s `parse()`/`update_from_json()` wrappers (previously only their internal free functions were tested) and all of `cli/prompt.rs` (previously zero tests) — 29 new tests total
- [~] Core orchestration test coverage — 9/10 `agent_debug/*.json` fixtures are wired into `cargo test --lib` via `FakeLlmClient`/`FakeVectorStore` (this is how the multi-critic dispatch bug below was caught); the last fixture combination and true integration tests against a live Ollama/Chroma are still open
- [ ] ~~Fix TUI redraw artifacts, improve selection/copy/paste, and add help screen~~ — moot as written: the interactive chat TUI this describes was deleted 2026-07-31 (~1,260 lines, see `TODO.md`). Only a non-interactive streaming-output renderer remains. Rebuilding an interactive TUI (if still wanted) is new work, not a bug fix — worth its own roadmap item rather than reusing this one
- [x] Optimize model option merging (removed the double JSON round-trip in `ModelArgs::build_generation_request`) — surfaced a separate, deeper pre-existing bug in the process (config-file `model_options` merging is currently a silent no-op), tracked in `TODO.md`, not fixed yet
- [x] Connection pooling for Ollama and Chroma clients — investigated, already satisfied by the existing architecture (single shared `Arc`-wrapped client per orchestrator run, reqwest's default pooling underneath, nothing disabling it), not a real gap — see `TODO.md` for the detail
- [x] CI workflow (`.github/workflows/ci.yml`): build + `cargo clippy --lib --tests` + `cargo test --lib` on push/PR — deliberately no `-D warnings` or `fmt --check` gate yet (pre-existing dead-code warnings and repo-wide fmt drift need cleanup first, see `TODO.md`)
- [x] Release v0.2.0 with clean `TODO.md` → `DONE` migration — version bumped in `Cargo.toml`/`README.md`, all `[x]`-completed items moved out of `TODO.md`'s priority sections into its `Done` list, `cargo check --lib`/`cargo test --lib`/`cargo clippy --lib --tests`/`cargo build --release` verified clean per the deploy checklist (fixed one flaky test found in the process — `cli::options::tests::test_read_options_file` raced another test over a shared filename)

**Milestone**: Reliable daily driver for local coding agents.

---

### Phase 2: Enhanced Local Agent Capabilities (v0.3.0) — Q3 2026

**Goal**: Make the fixed pipeline significantly more powerful while staying local

- [x] Structured tool calling framework (`agent/tools.rs::ToolName` — schema-validated, 13 typed tools, no more regex-only parsing)
- [x] **`cargo_clippy` typed tool** — the Worker (not the Scoper — same restriction as `cargo_check`/`cargo_dupes`, see `agent/tools.rs::prompt_scoper_tool_catalog`) can now request `cargo_clippy` to see clippy's own warnings directly, instead of guessing at what a "fix a clippy lint" task means. `orchestrator::cargo::cargo_clippy()` — `cargo clippy --message-format=short`, resource-limited and timeout-capped exactly like `cargo_check`; see `TODO.md` Done section for detail, including a correction of this item's original plan (JSON parsing wasn't actually the right shape to reuse).
- [x] **Multi-file patches per round** — `Stage::Implement` now allows up to 3 sequential `apply_patch` calls per round (design decision: N sequential calls, not a multi-file diff format — reuses `diffy` and the existing per-call scope/tracked-file/size checks unchanged), so a plan whose `FILES:` line names more than one file can land as a single commit instead of only ever touching the first. `Context::pending_patch` became `pending_patches: Vec<PendingPatch>`; `commit_add_targets`/`fallback_commit_message` (`orchestrator/git.rs`) now cover every file touched, not just one. See `TODO.md` Done section for full detail, including the backward-compatibility guarantee (a plan naming zero or one file is unaffected).
- [x] Parallel critic execution (`Orchestrator::run_critics_parallel`) — note: the execution mechanism itself was correct, but a separate construction bug meant `critics` was always empty in practice until fixed (see TODO.md's Done section)
- [x] Token-aware history management + automatic summarization triggers (`Stage::Retry` → Summarizer when the token estimate exceeds the model's history limit)
- [x] Persistent memory layer — the `memorize` tool writes to Chroma (`Agent::embed`), and `Orchestrator::recall_prior_memories` auto-recalls at session start using the goal text as a deterministic query; originally only worked when a Librarian was configured, extended (2026-08-03) so a memorize-only run with no Librarian at all still recalls via the Worker's own `embed_args`, since that's what `Memorize` already writes through — see `TODO.md` Done section
- [x] Improved RAG — relevance scoring/reranking (`providers/vector/chroma/rerank.rs`), multi-collection queries, document summarization before the Worker, and smarter chunking all done as of 2026-08-04. Multi-collection: `Query`'s `collection` field is now a list — the Librarian can name more than one collection in one query, each searched independently for the full `n_results` and rendered as its own labeled block, since different collections can have entirely incompatible metadata schemas. Document summarization: retrieved RAG content above a size threshold is condensed by a one-shot LLM call (reusing the Summarizer's configured model, a distinct prompt from `agent_role/summarizer.md`'s history-compression one) before it reaches a `TurnKind::Retrieval` turn; opt-in the same way whole-history compression already is (no Summarizer configured = no-op). Smarter chunking: `ruchat index` no longer falls back to embedding an entire file as one chunk whenever ctags finds no symbols — a real paragraph-boundary chunker (`chunk_by_paragraph`) now splits any sufficiently large file that isn't ctags-parseable code into coherent line-range chunks instead; `.md` also gained a correct `language: "markdown"` metadata value (it already had real ctags heading-based chunking via `chapter`/`section`/`subsection` kinds, just mislabeled as "unknown" before). See `TODO.md` Done section for all three.
- [x] Automatic collection management — new `ruchat chroma-init` subcommand (`providers/vector/chroma/init.rs`) reads a `db_config.json`-shaped file and ensures every documented collection exists via `get_or_create_collection`, instead of a manual `chroma-create` per collection. Idempotent by construction, verified live: re-running is a no-op for collections that already exist. See `TODO.md` Done section for detail, including a genuine nuance found while verifying against a real Chroma instance (collection metadata is only applied on first creation, not on an already-existing collection — matches Chroma's own get-or-create semantics, not a bug).
- [x] Resource-limited sandboxing for tool-invoked subprocesses — every cargo subprocess (`cargo_check`/`cargo_dupes`/the Tester's check+test) now gets `RLIMIT_AS`/`RLIMIT_CPU` via `orchestrator::cargo::limit_resources`, alongside the pre-existing wall-clock timeouts; see `TODO.md` Done section
- [x] Debug mode improvements (step-by-step execution, breakpoint support) — complete as of 2026-08-04. The fixed-sequence debug mode itself exists (`--debug-sequence`, `agent_debug/*.json`) and, corrected 2026-08-04: it *was* already wired into `cargo test --lib` via `run_fixture`/`build_test_orchestrator` (this bullet previously said otherwise, which was stale) — 9 of 11 fixture files had a dedicated test; the two `architect_librarian_*` combinations were the only real gap, now closed. Breakpoint support shipped the same day: `--step` (pause after every role) and repeatable `--breakpoint <role>` (pause after specific ones) on `ruchat pipe`, waiting on stdin (Enter to continue, `c` to stop pausing, `q` to abort) — `--debug-sequence`-only, never the real unattended `run_stage_machine`. Verified live against a real Ollama server. See `TODO.md` Done section.
- [x] Reconciled the legacy `Team`/`Manager` pipeline — `ruchat manager` now expands a saved `Team` preset into an `Orchestrator` config and runs the real stage machine
- [x] Pre-planning repo-grounding stage (`Scoper` role) — gathers repo facts before the Architect plans; shipped but was never in the original Phase 2 list
- [x] `apply_patch` hardening: diff-size cap, and automatic rollback of a rejected round's patch before looping back to `Plan`
- [x] `apply_patch` scope check against the Architect's plan — Architect prompt now declares a `FILES:` line, enforced (fail-open only when the line is absent) in `Validation::apply_patch`; see `TODO.md` Done section for detail

**Milestone**: Best-in-class local coding agent (plan → code → review → commit) that beats Python frameworks in speed and reliability.

---

### Phase 3: Controlled Extensibility (v0.4.0) — Q4 2026

**Goal**: Add flexibility without sacrificing predictability or local purity

- [x] **Resumable/crash-resilient runs** (checkpointed `Context`) — shipped 2026-08-04. `core/orchestrator/checkpoint.rs`: after every non-terminal stage transition, `Context` (turns, round, pending patches, trace index) plus the newly-entered `Stage` are written to `ruchat_checkpoint.json`; the file is deleted the moment a run reaches `Stage::Done` or `Stage::Escalate` (a deliberate, recorded outcome, not a crash — nothing left to resume). New `ruchat pipe --resume` reloads it and continues from the last completed stage instead of a fresh `Stage::Scope`, ignoring the prompt argument entirely (the checkpoint's own goal continues). Exactly the scope above: one plain JSON file, no distributed coordination, no partial-stage recovery — a stage that only half-ran when the process died is simply re-run in full from its start on resume, same as any other transition. `--resume` still needs the same `--team-model`/`--agentic` the original run used, since the checkpoint recovers conversation state, not which models/roles to reconstruct. Verified live: force-killed (`SIGKILL`) a real run mid-round against real Ollama models, confirmed a well-formed checkpoint survived with the actual conversation so far, resumed it and watched it correctly continue from round 2 to the iteration budget, then confirmed the checkpoint was cleared once it reached `Stage::Done`. See `TODO.md` Done section.
- [x] **Interactive human-in-the-loop approval gate** — shipped 2026-08-04. Identified via `comparisons/*.md`: AutoGen's UserProxy agents and LangGraph's interrupts both give a human an explicit mid-run pause/approve point; ruchat's only approval mechanism before this was automated Critics (an LLM-driven gate) plus post-hoc review of the committed branch. New `ruchat pipe --approve`: pauses the `Stage::Commit` arm, traces the latest plan and the real pending `git diff`, then blocks on a real terminal y/n via the same `ctx.trace()` + blocking `Io::read_line()` pattern already used (and already live-verified) for `--step`/`--breakpoint`. Anything other than an exact `y`/`Y`/`yes`/`Yes` answer routes through the existing `Stage::Escalate` arm — same checkpoint-clear/trace/break path every other escalation already uses, no special-casing needed. Off by default. See `TODO.md` Done section for the live-verification caveat.
- [ ] Configurable agent graph (simple DAG definition in JSON/TOML — limited cycles)
- [ ] Subgraph / reusable agent modules (e.g., "CodeReviewTeam", "ResearchTeam")
- [ ] Dynamic conditional edges based on approval signals or output patterns
- [ ] Plugin system for custom local tools (Rust crates or WASM)
- [ ] Multiple process types: `sequential`, `hierarchical`, `parallel` — "hierarchical" means an actual manager/sub-task decomposition (a top-level plan broken into sub-goals, each run through its own scoped stage-machine pass), not just the existing `ruchat manager`/`Team` preset expansion (which selects a fixed pipeline shape, doesn't decompose a goal into sub-goals)
- [x] **Pluggable LLM provider abstraction, cloud-optional — Anthropic (Claude) shipped 2026-08-04.** `agent/llm_client.rs`'s `LlmClient` trait (already built for `FakeLlmClient` in tests) turned out to be exactly the right seam: a new `providers/llm/anthropic` module implements it directly on top of `reqwest` + the small `eventsource-stream` crate for Anthropic's SSE-based Messages API, no trait changes needed. Opt-in via `--chat-provider anthropic` (`ruchat pipe`/`ask`) plus `--anthropic-model`/`--anthropic-api-key` (or `ANTHROPIC_API_KEY`) — never a default. Anthropic has no embeddings API, so `Orchestrator` now holds two clients instead of one shared `ollama` field: `chat` (swappable) and `embed` (always Ollama-backed, unconditionally — RAG/memorize/recall are unaffected by `--chat-provider`). Two crates the maintainer flagged as candidates, `octomind`/`octolib` and `agent-client-protocol`, were researched and deliberately not used — the former is a ~25-60-dependency multi-provider megalib with unclear streaming support and a mismatched `reqwest` version, the latter solves an unrelated problem (editor↔agent protocol, not agent↔LLM-provider — a genuinely separate idea for a future `ruchat`-as-a-Zed-agent conversation, not conflated with this one). See `TODO.md` Done section for the full writeup and `--vector` provider abstraction (LanceDB/SQLite-vec) remains open below.
- [ ] **Pluggable vector-store provider abstraction** (split out from the item above, since only the LLM side shipped) — `agent/llm_client.rs`'s `VectorStore` trait already exists (built for `FakeVectorStore` in tests) as the seam; Chroma remains the only implementation. Support for LanceDB or SQLite-vec as alternatives is unattempted.
- [ ] **Explicit chain-of-thought / step-reasoning prompting** for Architect/Worker — today's `agent_role/*.md` templates ask for a plan or an action directly, with no structured "think step by step, then answer" scaffold. Worth a scoped experiment (particularly for the Architect's planning step) once the agentic-evals harness (`core/agent/evals.rs`) has enough scenarios to tell whether it actually improves plan quality on local models rather than just adding tokens.

**Important Constraint**: All new features must remain fully local and offline-capable by default; anything that isn't (e.g. a cloud provider) must be explicit, opt-in configuration, never silently required.

**Milestone**: Ruchat becomes a serious lightweight alternative to LangGraph/CrewAI for local use cases.

---

### Open Design Question: Autonomous Goal-Setting

Not scoped into any phase above, deliberately — this needs an explicit decision, not a silent yes or no.

Ruchat's whole differentiator (see Comparison-Driven Positioning below) is predictable, approval-gated execution against a goal a human supplies: Architect plans, Critics/Validator gate, nothing lands without passing through those checkpoints. Autonomous goal-setting — the agent deciding *what* to work on next, not just how — is a different value proposition, closer to AutoGPT-style open-ended autonomy, and it's in real tension with "predictable and auditable." Adopting it isn't an engineering task like the items above; it's a positioning call that would need to be made explicitly, e.g. scoping it as a narrow, opt-in "propose the next goal, but a human still approves before it runs" mode rather than genuine unattended autonomy. Left here as a flagged question until there's an actual decision, rather than quietly folded into Phase 3/4 as if it were already settled.

---

### Phase 4: Performance & Scale (v0.5.0+) — 2027

- [ ] Async parallel agent execution where safe
- [ ] Model context window auto-management and smart chunking
- [ ] Built-in benchmarking suite vs LangGraph/CrewAI on local hardware
- [ ] Optional distributed mode (multiple local machines via simple message bus — still offline-first)
- [ ] Advanced observability (local trace viewer) — `comparisons/*.md` repeatedly call out LangSmith/AutoGen Studio-style inspection as a strength ruchat lacks; today's per-run `ruchat_traces/ruchat_trace_<N>.md` files (archived into `successes/`/`failures/` with a one-shot LLM summary once a run ends — see `ORCHESTRATION.md`) are a set of individually-navigable snapshots, not a queryable/searchable history across runs. Scope as a local, offline viewer over the existing `Context.turns` log and the `ruchat_traces/` archive (e.g. a `ruchat trace` subcommand listing/filtering past runs and rendering round-by-round turns/rejections), not a hosted service — stays consistent with the local-first-by-default posture above.

---

### Long-Term Vision (2027+)

- Become the de-facto standard for **local software engineering agents**
- Maintain a **local-first, predictable-by-default** philosophy, while staying open to opt-in cloud providers where they genuinely help (see Phase 3)
- Explore safe WASM-based tool sandboxing
- Support additional local vector/search backends without breaking core simplicity
- Provide migration path / interoperability layer for users coming from Python frameworks (export/import graphs)
- **Model fine-tuning / RLHF / reinforcement-driven self-improvement** — currently out of reach and not a priority: ruchat orchestrates existing local models rather than training them, and has none of the training infrastructure (labeled feedback pipelines, training compute/data management) this would need. Revisit only if/when it becomes genuinely feasible for a local-first tool to do this without turning ruchat into a model-training platform — not scoped into any phase above until that's true.

---

### Comparison-Driven Positioning

| Framework     | Ruchat Positioning                                      |
|---------------|---------------------------------------------------------|
| **LangGraph** | Faster, simpler, truly local alternative for linear + critic workflows |
| **CrewAI**    | More predictable and token-efficient than role-playing teams |
| **AutoGen**   | Avoids conversational chaos; explicit state and approval gates |
| **LangChain** | Lower-level but far more performant and local-first     |

Ruchat will **never** try to become a general-purpose Python-style agent framework.  
Instead, we aim to be the **best local-first, Rust-native, engineering-focused** orchestration layer.

**Success Metric**:  
By v0.4.0, Ruchat should feel like “LangGraph for people who want to stay fully local and actually ship code reliably.”

---

**Current Status (August 2026)**:  
Phase 1 (v0.2.0) is complete. Phase 2 (v0.3.0) is well underway: structured
tool calling, parallel critics, `apply_patch` hardening (diff-size cap,
rollback, and now a scope check against the Architect's declared `FILES:`
plan), the Team/Manager reconciliation, the Scoper role, resource-limited
(`RLIMIT_AS`/`RLIMIT_CPU`) cargo subprocess sandboxing, and automatic
cross-run memory recall at session start (`Orchestrator::recall_prior_memories`)
are all done — recall now works with or without a Librarian configured, since
a memorize-only run writes via the Worker's own `embed_args` regardless.
Also fixed this session: the CONTAINS-on-scalar-metadata client-side
evaluation bug (Chroma has no scalar-substring operator; a plain
`file CONTAINS 'x'` filter always returned zero rows through the server-side
path) is now applied consistently across `query.rs`/`get.rs`/`retrieve.rs`/
`delete.rs`, not just the RAG query path it was first found in. Two other
real, previously-unknown bugs were found and fixed along the way: the
config-file `model_options` merge was a silent no-op (`cli/options.rs`'s
field-allowlist gate was checking against an always-empty default shape),
and — found via `comparisons/*.md` being brought back in sync with the
codebase — the Librarian's RAG retrieval was silently rendering every result
as an empty string in real runs (`OutputArgs` derived `Default` instead of
matching its own documented clap CLI defaults). A `replace_in_file` tool
(search-and-replace as an `apply_patch` alternative) was tried and reverted
the same day — real runs showed no improvement over diff-based edits, so
`apply_patch` remains the sole write tool (see `TODO.md` Done section).
**Phase 2 (v0.3.0) is now fully complete**: automatic Chroma collection
management (`ruchat chroma-init`), multi-collection queries, per-document
summarization, smarter chunking, and debug-mode breakpoint support all
shipped 2026-08-04, on top of everything above. See `TODO.md` for the
live, priority-ranked task list (Phase 3 items are next), and
`comparisons/*.md` for the framework-by-framework detail behind the Phase 3
items above (resumable runs, interactive HITL) — both were identified from
gaps those comparisons made concrete, not from a generic feature wishlist.
The Phase 3 provider-abstraction and chain-of-thought-prompting items, and
the Open Design Question on autonomous goal-setting, above were added after
reviewing a general "roadmap to agentic AI" topic list against ruchat's
actual architecture and constraints — most of that list (memory, RAG,
multi-agent orchestration, planning) ruchat already does; model
fine-tuning/RLHF and cloud deployment/scaling don't fit its current
local-first-by-default, non-training-platform scope and are parked under
Long-Term Vision rather than actively planned. The LLM half of the
provider-abstraction item shipped the same day (Anthropic/Claude as an
opt-in `--chat-provider`, chat-only — see Phase 3 above and `TODO.md` Done
section); the vector-store half (LanceDB/SQLite-vec alternatives to Chroma)
remains open, split into its own item.

Contributions welcome — especially on testing, configuration, and tool framework.
