# Engineering Plugin Customization — ruchat

This file gives the `engineering` plugin (standup, review, debug, architecture,
incident, deploy-checklist, and their underlying skills) the repo-specific facts
it needs instead of generic defaults. Claude reads this automatically when
working in this repository.

## Project

- **ruchat** — a single-maintainer, local-first, Rust CLI for AI chat and
  multi-agent orchestration, built on **Ollama** (LLM inference) and
  **ChromaDB** (vector store / RAG). No cloud dependency by design.
- Maintainer: Roelof J.C. Kluin (`roel.kluin@gmail.com`).
- License: MIT. Version: see `Cargo.toml` (`0.1.2`, pre-1.0 — breaking changes
  are expected and not yet semver-gated).

## Tech Stack

```json
{
  "language": "Rust (edition 2024)",
  "techStack": ["Rust", "Tokio", "Clap", "Ollama (ollama-rs)", "ChromaDB", "Crossterm TUI", "Serde/JSON"],
  "defaultBranch": "master",
  "deployProcess": "manual: cargo build --release, symlinked `ruchat` binary; no CI/CD or packaging pipeline yet",
  "vcs": "git"
}
```

- Runtime dependencies: a local **Ollama** server and (optionally) a local
  **ChromaDB** instance (Docker). Nothing talks to an external cloud API.
- No async web framework — this is a CLI/TUI binary (`clap` + `crossterm`),
  not a service.

## Repository Layout

- `src/cli/` — argument parsing, config merging (`options.rs`), prompt building.
- `src/core/agent/` — the multi-agent orchestration engine (roles, protocol,
  pipeline, templates, tools, tokens).
- `src/core/orchestrator/` — the `Stage` state machine and its side effects
  (`cargo.rs` test/check, `git.rs` auto-commit, `fs.rs`, `scope.rs`, `search.rs`).
- `src/providers/llm/ollama/` and `src/providers/vector/chroma/` — the two
  external integrations, each behind their own module.
- `src/tui/` — interactive chat UI.
- `agent_role/*.md` — the literal prompt templates for each agent role
  (Architect, Worker, Validator, Critic, Librarian, Scoper, Summarizer).
- `agent_debug/*.json` — fixed-sequence fixtures for exercising the stage
  machine without a live Ollama/Chroma server; wired into `cargo test --lib`.
- Docs of record: `README.md` (user-facing), `INSTALL.md`, `ORCHESTRATION.md`
  (architecture deep-dive), `CONTEXT.md` (the shared `Context`/`Turn` data
  model), `ROADMAP.md` (phased plan), `TODO.md` (live task list).

## Architecture (for `/architecture` and `system-design`)

Ruchat's core is a **stage-machine multi-agent loop**, not a generic
graph framework — this is a deliberate positioning choice (see `ROADMAP.md`):
predictable linear flow with explicit approval gates beats LangGraph/CrewAI-style
flexibility for this use case.

```
Scope → Plan → Retrieve → Implement → Test → Validate → Critique → Reconcile → (Retry ↺ Plan | Accept → Commit)
```

- Roles: Scoper (optional, repo-fact gathering), Architect (required, plan-only,
  no tools), Librarian (optional, RAG via Chroma), Worker (required, only role
  with the full typed tool catalog), Tester (`cargo check`/`cargo test`, not an
  LLM), Validator (optional correctness verdict), Critics (optional, run
  concurrently), Summarizer (optional, token-budget triggered compression).
- State lives in one append-only `Context.turns: Vec<Turn>` log — see
  `CONTEXT.md` before proposing a design that reintroduces flat mutable string
  fields (`history`/`context`/`documents`); that pattern was deliberately
  replaced.
- Security posture: **no generic shell-execution tool**. The Worker/Scoper only
  get specific, typed, schema-validated tools (`agent/tools.rs::ToolName`).
  Any proposal to add a general shell/exec tool is a regression against this
  design decision, not a neutral addition.
