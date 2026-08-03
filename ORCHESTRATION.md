**Agent Orchestration in Ruchat**

Ruchat's agent orchestration is a **stage-machine multi-agent loop** built around
the `Orchestrator` (`core/orchestrator.rs`). It turns a single user goal into a
structured, auditable workflow using specialized LLM agents that collaborate
until the output is approved, rejected past the iteration budget, or escalated.

> **Note on `ruchat manager` / `Team`**: `ruchat manager` (`core/agent/manager.rs`)
> runs a named, persisted `Team` (`core/agent/team.rs`) — a saved
> `{name, goal, config}` preset where `config` is the exact same JSON shape as
> `--agentic`. `ManagerCommands::Run` builds a real `Orchestrator` from it and
> drives it through the same stage machine described below; it is not a
> separate execution engine anymore.

### Core Components

- **Orchestrator** (`core/orchestrator.rs`)
  - Holds the agent roster and drives the `Stage` state machine.
  - Manages a shared `Context` (goal, structured `turns` log, round counter — see `CONTEXT.md`).
  - Streams every token + trace/status event to the caller via `mpsc` (`run_task_stream`).
- **Agent** (`core/agent.rs`)
  - Thin wrapper around a model + per-role config (`agent_config`).
  - `query_stream()` builds role-specific chat messages (system + user, sent as
    separate `ChatMessage`s) and streams the response into `ctx.output`.
  - `execute_and_verify()` dispatches `ApplyPatch`/`Memorize` tool calls found in
    the just-streamed output.
- **Role** (`core/agent/role.rs`)
  - Defines the system framing + task text and builds the full prompt per role.
  - Injects the tool catalog (Worker, Scoper) or the goal/plan/history views (Architect).
- **Context / Turn** (`core/agent/types.rs`)
  - Append-only structured event log shared by all agents. See `CONTEXT.md` for the full breakdown.

### Agent Roles (roster)

| Role              | Required | Purpose                                                    | Approval Signal | Notes |
|-------------------|----------|-------------------------------------------------------------|------------------|-------|
| **Scoper**        | Optional | Decides if enough repo-specific detail is known to plan; if not, requests read-only lookups | verdict `READY` | Runs before Architect; can rewrite `ctx.goal` via `clarified_goal`; capped by `scope_iterations` (default 7) |
| **Architect**     | Yes      | Produces a concrete plan                                    | —                | No tool access — plan-only, text output |
| **Librarian**     | Optional | Formulates a Chroma query + retrieves docs                  | —                | RAG only; runs once, round 1; uses `db_config.json` |
| **Worker**        | Yes      | Implements the plan                                         | —                | Only role with the full tool catalog; can call one read-only tool before implementing |
| **Tester**        | Always runs | `cargo check` + `cargo test` against the Worker's applied patch | pass/fail | Not an LLM agent — a build step (`Validation::run_build_and_test`) |
| **Validator**     | Optional | Technical correctness verdict on the Worker's output         | `VALIDATED`      | Rejects with structured `{"verdict":"REJECTED","reason":...}` |
| **Critic(s)**     | Optional, multiple | Domain-specific review (security, performance, ...)   | configurable (`approval_signal`, default `APPROVED`) | Run **concurrently** via `futures_util::future::join_all`, each against a private `Context` snapshot |
| **Summarizer**    | Optional | Compresses `turns` when the estimated token count exceeds the model's history limit | —      | Triggered inside `Stage::Retry`, only when about to loop back to `Plan` |

### The `Stage` State Machine

`run_stage_machine` (`core/orchestrator.rs`) drives an explicit `Stage` enum —
not an implicit `for round in 1..=iterations` loop. Every transition is a match
arm that computes the *next* `Stage`:

