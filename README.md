# Ruchat

Ruchat is a powerful command-line AI chat and agent orchestration tool built on **Ollama** and **ChromaDB**. It supports interactive chats, RAG-augmented queries, structured tool calling, multi-agent orchestration (Architect → Worker → Critic → Validator), and direct Chroma vector database operations.

## Features

- **Single-shot & piped prompts** (`ask`, `pipe`)
- **Interactive TUI chat** with full editing, history, undo/redo (`chat`)
- **Multi-agent orchestration** with configurable Architect/Worker/Validator/Critic/Librarian teams (including RAG via Chroma)
- **Tool calling** (calculator, web search, browserless, weather, disk space, custom functions)
- **Structured JSON output** via `func-struct`
- **Full ChromaDB CLI**:
  - `embed`, `query`, `search`, `get`, `ls`, `create`, `modify`, `fork`, `delete`
- **Embedding & retrieval** with automatic ID generation and upsert support
- **Git integration** – AI can commit changes to feature branches
- **Configurable model options**, streaming, and advanced generation parameters

## Installation

1. **Prerequisites**
   - Rust (latest stable)
   - Ollama running (`OLLAMA_HOST=localhost:11434 ollama serve`)
   - Optional: ChromaDB (via Docker)

2. **Build**
   ```bash
   git clone https://github.com/RoelKluin/ruchat.git
   cd ruchat
   cargo build --release
   ```

3. **Run ChromaDB (optional, for RAG/embedding)**
   ```bash
   docker run -p 8000:8000 \
     -v ~/chroma_storage:/chroma/chroma \
     chromadb/chroma
   ```

## Basic Usage

```bash
# Simple question
./target/release/ruchat ask "Explain borrow checker in Rust"

# Pipe input
cat file.md | ./target/release/ruchat pipe

# Interactive TUI chat
./target/release/ruchat chat

# List models
./target/release/ruchat ollama-ls

# Multi-agent orchestration (recommended)
./target/release/ruchat ask --team-model "qwen2.5-coder:14b" "Refactor the CLI argument parsing"
```

### Agentic Mode Examples

```bash
# Quick team with one model
ruchat ask --team-model "qwen2.5-coder:14b" "Implement a new Chroma delete command"

# Full custom config (JSON)
ruchat ask --agentic '{
  "iterations": 4,
  "Architect": {"model": "qwen2.5:14b", "temperature": 0.0},
  "Worker": {"model": "qwen2.5-coder:14b"},
  "Validator": {"model": "qwen2.5:14b"}
}' "Add support for sparse vectors in queries"
```

### Chroma Commands

```bash
ruchat chroma-ls
ruchat embed "Some code" --collection repo_src-all-minilm_l6-v2
ruchat chroma-query --query "error handling" --n-results 5
ruchat chroma-search --query-vector 0.1,0.2,...   # advanced
```

See `ruchat --help` and subcommand help for all options.

## Configuration

- Model options via `--options <JSON|file>`
- Chroma connection via env vars (`CHROMA_SERVER`, `CHROMA_TOKEN`) or CLI flags
- Persistent team/manager state in `ruchat_manager.json`
- Collection definitions in `db_config.json`

## Project Status

**Version:** 0.1.2

Actively developed with focus on:
- Robust error handling and logging
- Improved configuration merging
- Better TUI editing experience
- Expanded agent orchestration and RAG capabilities

## Contributing

Contributions welcome! Please fork the repository and submit a PR.

See `TODO.txt` and `more_TODO.txt` for planned improvements (error handling, testing, performance, security, etc.).

## License

MIT License
