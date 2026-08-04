# Engineering Plugin Customization — ruchat

This file gives the `engineering` plugin (standup, review, debug, architecture,
incident, deploy-checklist, and their underlying skills) the repo-specific facts
it needs instead of generic defaults. Claude reads this automatically when
working in this repository.

## Project

- **ruchat** — a single-maintainer, local-first, Rust CLI for AI chat and
  multi-agent orchestration, built on **Ollama** (LLM inference) and
  **ChromaDB** (vector store / RAG). No cloud dependency by default — as of
  2026-08-04, **Anthropic (Claude) is an opt-in cloud chat-provider**
  (`--chat-provider anthropic`, `providers/llm/anthropic/`), chat only:
  Anthropic has no embeddings API, so RAG/`memorize`/recall stay Ollama-only
  unconditionally. See `ROADMAP.md` Phase 3 and `TODO.md` Done section.
- Maintainer: Roelof J.C. Kluin (`roel.kluin@gmail.com`).
- License: MIT. Version: see `Cargo.toml` (`0.1.2`, pre-1.0 — breaking changes
  are expected and not yet semver-gated).
- See Delegation policy at bottom — always check before writing boilerplate or running builds

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
  **ChromaDB** instance (Docker). By default nothing talks to an external
  cloud API; `--chat-provider anthropic` is the one opt-in exception (chat
  calls only — `api.anthropic.com`), never enabled unless explicitly asked
  for.
- No async web framework — this is a CLI/TUI binary (`clap` + `crossterm`),
  not a service.

## Repository Layout

- `src/cli/` — argument parsing, config merging (`options.rs`), prompt building.
- `src/core/agent/` — the multi-agent orchestration engine (roles, protocol,
  pipeline, templates, tools, tokens).
- `src/core/orchestrator/` — the `Stage` state machine and its side effects
  (`cargo.rs` test/check, `git.rs` auto-commit, `fs.rs`, `scope.rs`, `search.rs`).
- `src/providers/llm/ollama/`, `src/providers/llm/anthropic/` (opt-in cloud
  chat provider), and `src/providers/vector/chroma/` — the external
  integrations, each behind their own module and the shared `LlmClient`/
  `VectorStore` traits (`core/agent/llm_client.rs`).
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
  it. (A `replace_in_file` alternative to `apply_patch` was tried and reverted
  2026-08-03 — no real-run improvement over diff-based edits — so
  `apply_patch` remains the sole write tool; see `TODO.md` Done section.)
  Treat any change loosening these checks as security-relevant.
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
  unchanged (baseline re-verified 2026-08-04, corrected from a stale earlier
  number): `cargo clippy --lib --tests` reports **86** warnings total, of
  which only **~13** are genuine dead code — the other 63 are all
  `clippy::result_large_err`, tracing to one oversized `RuChatError::
  ChannelError` variant that trips the lint on every fallible function crate-
  wide (pre-existing, not new-code drift; see `TODO.md`). Repo-wide `cargo
  fmt --check` drift, previously ~76/193 files, was fully cleared 2026-08-04
  via a dedicated `cargo fmt` pass (see `TODO.md` Done section) — `cargo fmt
  --check` is a clean gate again; a new PR reintroducing drift IS a real
  finding now, not something to wave off as "pre-existing."

## Testing Strategy (for `testing-strategy`)

- Run `cargo test --lib` (unit tests only; no `tests/` integration suite by
  design — see above). Run `cargo clippy --lib --tests` for lint.
- `cargo fmt --check` is a clean gate as of 2026-08-04 (see above) — safe to
  propose adding it to CI now, though the CI branch-name bug (`main` vs.
  `master`, see `TODO.md`'s Security & CI section) should be fixed first or
  it won't actually run where it matters.
- Stage-machine coverage uses `FakeLlmClient`/`FakeVectorStore`/`FakeEmbeddingsClient`
  (`core/agent/llm_client.rs`) driven by `agent_debug/*.json` fixtures — all
  11 fixtures are wired up as of 2026-08-04 (the previously-gap
  `architect_librarian_worker[_validator]` combinations were closed).
- The three previously-`#[ignore]`d tests (`chroma::metadata::tests::
  test_get_metadata_valid`, `chroma::tests::test_create_table`,
  `chroma::tests::test_json_output`) were fixed 2026-08-04 — each was the
  test's own expectation being wrong, not a real logic bug (see `TODO.md`
  Done section). No more known-failing tests in `cargo test --lib`.
