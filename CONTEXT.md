**Context Structure Details**

```rust
pub(crate) struct Context {
    pub(crate) goal: String,          // Current goal (may be rewritten by the Scoper)
    pub(crate) turns: Vec<Turn>,      // Append-only structured event log — the real history
    pub(crate) output: String,        // Latest raw LLM response — transient scratch buffer
    pub(crate) context_config: Value, // db_config.json contents, loaded when a Librarian is configured
    pub(crate) round: u64,            // current round number, incremented on each Plan stage
    pub(crate) pending_patch: Option<PendingPatch>, // pre-image of a file apply_patch just wrote, for rollback on rejection
}

pub(crate) struct PendingPatch {
    pub(crate) path: String,
    pub(crate) original: String, // file content before the currently-applied patch
}

pub(crate) struct Turn {
    pub(crate) round: u64,
    pub(crate) kind: TurnKind,
    pub(crate) source: String,   // agent/tool name that produced this turn, e.g. "Worker", "GitBlame"
    pub(crate) content: String,
}

pub(crate) enum TurnKind {
    Plan,           // Architect output
    Implementation, // Worker output
    Retrieval,      // Librarian / on-demand Retrieve+read-tool output
    Rejection,      // Validator / Tester / Critic feedback
    Summary,        // Summarizer output, replaces collapsed turns
    System,         // system-level confirmations (e.g. MEMORIZE ack, Scoper notes)
}
```

`Context` no longer holds `history`/`context`/`rejections`/`documents` as flat strings — an
earlier version of this file described that design, but the code moved to a structured,
append-only event log (`turns: Vec<Turn>`) some time ago. Every agent still sees the
same *shape* of information; it's just derived from `turns` on demand instead of being
mutated field by field. If you're extending the orchestrator, add a new `TurnKind`
(or filter an existing one) rather than reintroducing a flat string field.

### Field-by-Field Breakdown

- **`goal: String`**
  The user's request. Not fully immutable: the **Scoper** can rewrite it via
  `clarified_goal` before planning starts, to correct a wrong premise or fill in
  scope the original prompt left implicit. Every subsequent agent sees the
  (possibly clarified) goal via `role.build_chat_messages()`.

- **`turns: Vec<Turn>`**
  The single source of truth for everything that has happened, tagged with the
  `round` it occurred in and a `TurnKind` describing what kind of event it is.
  Nothing is ever mutated in place except by `reconcile_rejections()` (dedup) and
  `collapse_to_summary()` (compression) — otherwise it's strictly append-only.

- **`output: String`**
  **Transient buffer** for the current agent's raw response. Cleared before each
  `query_stream()` call, filled by streaming, then read once by the caller (tool-call
  parsing, verdict parsing, or `ctx.push_turn()`) before the next agent overwrites it.

- **`context_config: Value`**
  Holds `db_config.json` (collection definitions, example queries, allowed
  `include` fields) — loaded once via `read_config_file()` when a Librarian is
  configured. Used by `build_collections_summary()` for the Librarian/Scoper prompts.

- **`round: u64`**
  Incremented once per `Stage::Plan` entry. Turns are filtered/windowed by round
  via the view methods below, so context shown to an agent is scoped to "up to
  this point in the run," not the entire unbounded log.

### Derived Views (replace the old flat-string fields)

These are computed from `turns` each time they're called — there's no cached
state to keep in sync:

- **`history_view(upto_round)`** — chronological transcript up to a round,
  excluding `Retrieval` turns (those render separately). This is what the old
  `history: String` field used to hold.
- **`context_view()`** — the *latest* `Plan` turn plus the *latest*
  `Implementation` turn only (not the full history). Replaces the old
  `context: String` ("PLAN:\n..." / "IMPLEMENTATION:\n...").
- **`documents_view(upto_round)`** — all `Retrieval` turns up to a round, most
  recent first. Replaces the old `documents: String`.
- **`is_approved()`** — `true` iff no turn has `TurnKind::Rejection` at all
  (checked across the whole log, not just the current round).
