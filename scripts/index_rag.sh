#!/bin/bash
# Keep the Chroma collections used for retrieval up to date.
#
# Costs zero Claude tokens by design: embeddings come from the local Ollama
# instance and everything here is a shell command. Safe to run in the
# background or from .git/hooks/post-commit.
#
# Collections (naming matches the existing repo_src/repo_hist convention):
#   repo_docs-*    the design docs - what a session would otherwise re-read
#   repo_lessons-* ruchat_traces/summaries/, the per-run agent-decision reviews
#
# Deliberately NOT re-indexing src/: at ~20k lines over 77 files, ripgrep plus
# a targeted read beats embedding retrieval for code you can name. repo_src
# already exists; leave it.

set -uo pipefail
cd "$(dirname "$0")/.." || exit 1

model="all-minilm:l6-v2"
suffix="${model//:/_}"
docs_collection="repo_docs-${suffix}"
lessons_collection="repo_lessons-${suffix}"

command -v ctags >/dev/null || { echo "index_rag: universal-ctags not on PATH" >&2; exit 1; }
[ -x ./ruchat ] || { echo "index_rag: ./ruchat not built" >&2; exit 1; }

# Reachable Ollama? Fail loudly rather than leaving a half-built collection.
curl -sf --max-time 5 "${OLLAMA_SERVER:-http://127.0.0.1:11434}/api/tags" >/dev/null \
  || { echo "index_rag: ollama unreachable, skipping" >&2; exit 1; }

ensure_collection() {
  ./ruchat chroma-ls 2>/dev/null | grep -q "$1" \
    || ./ruchat chroma-create --collection "$1" --metadata "{\"model\": \"$model\"}"
}

# `ruchat index` is incremental by default (.ruchat_index_state/<collection>.marker
# skips files unchanged since the last successful run), so re-running this on
# every commit is cheap.
index_docs() {
  ensure_collection "$docs_collection"
  ./ruchat index . --ext md --collection "$docs_collection" -m "$model" 2>&1 | tail -3
}

index_lessons() {
  [ -d ruchat_traces/summaries ] || { echo "index_rag: no summaries yet, skipping"; return 0; }
  local n
  n=$(find ruchat_traces/summaries -name 'ruchat_trace_*.md' | wc -l)
  [ "$n" -gt 0 ] || { echo "index_rag: no summaries yet, skipping"; return 0; }
  ensure_collection "$lessons_collection"
  ./ruchat index ruchat_traces/summaries --ext md --collection "$lessons_collection" -m "$model" 2>&1 | tail -3
}

case "${1:-all}" in
  docs)    index_docs ;;
  lessons) index_lessons ;;
  all)     index_docs; index_lessons ;;
  *)       echo "usage: $0 [all|docs|lessons]" >&2; exit 1 ;;
esac