```text
Recall    → Before Scope begins: if a Librarian (and its Chroma client) is
            configured, `Orchestrator::recall_prior_memories` runs a
            deterministic query (goal text, `n_results: 3` — no LLM call,
            unlike the Librarian's own on-demand query below) against
            whatever the `memorize` tool has written in past runs, pushed as
            a `TurnKind::Retrieval` turn tagged `"Memory"`. Failure (e.g. no
            memories exist yet) is traced and swallowed, never fails the run.
  ↓
Scope     → Scoper requests lookups (read_file/ripgrep/list_dir/git_*/read_tags/retrieve)
            via a JSON verdict; loops on itself until READY, scope_iterations
            exhausted, or output repeats (stall → forced progression to Plan).
  ↓
Plan      → Architect writes a plan. Round counter increments here. Identical
            output vs. the previous round → Stage::Escalate (stall guard).
  ↓
Retrieve  → Librarian runs (round 1 only, if configured).
  ↓
Implement → Worker responds. If it emitted a *read-only* tool call
            (Retrieve/Git*/ReadFile/ListDir/Ripgrep/ReadTags/CargoCheck/CargoDupes)
            and the per-run retrieve budget (default 2) allows it, the
            orchestrator executes it, appends the result, and re-asks the
            Worker once more in the same stage. `apply_patch`/`memorize` calls
            are executed afterward by `execute_and_verify`.
  ↓
Test      → cargo check + cargo test (60s / 120s timeouts) against the applied patch.
            Failure → Stage::Retry.
  ↓
Validate  → Validator verdict, if configured. REJECTED or unparseable → Stage::Retry.
  ↓
Critique  → All Critics run concurrently against a snapshot of the current
            output/plan/implementation. Each dissent becomes a Rejection turn.
  ↓
Reconcile → Any Rejection turns this round (after dedup)? → Retry : Accept
  ↓
Retry     → If the iteration budget is exhausted: surface the best-known
            implementation without committing (if one exists) or Escalate (if
            none does). Otherwise, maybe run the Summarizer, then back to Plan.
  ↓
Accept → Commit → `git checkout -b ai/feature-<timestamp>`, commit, return to
            the original branch → Done
```

`Escalate(reason)` and `Done` are terminal — the loop breaks and the reason (if
any) is traced to `.ruchat_trace.md` and the event stream.

### Tool Catalog (Worker + Scoper)

The canonical list lives in `agent/tools.rs::ToolName` — this table is generated
from `prompt_tool_catalog()`, the exact function that builds the Worker's prompt,
so it can't drift from what the model is actually told:

| Tool | Schema | Required fields |
|------|--------|------------------|
| `memorize` | `{"tool":"memorize","content":"<string>"}` | `content` |
| `apply_patch` | `{"tool":"apply_patch","diff":"<unified diff string>"}` | `diff` |
| `retrieve` | `{"tool":"retrieve","query":"<string>"}` | `query` |
| `git_log` | `{"tool":"git_log","path":"<string\|omit>","max_count":<int\|omit>}` | — |
| `git_blame` | `{"tool":"git_blame","path":"<string>"}` | `path` |
| `git_diff` | `{"tool":"git_diff","path":"<string\|omit>","staged":<bool\|omit>}` | — |
| `git_search_history` | `{"tool":"git_search_history","pattern":"<string>","mode":"message"\|"content","path":"<string\|omit>","max_count":<int\|omit>}` | `pattern`, `mode` |
| `read_file` | `{"tool":"read_file","path":"<string>","start":<int\|omit>,"end":<int\|omit>}` | `path` |
| `list_dir` | `{"tool":"list_dir","path":"<string>"}` | `path` |
| `ripgrep` | `{"tool":"ripgrep","pattern":"<string>","path":"<string\|omit>","glob":"<string\|omit>","max_count":<int\|omit>}` | `pattern` |
| `read_tags` | `{"tool":"read_tags","symbol":"<string\|omit>"}` | — |
| `cargo_check` | `{"tool":"cargo_check"}` | — |
| `cargo_dupes` | `{"tool":"cargo_dupes"}` | — |

Notes:
- `apply_patch` is gated: the target file must already be tracked by git
  (`git ls-files`), checked in `Validation::apply_patch` before the diff is applied.
- `apply_patch` also checks scope: if the Architect's plan ended with a
  `FILES: path1, path2` line (`agent_role/architect.md`), the diff's target
  must match one of those paths (`Context::planned_files`, `file_in_scope` in
  `agent/protocol.rs`) or the patch is refused. A plan without a `FILES:` line
  isn't restricted — this fails open by design since local models don't
  reliably emit new prompt conventions.
- `apply_patch` refuses diffs over `MAX_PATCH_DIFF_BYTES` (8,000 bytes) before
  ever touching disk, and records the pre-patch file content
  (`Context::record_patch`) so a rejection later in the same round (Test,
  Validate, or Critique) rolls the file back to its pre-patch state
  (`Context::revert_pending_patch`, called from `Stage::Retry` right before
  looping back to `Plan`) instead of leaving an unreviewed mutation in place
  for the next round to build on top of.