- **`reconcile_rejections()`** — dedups `Rejection` turns within the *current*
  round in place, returns whether any remain. This is what actually gates the
  `Stage::Reconcile → Retry | Accept` branch — replaces the old
  `rejections.is_empty()` check.

### Key Methods on Context

- `push_turn(kind, source, content)` — append an event; the only way `turns` grows.
- `read_config_file(path)` — loads `db_config.json` into `context_config`.
- `build_collections_summary()` — generates the collection description shown to
  the Librarian and the Scoper.
- `apply_debug_imputations(&Value)` — used only by `--debug-sequence` mode to
  seed `turns` with pre-fabricated Retrieval/Plan/Summary entries before running
  a fixed role sequence.
- `collapse_to_summary(text)` — drops all turns at or before the current round
  and replaces them with a single `Summary` turn; invoked from `Stage::Retry`
  when the Summarizer's token-budget check trips.
- `record_patch(path, original)` / `revert_pending_patches(tx)` — track and, if
  needed, undo every file write `apply_patch` makes in a round (a round can
  touch more than one file — see `ORCHESTRATION.md`'s tool catalog notes for
  the full rollback flow).
- `trace(tx, msg)` — sends a `Trace` event to the UI stream and rewrites this
  run's live file under `ruchat_traces/` with the current goal/context/history
  snapshot (including retrieval/tool-output turns, unlike the `HISTORY` prompt
  variable — see `full_history_view`/`trace_body`). `init_trace_index()` picks
  the run's file slot once, at the very start; `finalize_success_trace(summary)`
  / `finalize_failure_trace(summary)` archive the final result into
  `ruchat_traces/successes/` or `ruchat_traces/failures/` once the run ends, and
  `finalize_summary_trace(summary_body(..))` writes every run's standalone
  analysis — outcome plus a round-by-round review of the agents' decisions — to
  `ruchat_traces/summaries/`. See `ORCHESTRATION.md` for how both are generated.

### How Context Flows Through the Orchestration Loop

See `ORCHESTRATION.md` for the full `Stage` state machine — the short version:

```text
Scope   → Scoper looks up repo facts via read-only tools, may rewrite ctx.goal
Plan    → Architect writes a plan                              → Turn(Plan)
Retrieve→ Librarian runs a Chroma query (round 1 only)          → Turn(Retrieval)
Implement→ Worker implements, may call a read-only tool first   → Turn(Implementation)
Test    → cargo check + cargo test against the Worker's patch   → Turn(Rejection) on failure
Validate→ Validator verdict                                     → Turn(Rejection) on REJECTED
Critique→ Critics run concurrently                               → Turn(Rejection) per dissenting critic
Reconcile→ any Rejection turns this round? → Retry : Accept
Retry   → maybe Summarizer compresses turns, then back to Plan
Accept  → Commit → git checkout -b ai/feature-<ts>; commit; back to original branch
```

### Edge Cases & Design Notes

- **Token management**: `agent/tokens.rs` approximates token counts with the
  `cl100k_base` BPE (an OpenAI tokenizer) as a stand-in — Ollama doesn't expose a
  tokenize endpoint for arbitrary local models, so this is a documented
  approximation, not an exact per-model count. The Summarizer's trigger
  (`get_dynamic_history_limit()`) is only as accurate as that estimate.
- **Streaming safety**: `output` is cleared before each agent's `query_stream()`
  so partial responses can't leak between agents.
- **Debug mode**: `--debug-sequence <file.json>` bypasses `run_stage_machine`
  entirely and drives a fixed sequence of roles via `debug_stage_machine`, with
  `turns` pre-seeded from the file's `context_imputations`. Fixtures live in
  `agent_debug/*.json`.
- **Stall detection**: the Plan and Implement stages compare each round's raw
  output against the previous round's; an identical repeat is treated as a stall
  and forces `Stage::Escalate` rather than looping forever.

This `Context` struct is the single source of truth that makes multi-agent
collaboration possible while keeping the orchestration loop itself simple: agents
only ever read a *view* of `turns` and write back through `push_turn`.
