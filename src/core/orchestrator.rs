pub(crate) mod cargo;
pub(crate) mod checkpoint;
mod critique;
mod debug;
pub(crate) mod doc_summary;
pub(crate) mod fs;
pub(crate) mod git;
mod implement;
mod retrieval;
pub(crate) mod run_summary;
pub(crate) mod scope;
mod scope_stage;
pub(crate) mod search;
mod stage_machine;
mod stall_mitigation;
pub(super) mod task;
#[cfg(test)]
mod test_support;
mod worker_tools;

use crate::agent::Agent;
use crate::agent::event::StreamItem;
use crate::providers::vector::chroma::ChromaClientConfigArgs;
use crate::{Result, RuChatError};
use serde_json::Value;
pub(super) use task::TaskType;
use tokio::sync::mpsc;
use tokio_stream::Stream;
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::sync::CancellationToken;
// Define what the UI receives
pub type OrchestratorResult = Result<StreamItem>;
use crate::agent::llm_client::{LlmClient, VectorStore};
use serde::Deserialize;
use std::sync::Arc;

// Serialize/Deserialize: `Stage` is one of the fields `checkpoint.rs` persists across a
// resumable run — see that module for the full "Resumable/crash-resilient runs" mechanism.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
enum Stage {
    Scope,
    Plan,
    Retrieve,
    Implement,
    Test,
    Validate,
    Critique,
    Reconcile,
    Accept,
    Commit,
    Retry,
    Escalate(String),
    Done,
}

/// Everything `Orchestrator::run_task_stream` needs beyond `goal` and `cancel`, bundled to avoid
/// tripping `clippy::too_many_arguments`.
pub(crate) struct RunTaskOptions {
    pub(crate) debug_sequence: Option<String>,
    pub(crate) breakpoints: DebugBreakpoints,
    pub(crate) resume: bool,
    pub(crate) approve_commit: bool,
    pub(crate) trace_timings: bool,
}

/// Where a `--debug-sequence` run should pause for interactive step-by-step inspection —
/// `debug_stage_machine` only, never the real `run_stage_machine`, since this is specifically a
/// developer diagnostic tool for reproducing/inspecting a fixed role sequence, not something a
/// real (potentially unattended) run should ever block on. `Default` (both fields empty/false)
/// means no pausing at all — the original, fully unattended behavior, still the default when
/// neither `--step` nor `--breakpoint` is given.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct DebugBreakpoints {
    /// Pause after every role in the sequence.
    step: bool,
    /// Pause only after these specific role names (e.g. `["Worker", "Validator"]`).
    roles: Vec<String>,
}

impl DebugBreakpoints {
    pub(crate) fn new(step: bool, roles: Vec<String>) -> Self {
        Self { step, roles }
    }

    fn should_pause_after(&self, role: &str) -> bool {
        self.step || self.roles.iter().any(|r| r == role)
    }
}

pub(crate) struct Orchestrator {
    // Core pipeline
    scoper: Option<Agent>,
    architect: Agent,
    librarian: Option<Agent>,
    worker: Agent,
    // Consensus pipeline: All of these must return their specific approval signal
    critics: Vec<Agent>,
    summarizer: Option<Agent>,
    validator: Option<Agent>,
    orchestrator_config: Value,
    // Split from a single shared client (2026-08-04): Anthropic has no embeddings API, so
    // `embed` must always stay Ollama-backed while `chat` can be an opt-in Anthropic client
    // (`--chat-provider anthropic`, see `providers/llm/ollama/ask.rs`). Every existing call
    // site was already unambiguously one or the other — `query_stream`/`chat_stream` uses
    // became `chat`, `generate_embeddings`/`Query::query` uses became `embed`.
    chat: Arc<dyn LlmClient>,
    embed: Arc<dyn LlmClient>,
    client: Option<Arc<dyn VectorStore>>,
}

