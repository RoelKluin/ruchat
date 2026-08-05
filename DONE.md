# Ruchat Done items

## 0. CRITICAL

Fixed 2026-08-04 unless noted.

- [x] **Hard `Stage::Escalate` on identical Architect/Worker output killed runs** — traces 524–542; removed both escalates; `ctx.round > max_iterations` is sole backstop (orchestrator.rs).
- [x] **Worker calling read-only tool twice in one round burned the whole round** — trace 475; added one bounded nudge-and-reask before rejection (orchestrator.rs).
- [x] **Worker substituted `memorize` for a real fix while diagnostics were in context** — trace 475; `round_has_actionable_diagnostics` deterministically rejects no-op memorize (orchestrator.rs).
- [x] **apply_patch diffs missing `--- a/`/`+++ b/` headers were refused outright** — `ensure_diff_has_file_header` synthesizes them when plan names exactly one file (protocol.rs).
- [x] **Architect embedded a full apply_patch tool_call in its plan, Worker copied it verbatim** — traces 480/481/489; `strip_architect_tool_call_hallucination` truncates at tool_call fence/IMPLEMENTATION heading; `auto_ground_planned_file` injects real line-numbered target file content every round (orchestrator.rs).
- [x] **Blank line between diff hunks broke diffy parsing** — `normalize_diff_hunk_lines` no longer excludes blank lines from missing-context-prefix repair (protocol.rs).
- [x] **Unvalidated patch left applied on disk after budget exhaustion silently broke two subsequent runs** — traces 486/487/488; `Stage::Retry` branch now calls `revert_pending_patches` before `Stage::Done` (orchestrator.rs).
- [x] **`--summarizer-model` CLI flag did not exist** — added, mirrors `--validator-model` (ask.rs).
- [x] **`auto_ground_planned_file` re-injected ~4000-char dump every round with no dedup** — now drops any prior grounding turn before pushing fresh (orchestrator.rs).
- [x] **`--trace-timings` flag: per-turn `duration_ms` in trace files** — types.rs, orchestrator.rs.
- [x] **Architect generation runaway decoding loop never caught** — trace 524; `has_runaway_repetition` in `query_stream` breaks early on 6+ word phrase repeating 3+ times in a row (agent.rs); applies to all roles.
- [x] **Architect's markdown-heading IMPLEMENTATION: line wasn't stripped, leaking hallucinated tool_call** — trace 492; `strip_architect_tool_call_hallucination` trims leading #/*/space; `structured_call_from_value` promotes patch.diff to top-level diff as defense-in-depth (stall_mitigation.rs, tools.rs).
- [x] **Worker's diff removed wrong field (cfg not options) from same struct three rounds running** — trace 497; `Validation::apply_patch` guard: for pure-deletion diffs answering dead-code warnings, at least one removed line must mention the symbol clippy named; fails open (protocol.rs).
- [x] **Guard from item 13 compared line numbers, rejected correct edits with shifted offsets** — traces 499/500; rewritten position-independent; `diff_repair::realign_pure_deletion_hunks` re-anchors pure-deletion hunk onto real location (protocol.rs).
- [x] **5 of 8 archived commits changed no source at all, only changelog** — `Stage::Commit` now escalates when `ctx.pending_patches` is empty (orchestrator.rs).
- [x] **Commits now continue the most recent `ai/feature-*` branch instead of starting a new one** — `--feature-branch <name>` overrides; continued branch never `branch -D`'d on late failure (git.rs).
- [x] **Example tasks can now abandon intractable target and pick another** — `PICK_ANOTHER` clause in scripts/refactoring_examples.sh; gate deliberately does NOT get this clause (run_ruchat_raw).
- [x] **Every finished run now writes ruchat_traces/summaries/ruchat_trace_<N>.md** — goal, outcome, round-by-round review with GOOD:/BAD:/UNCLEAR: verdicts and LESSON: lines; 240s timeout, 24k-char head+tail clamp; any pre-clamp summary on large trace is unreliable (run_summary::generate_step_review).
- [x] **GPU/Ollama facts corrected by measurement 2026-08-05** — ollama-light (:11431) runs on CPU (Ollama's CUDA drops Maxwell CC 5.0); delegation policy routes everything to ollama-heavy (:11434); detail moved to ruchat-dev skill's references/gpu-and-ollama.md.
- [x] **Architect emits tool instructions the Worker then obeys, burning the round's one lookup** — trace 531; `plan_sanitize::strip_lookup_directives` strips lookup-tool directives from Worker's copy of plan before it renders (plan_sanitize.rs, role.rs).
- [x] **Run summary can flatly misstate the trace's own tool output** — `doc_summary.rs`'s prompt forbids answering from unrelated RAG matches; `run_summary.rs::ground_warning_claim` cross-checks against cargo's "generated N warnings" line, prepends correction note if mismatch (run_summary.rs, doc_summary.rs).
- [x] **`commit_feature_branch` failing left applied patch on disk, uncommitted, indefinitely** — trace 2026-08-05 against fixtures/gate-repo; `Stage::Commit`'s call site now calls `ctx.revert_pending_patches` before propagating error; gate passes `--feature-branch ai/gate-<ts>-<pid>` for always-fresh branch (orchestrator.rs, scripts/refactoring_examples.sh).
- [x] **`repo_src-all-minilm_l6-v2` held 1230 chunks, clean index gives 1535** — `scripts/index_rag.sh src` regenerates; not in post-commit path (too slow) — rerun after refactors moving/renaming src/ chunks.
- [x] **`run_stage_machine`'s loop had a dozen `?`-propagating calls skipping `finalize_trace`** — traces 524–542 sat unarchived; split into `run_stage_machine` (wrapper) and `run_stage_machine_loop` (returns Result<bool>); wrapper calls `finalize_trace` unconditionally (orchestrator.rs).

### NEXT ACTION (decided 2026-08-04)

- [x] **Gate task retargeted, 2026-08-04** — moved to dedicated fixture submodule 2026-08-05; `gate_remove_dead_trait` deletes `Evictor` in `fixtures/gate-repo` (not ruchat's own repo); keeps gate commits/ai/feature-* branches out of ruchat's history; `cargo_clippy`/`cargo_check` output independent of ruchat's codebase size; target no longer needs re-verification when ruchat's surrounding code shifts; caveat: `.gitmodules` URL is local absolute path, works on this machine only, not clone-portable until pushed to real remote (fixtures/gate-repo/README.md, scripts/refactoring_examples.sh).

## High Priority

### 2. Error Handling & Logging

### 3. Agent Orchestration
- [x] **Stale, corrected 2026-08-04** — per-document summarization, multi-collection queries, reranking already shipped in Phase 2 (doc_summary.rs, Query collection now a list, chroma/rerank.rs).

### 4. Security & CI

- [x] **ripgrep tool had no repo-root containment check and no `--` flag terminator before untrusted args** — RCE chain via `--pre=/bin/sh` + ability to write arbitrary tracked-file content; `fs.rs::canonicalize_within_repo` and `build_rg_args` enforce containment and `--` separator (fb54763).
- [x] **Ollama has no standard built-in auth mechanism** — any real deployment uses reverse proxy; no action taken.
- [x] **`--verbose` printed raw command line including `--chroma-token`/`--anthropic-api-key` secrets** — `cli/args.rs::redact_args` redacts both flag forms before printing.
- [x] **CI workflow `push` trigger targeted `main`, repo's actual branch is `master`** — CI silently never ran on direct pushes (one-line fix).
- [x] **`cargo_dupes` didn't call `limit_resources` like `cargo_check`/`cargo_clippy`** — now does.
- [x] **`ripgrep` had no wall-clock timeout** — added 20s timeout, matching every other subprocess tool.
- [x] **`git_blame` didn't insert `--` before `path`** — now matches `git_log`/`git_diff`.
- [x] **`AnthropicArgs` lacked manual redacting `Debug` impl** — added, mirroring `AnthropicClient`'s.

### 5. TUI Chat — closed 2026-08-04

Crossterm interactive layer deleted 2026-07-31 (~1,260 lines: ad0708d and neighbours); `src/tui/` now `io.rs` + `render.rs` (175 lines); `--step`/`--breakpoint`/`--approve` cover interactivity where it matters; rebuilding the TUI (if wanted) is new work, not a bug fix.

- [x] **Removed unused `crossterm` dependency** — decision: drop rather than carry for hypothetical TUI rebuild.
- [x] **`AgentEvent::Progress` producer** — done (see Done section).

## Medium Priority

### Code Quality & Maintainability
- [x] **Three failing unit tests had wrong expectations, not real logic bugs** — `test_get_metadata_valid` uses real JSON input; `test_create_table` asserts short field aliases; `test_json_output` fixture uses valid `Include` variant, checks substrings not exact JSON match.
- [x] **`cargo clippy` baseline cleaned up** — boxed `RuChatError::ChannelError`'s oversized payload (manual `From` impl replacing `#[from]`) dropped lib warnings from 86 to 12; removed 3 genuinely-dead items (`Issue` struct, `Context::is_approved`, `Validation::run_cargo_check`); marked `cli::utils::parse_key_val` `#[allow(dead_code)]` (intentional fixture for `core/agent/evals.rs`); fixed `redundant_closure`, `missing_transmute_annotations`, `assertions_on_constants`; down to 10 lib warnings; rest (EmbeddingsClient/LlmProvider traits, OllamaArgs::init_server, two build_generation_request methods, ChromaCollectionConfigArgs::get_or_create_collection, Agent::options, EmbedArgs::embed_raw_items) documented in TODO.md.

### Chroma / RAG
- [x] **`ruchat embed`/`ruchat index` failed on brand-new collection name** — `EmbedArgs::resolve_collection` now calls `get_or_create_collection` instead of create-free `get_collection` (live-verified against real Chroma server).
- [x] **`ruchat chroma-import` revived and fully wired** — added `git::git_log_hashes`, `git::git_show` to orchestrator/git.rs; added `EmbedArgs::new_for_ingestion`; fixed `Some(msg_meta)`/`UpdateMetadata` type mismatch; added first-ever tests for module (parse_show_output, parse_diff_hunks); live-verified against this repo's git history.
- [x] **`embed_script.sh` removed as dead weight** — `ruchat index`/`core/index.rs` supersedes it with real, tested implementation (ctags JSON `end` field / `build_symbol_metadata`'s next-symbol heuristic, not brace-counting); one stale doc reference updated (scripts/refactoring_examples.sh).

### Architecture & Providers

- [x] **`ask`/`pipe` lack their own `--vector-provider`/`--sqlite-vec-path` flags** — added, mirrored through `--collection` shortcut's Librarian-config injection in ask.rs::into_config (two tests confirm correct key landing per provider, no cross-contamination).
- [x] **`--vector-provider` / `--sqlite-vec-path` not in README.md / INSTALL.md** — added Features bullet and requirements note.
- [x] **SQLite-vec default backend had no direct test** — added test proving `--vector-provider sqlite-vec` reaches real `SqliteVecCollection`; Chroma-default branch still lacks equivalent (live server/stub needed), lower risk given existing coverage.
- [x] **SQLite-vec backend lacks error case test** — added `add_with_a_dimension_mismatch_returns_a_legible_error`.

### Performance
- [x] **Stale concern: output buffering before display** — `Agent::query_stream` forwards `StreamItem::ChatChunk` instantly; `tui/render.rs` writes to terminal immediately (verified 2026-08-04).
- [x] **Reviewed `reqwest` default features** — all used (default-tls, charset, http2, system-proxy); no trimming warranted; explicit `["json", "stream"]` also used.

### Security & Production Readiness

## Low Priority / Nice-to-have

## Done / Recently Completed

- [x] **`--trace-timings`: per-turn duration_ms in trace files** (2026-08-04).
- [x] **`git apply --check` second opinion on apply_patch diffy failures** — diagnostic only, doesn't change apply engine (protocol.rs).
- [x] **Grounded apply_patch diffs in real file content** — numbered rejection dump + ripgrep --context flag (protocol.rs).
- [x] **`cargo test --lib` no longer pollutes ruchat_traces/** — `cfg!(test)` early-return in trace/init_trace_index/finalize_*_trace; cleaned up 467 stray files.
- [x] **`ruchat index` skips unchanged files since last run** — `--force` to bypass; `.ruchat_index_state/<collection>.marker` (2026-08-04).
- [x] **`chroma-delete --force` whole-collection deletion** — help text clarified, already a feature (2026-08-04).
- [x] **Repo-wide `cargo fmt` pass** — 45 files; `fmt --check` clean since (2026-08-04).
- [x] **Resumable/crash-resilient runs** — `ruchat_checkpoint.json` after each stage transition; `--resume` flag (ROADMAP Phase 3); live-verified (SIGKILL mid-round, --resume continued correctly) (checkpoint.rs).
- [x] **Interactive `--approve` gate on Stage::Commit** — ROADMAP Phase 3; gate logic unit-tested; never live-reached Stage::Commit in 4 attempts (models stalled earlier).
- [x] **SQLite-vec vector-store backend** — real create/write/query, second VectorStore impl (ROADMAP Phase 3); `--vector-provider sqlite-vec` / `--sqlite-vec-path`; Chroma admin-subcommand parity deliberately not built (providers/vector/sqlite_vec).
- [x] **`--debug-sequence` breakpoints** — `--step`, `--breakpoint <role>` (ROADMAP Phase 2); debug_stage_machine only, never wired into real runs.
- [x] **Paragraph chunking for non-code files with no ctags symbols** — chunk_by_paragraph (core/index.rs); also fixed md/txt language detection (2026-08-04).
- [x] **Document summarization before Worker on large RAG retrievals** — maybe_summarize_retrieved_docs (doc_summary.rs); no-op without a Summarizer configured.
- [x] **Multi-collection queries** — Query.collection is Vec<String>, each queried/reranked independently; fixed clap short-flag collisions across 4 files.
- [x] **Closed debug-mode fixture gap** — 2 of 11 agent_debug/*.json fixtures never actually run by a test (2026-08-04).
- [x] **`ruchat chroma-init`** — reads db_config.json, get_or_create_collection per entry (providers/vector/chroma/init.rs).
- [x] **Anthropic (Claude) opt-in chat provider** — chat only, no embeddings API; RAG/memorize stay Ollama-only; `--chat-provider anthropic`; Orchestrator's ollama field split into chat/embed (providers/llm/anthropic/).
- [x] **Agentic evals** — live-model behavioral tests, `#[ignore]`d, run via `--ignored agent_eval` (agent/evals.rs); 3 starter evals; Architect one is genuinely flaky by design.
- [x] **`replace_in_file` tried then reverted** — no real-run improvement over diffs; diff syntax wasn't the failure mode; apply_patch stays the sole write tool.
- [x] **Sped up cargo build and ruchat index** — lld linker (.cargo/config.toml); bounded-concurrency ctags/embed phases (core/index.rs).
- [x] **Fixed memory recall always querying collection "default"** — fixed Architect repeating identical plan after file content disproved assumption (recall_prior_memories; architect.md).
- [x] **Run summary lists every contributing issue, wrapped at 120 chars** — (run_summary.rs, shared utils::text::wrap_line).
- [x] **Trace readability: Validator VALIDATED verdicts, approving critic reviews, Scoper raw output** — previously left no trace turn; now pushed unconditionally.
- [x] **Trace readability: Worker's read-only tool-call actions and apply_patch diffs** — weren't recorded / rendered as unreadable \n-escaped line; fixed render_turn_content_for_trace.
- [x] **Fixed CONTAINS on scalar metadata fields always returning zero rows** — Chroma's filter language has no scalar-substring op; added client-side metadata_matches evaluator (where.rs).
- [x] **Memory recall works without Librarian configured** — memorize-only runs can recall what they wrote; EmbedArgs gained collection_name/embed_model_name/client accessors.
- [x] **Wired real producer for AgentEvent::Progress** — progress_pct, sent from Stage::Plan each round.
- [x] **Chroma unreachable during Librarian retrieval degrades gracefully** — run_librarian_retrieval degrades; Ollama-unreachable-at-start left alone (can't proceed regardless).
- [x] **Fixed `pipe` regression: "No model specified" on startup** — resolve_model_slot_count; OllamaArgs::init's .max(1) had forced resolution even for empty defaults.
- [x] **`apply_patch` clear rejection for diffs spanning two files** — no longer cryptic parse crash; cleaned Librarian retrieval noise (raw Debug metadata, uncapped references list); switched ruchat index's file walk to `git ls-files` instead of unscoped recursive walk.
- [x] **Trace file overhaul** — one file per run (ruchat_traces/ruchat_trace_<N>.md), full unfiltered trace body, LLM-generated outcome summary on every run, archived to successes/ or failures/ (run_summary.rs, formerly postmortem.rs).
- [x] **Prompt-engineering pass over every agent_role/*.md template** — moved "no human available" rule into shared system message; strengthened validator.md/summarizer.md/critic.md (quality pass, no bug reported).
- [x] **`read_tags` auto-regenerates missing/stale tags file** — always scoped to `git ls-files -- '*.rs'` via stdin, never raw recursive walk; root-caused real incident: tags grew to 494MB/2.5M lines after recursive ctags sweep swept in gitignored docs/ dir; regenerated at 108KB.
- [x] **`apply_patch` rejection shows file's real current content (numbered)** — not just diffy's raw error; after a run showed Worker fabricating nonexistent function signature.
- [x] **Worker calling read-only tool twice in a round** — clearer rejection + proactive reminder right after first tool result.
- [x] **Worker replying with narrative walkthrough instead of tool_call** — rejected deterministically on first no-tool-call response, not via LLM Validator; architect.md/worker.md name and reject this pattern explicitly.
- [x] **Two `apply_patch` diff-parsing fixes from real failed run** — wrong hunk-header line counts; no --- a/+++ b/ headers (fix_hunk_header_counts, protocol.rs).
- [x] **Multi-file patches per round** — Stage::Implement loops up to 3-call patch_budget (should_continue_patch_loop); pending_patch became pending_patches: Vec<PendingPatch>.
- [x] **`cargo_clippy` typed Worker tool** — mirrors cargo_check's plain-text shape.
- [x] **Commit message body lines hard-wrapped at 80 chars** — models didn't reliably honor prompt instruction; fixed missing newline gluing role banners (wrap_commit_message_body, render.rs).
- [x] **Removed "querying 'model'" trace line spam** — model_summary() prints configured models once at run start instead.
- [x] **Three multi-critic run bugs** — commit_feature_branch staged whole working tree instead of just AI's change; commit messages were fixed uninformative string, now LLM-generated from real staged diff; concurrent critics interleaved onto same channel, each critic now gets its own local channel.
- [x] **Fixed secret-leaking log lines** — Librarian setup and --agentic parsing echoed raw config string (embeds chroma_token) on parse failure.
- [x] **Refreshed comparisons/*_COMPARISON.md** — all four described removed generic SHELL tool and old flat-string Context; updated all four; added Safety/Sandboxing row to each.
- [x] **Bug making Librarian RAG retrieval silently render as empty text** — OutputArgs derived Default, but clap default_value only applies via Parser::parse_from; every non-CLI construction (incl. real Librarian path) got empty fields list; added manual impl Default for OutputArgs.
- [x] **Automatic memory recall at session start** — before Stage::Scope (recall_prior_memories); deterministic query from ctx.goal; no-op before anything's memorized.
- [x] **Resource-limited sandboxing for cargo subprocesses** — RLIMIT_AS (4GiB) + RLIMIT_CPU via pre_exec, inherited by every child rustc/build-script process (orchestrator::cargo::limit_resources); Unix-only.
- [x] **BuildReport::rejection_message() surfaces parsed compile errors** — file:line:col and warnings to Worker, not just raw diagnostics string; fixed warnings-only compiles rendering as empty.
- [x] **Fixed model_options file/config merge being silent no-op** — gate checked serialized ModelOptions::default(), always `{}`; replaced with explicit MODEL_OPTION_KEYS allowlist (cli/options.rs).
- [x] **`apply_patch` scope check against Architect's plan** — plan's FILES: line bounds which files apply_patch accepts (Context::planned_files, protocol.rs); fails open when FILES: absent.
- [x] **v0.2.0 released** (2026-08-03).
- [x] **Fixed flaky test: two option-file tests raced on same relative path** — switched to tempfile::tempdir().
- [x] **Removed double JSON round-trip in ModelArgs::build_generation_request** — options::merge_options_json; surfaced model_options no-op bug.
- [x] **Global config file with profile support** — turned out to already exist and work (~/.config/ruchat/config.json, --profile); deleted dead duplicate reader (cli/serde.rs::read_config_file).
- [x] **Migrated diagnostic println!/eprintln! in src/core, src/providers to tracing** — left each subcommand's designed stdout output untouched.
- [x] **Fixed error handlers discarding useful diagnostic info** — model-not-found vs. Ollama-unreachable; ToolParseError::UnknownTool now carries actual bad name; replaced unwrap() in func_struct's chat loop and ~8 is_string()/unwrap() pairs.
- [x] **Added unit tests for include.rs/where.rs parse()/update_from_json() and cli/prompt.rs** — previously zero coverage.
- [x] **Fixed `cargo test --lib` uncompilable and `-h` test killing suite** — 33 errors from stale test code; clap exit(0) mid-suite silently killed other tests.
- [x] **Multi-critic consensus completely non-functional** — Agent::new's config lookup couldn't find flat critic config; Role::from_str didn't recognize "Critic_0"/"Critic_1" naming (orchestrator.rs, agent/role.rs).
- [x] **Wired 9 of 10 agent_debug/*.json fixtures into cargo test --lib** — new FakeLlmClient; fixed fixture naming bug ("Critic0" vs "Critic_0") that made multi-critic dispatch silently no-op.
- [x] **Added .github/workflows/ci.yml** — build + clippy + test on push/PR; no -D warnings/fmt --check yet.
- [x] **Investigated connection pooling** — already satisfied, one shared Arc<Client> per run, reqwest's default pooling applies.
- [x] **Consolidated TODO files into single TODO.md**.
- [x] **Improved model option merging with CLI flags**.
- [x] **env_logger / tracing integration**.
- [x] **Basic multi-agent orchestration with RAG support**.
- [x] **Git auto-commit feature branch on approval**.
- [x] **Robust Chroma CLI with where/include parsing**.
- [x] **Structured tool calling framework** — `agent/tools.rs::ToolName`, schema-validated, 13 typed tools (apply_patch, git_*, read_file, ripgrep, read_tags, cargo_check/cargo_dupes).
- [x] **Parallel critic execution** — Orchestrator::run_critics_parallel, futures_util::future::join_all.
- [x] **RAG relevance scoring / reranking** — providers/vector/chroma/rerank.rs, distance+lexical blend.
- [x] **Token-aware history management with automatic Summarizer trigger** — Stage::Retry, get_dynamic_history_limit.
- [x] **Pre-planning repo-grounding stage** — Scoper role (not in original TODO/ROADMAP).
- [x] **Structured Context event log** — Vec<Turn> + TurnKind replacing old flat-string history/context/documents/rejections.
- [x] **Reconciled legacy Team/Manager pipeline** — ruchat manager now runs saved Team preset through real Orchestrator instead of separate unvalidated linear engine.
- [x] **`apply_patch` diff-size cap and automatic rollback** — MAX_PATCH_DIFF_BYTES (agent/protocol.rs); Context::{record_patch,revert_pending_patch}.
- [x] **Confirmed "remove dead code" for conversation_tree.rs/legacy Team/Manager fully resolved**.
- [x] **Removed unused OrchestratorRun struct** — stale leftover, AgentPipeline is an enum; ask.rs/manager.rs already construct AgentPipeline::Orchestrator directly.
