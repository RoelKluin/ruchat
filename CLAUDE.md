# Engineering Plugin Customization - ruchat

This file gives the `engineering` plugin (standup, review, debug, architecture,
incident, deploy-checklist, and their underlying skills) the repo-specific
facts
it needs instead of generic defaults. Claude reads this automatically when
working in this repository.

## Project

**ruchat** - a single-maintainer, local-first, Rust CLI for AI chat and
multi-agent orchestration, built on **Ollama** (LLM inference) and
**ChromaDB** (vector store / RAG). No cloud dependency by default; **Anthropic
(Claude) is an opt-in chat-provider**
(`--chat-provider anthropic`), chat only: Anthropic has no embeddings API, so
RAG/`memorize`/recall stay Ollama-only unconditionally.

- Maintainer: Roelof J.C. Kluin (`roel.kluin@gmail.com`).
- License: MIT. Version: see `Cargo.toml` (pre-1.0 - breaking changes
  expected).
- Language: Rust (edition 2024); no cloud API by default; manual release build
  (`cargo build --release`).

## Documentation Pointers

**Architecture & Design:**

- `ORCHESTRATION.md` - the stage machine, role roster, Context/Turn data model,
  and orchestrator internals (read this for deep system design).
- `CONTEXT.md` - the shared `Context`/`Turn` append-only log structure
  (critical before proposing any state-shape changes).
- `ROADMAP.md` - phased plan, Phase 2's reliability gate, feature milestones,
  and positioning vs. LangGraph/CrewAI/AutoGen.

**Active Work:**

- `TODO.md` - live task list, prioritized by High/Medium/Low; pull the latest
  from here, not from memory.
- `Done.md` - completed tasks. **Completed tasks must be logged in `Done.md` as
  one-liner commit + critical context** (not full commit messages). This assists
  future development and helps trace potential regressions.
- `agent_debug/*.json` - fixture-driven stage-machine tests; see Testing
  Strategy below.

**Quick Reference:**

- `README.md` - user-facing quickstart and CLI examples.
- `INSTALL.md` - install prerequisites/steps.

## Tech Stack

```json
{
"language": "Rust (edition 2024)",
"techStack": ["Rust", "Tokio", "Clap", "Ollama", "ChromaDB", "Serde/JSON"],
"vcs": "git",
"ci": ".github/workflows/ci.yml (build + clippy + test on push/PR)"
}
```

Runtime: local **Ollama** server + optional local **ChromaDB** (Docker).
`--chat-provider anthropic` is opt-in only, never default.

## Code Review Focus (repo-specific concerns)

Prioritize these over generic checklists:

- **Error handling**: prefer `thiserror` + `#[from]` and `anyhow` context
  over
  `eprintln!`/`println!`/`unwrap`. Flag new `println!`/`eprintln!` in
  library code (`src/core`, `src/providers`) - `tracing` is the standard here.
- **Tool safety invariants** (`src/core/agent/tools.rs`,
  `src/core/orchestrator/fs.rs`): `read_file`/`list_dir` must keep refusing
  paths that canonicalize outside the repo root; `apply_patch` must keep
  requiring the target be tracked by `git ls-files`, stay under
  `MAX_PATCH_DIFF_BYTES`, and (when the plan declared a `FILES:` scope) match
  it. Treat any change loosening these checks as security-relevant.
- **No new generic shell/exec tool** - the security posture is explicit: only
  specific, typed, schema-validated tools for Worker/Scoper.
- **Test placement**: each module owns its own `#[cfg(test)] mod tests`
  block next to the code it tests (types are `pub(crate)`, so tests need to
  live inside the crate, not a separate `tests/` black-box suite).
- **`agent_debug/*.json` fixtures**: if a PR changes `Role`, `Stage`, or
  `ToolName`, verify the fixture JSON (role names like `Critic_0`, not
  `Critic0`) still matches - a naming mismatch has caused a real, shipped bug.
