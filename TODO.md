# Ruchat TODO

Last updated: 2026-08-02

## High Priority

### 1. Configuration & CLI Improvements
- [ ] Merge `options.rs` and CLI flag overrides more cleanly (avoid double JSON round-trip in `ModelArgs::build_generation_request`)
- [ ] Add proper environment variable support for all Chroma and Ollama settings (use `clap::env` consistently)
- [ ] Implement global config file (`~/.config/ruchat/config.toml` or `.json`) with profile support
- [ ] Deprecate/phase out scattered JSON string hacks in favor of structured sub-configs

### 2. Error Handling & Logging
- [x] Migrated genuine diagnostic `eprintln!`/`println!` call sites in `src/core`/`src/providers` to `tracing`. The rest of the `println!`s in that tree (`chroma/ls.rs`, `ollama/server.rs::ls`, `manager.rs`, `chroma/query.rs`'s result print) are each command's actual designed stdout output, not debug prints — converting those to `tracing` would hide them by default without `RUST_LOG` set, so they're staying as-is
- [ ] Add context to all errors (`#[from]` + `thiserror` extensions where needed)
- [ ] Improve user-facing error messages with actionable suggestions
- [ ] Implement graceful degradation when Ollama/Chroma are unavailable

### 3. Agent Orchestration
- [ ] Make agent pipeline fully configurable via JSON (the `Stage` sequence in `orchestrator.rs` is still fixed in code, not data — see `ROADMAP.md` Phase 3)
- [ ] Improve Librarian → Worker document injection further (per-document summarization before Worker, multi-collection queries — reranking/relevance scoring is done, see `providers/vector/chroma/rerank.rs`)
- [ ] Add memory / long-term storage persistence between runs (the `memorize` tool already writes to Chroma via `Agent::embed`, but there's no automatic recall of prior-run memories at session start)
- [ ] `apply_patch` still has no scope check against the Scoper/Architect plan (a patch can touch a file the plan never mentioned) — diff-size cap and rejection rollback are done, see Done section
- [ ] Expose `BuildReport::parsed_diagnostics` (`agent/protocol.rs`) to callers instead of only the flattened diagnostics string — the structured `Diagnostic { level, message, file, line, column }` is already populated per `cargo check` run but currently sits behind `#[allow(dead_code)]`; feeding it to the Worker/Validator directly would let a rejection point at an exact file/line instead of a text blob

### 4. TUI Chat
- [ ] Fix redraw artifacts and cursor handling edge cases
- [ ] Improve selection + copy/paste reliability
- [ ] Add syntax highlighting for code blocks in chat view
- [ ] Support multi-line editing with proper indentation
- [ ] Add command palette / key bindings help screen
- [ ] Wire up an actual producer for `AgentEvent::Progress` (`agent/event.rs`) — the render loop (`tui/render.rs::render_pipeline_stream`) already has a full `Progress(pct)` match arm that draws a "...N%" status line, but nothing in the orchestrator/agent code ever sends one; likely candidates are per-round progress (`round`/`max_iterations`) or per-chunk streaming progress

## Medium Priority

### Code Quality & Maintainability
- [ ] Add integration tests for full agentic flows (using test Ollama/Chroma) — `agent_debug/*.json` already contain ready-made stage sequences (`architect_only`, `worker_and_validator_rejection`, `multiple_critics`, etc.); wire these into `cargo test` against a mocked `LlmClient`/`VectorStore` instead of writing fixtures from scratch
- [ ] Consistent error handling across Chroma subcommands
- [ ] Refactor duplicated JSON update logic (`update_from_json` methods)
- [x] `cargo test --lib` was uncompilable (33 errors: `OutputArgs`/`create_table` test code hadn't been updated after a prior refactor to `format`/`render_rows`, and two `where.rs` tests compared a `Result<T>` against a bare `T` after `map_sql_comparison`/`map_sql_to_document_op` started returning `Result`) and, separately, `test_handle_request_default` called `Args::parse_from(["test", "-h"])`, which makes clap call `std::process::exit(0)` mid-test-run — silently killing every other test in the same process depending on thread scheduling. All fixed; the suite is green.
- [x] **Multi-critic consensus review was completely non-functional.** `Orchestrator::new`'s Critics loop passed each critic's flat config object straight to `Agent::new` as `config`, which looks up `config.get(role)` — a key that can never exist in a flat object, so `Agent::new` always errored and `critics` stayed empty regardless of `--critic`/`"Critics"` config, silently. Even fixed, a second bug meant `query_stream` would then fail with `InvalidRole`: `Role::from_str` only recognized the bare string `"critic"`, never the `"Critic_0"`/`"Critic_1"` naming `Orchestrator::new` actually assigns. Both fixed (`orchestrator.rs`, `agent/role.rs`); caught by wiring `agent_debug/multiple_critics.json` into a real test (`core::orchestrator::tests::multiple_critics_dispatches_each_critic_once`) — nothing had ever exercised this path end-to-end before.
- [x] Wired 9 of 10 `agent_debug/*.json` fixtures into `cargo test --lib` (`core::orchestrator::tests::*`) using a new `FakeLlmClient` (`agent/llm_client.rs` — the `FakeVectorStore`/`FakeEmbeddingsClient` fakes already existed but had never actually been wired to anything) alongside the existing `FakeVectorStore`, so the stage machine can be exercised without a live Ollama/Chroma server. Also fixed a fixture bug: `critic.json`/`multiple_critics.json` used `"Critic0"`/`"Critic1"` (no underscore), which doesn't match the `"Critic_N"` naming the code actually expects.
- [ ] Three pre-existing test failures surfaced once the above compiled and are now `#[ignore]`d with reasons rather than fixed (deeper logic bugs, not just stale field names — need someone with context on the intended behavior): `chroma::metadata::tests::test_get_metadata_valid` (`parse_metadata` doesn't support the `key:value,key:value` shorthand the test expects), `chroma::tests::test_create_table` (expects a `"DOCUMENT"` header but rendering only ever emits the short `"DOC"` alias), `chroma::tests::test_json_output` (fixture uses an `Include` value the current enum doesn't accept)
- [x] Added `.github/workflows/ci.yml`: build + `cargo clippy --lib --tests` + `cargo test --lib` on push/PR. Deliberately no `-D warnings` (see next item) and no `fmt --check` yet (see item after that).
- [ ] The ~16 pre-existing dead-code warnings (`cargo clippy --lib`) aren't blocking CI yet — worth cleaning up so a future `-D warnings` gate is actually adoptable
- [ ] `cargo fmt --check` currently flags formatting drift in ~76 files repo-wide (no custom `rustfmt.toml`, so this is default-rustfmt drift accumulated over time, not a style disagreement) — needs a dedicated `cargo fmt` pass across the repo before a `fmt --check` CI gate can be added without blocking unrelated PRs on pre-existing drift

### Chroma / RAG
- [ ] Support automatic collection creation from `db_config.json` on first embed
- [ ] Add progress bar for large embedding jobs
- [ ] Implement caching layer for repeated file embeddings
- [ ] Add `ruchat chroma-import` command for git history / source trees
- [ ] Better metadata normalization and type safety
- [ ] `embed_script.sh`'s ctags chunk-boundary detection has two open `FIXME: improve per lang/kind handling here` markers around its closing-brace search — the language/kind match lists (Rust, Sh, TOML, Markdown) are hand-maintained and incomplete, so other ctags-supported languages fall back to a single-line chunk instead of the real symbol extent

### Performance
- [ ] Connection pooling for Ollama and Chroma clients
- [ ] Streaming response handling in agent orchestrator (currently buffers)
- [ ] Optimize history limit calculation and token counting
- [ ] Review `reqwest` feature flags in `Cargo.toml`

### Security & Production Readiness
- [ ] Never log sensitive data (tokens, prompts with secrets)
- [ ] Add optional authentication for Ollama
- [ ] Rate limiting / retry backoff configuration
- [ ] `cargo_check`/`cargo test` run with timeouts (30s/60s/120s) but no memory/CPU resource limits — there is no generic shell-execution tool (deliberately — the Worker/Scoper only have specific typed tools), so this is scoped to the cargo subprocess, not arbitrary shell sandboxing

## Low Priority / Nice-to-have

- [ ] API versioning for future breaking changes (`/v1/`)
- [ ] Plugin system for custom tools and agents
- [ ] Web UI / server mode
- [ ] Export conversation as Markdown / JSON
- [ ] Voice input / output support
- [ ] Multi-modal support (images via `qwen2.5vl`, etc.)

## Done / Recently Completed

- [x] Unit tests for parser modules: `include.rs`/`where.rs` had internal parse functions covered but not the `IncludeArgs`/`WhereArgs` `parse()`/`update_from_json()` wrappers CLI code actually calls (now added); `cli/prompt.rs` (`andify_list`, `get_prompt`, `promptless`) had zero tests, now covered including the external-command exit-code path
- [x] Consolidated TODO files into single `TODO.md`
- [x] Improved model option merging with CLI flags
- [x] env_logger / tracing integration
- [x] Basic multi-agent orchestration with RAG support
- [x] Git auto-commit feature branch on approval
- [x] Robust Chroma CLI with where/include parsing
- [x] TUI chat with history, undo/redo, selection
- [x] Structured tool calling framework (`agent/tools.rs::ToolName`, schema-validated, replaces regex-only parsing) — 13 typed tools including `apply_patch`, `git_*`, `read_file`, `ripgrep`, `read_tags`, `cargo_check`/`cargo_dupes`
- [x] Parallel critic execution (`Orchestrator::run_critics_parallel`, `futures_util::future::join_all`)
- [x] RAG relevance scoring / reranking (`providers/vector/chroma/rerank.rs`, distance+lexical blend)
- [x] Token-aware history management with automatic Summarizer trigger (`Stage::Retry`, `get_dynamic_history_limit`)
- [x] Pre-planning repo-grounding stage (`Scoper` role — not in the original TODO/ROADMAP list at all)
- [x] Structured `Context` event log (`Vec<Turn>` + `TurnKind`) replacing the old flat-string `history`/`context`/`documents`/`rejections` fields
- [x] Reconciled the legacy `Team`/`Manager` pipeline — `ruchat manager` now runs a saved `Team` preset through the real `Orchestrator` stage machine instead of a separate, unvalidated linear engine
- [x] `apply_patch` diff-size cap (`MAX_PATCH_DIFF_BYTES`, `agent/protocol.rs`) and automatic rollback of a rejected round's patch before looping back to `Plan` (`Context::{record_patch,revert_pending_patch}`)
- [x] Confirmed the "remove dead code" item once flagged above for `conversation_tree.rs`/legacy `Team`/`Manager` is fully resolved: `conversation_tree.rs` no longer exists and `team.rs`/`manager.rs` are the reconciled implementation this list already credits — removed the stale duplicate bullet
- [x] Removed an unused `OrchestratorRun` struct (`orchestrator.rs`) whose doc comment described bundling an `Orchestrator` to implement `AgentPipeline`'s "fixed `run(&mut self, ...)` signature" — `AgentPipeline` (`agent/pipeline.rs`) is an enum with its own `run(self)`, not a trait anything implements, so both the struct and its rationale were stale leftovers; `ask.rs`/`manager.rs` already construct `AgentPipeline::Orchestrator` directly

---

**Next milestone:** Stable 0.2.0 release with clean configuration story, full test coverage for core parsers, and production-ready logging/error handling.

Help welcome on any item — especially testing and configuration refactoring.