- `apply_patch` now enforces a scope check against the Architect's plan (added
  2026-08-03, see `TODO.md` Done section): the Architect declares a `FILES:`
  line in its plan, and `Validation::apply_patch` (`agent/protocol.rs`) rejects
  a patch targeting a file not in that list. It fails open (no restriction)
  only when the plan omits the `FILES:` line entirely — not a residual gap,
  a deliberate choice since local models don't reliably follow new prompt
  conventions.
- Full detail: `ORCHESTRATION.md`.

## Code Review Focus (for `/review` and `code-review`)

Prioritize these repo-specific concerns over generic checklist items:

- **Error handling**: prefer `thiserror` + `#[from]` and `anyhow` context over
  `eprintln!`/`println!`/`unwrap`. Flag new `println!`/`eprintln!` in
  library code (`src/core`, `src/providers`) — `tracing` is the standard here.
- **Tool safety invariants** (`src/core/agent/tools.rs`,
  `src/core/orchestrator/fs.rs`): `read_file`/`list_dir` must keep refusing
  paths that canonicalize outside the repo root; `apply_patch` must keep
  requiring the target be tracked by `git ls-files`, stay under
  `MAX_PATCH_DIFF_BYTES`, and (when the plan declared a `FILES:` scope) match
  it. `replace_in_file` (added 2026-08-03 as an easier-to-generate-correctly
  alternative — no diff syntax, just an exact `old_string`/`new_string` pair)
  must keep the same git-tracked/size-cap/`FILES:` scope checks, plus its own:
  `old_string` must match exactly once, never zero or multiple times without
  being refused. Treat any change loosening these as security-relevant.
- **No new generic shell/exec tool** — see architecture note above.
- **Test placement**: each module owns its own `#[cfg(test)] mod tests`
  block next to the code it tests (types are `pub(crate)`, so tests need to
  live inside the crate, not a separate `tests/` black-box suite) — this is
  crate-wide, not confined to `core::orchestrator::tests` specifically. Don't
  ask for integration tests that can't
  compile against private types.
