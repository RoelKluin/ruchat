# Ruchat TODO

Last updated: 2026-03-24

## High Priority

### 1. Configuration & CLI Improvements
- [ ] Merge `options.rs` and CLI flag overrides more cleanly (avoid double JSON round-trip in `ModelArgs::build_generation_request`)
- [ ] Add proper environment variable support for all Chroma and Ollama settings (use `clap::env` consistently)
- [ ] Implement global config file (`~/.config/ruchat/config.toml` or `.json`) with profile support
- [ ] Deprecate/phase out scattered JSON string hacks in favor of structured sub-configs

### 2. Error Handling & Logging
- [ ] Replace remaining `eprintln!` and `println!` with structured `tracing` events
- [ ] Add context to all errors (`#[from]` + `thiserror` extensions where needed)
- [ ] Improve user-facing error messages with actionable suggestions
- [ ] Implement graceful degradation when Ollama/Chroma are unavailable

### 3. Agent Orchestration
- [ ] Make agent pipeline fully configurable via JSON (roles, order, dependencies, data flow)
- [ ] Add parallel execution support for independent critics
- [ ] Improve Librarian → Worker document injection (limit tokens, relevance scoring, summarization)
- [ ] Add memory / long-term storage persistence between runs
- [ ] Implement tool calling framework inside orchestrator (currently limited to MEMORIZE/SHELL)

### 4. TUI Chat
- [ ] Fix redraw artifacts and cursor handling edge cases
- [ ] Improve selection + copy/paste reliability
- [ ] Add syntax highlighting for code blocks in chat view
- [ ] Support multi-line editing with proper indentation
- [ ] Add command palette / key bindings help screen

## Medium Priority

### Code Quality & Maintainability
- [ ] Write unit tests for all parser modules (`where.rs`, `include.rs`, `prompt.rs`)
- [ ] Add integration tests for full agentic flows (using test Ollama/Chroma)
- [ ] Consistent error handling across Chroma subcommands
- [ ] Refactor duplicated JSON update logic (`update_from_json` methods)
- [ ] Remove dead code and old conversation_tree.rs duplicate

### Chroma / RAG
- [ ] Support automatic collection creation from `db_config.json` on first embed
- [ ] Add progress bar for large embedding jobs
- [ ] Implement caching layer for repeated file embeddings
- [ ] Add `ruchat chroma-import` command for git history / source trees
- [ ] Better metadata normalization and type safety

### Performance
- [ ] Connection pooling for Ollama and Chroma clients
- [ ] Streaming response handling in agent orchestrator (currently buffers)
- [ ] Optimize history limit calculation and token counting
- [ ] Review `reqwest` feature flags in `Cargo.toml`

### Security & Production Readiness
- [ ] Never log sensitive data (tokens, prompts with secrets)
- [ ] Add optional authentication for Ollama
- [ ] Rate limiting / retry backoff configuration
- [ ] Sandboxed tool execution (shell commands)

## Low Priority / Nice-to-have

- [ ] API versioning for future breaking changes (`/v1/`)
- [ ] Plugin system for custom tools and agents
- [ ] Web UI / server mode
- [ ] Export conversation as Markdown / JSON
- [ ] Voice input / output support
- [ ] Multi-modal support (images via `qwen2.5vl`, etc.)

## Done / Recently Completed

- [x] Consolidated TODO files into single `TODO.md`
- [x] Improved model option merging with CLI flags
- [x] env_logger / tracing integration
- [x] Basic multi-agent orchestration with RAG support
- [x] Git auto-commit feature branch on approval
- [x] Robust Chroma CLI with where/include parsing
- [x] TUI chat with history, undo/redo, selection

---

**Next milestone:** Stable 0.2.0 release with clean configuration story, full test coverage for core parsers, and production-ready logging/error handling.

Help welcome on any item — especially testing and configuration refactoring.
