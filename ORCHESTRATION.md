**Agent Orchestration in Ruchat**

Ruchat’s agent orchestration is a **multi-agent loop** built around the `Orchestrator` (`core/orchestrator.rs`). It turns a single user goal into a structured, auditable workflow using specialized LLM agents that collaborate until the output is approved or the iteration limit is reached.

### Core Components

- **Orchestrator** (`core/orchestrator.rs`)
  - Holds the pipeline of agents.
  - Manages a shared `Context` (goal, history, documents, rejections, output).
  - Runs the main loop (`execute_orchestration`) or debug sequence.
  - Streams every token + metadata to the caller via `mpsc` (used by `ask` command).

- **Agent** (`agent/agent.rs`)
  - Thin wrapper around a model + config.
  - `query_stream()` builds a role-specific prompt and streams the response.
  - Post-processes tool calls (`MEMORIZE`, `SHELL`).

- **Role** (`agent/role.rs`)
  - Defines system prompt + task for each participant.
  - `build_prompt()` injects context, history, documents, and hints.
  - `update_context()` records the agent’s output in the shared history.

- **Context** (`agent/types.rs`)
  - Single source of truth passed between agents.
  - Contains goal, documents (from RAG), history, rejections, and trace.

### Agent Roles (default pipeline)

| Role              | Required | Purpose                                      | Approval Signal      | Special Behaviour |
|-------------------|----------|----------------------------------------------|----------------------|-------------------|
| **Architect**     | Yes      | Produces a concrete plan                     | —                    | First agent |
| **Librarian**     | Optional | Formulates Chroma query + retrieves docs     | —                    | RAG only; uses `db_config.json` |
| **Worker**        | Yes      | Implements the plan (code, changes, etc.)    | —                    | Can call `MEMORIZE` / `SHELL` |
| **Validator**     | Optional | Technical correctness check                  | `VALIDATED`          | Rejects with `REJECTED: …` |
| **Critic**\*      | Optional | Domain-specific review (security, perf, …)   | —                    | Multiple allowed |
| **Summarizer**    | Optional | Compresses history when it grows too long    | —                    | Triggered by `history_limit` |

\* Critics can be supplied as an array under `"Critics"` in the JSON config.

### Execution Flow (normal mode)

```rust
for round in 1..=iterations {
    architect.query_stream(...)          // → plan
    librarian?.query_stream(...)         // → RAG documents (if enabled)
    worker.query_stream(...)             // → implementation
    worker.execute_and_verify(...)       // runs SHELL/MEMORIZE tools

    if validator { validator.query_stream(...) }   // reject → continue
    for critic in critics { critic.query_stream(...) }

    if ctx.is_approved() {
        git::commit_feature_branch(&ctx)   // auto-commit on success
        break;
    }

    if history_too_long { summarizer.query_stream(...) }
}
```

- **Streaming**: Every agent’s tokens are sent live to the TUI/CLI.
- **Trace events**: `RuChatError::Trace`, `StatusUpdate`, `ColorChange` give real-time feedback.
- **Rejection handling**: Non-approved output is recorded in `ctx.rejections`. Loop continues until approval or iteration limit.
- **Git integration**: On final approval the Worker’s output is committed to a new branch `ai/feature-<timestamp>`.

### Debug Mode (`--debug-sequence <file.json>`)

Bypasses the normal loop and runs a fixed sequence of roles with pre-injected context. Useful for reproducible testing:

```json
{
  "sequence": ["Architect", "Worker", "Validator", "Critic0"],
  "context_imputations": {
    "documents": "...",
    "history": "..."
  }
}
```

### Configuration (JSON passed to `--agentic`)

```json
{
  "iterations": 4,
  "history_limit": 8192,
  "Architect": { "model": "qwen2.5:14b", "temperature": 0.0 },
  "Worker":     { "model": "qwen2.5-coder:14b", "temperature": 0.7 },
  "Validator":  { "model": "qwen2.5:14b" },
  "Critics": [
    { "model": "qwen2.5:14b", "task": "Review for security issues" },
    { "model": "qwen2.5:14b", "task": "Review for performance" }
  ],
  "Librarian": { "model": "all-minilm:l6-v2", "chroma_client": "..." },
  "Summarizer": { "model": "qwen2.5:14b" }
}
```

- CLI shortcuts (`--team-model`, `--validator-model`, `--critic`) auto-populate the JSON.
- `get_options()` merges CLI flags, JSON file, and defaults.

### Current Limitations (see TODO.md)

- History is a simple string (no structured memory yet).
- Tool calling is minimal (`MEMORIZE` + `SHELL` only).
- Parallel critics are sequential.
- No persistent agent memory across runs (planned).

### How to Extend

1. Add a new role in `agent/role.rs`.
2. Register it in `Orchestrator::new()`.
3. (Optional) Add a dedicated field in the JSON config.

The system is deliberately lightweight yet extensible — everything flows through the shared `Context` and streaming channel, making new agents trivial to plug in.
