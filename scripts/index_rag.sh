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
src_collection="repo_src-${suffix}"

command -v ctags >/dev/null || { echo "index_rag: universal-ctags not on PATH" >&2; exit 1; }
[ -x ./ruchat ] || { echo "index_rag: ./ruchat not built" >&2; exit 1; }

# Reachable Ollama? Fail loudly rather than leaving a half-built collection.
curl -sf --max-time 5 "${OLLAMA_SERVER:-http://127.0.0.1:11434}/api/tags" >/dev/null \
  || { echo "index_rag: ollama unreachable, skipping" >&2; exit 1; }

ensure_collection() {
  ./ruchat chroma-ls 2>/dev/null | grep -q "$1" \
    || ./ruchat chroma-create --collection "$1" --metadata "{\"model\": \"$model\"}"
}

# Volatile corpora get a full rebuild, NOT an incremental re-index.
#
# `ruchat index` derives chunk IDs from chunk *content*, so re-indexing a file
# whose text changed inserts a new chunk and leaves the old one behind forever —
# measured 2026-08-05 on a two-section test doc: editing one section's body took
# the collection from 3 chunks to 5 to 7, with both the old and new text
# retrievable. The existing repo_src collection had already accumulated 68
# duplicated symbols this way, one of them 33 deep. Stale chunks are worse than
# a missing index: retrieval returns superseded content with nothing marking it
# as superseded. Deleting first is cheap here (~90 chunks, local embeddings).
#
# See TODO.md - the underlying ID scheme is a real ruchat bug, this is the
# workaround, not the fix.
rebuild_collection() {
  ./ruchat chroma-delete --collection "$1" --force >/dev/null 2>&1 || true
  rm -f ".ruchat_index_state/$1.marker" 2>/dev/null || true
  ensure_collection "$1"
}

index_docs() {
  rebuild_collection "$docs_collection"
  ./ruchat index . --ext md --collection "$docs_collection" -m "$model" 2>&1 | tail -3
}

# Summaries are append-only: `finalize_summary_trace` writes one file per
# finished run and never rewrites it, so incremental upsert is correct here and
# stays cheap as the directory grows. If that ever changes, this needs the
# rebuild treatment too.
index_lessons() {
  [ -d ruchat_traces/summaries ] || { echo "index_rag: no summaries yet, skipping"; return 0; }
  local n
  n=$(find ruchat_traces/summaries -name 'ruchat_trace_*.md' | wc -l)
  [ "$n" -gt 0 ] || { echo "index_rag: no summaries yet, skipping"; return 0; }
  ensure_collection "$lessons_collection"
  ./ruchat index ruchat_traces/summaries --ext md --collection "$lessons_collection" -m "$model" 2>&1 | tail -3
}

# repo_src is for ruchat's own Librarian role at runtime, not for a Claude session
# reading code — ripgrep plus a targeted read wins at this repo's size. It still has
# to be correct, since the agentic pipeline retrieves from it: it had accumulated 68
# duplicated symbols (one 33 deep) before this rebuild existed. Slower than the doc
# rebuild (~1200 chunks), so it is not in the post-commit path; run it after a
# refactor that moves or renames much of src/.
index_src() {
  rebuild_collection "$src_collection"
  ./ruchat index src --collection "$src_collection" -m "$model" 2>&1 | tail -3
}

case "${1:-all}" in
  docs)    index_docs ;;
  lessons) index_lessons ;;
  src)     index_src ;;
  all)     index_docs; index_lessons ;;
  full)    index_docs; index_lessons; index_src ;;
  *)       echo "usage: $0 [all|docs|lessons|src|full]" >&2; exit 1 ;;
esac