- **Pre-existing clippy baseline** (baseline re-verified 2026-08-04):
  `cargo clippy --lib --tests` reports ~86 warnings total, of which only ~13
  are genuine dead code - the other 63 are all `clippy::result_large_err`,
  tracing to one oversized `RuChatError::ChannelError` variant. Repo-wide
  `cargo fmt --check` is clean as of 2026-08-04 - new PR drift IS a real
  finding now, not pre-existing.

## Testing Strategy

- Run `cargo test --lib` (unit tests only; no `tests/` integration suite by
  design).
- `cargo clippy --lib --tests` for lint; `cargo fmt --check` for formatting.
- Stage-machine coverage uses `FakeLlmClient`/`FakeVectorStore` driven by
  `agent_debug/*.json` fixtures - all 11 fixtures are wired up as of 2026-08-04.
- **Agentic evals** (`core/agent/evals.rs`): marked `#[ignore]`, run
  explicitly
  with `cargo test --lib -- --ignored agent_eval`. These exercise a role's real
  prompt against a live Ollama server (not deterministic); expect flakiness by
  design - a red run can reflect prompt/model reliability, not necessarily a
  code bug.

## Standup / Activity

No project tracker or chat connector. Useful commands: `git log --oneline -15`,
`git log --stat -5`.

## Deploy Checklist

No deployment target (CLI binary, not a service). "Deploy" = release build:

- `cargo check` and `cargo test --lib` pass.
- `cargo clippy --lib --tests` reviewed (pre-existing warnings tolerated; new
  ones aren't).
- `cargo build --release`; confirm the `ruchat` symlink resolves.
- Manual smoke test against running Ollama server (`ruchat ask "..."`) and (if
  Chroma-dependent changes) a running ChromaDB container (`start_chroma.sh`).

## Incident Response / Debug

No production service. "Incidents" = broken build, test regression, or stage
machine stalling.

- Use `--debug-sequence <file.json>` with an `agent_debug/*.json` fixture to
  reproduce deterministically instead of live Ollama/Chroma run.
- Runtime traces live only in memory during a run. On finish, an LLM summary
  (goal, outcome, round-by-round verdict, lessons) is archived directly to
  `ruchat_traces/successes/` or `ruchat_traces/failures/` - no raw trace is ever
  written to disk.
- **Quick pattern search**: `grep -h '^LESSON:'
  ruchat_traces/{successes,failures}/*.md | sort | uniq -c | sort -rn` -
  fastest way to see recurrent failure patterns instead of reading traces one
  by one.

## Delegation policy

- **Boilerplate, trait impls, test scaffolding, first-pass review** -> use the
  rust-local-\* subagents.
- **Long build/test/clippy output** -> route through build-log-summarizer, never
  paste raw.
- **Codebase context** -> query the chromadb MCP tool instead of re-reading
  whole files. Collections: `repo_docs-*` (design docs), `repo_lessons-*`
  (per-run agent-decision reviews), `repo_src-*` (ctags chunks), `repo_hist-*`
  (commit history). `scripts/index_rag.sh` refreshes and runs automatically from
  `.git/hooks/post-commit`.
- **Code you can already name** - ripgrep + targeted read beats RAG (~20k lines
  over 77 files is small).
- **Reach for `repo_lessons-*`** for questions RAG can answer but grep cannot:
  "has this failure mode happened before?"
- **Reserve your own reasoning** for: borrow-checker/lifetime issues,
  architecture, concurrency bugs, anything in the agent-loop core.

**All local-model delegation goes to `ollama-heavy` (:11434, the 3090), one at
a time.** The `ollama-light` instance (:11431, Tesla M10s) runs on CPU
(Ollama's CUDA build drops Maxwell CC 5.0), slower than queuing on the 3090. Do
not dispatch work to it; parallel dispatch of two local-model tasks buys
nothing (they serialize at the server). Parallelism matters only when the
second task is _not_ local inference (shell command, file read, web fetch).
`ollama-heavy` also shares :11434 with ruchat itself, so turn delegation off
while measuring a live run. For GPU/instance detail and measurements, read
`references/gpu-and-ollama.md` in the ruchat-dev skill.