- `read_file`/`list_dir` refuse any path that canonicalizes outside the repo root.
- `read_file` truncates output past 32,000 bytes with a note to request a
  narrower range instead.
- `read_tags` requires a `tags` file generated by `universal-ctags -R .` — it does
  not regenerate one on demand.
- There is deliberately **no generic shell-execution tool** — every capability
  the Worker/Scoper has is a specific, narrowly-scoped, typed tool. This is a
  security posture, not a gap: earlier design notes referenced a general `SHELL`
  tool, but the typed-tool set replaced it.

### Debug Mode (`--debug-sequence <file.json>`)

Bypasses the stage machine and drives a **fixed sequence** of roles with
pre-seeded `Context.turns`, for reproducible testing:

```json
{
  "sequence": ["Architect", "Worker", "Validator", "Critic_0"],
  "context_imputations": {
    "documents": "...",
    "context": "...",
    "history": "..."
  }
}
```

Fixtures for this live in `agent_debug/*.json` (e.g. `worker_and_validator_rejection.json`,
`multiple_critics.json`) and are wired into `cargo test --lib` as
`core::orchestrator::tests::*` (see `core/orchestrator.rs`), running against a
scripted `FakeLlmClient`/`FakeVectorStore` instead of a live server. Wiring
these up caught a real, previously-undetected bug: `Orchestrator::new`'s
Critics-construction loop passed each critic's flat `{"model":...,"task":...}`
object straight to `Agent::new` as its `config`, but `Agent::new` looks up
`config.get(role)` expecting the config nested under its own role key — so
`Agent::new` always returned `Err` and `critics` was silently always empty,
regardless of how many were configured via `--critic`/`--agentic`'s
`"Critics"` array. A second, compounding bug meant that even a correctly
constructed critic would have failed `query_stream` immediately afterward:
`Role::from_str` only recognized the bare string `"critic"`, not the
`"Critic_0"`/`"Critic_1"` naming `Orchestrator::new` actually assigns each
one. Both are fixed now — multi-critic consensus review actually runs.

### Configuration (JSON passed to `--agentic`)

```json
{
  "iterations": 4,
  "scope_iterations": 7,
  "Scoper": { "model": "qwen2.5:14b" },
  "Architect": { "model": "qwen2.5:14b", "temperature": 0.0 },
  "Worker":     { "model": "qwen2.5-coder:14b", "temperature": 0.7 },
  "Validator":  { "model": "qwen2.5:14b" },
  "Critics": [
    { "model": "qwen2.5:14b", "task": "Review for security issues", "approval_signal": "APPROVED" },
    { "model": "qwen2.5:14b", "task": "Review for performance" }
  ],
  "Librarian": { "model": "all-minilm:l6-v2", "chroma_client": "..." },
  "Summarizer": { "model": "qwen2.5:14b" }
}
```

- CLI shortcuts (`--team-model`, `--validator-model`, `--critic`) auto-populate the JSON.
- `get_options()` merges CLI flags, JSON file, and defaults per agent.

### Current Limitations

- No configurable agent graph yet — the `Stage` sequence above is fixed in code, not data (this is `ROADMAP.md` Phase 3).
- Token counting is an approximation (`cl100k_base` BPE) since Ollama doesn't expose per-model tokenizers.
- Test coverage is `#[cfg(test)]` unit tests inside the crate (`core::orchestrator::tests`), not black-box `tests/` integration tests — the orchestrator's types are `pub(crate)`, so external tests can't reach them without a much bigger visibility change than this covers. Only 9 of the 10 `agent_debug/*.json` fixtures are wired up (the two `architect_librarian_worker[_validator]` combinations are covered indirectly by the simpler fixtures that exercise the same roles).

### How to Extend

1. Add a new role variant in `core/agent/role.rs` (`Role` enum + `build_chat_messages` arm + `get_task`/`get_color`/`FromStr`/`Display`).
2. Wire it into `Orchestrator::new()` and, if it participates in the stage loop, add/extend a `Stage` arm in `run_stage_machine`.
3. If it needs new tool access, add a `ToolName` variant + schema in `agent/tools.rs` and a dispatch arm in `Orchestrator::handle_structured_tool`.
4. (Optional) Add a dedicated JSON config field and CLI shortcut.

Everything flows through the shared `Context.turns` log and the streaming
channel, so new agents are additive rather than requiring changes to existing ones.