- **`agent_debug/*.json` fixtures**: if a PR changes `Role`, `Stage`, or
  `ToolName`, check whether the fixture JSON (role names like `Critic_0`, not
  `Critic0`) still matches — a naming mismatch here has caused a real,
  previously-shipped bug (multi-critic dispatch silently no-op'd).
- Known pre-existing issues that are *not* new findings if reintroduced
  unchanged: ~16 `cargo clippy --lib` dead-code warnings, and repo-wide
  `cargo fmt --check` drift across ~76 files (no `rustfmt.toml`; default
  formatting, not a style disagreement) — see `TODO.md`.

## Testing Strategy (for `testing-strategy`)

- Run `cargo test --lib` (unit tests only; no `tests/` integration suite by
  design — see above). Run `cargo clippy --lib --tests` for lint.
- `cargo fmt --check` is **not** a clean gate today (pre-existing drift) —
  don't propose it as a CI blocker without first proposing the repo-wide
  `cargo fmt` pass called out in `TODO.md`.
- Stage-machine coverage uses `FakeLlmClient`/`FakeVectorStore`/`FakeEmbeddingsClient`
  (`core/agent/llm_client.rs`) driven by `agent_debug/*.json` fixtures — 9 of
  10 fixtures are wired up; the two `architect_librarian_worker[_validator]`
  combinations are only indirectly covered.
- Three known-failing tests are `#[ignore]`d with reasons, not silently
  skipped: `chroma::metadata::tests::test_get_metadata_valid`,
  `chroma::tests::test_create_table`, `chroma::tests::test_json_output` — see
  `TODO.md` for the specific logic gap in each before "fixing" them by
  changing the assertion instead of the behavior.
- No CI workflow exists yet in this checkout (`.github/workflows/` is absent)
  despite `TODO.md` referencing one — verify locally with the commands above
  before treating a change as "tested."
- **Agentic evals** (`core/agent/evals.rs`, added 2026-08-03): a distinct
  category from everything else here — these drive a role's real prompt
  template against a *live* Ollama server (not `FakeLlmClient`) with a
  specific scenario, checking that the actual model behaves as that role's
  prompt intends. Every one is `#[ignore]`d (not part of `cargo test --lib`);
  run explicitly with `cargo test --lib -- --ignored agent_eval`. Expect some
  flakiness by design — they depend on live model behavior, not deterministic
  code — a red run can be a genuine finding about prompt/model reliability,
  not necessarily a code bug; see the eval's own comment before assuming
  either way.

## Tech Debt Priorities (for `tech-debt`)

Pull the live, maintainer-ranked list from `TODO.md` rather than re-deriving
priorities from scratch — it already separates High/Medium/Low and has a
"Done" section to avoid re-reporting fixed issues. Highlights as of the last
update:
- High: config/CLI merge duplication, `println!`/`eprintln!` → `tracing`
  migration, making the `Stage` sequence data-driven (`ROADMAP.md` Phase 3),
  TUI redraw/selection bugs.
- Medium: parser unit tests (`where.rs`/`include.rs`/`prompt.rs`), duplicated
  `update_from_json` logic, dead legacy `Team`/`Manager` pipeline remnants
  (note: `ORCHESTRATION.md` says this was already reconciled — verify current
  state before re-flagging), the clippy/fmt drift above.
- Low / nice-to-have: config profiles, plugin system for tools, web UI,
  multi-modal support.

## Standup / Activity (for `/standup`)

No project tracker or chat connector — standups are commit-log based. Useful
commands: `git log --oneline -15`, `git log --stat -5`. Recent work trends
toward tool-call parsing robustness, scoper/role fixes, and template-driven
role prompts (see `git log`).

## Deploy Checklist (for `/deploy-checklist`)

There is no deployment target (CLI binary, not a service) and no CI/CD
pipeline yet. Treat "deploy" as "cut a release build":
1. `cargo check` and `cargo test --lib` pass.
2. `cargo clippy --lib --tests` reviewed (pre-existing warnings tolerated;
   new ones aren't).
3. `cargo build --release`; confirm the `ruchat` symlink still resolves
   (`ruchat -> target/release/ruchat`).
4. Manual smoke test against a running Ollama server (`ruchat ask "..."`) and,
   if Chroma-dependent changes are involved, a running ChromaDB container
   (`start_chroma.sh`).
5. No automatic rollback mechanism exists — this is source-controlled only.

## Incident Response / Debug (for `/incident`, `/debug`, `incident-response`)

There is no production service or on-call rotation — "incidents" in this repo
mean a broken build, a regression caught by `cargo test`, or the stage machine
stalling/escalating at runtime. Use `--debug-sequence <file.json>` with an
`agent_debug/*.json` fixture to reproduce a specific role sequence
deterministically instead of a live Ollama/Chroma run. Runtime issues also
write to a per-run file under `ruchat_traces/` (current goal/context/history
snapshot, refreshed every turn) — check that file first when triaging a stuck
or escalated run; each run gets its own `ruchat_trace_<N>.md`, moved into
`ruchat_traces/successes/` or `ruchat_traces/failures/` (with a one-shot LLM
summary of why the run ended that way) once it finishes, so old runs are
never overwritten by new ones.

## Documentation (for `documentation`)

Keep changes consistent with the existing doc split — don't duplicate content
across files:
- `README.md` — user-facing quickstart and CLI examples.
- `INSTALL.md` — install prerequisites/steps only.
- `ORCHESTRATION.md` — architecture/roles/stage machine (the design doc).
- `CONTEXT.md` — the `Context`/`Turn` data model specifically.
- `ROADMAP.md` — phased, dated plan and positioning vs. LangGraph/CrewAI/AutoGen.
- `TODO.md` — the live, checkbox-style task list (including a "Done" section —
  update it rather than leaving completed items unmarked).