impl Orchestrator {
    pub(crate) async fn new(
        mut orchestrator_config: Value,
        chat: Arc<dyn LlmClient>,
        embed: Arc<dyn LlmClient>,
        cfg: &Value,
    ) -> Result<Self> {
        let task_type = TaskType::deserialize(&orchestrator_config).ok();
        let task_type = task_type.as_ref();
        // 1. Extract Core Agents
        let architect = Agent::new(
            &mut orchestrator_config,
            "Architect",
            true,
            task_type,
            cfg.clone(),
        )
        .await?;
        let worker = Agent::new(
            &mut orchestrator_config,
            "Worker",
            true,
            task_type,
            cfg.clone(),
        )
        .await?;

        let validator = Agent::new(
            &mut orchestrator_config,
            "Validator",
            false,
            task_type,
            cfg.clone(),
        )
        .await
        .ok();
        let summarizer = Agent::new(
            &mut orchestrator_config,
            "Summarizer",
            false,
            None,
            cfg.clone(),
        )
        .await
        .ok();
        let scoper = Agent::new(
            &mut orchestrator_config,
            "Scoper",
            false,
            task_type,
            cfg.clone(),
        )
        .await
        .ok();

        let mut critics = Vec::new();
        if let Some(critic_list) = orchestrator_config
            .get("Critics")
            .and_then(|v| v.as_array())
        {
            for (i, c_val) in critic_list.iter().enumerate() {
                // `Agent::new` looks up `config.get(role)` — it expects the role's
                // config nested under its own key (as every other role's config is
                // in `orchestrator_config`), not the flat per-critic object itself.
                // Passing `c_val.clone()` directly here used to mean `config.get(role)`
                // could never find anything, so `Agent::new` always returned
                // `Err(MissingAgent)`, silently swallowed below — Critics were never
                // actually constructed via `--agentic`/`--critic`, regardless of how
                // many were configured.
                let role = format!("Critic_{i}");
                let mut c_config = serde_json::json!({ role.clone(): c_val.clone() });
                if let Ok(agent) =
                    Agent::new(&mut c_config, &role, true, task_type, cfg.clone()).await
                {
                    critics.push(agent);
                }
            }
        }

        let mut librarian = None;
        let mut client = None;
        if let Ok(mut lib) = Agent::new(
            &mut orchestrator_config,
            "Librarian",
            false,
            None,
            cfg.clone(),
        )
        .await
        {
            // `vector_provider` (Librarian role config key, mirrors `EmbedArgs`'s
            // `--vector-provider`) picks which backend `chroma_client`/`sqlite_vec_client`
            // below is interpreted as — defaults to Chroma, unchanged from before this key
            // existed.
            let provider = lib
                .remove_str("vector_provider")
                .ok()
                .and_then(|s| serde_json::from_value(serde_json::Value::String(s)).ok())
                .unwrap_or(crate::VectorProvider::Chroma);
            let concrete_client: Arc<dyn VectorStore> = match provider {
                crate::VectorProvider::Chroma => {
                    let mut client_config = ChromaClientConfigArgs::default();
                    lib.remove_str("chroma_client").and_then(|s| {
                        let val = s.parse::<serde_json::Value>()?;
                        client_config.update_from_json(&val).map_err(|e| {
                            // Deliberately not logging `s` itself: `chroma_client` config
                            // legitimately carries `chroma_token` (a secret), and the parse
                            // error already includes enough position/context to debug without
                            // echoing the raw string.
                            tracing::error!(error = ?e, "Failed to parse chroma_client config as JSON");
                            e
                        }).map_err(RuChatError::AnyhowError)
                    })?;
                    Arc::new(
                        client_config
                            .create_client(cfg)
                            .await
                            .map_err(RuChatError::AnyhowError)?,
                    ) as Arc<dyn VectorStore>
                }
                crate::VectorProvider::SqliteVec => {
                    let mut client_config = crate::sqlite_vec::SqliteVecClientConfigArgs::default();
                    if let Ok(s) = lib.remove_str("sqlite_vec_client") {
                        let val: serde_json::Value = s
                            .parse()
                            .map_err(|e: serde_json::Error| RuChatError::AnyhowError(e.into()))?;
                        client_config.update_from_json(&val)?;
                    }
                    Arc::new(client_config.create_client().await?) as Arc<dyn VectorStore>
                }
            };
            client = Some(concrete_client);

            librarian = Some(lib);
        }

        // No Librarian configured (or its client failed to build): `recall_prior_memories`
        // still needs a Chroma client to look up whatever the Worker's `Memorize` tool call
        // writes via `Agent::embed` (the Worker's own `embed_args`) — otherwise a memorize-only
        // run could write memories it could never recall (see `TODO.md`). Gated on the Worker
        // actually having `embed_args` explicitly configured, not `EmbedArgs::default()`'s
        // fallback: unlike a Memorize *call* (an explicit action the Worker only takes when it
        // decides to), this runs unconditionally at the start of every single run, so silently
        // defaulting here would mean every run — including ones with no interest in Chroma at
        // all — pays for an attempted connection to `EmbedArgs::default()`'s literal
        // `localhost:8000`/`"default"` collection. Built here, once, at construction time
        // rather than per-recall-call so `recall_prior_memories` itself stays a synchronous,
        // no-network-if-`client`-is-`None` read of `self.client` — the same reason
        // `build_test_orchestrator`'s hand-built `Orchestrator` literals can set `client`
        // directly and keep the existing fixture tests fully offline. A failure to build here
        // (e.g. an invalid `chroma_server` URL) is swallowed, not propagated: recall is
        // best-effort everywhere else too (see the doc comment on `recall_prior_memories`).
        if client.is_none()
            && let Some(embed_args) = worker.embed_args.as_ref()
            && let Ok(concrete_client) = embed_args.client(cfg).await
        {
            // `EmbedArgs::client` already returns `Arc<dyn VectorStore>` (and already respects
            // `vector_provider`) — no extra wrapping needed here.
            client = Some(concrete_client);
        }

        Ok(Self {
            scoper,
            architect,
            worker,
            validator,
            summarizer,
            critics,
            librarian,
            orchestrator_config,
            chat,
            embed,
            client,
        })
    }

