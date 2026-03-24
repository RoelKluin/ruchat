**Context Structure Details**

```rust
pub(crate) struct Context {
    pub(crate) goal: String,           // Original user request
    pub(crate) history: String,        // Full conversation trace (grows over iterations)
    pub(crate) output: String,         // Latest raw LLM response (cleared before each agent)
    pub(crate) context: String,        // Current working plan / implementation (updated by Architect/Worker)
    pub(crate) rejections: String,     // Accumulated rejection reasons from Validator/Critics
    pub(crate) documents: String,      // RAG results injected by Librarian (markdown-formatted)
    pub(crate) config: Value,          // Raw JSON config (db_config.json + agent settings)
}
```

### Field-by-Field Breakdown

- **`goal: String`**  
  Immutable user prompt passed at the start (`ruchat ask "..."`).  
  Used by every agent via `role.build_prompt()`.

- **`history: String`**  
  Accumulates **everything** that happened:  
  - Architect plans  
  - Worker implementations  
  - Critic/Validator feedback  
  - Summarizer compressions  
  - Rejection logs  
  Grows linearly with iterations. When it exceeds `history_limit` (default from model context size), the **Summarizer** compresses it.

- **`output: String`**  
  **Temporary buffer** for the current agent’s response.  
  Cleared before each `query_stream()`.  
  After streaming finishes, it is processed (`parse_tool_call`, `update_context`, rejection check).

- **`context: String`**  
  **Working memory** for the current state of the solution.  
  - After Architect → `"PLAN:\n..."`  
  - After Worker   → `"IMPLEMENTATION:\n..."`  
  Passed to subsequent agents so they know what was decided/implemented so far.

- **`rejections: String`**  
  Concatenated failure reasons from Validator and all Critics.  
  `ctx.is_approved()` returns `true` only when this string is empty.

- **`documents: String`**  
  Markdown-formatted retrieval results from Chroma (via Librarian).  
  Injected once (round 1) and made available to the Worker.

- **`config: Value`**  
  Holds the full JSON configuration loaded from `db_config.json` (collection definitions, example queries, allowed fields, etc.).  
  Used by Librarian to build collection summary and by agents for metadata.

### How Context Flows Through the Orchestration Loop

```text
Start
  ↓
Architect.query_stream() → writes plan to output
  ↓
role.update_context()    → context = "PLAN:\n..."
  ↓
Librarian (if enabled)
  → formulates Chroma query from goal+plan
  → retrieves documents → ctx.documents = "..."
  ↓
Worker.query_stream()    → uses documents + context
  ↓
Worker.execute_and_verify() → runs SHELL / MEMORIZE tools
  ↓
Validator + Critics
  → if any outputs "REJECTED..." → append to rejections
  ↓
if rejections.is_empty() → commit_feature_branch() + break
else
  if history too long → Summarizer.compress() → history = summary
  continue next round
```

### Key Methods on Context

- `read_config_file(path)` — loads `db_config.json` into `config`
- `build_collections_summary()` — generates the rich collection description shown to Librarian
- `apply_debug_imputations()` — used only in `--debug-sequence` mode to inject test data
- `print_debug_info()` — dumps full state for debugging
- `trace(tx, msg)` — sends trace events to the UI stream (dimmed output)
- `is_approved()` — `rejections.is_empty()`

### Edge Cases & Design Notes

- **Token management**: No automatic token counting yet (relies on model-specific `get_dynamic_history_limit()`). Summarizer is the safety valve.
- **Immutability**: `goal` and `config` are effectively read-only after initialization.
- **Streaming safety**: `output` is cleared before each agent so partial responses don’t leak between agents.
- **Debug mode**: Bypasses normal loop and runs exact sequence with pre-filled `documents`/`history`.

### Current Limitations (from TODO)

- History is a plain `String` → no structured events or vector memory yet.
- No persistent context across separate `ruchat` invocations.
- `documents` can become very large; no automatic chunking/summarization before feeding to Worker.

This `Context` struct is the **single source of truth** that makes the multi-agent collaboration possible while keeping the orchestration logic simple and linear. All agents read from it and write back through well-defined update paths.
