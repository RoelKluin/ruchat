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

- [ ] Consolidate configuration system (`config.toml` + environment variables + CLI overrides)
- [ ] Full structured logging (`tracing`) with configurable levels and JSON output
- [ ] Comprehensive error handling with actionable messages
- [ ] Unit + integration test coverage for all parsers (`where.rs`, `prompt.rs`, `include.rs`) and core orchestration
- [ ] Fix TUI redraw artifacts, improve selection/copy/paste, and add help screen
- [ ] Optimize model option merging (remove double JSON round-trip)
- [ ] Add connection pooling for Ollama and Chroma clients
- [ ] Release v0.2.0 with clean `TODO.md` → `DONE` migration

**Milestone**: Reliable daily driver for local coding agents.

---

### Phase 2: Enhanced Local Agent Capabilities (v0.3.0) — Q3 2026

**Goal**: Make the fixed pipeline significantly more powerful while staying local

- [ ] Structured tool calling framework (replace regex parsing with proper schema + execution)
- [ ] Persistent memory layer (Chroma-based long-term memory for agents)
- [ ] Automatic collection management (`ruchat chroma-init` from `db_config.json`)
- [ ] Parallel critic execution (run security/perf critics concurrently)
- [ ] Improved RAG: relevance scoring, document summarization before Worker, multi-collection queries
- [ ] Built-in code execution sandbox (safe `SHELL` tool with timeout + resource limits)
- [ ] Token-aware history management + automatic summarization triggers
- [ ] Debug mode improvements (step-by-step execution, breakpoint support)

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

**Current Status (March 2026)**:  
Phase 1 is ~60% complete. Focus is now on configuration system and testing before v0.2.0.

Contributions welcome — especially on testing, configuration, and tool framework.