- A CI workflow does exist (`.github/workflows/ci.yml`: build + `cargo
  clippy --lib --tests` + `cargo test --lib` on push/PR, deliberately no
  `-D warnings`/`fmt --check` gate yet), but its `push` trigger targets
  branch `main` while this repo's actual default branch is `master` — a
  real, unfixed bug found 2026-08-04 (see `TODO.md`'s Security & CI
  section) meaning direct pushes to `master` currently get zero CI signal,
  only PRs do. Verify locally with the commands above regardless until
  that's fixed.
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

## Delegation policy

- Boilerplate, trait impls, test scaffolding, first-pass review → use the rust-local-* subagents.
- Long build/test/clippy output → route through build-log-summarizer, never paste raw.
- Codebase context → query the chromadb MCP tool for relevant snippets instead of re-reading whole files.
- Reserve your own (Sonnet) reasoning for: borrow-checker/lifetime issues, architecture, concurrency bugs, anything in the agent-loop core.
- Escalate to Opus only after Sonnet has made a real attempt and hit a wall — not as a first resort.

## Parallel dispatch
The Tesla-backed light model (ollama-light) is slow per-task but frees the 3090
for heavy work. When a task involves both a substantial coding change AND
independent auxiliary work (build log summarization, test scaffolding, docstrings,
doc updates), dispatch them as separate Task calls in the same turn — do not wait
for the heavy task to finish before starting the light one. Only sequence them if
the light task depends on the heavy task's output.

**Caveat on that premise (2026-08-05, unresolved):** "frees the 3090" assumes the
Tesla instance actually runs on the Teslas. Observed otherwise — `ollama ps` on
:11431 reported `qwen2.5:7b` at **100% CPU** despite that instance being correctly
pinned to the Tesla GPUs. Tesla M10 is Maxwell (compute capability 5.0) and recent
Ollama CUDA builds have been dropping the older architectures, so the cards may be
driver-visible but unusable by Ollama. If so, ollama-light is plain CPU inference —
slower than just queueing on the 3090, and the split above is a net loss. Check
`journalctl -u ollama | grep -i "compute capability\|cuda"` before relying on it.

## GPU / Ollama instance map

Host-specific, recorded 2026-08-05 so it doesn't get re-derived every session.

| GPU | Device | Notes |
|-----|--------|-------|
| 0 | RTX 3090 Ti, 24 GB | the fast one; UUID `GPU-5fe0e911-80e4-ca27-25af-8002a47f5a67` |
| 1–4 | Tesla M10, 8 GB each | Maxwell CC 5.0 — see the caveat above |

| Port | Used by | Pinned to |
|------|---------|-----------|
| 11434 | ruchat's default, and the `ollama-heavy` MCP server (`qwen2.5-coder:32b`) | should be GPU 0 |
| 11431 | the `ollama-light` MCP server (`qwen2.5:7b`) | `CUDA_VISIBLE_DEVICES=1,2,3,4` |

ruchat has **no** GPU-selection option and cannot have a meaningful one: Ollama's
HTTP API has no parameter for choosing a device (`num_gpu` sets layer offload
count, not which card), so placement is decided entirely by the Ollama server at
model-load time. Selecting a GPU therefore means selecting an instance — use
`ruchat -s/--server` (`OLLAMA_SERVER`). Don't add a `--gpu` flag; it would
silently do nothing.

Pin on the **server** by UUID, not index: CUDA enumerates `FASTEST_FIRST` by
default, so `CUDA_VISIBLE_DEVICES=0` is not guaranteed to be the card `nvidia-smi`
calls 0 (or set `CUDA_DEVICE_ORDER=PCI_BUS_ID` as well).

**Known-broken as of 2026-08-05:** the :11434 instance was running with a literal
unsubstituted template placeholder — `CUDA_VISIBLE_DEVICES=N` and
`OLLAMA_HOST=0.0.0.0:1143N` (Ollama couldn't parse the port and silently fell back
to 11434). A malformed device allowlist is the likely cause of runs landing on a
Tesla unpredictably. Verify it's fixed before trusting any timing measurement,
including the reliability gate in `TODO.md` section 0.
