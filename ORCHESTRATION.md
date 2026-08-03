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
            (Retrieve/Git*/ReadFile/ListDir/Ripgrep/ReadTags/CargoCheck/
            CargoClippy/CargoDupes) and the per-run retrieve budget (default
            2) allows it, the orchestrator executes it, appends the result,
            pushes an explicit System-turn reminder that the Worker must now
            act (not call another read-only tool), and re-asks the Worker
            once more in the same stage. If the Worker calls a read-only tool
            *again* anyway (its one lookup already spent), `execute_and_verify`
            rejects it with a specific, actionable reason instead of a
            generic "unexpected tool" message. `apply_patch`/`memorize` calls
            are executed afterward by
            `execute_and_verify` (`Orchestrator::run_implement_patch_loop`).
            A successful `apply_patch` doesn't necessarily end the round: if
            the plan's `FILES:` line named more files than have been patched
            so far, and a per-round patch budget (default 3, reset every
            round) isn't exhausted, the orchestrator re-asks the Worker for
            the next file instead of moving on — see
            `should_continue_patch_loop`. A plan naming zero or one file
            behaves exactly like a single-patch round always did. If the
            Worker's *first* attempt this round produced no recognized tool
            call at all (e.g. a narrative walkthrough instead of an actual
            tool call) the round is rejected immediately with a precise,
            deterministic reason — not silently sent to `Stage::Test` to
            trivially pass on unchanged code.
  ↓
Test      → cargo check + cargo test (60s / 120s timeouts) against the applied patch.
            Failure → Stage::Retry.
  ↓
Validate  → Validator verdict, if configured. REJECTED or unparseable → Stage::Retry.
  ↓
Critique  → All Critics run concurrently against a snapshot of the current
            output/plan/implementation. Each critic streams into its own local
            channel (not the shared output stream, to avoid interleaving two
            critics' text mid-token); once every critic finishes,
            `run_critics_parallel` emits one complete, labeled trace per
            critic ("[Critic 'Security']: ..."). Each dissent becomes a
            Rejection turn.
  ↓
Reconcile → Any Rejection turns this round (after dedup)? → Retry : Accept
  ↓
Retry     → If the iteration budget is exhausted: surface the best-known
            implementation without committing (if one exists) or Escalate (if
            none does). Otherwise, maybe run the Summarizer, then back to Plan.
  ↓
Accept → Commit → `git checkout -b ai/feature-<timestamp>`, stage only
            `featured_changes.md` and every file `apply_patch` touched this
            round (not `git add .`), generate a commit message via a direct
            LLM call over the staged diff (falls back to a deterministic
            message on failure), commit, return to the original branch → Done
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
| `cargo_clippy` | `{"tool":"cargo_clippy"}` | — |
| `cargo_dupes` | `{"tool":"cargo_dupes"}` | — |

Notes:
- `apply_patch` tolerates two common local-model diff mistakes before ever trying to parse the
  diff: `normalize_diff_hunk_lines` repairs a missing leading space on unchanged hunk lines, and
  `fix_hunk_header_counts` recomputes each `@@ -start,count +start,count @@` hunk header's count
  fields from the hunk body itself (models reliably get this line-count bookkeeping wrong even
  when the actual `+`/`-` content is correct — `diffy` otherwise rejects the whole patch with
  "hunk header does not match hunk"). Neither changes what the diff says to add/remove, only
  fixes bookkeeping the parser needs but the body itself fully determines. A diff with no
  `--- a/<file>` header line at all still can't be applied — there's no safe way to infer a
  target rather than trusting the header — but gets an actionable rejection message telling the
  Worker to add one, instead of a generic parse error.
- If a syntactically valid diff still fails to apply (`diffy::apply`'s `Err` arm — the context
  lines don't match the target's real content, almost always because the Worker guessed/
  hallucinated the file instead of reading it), the rejection includes the file's actual current
  content directly (capped at `MAX_SHOWN_ORIGINAL_CHARS`, 4,000 chars) rather than just the raw
  `diffy` error. This lets the Worker write a correct diff on its very next attempt without
  needing a separate `read_file` call, which may not even be available if `retrieve_budget` is
  already exhausted.
- `apply_patch` is gated: the target file must already be tracked by git
  (`git ls-files`), checked in `Validation::apply_patch` before the diff is applied.
- `apply_patch` also checks scope: if the Architect's plan ended with a
  `FILES: path1, path2` line (`agent_role/architect.md`), the diff's target
  must match one of those paths (`Context::planned_files`, `file_in_scope` in
  `agent/protocol.rs`) or the patch is refused. A plan without a `FILES:` line
  isn't restricted — this fails open by design since local models don't
  reliably emit new prompt conventions.
- `apply_patch` refuses diffs over `MAX_PATCH_DIFF_BYTES` (8,000 bytes, per
  call — not per round) before ever touching disk, and records each touched
  file's pre-patch content (`Context::record_patch`, keeping the *first*
  original if the same file is patched twice in one round) so a rejection
  later in the same round (Test, Validate, or Critique) rolls every file this
  round touched back to its pre-patch state (`Context::revert_pending_patches`,
  called from `Stage::Retry` right before looping back to `Plan`) instead of
  leaving an unreviewed mutation in place for the next round to build on top
  of.
- A round can apply up to 3 sequential `apply_patch` calls (`Stage::Implement`'s
  per-round patch budget), one per distinct file, when the plan's `FILES:`
  line named more than one — see the stage-machine diagram above. Each call
  is independently subject to the same tracked-file, scope, and diff-size
  checks.
- `read_file`/`list_dir` refuse any path that canonicalizes outside the repo root.
- `read_file` truncates output past 32,000 bytes with a note to request a
  narrower range instead.
- `read_tags` transparently keeps the repo-root `tags` file fresh: it checks whether `tags` is
  missing or older than any git-tracked `*.rs` file and regenerates it first if so
  (`orchestrator::search::{tracked_rust_files,tags_are_stale,regenerate_tags}`), rather than
  requiring the caller to separately check/update/read across its limited per-round tool budget
  (`agent_role/worker.md`'s one-lookup-per-round rule). Regeneration is deliberately scoped to
  `git ls-files -- '*.rs'`, never a raw recursive walk (`ctags -R .`, this note's own previous —
  and actively harmful — advice): this repo has a large, gitignored-but-physically-present
  `docs/` directory of saved reference webpages that a recursive walk happily sweeps in, which
  is exactly how a prior `tags` file ballooned to ~494 MB / 2.5M lines of unusable minified-JS
  noise in practice.
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
