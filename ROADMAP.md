# Ruchat Roadmap

**Vision**  
Ruchat remains the **fastest, lightest, fully-local** AI agent orchestration tool built for software engineering workflows.  
It stays 100% Rust-native, zero Python dependencies, and runs entirely offline with **Ollama + Chroma** (or future local vector DBs).  
We prioritize **predictability**, **performance**, **token efficiency**, and **tight integration** with local tools (Git, file system, terminal) over general-purpose flexibility offered by LangChain, LangGraph, AutoGen, or CrewAI.

**Core Differentiators to Preserve**
- Fully local-first (Ollama + Chroma mandatory, no cloud)
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
- [ ] **`cargo_clippy` typed tool** — identified 2026-08-03 while writing `scripts/refactoring_examples_todo.sh`: the Worker/Scoper can request `cargo_check`/`cargo_dupes` but have no read-only tool to see clippy's own warnings/suggestions, so any task phrased as "fix a clippy lint" has to guess at what clippy would flag instead of seeing it directly. Straightforward, low-risk addition — same shape as `cargo_check` (`orchestrator/cargo.rs`/`agent/protocol.rs`), just `cargo clippy --message-format=json` parsed the same way `run_build_and_test` already parses `cargo check`'s JSON output (`parse_cargo_json_diagnostics`), reused rather than duplicated.
- [ ] **Multi-file patches per round** — identified 2026-08-03 alongside the above: `Stage::Implement` gives the Worker exactly one write-tool call per round (`execute_and_verify` handles a single `ApplyPatch`, and the read-tool-then-reask loop only permits *read-only* tools, not a second patch), and `apply_patch` itself applies one `diffy::Patch` to one file. So even though the Architect's plan can already *name* multiple files (`FILES:` line, see the scope-check item below), the Worker can only ever act on one of them per round today — a task like "rename this function and update its call sites in two other files" isn't achievable in a single accepted commit yet, only by accident across multiple separate runs. Needs a design decision (allow N sequential `apply_patch` calls per round up to some cap, vs. a multi-file diff format) before attempting.
- [x] Parallel critic execution (`Orchestrator::run_critics_parallel`) — note: the execution mechanism itself was correct, but a separate construction bug meant `critics` was always empty in practice until fixed (see TODO.md's Done section)
- [x] Token-aware history management + automatic summarization triggers (`Stage::Retry` → Summarizer when the token estimate exceeds the model's history limit)
- [~] Persistent memory layer — the `memorize` tool writes to Chroma (`Agent::embed`), and `Orchestrator::recall_prior_memories` now auto-recalls at session start using the goal text as a deterministic query, but only when a Librarian is configured (reuses its Chroma client) — a memorize-only, Librarian-less run still can't recall; see `TODO.md` Done section
- [~] Improved RAG — relevance scoring/reranking is done (`providers/vector/chroma/rerank.rs`); document summarization before the Worker and multi-collection queries are still open
- [ ] Automatic collection management (`ruchat chroma-init` from `db_config.json`)
- [x] Resource-limited sandboxing for tool-invoked subprocesses — every cargo subprocess (`cargo_check`/`cargo_dupes`/the Tester's check+test) now gets `RLIMIT_AS`/`RLIMIT_CPU` via `orchestrator::cargo::limit_resources`, alongside the pre-existing wall-clock timeouts; see `TODO.md` Done section
- [ ] Debug mode improvements (step-by-step execution, breakpoint support) — the fixed-sequence debug mode itself exists (`--debug-sequence`, `agent_debug/*.json`), but isn't wired into `cargo test` yet and has no breakpoints
- [x] Reconciled the legacy `Team`/`Manager` pipeline — `ruchat manager` now expands a saved `Team` preset into an `Orchestrator` config and runs the real stage machine
- [x] Pre-planning repo-grounding stage (`Scoper` role) — gathers repo facts before the Architect plans; shipped but was never in the original Phase 2 list
- [x] `apply_patch` hardening: diff-size cap, and automatic rollback of a rejected round's patch before looping back to `Plan`
- [x] `apply_patch` scope check against the Architect's plan — Architect prompt now declares a `FILES:` line, enforced (fail-open only when the line is absent) in `Validation::apply_patch`; see `TODO.md` Done section for detail

**Milestone**: Best-in-class local coding agent (plan → code → review → commit) that beats Python frameworks in speed and reliability.

---

### Phase 3: Controlled Extensibility (v0.4.0) — Q4 2026

**Goal**: Add flexibility without sacrificing predictability or local purity

- [ ] **Resumable/crash-resilient runs** (checkpointed `Context`) — identified via `comparisons/*.md`: every framework compared against (LangGraph explicitly, AutoGen/CrewAI more loosely) offers some form of durable/checkpointed execution, while a killed or crashed ruchat process currently loses all progress and restarts from `Stage::Scope`. Scope this as a lightweight, local-first mechanism true to ruchat's philosophy — not a distributed system: persist `Context` (turns, round, pending patch) to a local file after each stage transition, and add a `--resume` flag that reloads it and continues from the last completed stage instead of a Temporal/LangGraph-style durable-execution engine.
- [ ] **Interactive human-in-the-loop approval gate** — identified via `comparisons/*.md`: AutoGen's UserProxy agents and LangGraph's interrupts both give a human an explicit mid-run pause/approve point; ruchat's only approval mechanism today is automated Critics (an LLM-driven gate) plus post-hoc review of the committed branch. Add an optional pause (e.g. before `Stage::Commit`) that prints the pending plan/diff and waits for an explicit terminal y/n before proceeding — keeps ruchat's "predictable, auditable" ethos while closing a real, comparison-driven gap rather than adding open-ended interactivity.
- [ ] Configurable agent graph (simple DAG definition in JSON/TOML — limited cycles)
- [ ] Subgraph / reusable agent modules (e.g., "CodeReviewTeam", "ResearchTeam")
- [ ] Dynamic conditional edges based on approval signals or output patterns
- [ ] Plugin system for custom local tools (Rust crates or WASM)
- [ ] Multiple process types: `sequential`, `hierarchical` (lightweight manager), `parallel`
- [ ] Local vector DB abstraction (Chroma primary, support for LanceDB or SQLite-vec as alternatives)

**Important Constraint**: All new features must remain fully local and offline-capable.

**Milestone**: Ruchat becomes a serious lightweight alternative to LangGraph/CrewAI for local use cases.

---

### Phase 4: Performance & Scale (v0.5.0+) — 2027

- [ ] Async parallel agent execution where safe
- [ ] Model context window auto-management and smart chunking
- [ ] Built-in benchmarking suite vs LangGraph/CrewAI on local hardware
- [ ] Optional distributed mode (multiple local machines via simple message bus — still offline-first)
- [ ] Advanced observability (local trace viewer) — `comparisons/*.md` repeatedly call out LangSmith/AutoGen Studio-style inspection as a strength ruchat lacks; today's `.ruchat_trace.md` + colored terminal events are a snapshot/stream, not a navigable history. Scope as a local, offline viewer over the existing `Context.turns` log (e.g. a `ruchat trace` subcommand rendering round-by-round turns/rejections), not a hosted service — stays consistent with the "no cloud dependency" constraint above.

---

### Long-Term Vision (2027+)

- Become the de-facto standard for **local software engineering agents**
- Maintain strict “fully local + predictable” philosophy
- Explore safe WASM-based tool sandboxing
- Support additional local vector/search backends without breaking core simplicity
- Provide migration path / interoperability layer for users coming from Python frameworks (export/import graphs)

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
cross-run memory recall at session start (`Orchestrator::recall_prior_memories`,
gated on a configured Librarian) are all done. Two real, previously-unknown
bugs were also found and fixed along the way: the config-file `model_options`
merge was a silent no-op (`cli/options.rs`'s field-allowlist gate was checking
against an always-empty default shape), and — found via `comparisons/*.md`
being brought back in sync with the codebase — the Librarian's RAG retrieval
was silently rendering every result as an empty string in real runs
(`OutputArgs` derived `Default` instead of matching its own documented clap
CLI defaults). Still open in Phase 2: further RAG improvements (per-document
summarization, multi-collection queries), automatic Chroma collection
management, and extending memory recall to work without a Librarian
configured. See `TODO.md` for the live, priority-ranked task list, and
`comparisons/*.md` for the framework-by-framework detail behind the Phase 3
items above (resumable runs, interactive HITL) — both were identified from
gaps those comparisons made concrete, not from a generic feature wishlist.

Contributions welcome — especially on testing, configuration, and tool framework.
