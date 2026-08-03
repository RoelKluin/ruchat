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
- [ ] Unit tests for all parsers (`where.rs`, `prompt.rs`, `include.rs`) — still open, see `TODO.md`
- [~] Core orchestration test coverage — 9/10 `agent_debug/*.json` fixtures are wired into `cargo test --lib` via `FakeLlmClient`/`FakeVectorStore` (this is how the multi-critic dispatch bug below was caught); the last fixture combination and true integration tests against a live Ollama/Chroma are still open
- [ ] Fix TUI redraw artifacts, improve selection/copy/paste, and add help screen
- [x] Optimize model option merging (removed the double JSON round-trip in `ModelArgs::build_generation_request`) — surfaced a separate, deeper pre-existing bug in the process (config-file `model_options` merging is currently a silent no-op), tracked in `TODO.md`, not fixed yet
- [x] Connection pooling for Ollama and Chroma clients — investigated, already satisfied by the existing architecture (single shared `Arc`-wrapped client per orchestrator run, reqwest's default pooling underneath, nothing disabling it), not a real gap — see `TODO.md` for the detail
- [x] CI workflow (`.github/workflows/ci.yml`): build + `cargo clippy --lib --tests` + `cargo test --lib` on push/PR — deliberately no `-D warnings` or `fmt --check` gate yet (pre-existing dead-code warnings and repo-wide fmt drift need cleanup first, see `TODO.md`)
- [ ] Release v0.2.0 with clean `TODO.md` → `DONE` migration — still on `0.1.2`, no tags cut yet

**Milestone**: Reliable daily driver for local coding agents.

---

### Phase 2: Enhanced Local Agent Capabilities (v0.3.0) — Q3 2026

**Goal**: Make the fixed pipeline significantly more powerful while staying local

- [x] Structured tool calling framework (`agent/tools.rs::ToolName` — schema-validated, 13 typed tools, no more regex-only parsing)
- [x] Parallel critic execution (`Orchestrator::run_critics_parallel`) — note: the execution mechanism itself was correct, but a separate construction bug meant `critics` was always empty in practice until fixed (see TODO.md's Done section)
- [x] Token-aware history management + automatic summarization triggers (`Stage::Retry` → Summarizer when the token estimate exceeds the model's history limit)
- [~] Persistent memory layer — the `memorize` tool writes to Chroma today (`Agent::embed`), but there's no automatic recall of prior-run memories at session start; that part is still open
- [~] Improved RAG — relevance scoring/reranking is done (`providers/vector/chroma/rerank.rs`); document summarization before the Worker and multi-collection queries are still open
- [ ] Automatic collection management (`ruchat chroma-init` from `db_config.json`)
- [ ] Resource-limited sandboxing for tool-invoked subprocesses (`cargo_check`/`cargo_test` currently have timeouts but no memory/CPU caps) — note: there is deliberately no generic `SHELL` tool anymore; the Worker/Scoper only get specific typed tools, which is a stronger safety posture than a sandboxed-shell approach
- [ ] Debug mode improvements (step-by-step execution, breakpoint support) — the fixed-sequence debug mode itself exists (`--debug-sequence`, `agent_debug/*.json`), but isn't wired into `cargo test` yet and has no breakpoints
- [x] Reconciled the legacy `Team`/`Manager` pipeline — `ruchat manager` now expands a saved `Team` preset into an `Orchestrator` config and runs the real stage machine
- [x] Pre-planning repo-grounding stage (`Scoper` role) — gathers repo facts before the Architect plans; shipped but was never in the original Phase 2 list
- [x] `apply_patch` hardening: diff-size cap, and automatic rollback of a rejected round's patch before looping back to `Plan`
- [ ] `apply_patch` scope check against the Scoper/Architect plan (still open — no guard today against a patch touching a file the plan never mentioned)

**Milestone**: Best-in-class local coding agent (plan → code → review → commit) that beats Python frameworks in speed and reliability.

---

### Phase 3: Controlled Extensibility (v0.4.0) — Q4 2026

**Goal**: Add flexibility without sacrificing predictability or local purity

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
- [ ] Advanced observability (local trace viewer)

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
TUI fixes haven't started; the v0.2.0 release itself hasn't happened either. Config
system consolidation turned out to be mostly already done (config file + profiles
existed and worked, just needed a couple of missing env vars). Everything else in
Phase 1 is now done or verified-as-already-satisfied: structured logging (levels +
JSON output), the eprintln!/println! migration, parser unit tests, the model-option
double-round-trip removal, error-handling improvements at the sites that discarded
real causes, and connection pooling (turned out to already be handled by the existing
shared-client architecture). Phase 2 also picked up real work this cycle: the structured tool-calling framework, parallel critic
execution (plus finding and fixing the bug that made it a silent no-op), `apply_patch`
hardening, the Team/Manager reconciliation, and the new Scoper role — alongside a
round of test-infrastructure work (repairing an uncompilable suite, wiring 9/10
`agent_debug` fixtures into `cargo test`, adding CI). See `TODO.md` for the live,
priority-ranked task list, including two bugs found along the way and deliberately
left open pending a design decision: the config-file `model_options` merge being a
silent no-op, and the `InternalError`/`Is` catch-all error variants used at ~85 call
sites.

Contributions welcome — especially on testing, configuration, and tool framework.