    /// `cancel` is caller-owned rather than created here, so a Ctrl-C handler sitting above the
    /// stream (`render_pipeline_stream`) can trigger it directly — see that function's doc
    /// comment. Before this, the only way to stop a run early was the OS's default SIGINT
    /// handling, which kills the process outright with no chance for `finalize_trace` to run:
    /// confirmed live 2026-08-05 (traces 554-556 sat unarchived even after the `?`-early-return
    /// fix below, because none of them ever produced an `Err` for that fix to catch — they were
    /// killed from outside the program entirely).
    pub(crate) fn run_task_stream(
        mut self,
        goal: String,
        options: RunTaskOptions,
        cancel: CancellationToken,
    ) -> impl Stream<Item = OrchestratorResult> {
        let RunTaskOptions {
            debug_sequence,
            breakpoints,
            resume,
            approve_commit,
            trace_timings,
        } = options;
        let (tx, rx) = mpsc::channel(100);
        let (done_tx, done_rx) = tokio::sync::oneshot::channel::<()>();

        // Watcher now races two exits: receiver dropped early (cancellation),
        // or the task below finished normally (done_rx). Either way this task
        // exits and drops `watcher_tx` — without that second branch, watcher_tx
        // is held alive forever on a clean run, since closed() only fires on
        // receiver drop, and the receiver only drops once the stream ends,
        // which never happens while watcher_tx keeps the channel open. That
        // was the deadlock.
        let watcher_tx = tx.clone();
        let watcher_cancel = cancel.clone();
        tokio::spawn(async move {
            tokio::select! {
                _ = watcher_tx.closed() => {
                    watcher_cancel.cancel();
                }
                _ = done_rx => {
                    // Normal completion — just exit and drop watcher_tx.
                }
            }
        });

        let task_cancel = cancel.clone();
        tokio::spawn(async move {
            let result = if let Some(path) = debug_sequence {
                self.debug_stage_machine(goal, path, tx.clone(), task_cancel, breakpoints)
                    .await
            } else {
                self.run_stage_machine(
                    goal,
                    tx.clone(),
                    task_cancel,
                    resume,
                    approve_commit,
                    trace_timings,
                )
                .await
            };
            if let Err(e) = result
                && !matches!(e, RuChatError::Cancelled)
            {
                let _ = tx.send(Err(e)).await;
            }
            let _ = done_tx.send(());
        });

        ReceiverStream::new(rx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::llm_client::{FakeLlmClient, VectorCollection};
    use serde_json::json;
    use test_support::base_config;

    // Regression: `Orchestrator::new`'s Librarian setup used to log the raw `chroma_client`
    // config string verbatim on parse failure (`config = %s`). That string legitimately carries
    // `chroma_token` (a secret), so a malformed-but-token-bearing config leaked it to logs.
    // Uses the real `Orchestrator::new` (not `build_test_orchestrator`) since the bug is in its
    // Librarian-construction branch specifically.
    #[test]
    fn librarian_chroma_client_parse_failure_does_not_log_the_raw_config() {
        use crate::agent::llm_client::FakeLlmClient;
        use std::io::Write;
        use std::sync::{Arc, Mutex};

        #[derive(Clone)]
        struct BufWriter(Arc<Mutex<Vec<u8>>>);
        impl Write for BufWriter {
            fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(data);
                Ok(data.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }

        let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
        let writer = BufWriter(buf.clone());
        let subscriber = tracing_subscriber::fmt()
            .with_writer(move || writer.clone())
            .finish();

        let secret = "super-secret-chroma-token-xyz";
        // Valid JSON syntax (so the outer `s.parse::<Value>()` succeeds) but neither a string
        // nor an object, so `ChromaClientConfigArgs::update_from_json` — the call this test
        // targets — rejects it via its `val.as_object().ok_or_else(...)` branch, with the
        // secret embedded in the string that would have been logged pre-fix.
        let malformed = format!(r#"["chroma_token", "{secret}"]"#);
        let config = json!({
            "Architect": { "model": "fake" },
            "Worker": { "model": "fake" },
            "Librarian": { "model": "fake", "embed_model": "fake-embed", "chroma_client": malformed },
        });

        // Current-thread runtime, deliberately: `with_default` sets a thread-local subscriber,
        // so `Orchestrator::new`'s `tracing::error!` call must run on this same thread to be
        // captured — a multi-thread runtime would poll it on a worker thread instead, silently
        // making this assertion vacuous (nothing captured, so it trivially "contains no secret").
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let result = tracing::subscriber::with_default(subscriber, || {
            rt.block_on(Orchestrator::new(
                config,
                Arc::new(FakeLlmClient::new(vec![])),
                Arc::new(FakeLlmClient::new(vec![])),
                &json!({}),
            ))
        });

        assert!(
            result.is_err(),
            "malformed chroma_client JSON should still be rejected"
        );
        let logged = String::from_utf8(buf.lock().unwrap().clone()).unwrap();
        assert!(
            !logged.contains(secret),
            "secret leaked into log output: {logged}"
        );
    }

    // Regression canary for debug-mode breakpoint support (maintainer: "keep on working on
    // roadmap entries, overnight"). `should_pause_after` is the one piece of this feature with
    // real branching logic that doesn't need a live terminal to exercise — the actual pause
    // (blocking stdin read via `Io`) was instead verified live: `--debug-sequence
    // agent_debug/architect_only.json --step` against a real Ollama server, confirming the
    // breakpoint trace message appears correctly ordered after `print_debug_info`'s state dump
    // (both go through the same `ctx.trace` channel, avoiding a race with a direct stdout
    // write), that a piped Enter resumes to completion, and that 'q' aborts cleanly.
    #[test]
    fn debug_breakpoints_default_never_pauses() {
        let bp = DebugBreakpoints::default();
        assert!(!bp.should_pause_after("Architect"));
        assert!(!bp.should_pause_after("Worker"));
    }

    #[test]
    fn debug_breakpoints_step_pauses_after_every_role() {
        let bp = DebugBreakpoints::new(true, vec![]);
        assert!(bp.should_pause_after("Architect"));
        assert!(bp.should_pause_after("AnyRoleAtAll"));
    }

    #[test]
    fn debug_breakpoints_named_pauses_only_after_listed_roles() {
        let bp = DebugBreakpoints::new(false, vec!["Worker".to_string(), "Validator".to_string()]);
        assert!(bp.should_pause_after("Worker"));
        assert!(bp.should_pause_after("Validator"));
        assert!(!bp.should_pause_after("Architect"));
        assert!(!bp.should_pause_after("Librarian"));
    }

    // Exercises `Orchestrator::new`'s actual branch-selection logic for
    // `Librarian.vector_provider` (unlike every other Librarian test above,
    // which goes through `build_test_orchestrator`'s hand-built bypass
    // specifically to avoid constructing a real Chroma client) — this is the
    // one test that runs the real constructor end-to-end for the SQLite-vec
    // path, against a real on-disk database seeded with real content ahead
    // of time (no live Ollama needed for the embeddings themselves; raw
    // vectors are enough to prove the KNN round trip through the whole
    // `Orchestrator::new` -> `self.client` -> `query_collection` chain).
    #[tokio::test]
    async fn orchestrator_new_builds_a_working_sqlite_vec_client_when_librarian_requests_it() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("librarian.sqlite3");

        {
            let client = crate::sqlite_vec::SqliteVecClient::open(&db_path).unwrap();
            let collection = client.collection("notes").unwrap();
            collection
                .add(
                    vec!["a".into()],
                    vec![vec![1.0, 0.0]],
                    Some(vec![Some("seeded real content".into())]),
                    None,
                )
                .await
                .unwrap();
        }

        let mut config = base_config();
        config["Librarian"] = json!({
            "model": "fake",
            "vector_provider": "sqlite-vec",
            "sqlite_vec_client": { "sqlite_vec_path": db_path.to_str().unwrap() },
        });

        let orchestrator = Orchestrator::new(
            config,
            Arc::new(FakeLlmClient::new(vec![])),
            Arc::new(FakeLlmClient::new(vec![])),
            &json!({}),
        )
        .await
        .unwrap();

        let client = orchestrator
            .client
            .expect("Librarian's sqlite-vec client should have been built");
        let response = client
            .query_collection("notes", vec![vec![0.9, 0.1]], Some(1), None, None, None)
            .await
            .unwrap();
        assert_eq!(
            response.documents.unwrap()[0],
            vec![Some("seeded real content".to_string())]
        );
    }
}
