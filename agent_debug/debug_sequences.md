# Debug Sequences for `--debug-sequence`

Place these JSON files in `examples/debug/` (or anywhere) and run with:

```bash
./ruchat ask "Your goal here" \
  --agentic '{ "Architect": {"model":"qwen3-coder:latest"}, "Worker": {"model":"qwen3-coder:latest"}, ... }' \
  --debug-sequence examples/debug/NAME.json
```

Only the **first** agent in the sequence receives the `context_imputations`.
Librarian always performs a **real** Chroma query when present.

### 1. Architect only (plan generation)
```json
{
  "sequence": ["Architect"],
  "context_imputations": {
    "history": "User wants to refactor error handling in orchestrator.rs to proper Result propagation."
  }
}
```
Command: `--debug-sequence examples/debug/architect.json`

### 2. Librarian only (real Chroma query test)
```json
{
  "sequence": ["Librarian"]
}
```
Command: `--debug-sequence examples/debug/librarian.json`

### 3. Librarian → Worker (most useful for implementation debugging)
```json
{
  "sequence": ["Librarian", "Worker"],
  "context_imputations": {
    "documents": "Relevant snippets from Chroma:\n- Use ? instead of unwrap\n- anyhow::Context for error chaining\n- Keep Result<T, anyhow::Error>",
    "context": "Architect plan: Replace all unwrap/panic with proper ? propagation and add context."
  }
}
```
Command: `--debug-sequence examples/debug/librarian_worker.json`

### 4. Architect → Librarian → Worker (full early pipeline)
```json
{
  "sequence": ["Architect", "Librarian", "Worker"],
  "context_imputations": {
    "history": "Previous round: user requested error-handling refactor."
  }
}
```
Command: `--debug-sequence examples/debug/architect_librarian_worker.json`

### 5. Worker → Validator (test approval/rejection path)
```json
{
  "sequence": ["Worker", "Validator"],
  "context_imputations": {
    "documents": "Code from previous step uses unwrap on every Result.",
    "context": "Worker output contains several unwrap() calls."
  }
}
```
Command: `--debug-sequence examples/debug/worker_validator.json`

### 6. Worker → Validator (force rejection test)
```json
{
  "sequence": ["Worker", "Validator"],
  "context_imputations": {
    "context": "Worker produced code that still uses .unwrap() everywhere."
  }
}
```
Command: `--debug-sequence examples/debug/worker_validator_reject.json`

### 7. Critic test (single critic)
```json
{
  "sequence": ["Critic0"]
}
```
Command: `--debug-sequence examples/debug/critic0.json`

### 8. Multiple critics
```json
{
  "sequence": ["Critic0", "Critic1"]
}
```
Command: `--debug-sequence examples/debug/critics.json`

### 9. Summarizer test
```json
{
  "sequence": ["Summarizer"],
  "context_imputations": {
    "history": "Long conversation with many rejections about unwrap usage."
  }
}
```
Command: `--debug-sequence examples/debug/summarizer.json`

### 10. Full short debug run (Architect → Librarian → Worker → Validator)
```json
{
  "sequence": ["Architect", "Librarian", "Worker", "Validator"],
  "context_imputations": {
    "history": "User goal: refactor error handling to use ? and anyhow::Context."
  }
}
```
Command: `--debug-sequence examples/debug/full_short.json`

### 11. Validator only (quick approval/rejection check)
```json
{
  "sequence": ["Validator"],
  "context_imputations": {
    "context": "Worker code still contains unwrap() on line 42."
  }
}
```
Command: `--debug-sequence examples/debug/validator.json`
