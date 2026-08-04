pub(crate) mod cargo;
pub(crate) mod checkpoint;
pub(crate) mod doc_summary;
pub(crate) mod fs;
pub(crate) mod git;
pub(crate) mod run_summary;
pub(crate) mod scope;
pub(crate) mod search;
pub(super) mod task;

use crate::agent::Agent;
use crate::agent::event::{AgentEvent, StreamItem};
use crate::agent::protocol::Validation;
use crate::agent::tools::{self, ToolName};
use crate::agent::types::{Context, TurnKind};
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
use super::agent::json_extract::strip_json_fences;
use crate::agent::llm_client::{LlmClient, VectorStore};
use crate::providers::vector::chroma::query::Query;
use crate::retry_transient;
use git::commit_feature_branch;
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

#[derive(serde::Deserialize)]
struct ValidatorVerdict {
    verdict: String,
    #[serde(default)]
    reason: String,
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

/// Whether a `--approve` commit-gate answer counts as approval — deliberately strict (an exact
/// "y"/"yes", case-insensitive-ish via explicit variants, trimmed) rather than "anything not
/// starting with n": a HITL approval gate that defaults to yes on ambiguous or accidental input
/// (a stray keystroke, a blank line from a fumbled Enter) would defeat the entire point of the
/// gate. Everything else, including an empty line, counts as rejection.
fn is_approval_yes(answer: &str) -> bool {
    matches!(answer.trim(), "y" | "Y" | "yes" | "Yes")
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

    pub(crate) fn run_task_stream(
        mut self,
        goal: String,
        debug_sequence: Option<String>,
        breakpoints: DebugBreakpoints,
        resume: bool,
        approve_commit: bool,
    ) -> impl Stream<Item = OrchestratorResult> {
        let (tx, rx) = mpsc::channel(100);
        let cancel = CancellationToken::new();
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
                self.run_stage_machine(goal, tx.clone(), task_cancel, resume, approve_commit)
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

    async fn run_critics_parallel(
        &mut self,
        ctx: &mut Context,
        tx: &mpsc::Sender<OrchestratorResult>,
    ) -> Result<()> {
        let snapshot_output = ctx.output.clone();
        let snapshot_plan_impl = ctx.context_view();
        let mut futs = Vec::new();
        let round = ctx.round;
        for critic in &mut self.critics {
            let approval_signal = critic
                .get_str("approval_signal")
                .unwrap_or("APPROVED")
                .to_string();
            let label = critic
                .get_str("name")
                .or_else(|_| critic.get_str("role"))
                .unwrap_or("Critic")
                .to_string();
            let mut scratch = Context::new(ctx.goal.clone());
            scratch.output = snapshot_output.clone();
            scratch.push_turn(
                TurnKind::Implementation,
                "snapshot",
                snapshot_plan_impl.clone(),
            );
            scratch.round = round;
            let ollama = &self.chat;
            futs.push(async move {
                // Critics run concurrently (`join_all` below), but `query_stream` streams
                // token-by-token into whatever `tx` it's given — forwarding all of them into
                // one shared channel interleaves multiple critics' output character-by-
                // character with no way for a renderer to tell them apart. So each critic
                // streams into its own local channel instead; nobody renders it live, a
                // background task just drains it to avoid blocking `query_stream` on a full
                // buffer. The caller emits one clearly-labeled, complete block per critic on
                // the real `tx` afterward, sequentially, once every critic has finished.
                let (local_tx, mut local_rx) = mpsc::channel(100);
                let drain = async { while local_rx.recv().await.is_some() {} };
                let query = async {
                    let r = retry_transient!(critic.query_stream(ollama, &mut scratch, &local_tx));
                    drop(local_tx);
                    r
                };
                let (result, ()) = tokio::join!(query, drain);
                result.map(|_| (label, scratch.output, approval_signal))
            });
        }
        let results = futures_util::future::join_all(futs).await;
        for res in results {
            match res {
                Ok((label, text, approval_signal)) => {
                    ctx.trace(tx, format!("[Critic '{label}']:\n{text}")).await;
                    let source = format!("Critic '{label}'");
                    if !text.contains(&approval_signal) {
                        ctx.push_turn(TurnKind::Rejection, &source, text);
                    } else {
                        // Unlike the rejection arm above, an approving critic's review used to
                        // push no turn at all — only the ephemeral `ctx.trace(...)` call above
                        // saw it, which shows up live on the console/event stream but is never
                        // added to `ctx.turns`, so it's gone from the persisted trace file the
                        // next time it's rewritten. An approving review is still an action this
                        // critic took and should be just as visible as a rejecting one.
                        ctx.push_turn(TurnKind::System, &source, text);
                    }
                }
                Err(e) => {
                    // A critic that exhausts retries must count as a
                    // rejection, not a silent no-op — otherwise an
                    // unreachable/erroring critic is indistinguishable from
                    // an approving one, inverting the consensus gate's intent.
                    ctx.push_turn(
                        TurnKind::Rejection,
                        "Critic",
                        format!("critic failed to produce a verdict: {e}"),
                    );
                }
            }
        }
        Ok(())
    }

    /// Retrieved documents at or above this size are worth spending an LLM call to compress
    /// before they reach the Worker's prompt — below it, the token savings wouldn't justify the
    /// extra round trip (or the small risk of the compression step itself introducing an
    /// error). A fixed threshold, not proportional to the model's context window: a single
    /// retrieval being "a few dense paragraphs" is the right trigger regardless of how large the
    /// overall history budget happens to be.
    const DOC_SUMMARIZATION_THRESHOLD_TOKENS: u64 = 800;

    /// Compresses `docs` (raw retrieved RAG content) before it's pushed as a `TurnKind::
    /// Retrieval` turn, if a Summarizer is configured and `docs` is large enough to be worth it
    /// (see `DOC_SUMMARIZATION_THRESHOLD_TOKENS`). Reuses the Summarizer's configured *model*,
    /// not its `agent_role/summarizer.md` *template* (that template is specifically about
    /// compressing round history, not retrieved documents — seeing
    /// `doc_summary::summarize_retrieved_documents`'s own doc comment for why a distinct prompt
    /// is used instead). Opt-in the same way whole-history compression already is: a run with no
    /// Summarizer configured sees this as a complete no-op, identical to before this existed.
    /// Never fails the round: a summarization failure falls back to the original, uncompressed
    /// `docs` rather than losing the retrieval outright — a diagnostic nicety failing must never
    /// cost the round its actual context.
    async fn maybe_summarize_retrieved_docs(
        &self,
        docs: String,
        ctx: &mut Context,
        tx: &mpsc::Sender<OrchestratorResult>,
    ) -> String {
        let Some(summarizer) = self.summarizer.as_ref() else {
            return docs;
        };
        let before = crate::agent::tokens::count_tokens(&docs);
        if before < Self::DOC_SUMMARIZATION_THRESHOLD_TOKENS {
            return docs;
        }
        let model = summarizer.get_str("model").unwrap_or("");
        match doc_summary::summarize_retrieved_documents(&self.chat, model, &ctx.goal, &docs).await
        {
            Ok(summary) => {
                let after = crate::agent::tokens::count_tokens(&summary);
                ctx.trace(
                    tx,
                    format!(
                        "Condensed retrieved documents (~{before} → ~{after} tokens) before \
                         adding them to context."
                    ),
                )
                .await;
                summary
            }
            Err(e) => {
                tracing::warn!(error = %e, "document summarization failed; using raw retrieved content");
                docs
            }
        }
    }

    async fn run_librarian_retrieval(
        &mut self,
        ctx: &mut Context,
        tx: &mpsc::Sender<OrchestratorResult>,
    ) -> Result<()> {
        let client = self.client.as_ref().ok_or_else(|| {
            RuChatError::Is("Librarian provided without chroma client config".into())
        })?;
        let librarian = self
            .librarian
            .as_mut()
            .ok_or_else(|| RuChatError::Is("Librarian not enabled".into()))?;

        retry_transient!(librarian.query_stream(&self.chat, ctx, tx))?;

        let mut q = Query::default();
        match serde_json::from_str::<Value>(strip_json_fences(&ctx.output)) {
            Ok(json_val) => {
                let _ = q.update_from_json(json_val);
            }
            Err(parse_err) => {
                // One corrective re-ask before giving up, mirroring the
                // Validator's "unparseable == not silently ignored" stance.
                ctx.trace(
                    tx,
                    format!(
                        "Librarian output was not valid JSON ({parse_err}); re-prompting once."
                    ),
                )
                .await;
                ctx.push_turn(
                    crate::agent::types::TurnKind::System,
                    "System",
                    format!(
                        "Your previous response was not valid JSON: {parse_err}. \
                         Return ONLY the JSON object described in your instructions, \
                         no fences, no preamble."
                    ),
                );
                retry_transient!(librarian.query_stream(&self.chat, ctx, tx))?;
                match serde_json::from_str::<Value>(strip_json_fences(&ctx.output)) {
                    Ok(json_val) => {
                        let _ = q.update_from_json(json_val);
                    }
                    Err(e2) => {
                        ctx.trace(
                            tx,
                            format!(
                                "Librarian still not valid JSON after retry ({e2}) — skipping RAG"
                            ),
                        )
                        .await;
                    }
                }
            }
        }

        // Unlike the Librarian's own `query_stream` calls above (an Ollama call, retried by
        // `retry_transient!` and left to propagate — if Ollama itself is unreachable the whole
        // run is dead anyway, Architect/Worker need it too), a failure here is specifically the
        // Chroma-backed lookup (`Query::query` calls `client.query_collection`). Chroma being
        // down for this one on-demand retrieval must not kill Architect/Worker/Test/Commit, none
        // of which need RAG context to function — degrade gracefully instead, mirroring
        // `recall_prior_memories`'s same stance for the deterministic pre-run recall.
        match librarian
            .retrieve_and_generate(client, &self.embed, q)
            .await
        {
            Ok(docs) => {
                let docs = self.maybe_summarize_retrieved_docs(docs, ctx, tx).await;
                ctx.push_turn(TurnKind::Retrieval, "Librarian", docs);
            }
            Err(e) => {
                tracing::warn!(error = %e, "Librarian retrieval failed; continuing without RAG context");
                ctx.trace(
                    tx,
                    format!("Librarian retrieval skipped this round (retrieval failed): {e}"),
                )
                .await;
                ctx.push_turn(
                    TurnKind::System,
                    "System",
                    format!(
                        "RAG retrieval was skipped this round because the retrieval lookup \
                         failed (Chroma may be unreachable): {e}. Continuing without retrieved \
                         context."
                    ),
                );
            }
        }
        Ok(())
    }

    /// Recalls prior memories relevant to this run's goal, if any, before the stage machine
    /// begins. Unlike `run_librarian_retrieval` (the Librarian's on-demand, LLM-shaped query
    /// during `Stage::Retrieve`), this is deterministic — the goal text itself is the query,
    /// no LLM call needed to write a query spec, since there's no other context yet at session
    /// start to reason about narrowing it further. If a Librarian is configured, reuses its
    /// Chroma client/`embed_model`/`memory_collection` (set alongside `task_hint` in `ask.rs`).
    /// Otherwise falls back to wherever the Worker's `Memorize` tool call actually writes
    /// (`Agent::embed` → the Worker's own `embed_args`, or `EmbedArgs::default()` if unset) —
    /// so a memorize-only run with no Librarian at all can still recall what it wrote, instead
    /// of being permanently unable to (see `TODO.md`). Pushed as a `TurnKind::Retrieval` turn
    /// tagged "Memory" (not "Librarian") so it's distinguishable in `history_view`/traces from
    /// an on-demand retrieval, though both feed `documents_view` identically. Never fails the
    /// run: an empty/missing collection (e.g. the very first run, before anything has ever been
    /// memorized) is the normal case, not an error, so a query failure is traced and swallowed
    /// rather than propagated.
    async fn recall_prior_memories(
        &self,
        ctx: &mut Context,
        tx: &mpsc::Sender<OrchestratorResult>,
    ) {
        let Some(client) = self.client.as_ref() else {
            return;
        };

        let mut query_json = serde_json::json!({
            "query": [ctx.goal.clone()],
            "n_results": 3,
        });

        // Unlike `run_librarian_retrieval` (where the Librarian's own LLM picks a collection
        // name as part of its JSON query, guided by its `task_hint`), this ad-hoc pre-run
        // recall has no LLM step to ask — without an explicit "collection" key here,
        // `Query::default()`'s `ChromaCollectionConfigArgs::default()` falls back to the
        // literal collection named "default". With a Librarian configured, `memory_collection`
        // (set alongside `task_hint` in `ask.rs`) supplies the right one. Without one, fall
        // back to wherever the Worker's `Memorize` tool call actually writes (`Agent::embed` →
        // the Worker's own `embed_args`, or `EmbedArgs::default()` if unset) — `self.client`
        // itself was already resolved the same way in `Orchestrator::new` for exactly this case.
        let embed_model = if let Some(librarian) = self.librarian.as_ref() {
            if let Ok(collection) = librarian.get_str("memory_collection") {
                query_json["collection"] = serde_json::json!(collection);
            }
            librarian
                .get_str("embed_model")
                .unwrap_or("all-minilm:l6-v2")
                .to_string()
        } else {
            let embed_args = self.worker.embed_args.clone().unwrap_or_default();
            query_json["collection"] = serde_json::json!(embed_args.collection_name());
            embed_args.embed_model_name()
        };

        let mut q = Query::default();
        let _ = q.update_from_json(query_json);
        match q.query(client, &self.embed, &embed_model).await {
            Ok(docs) if !docs.trim().is_empty() => {
                let docs = self.maybe_summarize_retrieved_docs(docs, ctx, tx).await;
                ctx.push_turn(TurnKind::Retrieval, "Memory", docs);
            }
            Ok(_) => {}
            Err(e) => {
                ctx.trace(tx, format!("Memory recall skipped: {e}")).await;
            }
        }
    }

    /// One-line, once-per-run summary of which model each configured role uses. Printed a
    /// single time at the start of the run (see `run_stage_machine`) instead of repeating
    /// "querying 'model'..." on every single turn (every role, every round) — each role's own
    /// colored banner already identifies who's speaking once the run is underway, so restating
    /// the model there added noise without new information.
    fn model_summary(&self) -> String {
        let mut parts = vec![
            format!(
                "Architect={}",
                self.architect.get_str("model").unwrap_or("?")
            ),
            format!("Worker={}", self.worker.get_str("model").unwrap_or("?")),
        ];
        if let Some(a) = self.scoper.as_ref() {
            parts.push(format!("Scoper={}", a.get_str("model").unwrap_or("?")));
        }
        if let Some(a) = self.librarian.as_ref() {
            parts.push(format!("Librarian={}", a.get_str("model").unwrap_or("?")));
        }
        if let Some(a) = self.validator.as_ref() {
            parts.push(format!("Validator={}", a.get_str("model").unwrap_or("?")));
        }
        for c in &self.critics {
            let label = c
                .get_str("name")
                .or_else(|_| c.get_str("role"))
                .unwrap_or("Critic");
            parts.push(format!("{label}={}", c.get_str("model").unwrap_or("?")));
        }
        if let Some(a) = self.summarizer.as_ref() {
            parts.push(format!("Summarizer={}", a.get_str("model").unwrap_or("?")));
        }
        format!(
            "Models: {} — full prompts logged to ruchat_traces/ as the run progresses.",
            parts.join(", ")
        )
    }

    async fn run_stage_machine(
        &mut self,
        goal: String,
        tx: mpsc::Sender<OrchestratorResult>,
        cancel: CancellationToken,
        resume: bool,
        approve_commit: bool,
    ) -> Result<()> {
        let max_iterations = self
            .orchestrator_config
            .get("iterations")
            .and_then(|v| v.as_u64())
            .unwrap_or(3);
        let max_scope_iterations = self
            .orchestrator_config
            .get("scope_iterations")
            .and_then(|v| v.as_u64())
            .unwrap_or(7);
        // See `checkpoint.rs` for the full "Resumable/crash-resilient runs" mechanism (ROADMAP.md
        // Phase 3). A fresh run starts from `Context::new`/`Stage::Scope`, same as always; `--
        // resume` reloads the last-completed stage's checkpoint instead — `goal` above is
        // ignored in that case, since resuming continues the *same* task, not a new one.
        let (mut ctx, mut stage) = if resume {
            checkpoint::Checkpoint::load(std::path::Path::new(checkpoint::CHECKPOINT_PATH))
                .await?
                .into_context_and_stage()
        } else {
            (Context::new(goal), Stage::Scope)
        };
        let ctx = &mut ctx;
        if !resume {
            // A resumed `Context` already carries the `trace_index` its checkpoint captured —
            // allocating a fresh one here would start a new trace file and lose the pre-crash
            // history the old one had.
            ctx.init_trace_index().await;
        }

        if let Some(librarian) = self.librarian.as_ref() {
            ctx.read_config_file(
                librarian
                    .get_str("db_config_path")
                    .unwrap_or("db_config.json"),
            )?;
        }
        self.recall_prior_memories(ctx, &tx).await;
        ctx.trace(&tx, self.model_summary()).await;

        let mut retrieve_budget: u32 = 2; // conservative cap on Worker-initiated retrievals per run
        let mut scope_round = 0;
        let mut last_scope_output: Option<String> = None;
        let mut last_architect_output: Option<String> = None;
        // Set only by `Stage::Commit` succeeding — both `Stage::Escalate` and `Stage::Retry`'s
        // iteration-budget-exhausted branch reach `Stage::Done` without ever going through
        // Commit, and both are "the agents did not reach a successful, committed result" even
        // though only one of them is technically an escalation.
        let mut success = false;

        loop {
            // Checked once per stage transition — this is the boundary the
            // success metric refers to. It does NOT preempt a stage already
            // in flight (e.g. a 120s `cargo test` won't be killed mid-run by
            // this check alone); see `run_build_and_test`'s own cancellation
            // wrapping below for that case.
            if cancel.is_cancelled() {
                return Err(RuChatError::Cancelled);
            }
            stage = match stage {
                Stage::Done => {
                    // A deliberate, recorded outcome — not a crash — so there's nothing left to
                    // `--resume`; see `checkpoint.rs::Checkpoint::clear`'s doc comment.
                    checkpoint::Checkpoint::clear(std::path::Path::new(
                        checkpoint::CHECKPOINT_PATH,
                    ))
                    .await;
                    let _ = tx.send(Ok(StreamItem::Event(AgentEvent::Done))).await;
                    break;
                }
                Stage::Escalate(reason) => {
                    checkpoint::Checkpoint::clear(std::path::Path::new(
                        checkpoint::CHECKPOINT_PATH,
                    ))
                    .await;
                    ctx.trace(&tx, format!("ESCALATED: {reason}")).await;
                    break;
                }
                Stage::Plan => {
                    ctx.round += 1;
                    // Per-round progress signal for a user watching a long run — the natural
                    // per-round checkpoint the orchestrator already tracks via `ctx.round`/
                    // `max_iterations`. Fire-and-forget like the `Done` event below: a dropped
                    // receiver here is already handled by `cancel` on the next loop iteration,
                    // so there's nothing more useful to do with a send error than ignore it.
                    let pct = progress_pct(ctx.round, max_iterations);
                    let _ = tx
                        .send(Ok(StreamItem::Event(AgentEvent::Progress(pct))))
                        .await;
                    if ctx.round > max_iterations {
                        Stage::Escalate("max iterations reached without acceptance".into())
                    } else {
                        retry_transient!(self.architect.query_stream(&self.chat, ctx, &tx))?;
                        // `architect.md` explicitly forbids the Architect from ever emitting a
                        // `tool_call` — it's plan-only, no tools — but a live-verified run (see
                        // TODO.md's pinned reliability item) showed it doing so anyway, embedding
                        // a full, hallucinated `apply_patch` diff (a phantom comment line that
                        // didn't exist in the real file) right inside its "plan." The Worker then
                        // copied that exact broken diff verbatim, unchanged, across 3 separate
                        // rounds — it wasn't reasoning about the real file content at all, just
                        // parroting the Architect's fabrication. Stripped deterministically here,
                        // before the plan is ever stored: with nothing ready-made to copy, the
                        // Worker has to construct its own diff from DOCUMENTS/real content.
                        ctx.output = strip_architect_tool_call_hallucination(&ctx.output);
                        // A repeated plan is NOT treated as fatal. It used to trigger an
                        // immediate `Stage::Escalate` on the very first repeat — measured against
                        // real runs (see TODO.md's pinned reliability item), that was killing the
                        // overwhelming majority of them after just 1-2 of a configured 5-round
                        // `max_iterations` budget, well before it was exhausted: a repeated plan
                        // doesn't mean no progress is possible, since the Worker/Test/Validate
                        // stages can still behave differently this round (e.g. producing a
                        // corrected diff informed by a rejection now in context that wasn't
                        // there last round). `ctx.round > max_iterations` above already bounds a
                        // genuine infinite stall without a separate fast-fail — same posture the
                        // Scoper's own identical-output handling already uses (forces
                        // progression instead of escalating).
                        if let Some(prev) = &last_architect_output
                            && prev == &ctx.output
                        {
                            ctx.push_turn(
                                TurnKind::System,
                                "Orchestrator",
                                "Note: this plan is identical to the previous round's. If the \
                                rejection reason above suggests a different approach, use it \
                                now — otherwise proceeding with the same plan is fine as long \
                                as the implementation actually changes this round."
                                    .into(),
                            );
                        }
                        last_architect_output = Some(ctx.output.clone());
                        // Without this, context_view() never finds a Plan
                        // turn in a real run (only debug_stage_machine
                        // pushed one) — the Worker, Critics, and the
                        // Architect's own next round all read an empty
                        // "PLAN:" section and effectively improvise from
                        // scratch each round instead of building on it.
                        ctx.push_turn(TurnKind::Plan, "Architect", ctx.output.clone());
                        Stage::Retrieve
                    }
                }
                Stage::Retrieve => {
                    if ctx.round == 1 && self.librarian.is_some() {
                        self.run_librarian_retrieval(ctx, &tx).await?;
                    }
                    auto_ground_planned_file(ctx).await;
                    Stage::Implement
                }
                Stage::Implement => {
                    retry_transient!(self.worker.query_stream(&self.chat, ctx, &tx))?;

                    if let Ok(call) = tools::parse_tool_call(&ctx.output)
                        && is_read_only_worker_tool(&call.tool)
                        && retrieve_budget > 0
                    {
                        retrieve_budget -= 1;
                        // Records the read-only tool-call *action* itself — before
                        // `handle_structured_tool` runs it and the reask below overwrites
                        // `ctx.output` with the Worker's next response. Without this, the trace
                        // only ever showed the tool's *output* (the Retrieval turn
                        // `handle_structured_tool` pushes below), never what was actually
                        // called or with what arguments — the console showed the action (via
                        // the streamed response) but the trace file didn't, making it unclear
                        // after the fact which tool produced which result.
                        ctx.push_turn(TurnKind::Implementation, "Worker", ctx.output.clone());
                        // A failing tool call (bad git args, missing ripgrep, a
                        // vanished file) must not abort the whole run — same
                        // posture as the Scoper's identical dispatch below.
                        // Record the failure as a turn and let the Worker see
                        // it and try something else, instead of propagating a
                        // fatal error out of the stage machine entirely.
                        match self.handle_structured_tool(&call, ctx, &tx).await {
                            Err(e) => {
                                ctx.push_turn(
                                    TurnKind::System,
                                    "Orchestrator",
                                    format!("tool call failed: {e}"),
                                );
                            }
                            // Explicit, immediate reminder right before the reask — not just
                            // documented once in the prompt — since a rule stated at the top of
                            // a long prompt is easy for smaller local models to lose track of by
                            // generation time. Local models reliably keep calling the same (or
                            // another) read-only tool again instead of switching to act on a
                            // result they already have; `run_implement_patch_loop` rejects that
                            // if it happens anyway, but heading it off here avoids burning the
                            // round on a rejection at all.
                            Ok(()) => {
                                ctx.push_turn(
                                    TurnKind::System,
                                    "Orchestrator",
                                    "Tool result is above. You've used this round's one \
                                    information-lookup — you must now emit exactly one \
                                    apply_patch (or memorize) tool_call. Do not call another \
                                    read-only tool."
                                        .into(),
                                );
                            }
                        }
                        retry_transient!(self.worker.query_stream(&self.chat, ctx, &tx))?;

                        // Bounded second chance, in-round: real traces (see TODO.md's pinned
                        // reliability item) show the Worker very often ignoring the reminder
                        // above and calling a read-only tool again anyway (a mechanical local-
                        // model mistake — repeating a just-completed action) — which used to
                        // fall straight through to `execute_and_verify`'s rejection below and
                        // burn the *entire* round on it. One more sharper nudge-and-reask first:
                        // does NOT re-spend `retrieve_budget` or re-run the tool (its result is
                        // already in context, rerunning it adds nothing) — just a stronger,
                        // final reminder. If the Worker still won't switch after this, the
                        // existing `execute_and_verify` "called X again" rejection (agent.rs)
                        // takes over exactly as before and the round is spent via the normal
                        // `Stage::Retry` path — bounded, not an infinite loop.
                        if let Ok(repeat_call) = tools::parse_tool_call(&ctx.output)
                            && is_read_only_worker_tool(&repeat_call.tool)
                        {
                            ctx.push_turn(TurnKind::Implementation, "Worker", ctx.output.clone());
                            ctx.push_turn(
                                TurnKind::System,
                                "Orchestrator",
                                format!(
                                    "You called '{:?}' again — its result is already shown \
                                    above, calling it again will not run it a second time and \
                                    will not add anything new. This is your last chance this \
                                    round: emit exactly one apply_patch (or memorize) \
                                    tool_call now, with no other tool calls.",
                                    repeat_call.tool
                                ),
                            );
                            retry_transient!(self.worker.query_stream(&self.chat, ctx, &tx))?;
                        }
                    }
                    ctx.push_turn(TurnKind::Implementation, "Worker", ctx.output.clone());
                    self.run_implement_patch_loop(ctx, &tx).await?
                }
                Stage::Test => {
                    let report = Validation::run_build_and_test(&cancel).await?;
                    if !report.compiled || !report.tests_passed {
                        ctx.push_turn(TurnKind::Rejection, "Tester", report.rejection_message());
                        Stage::Retry
                    } else {
                        Stage::Validate
                    }
                }
                Stage::Validate => {
                    if let Some(validator) = self.validator.as_mut() {
                        retry_transient!(validator.query_stream(&self.chat, ctx, &tx))?;
                        let stripped = strip_json_fences(&ctx.output);
                        match serde_json::from_str::<ValidatorVerdict>(stripped).ok() {
                            Some(v) if v.verdict.eq_ignore_ascii_case("REJECTED") => {
                                ctx.push_turn(TurnKind::Rejection, "Validator", v.reason);
                                Stage::Retry
                            }
                            Some(_) => {
                                // Unlike the REJECTED/unparseable arms below, a VALIDATED
                                // verdict used to push no turn at all — the Validator's action
                                // was streamed live to the console but never recorded, so it
                                // was invisible in the trace file afterward even though nothing
                                // went wrong. Every agent's actual output should be visible in
                                // the trace, not just the ones that trigger a rejection.
                                ctx.push_turn(TurnKind::System, "Validator", ctx.output.clone());
                                Stage::Critique
                            }
                            None => {
                                // Conservative: unparseable verdict is treated
                                // as a rejection rather than silently passing.
                                ctx.push_turn(
                                    TurnKind::Rejection,
                                    "Validator",
                                    format!(
                                        "Validator produced unparseable output: {}",
                                        ctx.output
                                    ),
                                );
                                Stage::Retry
                            }
                        }
                    } else {
                        Stage::Critique
                    }
                }
                Stage::Critique => {
                    self.run_critics_parallel(ctx, &tx).await?;
                    Stage::Reconcile
                }
                Stage::Reconcile => {
                    if ctx.reconcile_rejections() {
                        Stage::Retry
                    } else {
                        Stage::Accept
                    }
                }
                Stage::Retry => {
                    if ctx.round >= max_iterations {
                        if ctx.turns.iter().any(|t| t.kind == TurnKind::Implementation) {
                            ctx.trace(&tx, "Iteration budget exhausted — surfacing best-known implementation, NOT committed, unresolved feedback remains.".into()).await;
                            Stage::Done // deliberately not Commit — don't auto-commit an unvalidated patch
                        } else {
                            Stage::Escalate("repeated rejections, iteration budget exhausted, no implementation produced".into())
                        }
                    } else {
                        // Looping back for another attempt — a patch this round already
                        // applied (Test/Validate/Critique rejected it after the fact) must
                        // not be left in place, or the next Worker round starts editing an
                        // unreviewed mutation instead of the last known-good state.
                        ctx.revert_pending_patches(&tx).await;
                        if let Some(summarizer) = self.summarizer.as_mut() {
                            let approx_tokens: u64 = ctx
                                .turns
                                .iter()
                                .map(|t| crate::agent::tokens::count_tokens(&t.content))
                                .sum();
                            if approx_tokens > summarizer.get_dynamic_history_limit() {
                                retry_transient!(summarizer.query_stream(&self.chat, ctx, &tx))?;
                                ctx.collapse_to_summary(ctx.output.clone());
                            }
                        }
                        Stage::Plan
                    }
                }
                Stage::Accept => Stage::Commit,
                Stage::Commit => {
                    // Optional interactive human-in-the-loop approval gate — off by default
                    // (`--approve`). ruchat's only approval mechanism otherwise is automated
                    // Critics (an LLM-driven gate) plus post-hoc review of the committed branch;
                    // this closes the real gap AutoGen's UserProxy/LangGraph's interrupts cover
                    // (identified via `comparisons/*.md`) without adding open-ended
                    // interactivity elsewhere — just this one, well-known pause point. Rejecting
                    // just returns `Stage::Escalate` like any other escalation in this match —
                    // the top-level `Stage::Escalate` arm above already handles clearing the
                    // checkpoint, tracing, and breaking, so there's nothing special to do here.
                    let approved = if approve_commit {
                        let plan = ctx
                            .turns
                            .iter()
                            .rev()
                            .find(|t| t.kind == TurnKind::Plan)
                            .map(|t| t.content.clone())
                            .unwrap_or_else(|| "(no plan turn found)".to_string());
                        let diff = git::git_diff(None, false)
                            .await
                            .unwrap_or_else(|e| format!("(failed to compute diff: {e})"));
                        ctx.trace(
                            &tx,
                            format!(
                                "[APPROVAL REQUIRED] About to commit. Latest plan:\n{plan}\n\n\
                                 Pending diff:\n{diff}\n\nType 'y' to commit, anything else to \
                                 stop without committing."
                            ),
                        )
                        .await;
                        let mut io = crate::io::Io::new();
                        let answer = io.read_line().await.unwrap_or_default();
                        is_approval_yes(&answer)
                    } else {
                        true
                    };

                    if !approved {
                        Stage::Escalate("commit rejected by human approval gate".into())
                    } else {
                        // Optional dedicated model for commit-message generation
                        // (`commit_message_model`); falls back to the Worker's model — always
                        // configured, since Worker is a required agent — rather than a made-up
                        // default, so the message-writer uses whatever the user already trusted
                        // enough to implement the change.
                        let commit_model = self
                            .orchestrator_config
                            .get("commit_message_model")
                            .and_then(|v| v.as_str())
                            .or_else(|| self.worker.get_str("model").ok())
                            .unwrap_or("qwen2.5-coder:14b")
                            .to_string();
                        commit_feature_branch(ctx, self.chat.as_ref(), &commit_model).await?;
                        success = true;
                        Stage::Done
                    }
                }
                Stage::Scope => {
                    if self.scoper.is_none() || scope_round >= max_scope_iterations {
                        if !ctx.turns.iter().any(|t| t.kind == TurnKind::Retrieval) {
                            // Scope gave up with zero successful lookups — don't send Architect
                            // into a vacuum. Force one deterministic, tool-free grounding step.
                            ctx.push_turn(
                                TurnKind::System,
                                "Orchestrator",
                                "Scope stage produced no retrieved information — forcing a repo listing before planning".into(),
                            );
                            let listing = crate::orchestrator::fs::list_dir(".")
                                .await
                                .unwrap_or_default();
                            ctx.push_turn(TurnKind::Retrieval, "Orchestrator", listing);
                        }
                        Stage::Plan
                    } else {
                        scope_round += 1;
                        let stage = self.run_scope_stage(ctx, &tx).await?;
                        if let Some(prev) = &last_scope_output
                            && prev == &ctx.output
                        {
                            ctx.trace(
                                &tx,
                                "Scoper repeated identical output — forcing progression to Plan"
                                    .into(),
                            )
                            .await;
                            Stage::Plan
                        } else {
                            last_scope_output = Some(ctx.output.clone());
                            stage
                        }
                    }
                }
            };
            // `Stage::Done`/`Stage::Escalate` both `break` above, before reaching here — this
            // only ever runs for a genuine, non-terminal transition, which is exactly "after
            // each stage transition" per the checkpoint's own scoping.
            checkpoint::Checkpoint::save(
                ctx,
                &stage,
                std::path::Path::new(checkpoint::CHECKPOINT_PATH),
            )
            .await;
        }
        ctx.trace(&tx, String::new()).await;
        self.finalize_trace(ctx, &tx, success).await;
        Ok(())
    }

    /// Analyzes the just-finished run's trace with a single LLM call and archives the result —
    /// `ruchat_traces/successes/` (summary only) if `Stage::Commit` succeeded, otherwise
    /// `ruchat_traces/failures/` (summary plus the full trace) — removing the live
    /// in-progress file either way. If the LLM call itself fails (Ollama unreachable, timeout,
    /// empty response), the run is still archived, just with a placeholder note instead of a
    /// real summary — a diagnostic nicety failing must never mask or replace the original
    /// outcome, nor leave the run's trace file orphaned outside both archive directories.
    async fn finalize_trace(
        &self,
        ctx: &Context,
        tx: &mpsc::Sender<OrchestratorResult>,
        success: bool,
    ) {
        let model = self
            .orchestrator_config
            .get("failure_analysis_model")
            .and_then(|v| v.as_str())
            .or_else(|| self.worker.get_str("model").ok())
            .unwrap_or("qwen2.5-coder:14b")
            .to_string();
        let body = ctx.trace_body();
        let result = if success {
            run_summary::generate_success_summary(self.chat.as_ref(), &model, &ctx.goal, &body)
                .await
        } else {
            run_summary::generate_failure_summary(self.chat.as_ref(), &model, &ctx.goal, &body)
                .await
        };
        let summary = result.unwrap_or_else(|e| {
            tracing::warn!(error = ?e, success, "run summary generation failed");
            format!("(automatic summary generation failed: {e})")
        });
        if success {
            ctx.finalize_success_trace(&summary).await;
        } else {
            ctx.finalize_failure_trace(&summary).await;
        }
        let prefix = if success {
            "Run succeeded"
        } else {
            "Run did not succeed"
        };
        let _ = tx
            .send(Ok(StreamItem::Event(AgentEvent::Trace(format!(
                "{prefix}: {summary}"
            )))))
            .await;
    }

    async fn debug_stage_machine(
        &mut self,
        goal: String,
        path: String,
        tx: mpsc::Sender<OrchestratorResult>,
        cancel: CancellationToken,
        mut breakpoints: DebugBreakpoints,
    ) -> Result<()> {
        let debug_json: Value = serde_json::from_str(&tokio::fs::read_to_string(path).await?)?;
        let sequence: Vec<String> = debug_json["sequence"]
            .as_array()
            .ok_or(RuChatError::Is("missing 'sequence' array".into()))?
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();

        let imputations = debug_json
            .get("context_imputations")
            .cloned()
            .unwrap_or_default();

        let mut ctx = Context::new(goal);
        ctx.init_trace_index().await;
        ctx.apply_debug_imputations(&imputations);

        // Debug sequences have no natural "round"; number each step so round-scoped
        // views (history_view/documents_view/context_view) still window correctly.
        for (step, role) in sequence.into_iter().enumerate() {
            if cancel.is_cancelled() {
                return Err(RuChatError::Cancelled);
            }
            ctx.round = step as u64 + 1;

            if role == "Librarian" {
                self.run_librarian_retrieval(&mut ctx, &tx).await?;
            } else {
                let kind = match role.as_str() {
                    "Architect" => {
                        self.architect
                            .query_stream(&self.chat, &mut ctx, &tx)
                            .await?;
                        TurnKind::Plan
                    }
                    "Worker" => {
                        self.worker.query_stream(&self.chat, &mut ctx, &tx).await?;
                        TurnKind::Implementation
                    }
                    "Validator" => {
                        self.validator
                            .as_mut()
                            .ok_or(RuChatError::Is("Validator not enabled".into()))?
                            .query_stream(&self.chat, &mut ctx, &tx)
                            .await?;
                        let reason = strip_json_fences(&ctx.output).to_string();
                        ctx.trace(&tx, format!("[REJECTED] {reason}")).await;
                        TurnKind::Rejection
                    }
                    "Summarizer" => {
                        self.summarizer
                            .as_mut()
                            .ok_or(RuChatError::Is("Summarizer not enabled".into()))?
                            .query_stream(&self.chat, &mut ctx, &tx)
                            .await?;
                        TurnKind::Summary
                    }
                    "Scoper" => {
                        self.scoper
                            .as_mut()
                            .ok_or(RuChatError::Is("Scoper not enabled".into()))?
                            .query_stream(&self.chat, &mut ctx, &tx)
                            .await?;
                        TurnKind::Plan
                    }
                    r if r.starts_with("Critic") => {
                        // "Critic_0", "Critic_1", ... — matches the naming in Orchestrator::new.
                        let idx: usize = r
                            .strip_prefix("Critic_")
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(0);
                        self.critics
                            .get_mut(idx)
                            .ok_or(RuChatError::Is("Critic index out of bounds".into()))?
                            .query_stream(&self.chat, &mut ctx, &tx)
                            .await?;
                        let reason = strip_json_fences(&ctx.output).to_string();
                        ctx.trace(&tx, format!("[REJECTED] {reason}")).await;
                        TurnKind::Rejection
                    }
                    _ => return Err(RuChatError::Is(format!("Unknown agent: {role}"))),
                };
                ctx.push_turn(kind, &role, ctx.output.clone());
            }

            ctx.print_debug_info(&tx, &role).await;

            // Sent via `ctx.trace` (the same channel `print_debug_info`'s state dump just went
            // through), not a direct stdout write, so the two can't race — the renderer sees
            // this prompt strictly after the state it's a prompt *about*. Only the actual
            // blocking read needs real stdin, via the same `Io` type used elsewhere in this
            // codebase for interactive prompts (`func`'s REPL, `AskArgs::ask`'s stdin fallback).
            if breakpoints.should_pause_after(&role) {
                ctx.trace(
                    &tx,
                    format!(
                        "[BREAKPOINT] Paused after round {} ({role}). Press Enter to \
                         continue, 'c' to continue without further pauses, 'q' to abort.",
                        ctx.round,
                    ),
                )
                .await;
                let mut io = crate::io::Io::new();
                match io.read_line().await.unwrap_or_default().trim() {
                    "q" | "Q" => return Err(RuChatError::Cancelled),
                    "c" | "C" => breakpoints = DebugBreakpoints::default(),
                    _ => {}
                }
            }
        }

        ctx.trace(
            &tx,
            "DEBUG SEQUENCE COMPLETE — real Librarian query used when present".to_string(),
        )
        .await;
        Ok(())
    }

    async fn handle_retrieve(
        &mut self,
        query_text: &str,
        ctx: &mut Context,
        tx: &mpsc::Sender<OrchestratorResult>,
    ) -> Result<()> {
        let client = self.client.as_ref().ok_or_else(|| {
            RuChatError::Is("Retrieve tool called but no Chroma client is configured".into())
        })?;
        let model = self
            .librarian
            .as_ref()
            .and_then(|l| l.get_str("embed_model").ok())
            .unwrap_or("all-minilm:l6-v2")
            .to_string();

        let mut q = Query::default();
        q.update_from_json(serde_json::json!({ "query": [query_text] }))?;

        let docs = q.query(client, &self.embed, &model).await?;
        let docs = self.maybe_summarize_retrieved_docs(docs, ctx, tx).await;
        ctx.push_turn(TurnKind::Retrieval, "Retrieve", docs);
        Ok(())
    }

    /// Dispatches a validated structured tool call from `Stage::Implement`.
    /// Only the read-only tools reach here; `Memorize`/`ApplyPatch` are
    /// handled later by `Agent::execute_and_verify` since they mutate state
    /// tied to the agent's own config, not the orchestrator's.
    async fn handle_structured_tool(
        &mut self,
        call: &tools::StructuredToolCall,
        ctx: &mut Context,
        tx: &mpsc::Sender<OrchestratorResult>,
    ) -> Result<()> {
        match call.tool {
            ToolName::Retrieve => {
                let query = call.args["query"].as_str().unwrap_or_default();
                self.handle_retrieve(query, ctx, tx).await
            }
            ToolName::GitLog => {
                let path = opt_str(&call.args, "path");
                let max_count = call
                    .args
                    .get("max_count")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as u32);
                let out = git::git_log(path, max_count).await?;
                ctx.push_turn(TurnKind::Retrieval, "GitLog", out);
                Ok(())
            }
            ToolName::GitBlame => {
                let path = call
                    .args
                    .get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default();
                let out = git::git_blame(path).await?;
                ctx.push_turn(TurnKind::Retrieval, "GitBlame", out);
                Ok(())
            }
            ToolName::GitDiff => {
                let path = opt_str(&call.args, "path");
                let staged = call
                    .args
                    .get("staged")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let out = git::git_diff(path, staged).await?;
                ctx.push_turn(TurnKind::Retrieval, "GitDiff", out);
                Ok(())
            }
            ToolName::GitSearchHistory => {
                let pattern = call.args["pattern"].as_str().unwrap_or_default();
                let mode = call.args["mode"].as_str().unwrap_or("message");
                let path = opt_str(&call.args, "path");
                let max_count = call
                    .args
                    .get("max_count")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as u32);
                let out = git::git_search_history(pattern, mode, path, max_count).await?;
                ctx.push_turn(TurnKind::Retrieval, "GitSearchHistory", out);
                Ok(())
            }
            ToolName::ReadFile => {
                let path = call.args["path"].as_str().unwrap_or_default();
                let start = call
                    .args
                    .get("start")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as u32);
                let end = call
                    .args
                    .get("end")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as u32);
                let out = crate::orchestrator::fs::read_file(path, start, end).await?;
                ctx.push_turn(TurnKind::Retrieval, "ReadFile", out);
                Ok(())
            }
            ToolName::ListDir => {
                let path = call.args["path"].as_str().unwrap_or_default();
                let out = crate::orchestrator::fs::list_dir(path).await?;
                ctx.push_turn(TurnKind::Retrieval, "ListDir", out);
                Ok(())
            }
            ToolName::Ripgrep => {
                let pattern = call.args["pattern"].as_str().unwrap_or_default();
                let path = opt_str(&call.args, "path");
                let glob = opt_str(&call.args, "glob");
                let max_count = call
                    .args
                    .get("max_count")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as u32);
                let context = call
                    .args
                    .get("context")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as u32);
                let out =
                    crate::orchestrator::search::ripgrep(pattern, path, glob, max_count, context)
                        .await?;
                ctx.push_turn(TurnKind::Retrieval, "Ripgrep", out);
                Ok(())
            }
            ToolName::ReadTags => {
                let symbol = opt_str(&call.args, "symbol");
                let out = crate::orchestrator::search::read_tags(symbol).await?;
                ctx.push_turn(TurnKind::Retrieval, "ReadTags", out);
                Ok(())
            }
            ToolName::CargoCheck => {
                let out = crate::orchestrator::cargo::cargo_check().await?;
                ctx.push_turn(TurnKind::Retrieval, "CargoCheck", out);
                Ok(())
            }
            ToolName::CargoClippy => {
                let out = crate::orchestrator::cargo::cargo_clippy().await?;
                ctx.push_turn(TurnKind::Retrieval, "CargoClippy", out);
                Ok(())
            }
            ToolName::CargoDupes => {
                let out = crate::orchestrator::cargo::cargo_dupes().await?;
                ctx.push_turn(TurnKind::Retrieval, "CargoDupes", out);
                Ok(())
            }
            ToolName::Memorize | ToolName::ApplyPatch => Ok(()),
        }
    }

    /// `Stage::Implement`'s patch loop, run once the Worker's turn is already pushed as an
    /// `Implementation` turn. Allows up to a per-round `patch_budget` of sequential
    /// `apply_patch` calls (reset fresh every time this stage is entered, i.e. every new round —
    /// unlike `retrieve_budget`, which is a conservative cap for the whole run) so a plan naming
    /// multiple files in its `FILES:` line can land as one commit instead of only ever touching
    /// the first of them (see `should_continue_patch_loop`). Ends the same way a single-patch
    /// round always did on `Failure` (reject/retry) or a successful `Memorize`/no-op reask after
    /// at least one patch already landed (proceed to Test) — the one new case is a Worker turn
    /// that produced no recognized tool_call *at all* on its first attempt this round (e.g. a
    /// narrative walkthrough instead of an actual tool call): rejected immediately with a
    /// precise, deterministic reason rather than silently proceeding to a Test cycle that will
    /// trivially pass (nothing changed) and hoping the Validator LLM happens to notice.
    async fn run_implement_patch_loop(
        &mut self,
        ctx: &mut Context,
        tx: &mpsc::Sender<OrchestratorResult>,
    ) -> Result<Stage> {
        let mut patch_budget: u32 = 3;
        let mut any_patch_applied = false;
        loop {
            let parsed_tool = tools::parse_tool_call(&ctx.output).ok().map(|c| c.tool);
            let is_apply_patch = matches!(parsed_tool, Some(ToolName::ApplyPatch));
            match self.worker.execute_and_verify(ctx).await? {
                Validation::Failure(err) => {
                    ctx.push_turn(TurnKind::Rejection, "ApplyPatch", err);
                    return Ok(Stage::Retry);
                }
                // A real failure mode, not hypothetical: a live run showed the Worker calling
                // `cargo_clippy`, seeing genuine warnings, then `memorize`-ing a note about them
                // instead of ever calling `apply_patch` — and the (non-deterministic) Validator
                // approved that exact substitution on one round after correctly rejecting the
                // identical thing the round before (see TODO.md's pinned reliability item).
                // Caught deterministically here, before it ever reaches the Validator, rather
                // than trusting an LLM verdict to catch it reliably every time.
                Validation::Success
                    if !any_patch_applied
                        && matches!(parsed_tool, Some(ToolName::Memorize))
                        && round_has_actionable_diagnostics(ctx) =>
                {
                    let content = "refused: you memorized information instead of applying a \
                        fix. This round's cargo_clippy/cargo_check output above shows real, \
                        actionable warnings or errors — memorize does not change any code, so \
                        the reported issue is still unresolved. Call apply_patch to actually \
                        fix it now."
                        .to_string();
                    ctx.push_turn(TurnKind::Rejection, "Worker", content);
                    return Ok(Stage::Retry);
                }
                Validation::Success if is_apply_patch => {
                    any_patch_applied = true;
                    patch_budget = patch_budget.saturating_sub(1);
                    if !should_continue_patch_loop(ctx, patch_budget) {
                        return Ok(Stage::Test);
                    }
                    ctx.push_turn(
                        TurnKind::System,
                        "Orchestrator",
                        "Patch applied. The plan's FILES: line names more files than you've \
                        changed so far — call apply_patch for the next one now, or emit no \
                        tool call if you're done."
                            .into(),
                    );
                    retry_transient!(self.worker.query_stream(&self.chat, ctx, tx))?;
                    ctx.push_turn(TurnKind::Implementation, "Worker", ctx.output.clone());
                }
                // `Validation::Skip` means `parse_tool_call` found nothing it recognized
                // anywhere in the Worker's output — no tool_call fence, no bare-diff fallback
                // either. On a *follow-up* reask within this same round (after at least one
                // apply_patch already succeeded) that's the Worker's normal way of signaling
                // "I'm done" — fine, proceed to Test. On the *first* attempt this round it means
                // the Worker did nothing actionable at all.
                Validation::Skip if !any_patch_applied => {
                    ctx.push_turn(
                        TurnKind::Rejection,
                        "Worker",
                        NO_TOOL_CALL_REJECTION.to_string(),
                    );
                    return Ok(Stage::Retry);
                }
                _ => return Ok(Stage::Test),
            }
        }
    }

    async fn run_scope_stage(
        &mut self,
        ctx: &mut Context,
        tx: &mpsc::Sender<OrchestratorResult>,
    ) -> Result<Stage> {
        let scoper = self
            .scoper
            .as_mut()
            .ok_or_else(|| RuChatError::Is("Scoper not enabled".into()))?;

        retry_transient!(scoper.query_stream(&self.chat, ctx, tx))?;
        // The Scoper's own raw output used to only ever reach `ctx.turns` in fragments — the
        // `notes` field below if non-empty, a rejected-lookup reason, a failed-lookup message —
        // never the actual action it took this round. A round where the Scoper found nothing
        // notable to say (empty notes, goal already READY) left no trace of it having run at
        // all, even though its output was streamed live to the console. Record it unconditionally.
        ctx.push_turn(TurnKind::System, "Scoper", ctx.output.clone());

        let Some(verdict) = scope::parse_scope_verdict(&ctx.output) else {
            ctx.trace(
                tx,
                "Scoper output was not valid JSON — proceeding to Plan with the goal as-is".into(),
            )
            .await;
            return Ok(Stage::Plan);
        };

        if !verdict.notes.is_empty() {
            ctx.push_turn(TurnKind::System, "Scoper", verdict.notes.clone());
        }
        if !verdict.clarified_goal.is_empty() {
            ctx.goal = verdict.clarified_goal;
        }

        if verdict.verdict.eq_ignore_ascii_case("READY") {
            return Ok(Stage::Plan);
        }

        for item in verdict.information_needed {
            if let Some(reason) = looks_like_placeholder(&item) {
                ctx.push_turn(
                    TurnKind::System,
                    "Scoper",
                    format!(
                        "rejected lookup request: {reason}. You must use a real value — if \
                        INFORMATION GATHERED SO FAR already contains a path from an earlier \
                        ripgrep/list_dir result, copy that exact path; otherwise run ripgrep or \
                        list_dir first to discover one."
                    ),
                );
                continue;
            }
            match tools::structured_call_from_value(item) {
                Ok(call) => {
                    if let Err(e) = self.handle_structured_tool(&call, ctx, tx).await {
                        ctx.push_turn(TurnKind::System, "Scoper", format!("lookup failed: {e}"));
                    }
                }
                Err(e) => {
                    ctx.push_turn(
                        TurnKind::System,
                        "Scoper",
                        format!("invalid information_needed entry: {e}"),
                    );
                }
            }
        }
        Ok(Stage::Scope)
    }
}

/// Computes `AgentEvent::Progress`'s round-based completion percentage, `[0.0, 100.0]`, for
/// `Stage::Plan`. A coarse, monotonically-increasing signal for a user watching a long run to
/// gauge proximity to the iteration budget — not a precise ETA, since a single round (e.g. one
/// with a slow `cargo test`) can still take arbitrarily long. Pulled out as its own function for
/// direct unit testing, the same tradeoff `should_continue_patch_loop` below makes: exercising
/// `run_stage_machine` itself through a full round requires either a live Ollama/Chroma round
/// trip or `Stage::Test`'s real `cargo test` invocation, both out of scope for a `--lib` unit
/// test per this file's existing test-placement precedent.
fn progress_pct(round: u64, max_iterations: u64) -> f32 {
    if max_iterations == 0 {
        return 100.0;
    }
    (round as f32 / max_iterations as f32 * 100.0).min(100.0)
}

/// Whether `Stage::Implement`'s multi-file patch loop should re-ask the Worker for another
/// `apply_patch` call after a successful one, instead of finalizing the round. Deliberately
/// count-based rather than matching planned paths against patched ones exactly (that would
/// duplicate `protocol.rs::file_in_scope`'s suffix-matching for little practical benefit here):
/// a plan with no `FILES:` line (or exactly one file) always returns `false` once one patch has
/// landed, so a single-file round behaves identically to before this loop existed.
fn should_continue_patch_loop(ctx: &Context, remaining_patch_budget: u32) -> bool {
    remaining_patch_budget > 0 && ctx.planned_files().len() > ctx.pending_patches.len()
}

/// Rejection reason for a Worker turn with no recognized tool call anywhere in it — see
/// `Orchestrator::run_implement_patch_loop`.
const NO_TOOL_CALL_REJECTION: &str = "refused: no recognized tool_call found anywhere in your \
    output — every round you must emit exactly one tool_call (e.g. apply_patch, memorize). A \
    narrative walkthrough, an explanation of what you would do, or any other prose without an \
    actual tool_call accomplishes nothing on its own.";

/// Read-only tools the Worker may call once per run as a budgeted information-lookup
/// (`retrieve_budget`) — every other Worker tool call must be `apply_patch`/`memorize`. Pulled
/// out as its own predicate so `Stage::Implement`'s first-call check and its second-chance check
/// (see the nudge-and-reask loop below) can't drift apart into two different tool lists.
fn is_read_only_worker_tool(tool: &ToolName) -> bool {
    matches!(
        tool,
        ToolName::Retrieve
            | ToolName::GitLog
            | ToolName::GitBlame
            | ToolName::GitDiff
            | ToolName::GitSearchHistory
            | ToolName::ReadFile
            | ToolName::ListDir
            | ToolName::Ripgrep
            | ToolName::ReadTags
            | ToolName::CargoCheck
            | ToolName::CargoClippy
            | ToolName::CargoDupes
    )
}

/// True if this round already retrieved real, actionable `cargo_clippy`/`cargo_check` output —
/// i.e. it actually contains a compiler/clippy diagnostic, not just a clean "nothing to report"
/// run. Used by `run_implement_patch_loop` to catch a `memorize` call substituting for an actual
/// fix: a real live-verified run (see TODO.md's pinned reliability item) showed the Worker
/// calling `cargo_clippy`, seeing genuine warnings, then `memorize`-ing a note about them instead
/// of ever calling `apply_patch` — and the Validator (an LLM call, not deterministic) approved
/// that exact substitution on one round after correctly rejecting the identical thing the round
/// before. This is a deterministic backstop specifically for that failure shape, not a general
/// replacement for the Validator's broader judgment elsewhere.
fn round_has_actionable_diagnostics(ctx: &Context) -> bool {
    ctx.turns.iter().any(|t| {
        t.round == ctx.round
            && t.kind == TurnKind::Retrieval
            && matches!(t.source.as_str(), "CargoClippy" | "CargoCheck")
            && (t.content.contains("warning:") || t.content.contains("error:"))
    })
}

/// `agent_role/architect.md` explicitly forbids the Architect from ever emitting a `tool_call` —
/// it's plan-only, no tools, only the Worker calls tools — but real runs show the local model
/// doing it anyway, embedding a full (often hallucinated) `apply_patch` diff inside what's
/// nominally its plan. Truncates at the first `\`\`\`tool_call` fence and drops everything from
/// there on — nothing after a hallucinated tool call in a "plan" is trustworthy plan content
/// either — plus a trailing bare `IMPLEMENTATION:` label immediately before it, if present, so no
/// dangling empty heading is left behind. Leaves the plan untouched if it never emitted one.
fn strip_architect_tool_call_hallucination(plan: &str) -> String {
    let Some(idx) = plan.find("```tool_call") else {
        return plan.to_string();
    };
    let truncated = plan[..idx].trim_end();
    truncated
        .strip_suffix("IMPLEMENTATION:")
        .map(str::trim_end)
        .unwrap_or(truncated)
        .to_string()
}

/// Cap on how much of a planned target file's real content gets auto-injected per round — same
/// size as `MAX_SHOWN_ORIGINAL_CHARS`'s post-rejection grounding dump (`agent/protocol.rs`), just
/// proactive instead of reactive.
const MAX_AUTO_GROUNDED_FILE_CHARS: usize = 4_000;

/// Proactively shows the plan's target file's real, line-numbered content to the Worker before it
/// ever writes a diff, instead of only doing this reactively after a failed `apply_patch` (see
/// `Validation::apply_patch`'s existing grounding-on-mismatch rejection). Real live-verified runs
/// (see TODO.md's pinned reliability item) repeatedly showed the Worker writing a diff against a
/// file's *guessed* content — sometimes without ever calling `read_file` on it at all, sometimes
/// after calling it once but the content apparently not being attended to by a later round of a
/// multi-round run — fabricating fields or comments that don't exist in the real file. Only acts
/// when the plan names exactly one file (same "unambiguous" bar `ensure_diff_has_file_header`
/// uses) — with zero or multiple planned files there's no single safe target to show.
/// Deliberately re-injected every round, not just once: the same recency reasoning behind
/// everything else in this section — local models attend far better to content near the end of
/// their context than to something shown several rounds ago. Any *earlier* grounding turn is
/// removed before the fresh one is pushed, rather than left in place alongside it — without this,
/// a multi-round run repeatedly targeting the same file (the common case) would accumulate one
/// near-duplicate ~4000-char dump per round with nothing to ever compress them (these test runs
/// have no Summarizer configured), a real, self-inflicted contributor to context bloat found
/// while investigating why later rounds of a run seemed to ignore instructions that were
/// technically already in context — see TODO.md's pinned reliability item. Keeping exactly one,
/// always freshly re-inserted, gets the recency benefit without the duplication cost. Best-effort
/// like every other diagnostic-nicety path in this codebase (`Checkpoint::save`): a
/// missing/unreadable file (about to be created, or a real I/O error) is silently skipped rather
/// than surfaced as an error, since this is a proactive nicety, not a required step — and it
/// never costs `retrieve_budget`, since it's orchestrator-driven, not a Worker-initiated lookup.
async fn auto_ground_planned_file(ctx: &mut Context) {
    let planned = ctx.planned_files();
    let [target] = planned.as_slice() else {
        return;
    };
    let Ok(content) = tokio::fs::read_to_string(target).await else {
        return;
    };
    ctx.turns.retain(|t| t.source != "AutoGroundedFile");
    let numbered: String = content
        .lines()
        .enumerate()
        .map(|(i, line)| format!("{}:{line}", i + 1))
        .collect::<Vec<_>>()
        .join("\n");
    let shown: String = numbered
        .chars()
        .take(MAX_AUTO_GROUNDED_FILE_CHARS)
        .collect();
    let truncated_note = if numbered.chars().count() > MAX_AUTO_GROUNDED_FILE_CHARS {
        format!(
            "\n... (truncated, {} bytes total — request a narrower range with read_file if you \
            need more)",
            content.len()
        )
    } else {
        String::new()
    };
    ctx.push_turn(
        TurnKind::Retrieval,
        "AutoGroundedFile",
        format!(
            "The plan's FILES: line names '{target}' — here is its real current content, with \
            line numbers (N:content), so your diff's context lines and its @@ -a,b +c,d @@ hunk \
            header match it exactly. Do not guess or assume content not shown here:\n\n\
            {shown}{truncated_note}"
        ),
    );
}

/// Treats an explicit empty string the same as an omitted optional field.
/// Models reliably emit `"path": ""` instead of leaving an optional arg out
/// entirely, and downstream commands (e.g. `git log -- ""`) reject an empty
/// pathspec outright rather than treating it as "no restriction" — this
/// normalizes that before it ever reaches them.
fn opt_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
}

/// Catches values the model copied from prompt scaffolding instead of
/// producing real ones — angle-bracket placeholders (`<path>`), the literal
/// word `placeholder`, or template-looking segments like `path/to/`. Cheap
/// heuristic, not exhaustive; it exists to turn a wasted I/O round-trip into
/// an immediate, specific correction instead.
fn looks_like_placeholder(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => {
            let lower = s.to_lowercase();
            if s.contains('<') && s.contains('>') {
                Some(format!("'{s}' looks like a placeholder, not a real value"))
            } else if lower.contains("path/to") || lower.contains("<") {
                Some(format!("'{s}' looks like a template, not a real value"))
            } else {
                None
            }
        }
        Value::Object(map) => map.values().find_map(looks_like_placeholder),
        Value::Array(arr) => arr.iter().find_map(looks_like_placeholder),
        _ => None,
    }
}

/// Runs each `agent_debug/*.json` fixture through the real stage machine
/// (`debug_stage_machine`, via `run_task_stream`) against a scripted
/// `FakeLlmClient`/`FakeVectorStore` instead of a live Ollama/Chroma server.
/// These were sitting unused as fixtures on disk before this — nothing
/// exercised them, which is exactly how `multiple_critics.json`/`critic.json`
/// carried a naming bug (`"Critic0"` instead of `"Critic_0"`, silently
/// falling back to critic index 0 every time) for who knows how long.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::llm_client::fake_vector_store::FakeVectorStore;
    use crate::agent::llm_client::{FakeLlmClient, VectorCollection};
    use chroma::types::QueryResponse;
    use serde_json::json;

    /// Builds an `Orchestrator` by hand (not `Orchestrator::new`, which always
    /// constructs a real `ChromaHttpClient` for the Librarian) so Librarian
    /// fixtures can run against a `FakeVectorStore` instead of a live Chroma
    /// server. Each role's `Agent` is still built through the real
    /// `Agent::new`, so config parsing/merging is exercised exactly as in
    /// production — only the network-facing clients are swapped.
    async fn build_test_orchestrator(
        mut config: Value,
        responses: Vec<&str>,
        query_response: Option<QueryResponse>,
    ) -> Orchestrator {
        let architect = Agent::new(&mut config, "Architect", true, None, json!({}))
            .await
            .unwrap();
        let worker = Agent::new(&mut config, "Worker", true, None, json!({}))
            .await
            .unwrap();
        let validator = Agent::new(&mut config, "Validator", false, None, json!({}))
            .await
            .ok();
        let summarizer = Agent::new(&mut config, "Summarizer", false, None, json!({}))
            .await
            .ok();
        let scoper = Agent::new(&mut config, "Scoper", false, None, json!({}))
            .await
            .ok();
        let librarian = Agent::new(&mut config, "Librarian", false, None, json!({}))
            .await
            .ok();

        let mut critics = Vec::new();
        if let Some(critic_list) = config.get("Critics").and_then(|v| v.as_array()) {
            for (i, c_val) in critic_list.iter().enumerate() {
                let role = format!("Critic_{i}");
                let mut c_config = json!({ role.clone(): c_val.clone() });
                if let Ok(agent) = Agent::new(&mut c_config, &role, true, None, json!({})).await {
                    critics.push(agent);
                }
            }
        }

        let client: Option<Arc<dyn VectorStore>> = query_response
            .map(|response| Arc::new(FakeVectorStore { response }) as Arc<dyn VectorStore>);

        Orchestrator {
            scoper,
            architect,
            worker,
            librarian,
            critics,
            summarizer,
            validator,
            orchestrator_config: config,
            chat: Arc::new(FakeLlmClient::new(responses)),
            embed: Arc::new(FakeLlmClient::new(vec![])),
            client,
        }
    }

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

    fn fake_query_response() -> QueryResponse {
        QueryResponse {
            ids: vec![vec!["doc1".to_string()]],
            embeddings: None,
            documents: Some(vec![vec![Some("fake retrieved document".to_string())]]),
            uris: None,
            metadatas: None,
            distances: None,
            include: vec![],
        }
    }

    /// Runs a fixture to completion and returns every streamed item —
    /// panics (failing the test) if the stage machine ever surfaces an
    /// `Err`, which is the thing debug-mode is meant to make crisp to catch.
    async fn run_fixture(
        fixture: &str,
        config: Value,
        responses: Vec<&str>,
        query_response: Option<QueryResponse>,
    ) -> Vec<StreamItem> {
        let orchestrator = build_test_orchestrator(config, responses, query_response).await;
        let path = format!("agent_debug/{fixture}");
        let stream = orchestrator.run_task_stream(
            "test goal".to_string(),
            Some(path),
            DebugBreakpoints::default(),
            false,
            false,
        );
        tokio_stream::StreamExt::collect::<Vec<Result<StreamItem>>>(stream)
            .await
            .into_iter()
            .map(|r| r.expect("debug sequence produced an error"))
            .collect()
    }

    fn base_config() -> Value {
        json!({
            "Architect": { "model": "fake" },
            "Worker": { "model": "fake" },
        })
    }

    #[test]
    fn is_read_only_worker_tool_covers_every_budgeted_lookup_tool() {
        for tool in [
            ToolName::Retrieve,
            ToolName::GitLog,
            ToolName::GitBlame,
            ToolName::GitDiff,
            ToolName::GitSearchHistory,
            ToolName::ReadFile,
            ToolName::ListDir,
            ToolName::Ripgrep,
            ToolName::ReadTags,
            ToolName::CargoCheck,
            ToolName::CargoClippy,
            ToolName::CargoDupes,
        ] {
            assert!(
                is_read_only_worker_tool(&tool),
                "{tool:?} should be a budgeted read-only lookup tool"
            );
        }
    }

    #[test]
    fn is_read_only_worker_tool_excludes_the_write_tools() {
        assert!(!is_read_only_worker_tool(&ToolName::ApplyPatch));
        assert!(!is_read_only_worker_tool(&ToolName::Memorize));
    }

    #[test]
    fn patch_loop_does_not_continue_when_the_plan_named_no_files() {
        // No FILES: line at all (or a plan the parser found none in) — the common/legacy case.
        // Must behave exactly like the single-patch-per-round flow this loop replaced.
        let mut ctx = Context::new("goal".to_string());
        ctx.push_turn(
            TurnKind::Plan,
            "Architect",
            "just do it, no files line".to_string(),
        );
        ctx.record_patch("src/foo.rs".to_string(), "original".to_string());
        assert!(!should_continue_patch_loop(&ctx, 2));
    }

    #[test]
    fn patch_loop_does_not_continue_when_the_plan_named_exactly_one_file() {
        let mut ctx = Context::new("goal".to_string());
        ctx.push_turn(TurnKind::Plan, "Architect", "FILES: src/foo.rs".to_string());
        ctx.record_patch("src/foo.rs".to_string(), "original".to_string());
        assert!(!should_continue_patch_loop(&ctx, 2));
    }

    #[test]
    fn patch_loop_continues_when_the_plan_named_more_files_than_patched_so_far() {
        let mut ctx = Context::new("goal".to_string());
        ctx.push_turn(
            TurnKind::Plan,
            "Architect",
            "FILES: src/foo.rs, src/bar.rs".to_string(),
        );
        ctx.record_patch("src/foo.rs".to_string(), "original".to_string());
        assert!(should_continue_patch_loop(&ctx, 2));
    }

    #[test]
    fn patch_loop_stops_once_every_planned_file_is_patched_even_with_budget_left() {
        let mut ctx = Context::new("goal".to_string());
        ctx.push_turn(
            TurnKind::Plan,
            "Architect",
            "FILES: src/foo.rs, src/bar.rs".to_string(),
        );
        ctx.record_patch("src/foo.rs".to_string(), "original".to_string());
        ctx.record_patch("src/bar.rs".to_string(), "original".to_string());
        assert!(!should_continue_patch_loop(&ctx, 5));
    }

    #[test]
    fn patch_loop_stops_when_budget_is_exhausted_even_with_files_still_unplanned() {
        let mut ctx = Context::new("goal".to_string());
        ctx.push_turn(
            TurnKind::Plan,
            "Architect",
            "FILES: src/foo.rs, src/bar.rs, src/baz.rs".to_string(),
        );
        ctx.record_patch("src/foo.rs".to_string(), "original".to_string());
        assert!(!should_continue_patch_loop(&ctx, 0));
    }

    #[test]
    fn progress_pct_is_zero_before_the_first_round() {
        assert_eq!(progress_pct(0, 3), 0.0);
    }

    #[test]
    fn progress_pct_scales_linearly_with_round_over_max_iterations() {
        // f32 division order affects the last bit or two, so compare with a small epsilon
        // rather than exact equality (100.0/3.0 computed independently here vs. inside
        // progress_pct doesn't round identically).
        assert!((progress_pct(1, 3) - 100.0 / 3.0).abs() < 0.001);
        assert!((progress_pct(2, 3) - 200.0 / 3.0).abs() < 0.001);
        assert_eq!(progress_pct(3, 3), 100.0);
    }

    #[test]
    fn progress_pct_clamps_at_100_when_round_exceeds_max_iterations() {
        // `Stage::Plan`'s escalate branch increments `ctx.round` past `max_iterations`
        // before checking the budget — progress must still read as a sane percentage,
        // not something like 133%.
        assert_eq!(progress_pct(4, 3), 100.0);
    }

    #[test]
    fn progress_pct_does_not_divide_by_zero_when_max_iterations_is_zero() {
        assert_eq!(progress_pct(0, 0), 100.0);
        assert_eq!(progress_pct(5, 0), 100.0);
    }

    #[tokio::test]
    async fn model_summary_lists_every_configured_role_once() {
        let mut config = base_config();
        config["Validator"] = json!({ "model": "validator-model" });
        config["Critics"] = json!([{ "model": "critic-model", "name": "Security" }]);
        let orchestrator = build_test_orchestrator(config, vec![], None).await;

        let summary = orchestrator.model_summary();
        assert!(summary.contains("Architect=fake"));
        assert!(summary.contains("Worker=fake"));
        assert!(summary.contains("Validator=validator-model"));
        assert!(summary.contains("Security=critic-model"));
        assert!(summary.contains("ruchat_traces/"));
    }

    // Regression: the old `.ruchat_trace.md` gave no indication of *why* an unsuccessful run
    // (escalated, or the iteration budget exhausted without ever reaching `Stage::Commit`)
    // failed — a maintainer had to read every round of a possibly long trace to reconstruct
    // that themselves. `finalize_trace` (called from `run_stage_machine` at the very end, with
    // the `success` flag — set only by `Stage::Commit` succeeding) makes one direct LLM call
    // over the trace and reports the result as a `Trace` event; the actual archival (real file
    // I/O under `ruchat_traces/failures/`, via `Context::finalize_failure_trace`) is covered
    // separately by that method's own doc comment / by inspection, same as the old
    // `prepend_failure_summary` was.
    #[tokio::test]
    async fn finalize_trace_sends_a_failure_trace_event_with_the_analysis() {
        let orchestrator = build_test_orchestrator(
            base_config(),
            vec!["Worker kept repeating itself and never produced a valid patch"],
            None,
        )
        .await;
        let ctx = Context::new("fix the bug".to_string());
        let (tx, mut rx) = mpsc::channel(100);

        orchestrator.finalize_trace(&ctx, &tx, false).await;

        let event = rx.recv().await.expect("expected a trace event");
        match event.expect("expected Ok") {
            StreamItem::Event(AgentEvent::Trace(msg)) => {
                assert!(msg.starts_with("Run did not succeed:"));
                assert!(
                    msg.contains("Worker kept repeating itself and never produced a valid patch")
                );
            }
            other => panic!("expected a Trace event, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn finalize_trace_sends_a_success_trace_event_with_the_analysis() {
        let orchestrator = build_test_orchestrator(
            base_config(),
            vec!["Renamed the helper and updated every call site."],
            None,
        )
        .await;
        let ctx = Context::new("rename a function".to_string());
        let (tx, mut rx) = mpsc::channel(100);

        orchestrator.finalize_trace(&ctx, &tx, true).await;

        let event = rx.recv().await.expect("expected a trace event");
        match event.expect("expected Ok") {
            StreamItem::Event(AgentEvent::Trace(msg)) => {
                assert!(msg.starts_with("Run succeeded:"));
                assert!(msg.contains("Renamed the helper and updated every call site."));
            }
            other => panic!("expected a Trace event, got {other:?}"),
        }
    }

    // Regression test for a real failure: the Worker replied with a narrative walkthrough
    // ("### Identified First Warning... ### Applying the Fix...") wrapped around fenced blocks
    // that weren't actually a tool_call (a ```bash block naming a tool as if it were a shell
    // command, then a ```rust block with raw source, not a diff) — `parse_tool_call` correctly
    // found nothing, but the orchestrator used to treat that identically to a successful
    // `memorize` and silently proceed to `Stage::Test`, wasting a cycle and leaving the Worker
    // with no specific feedback about what it did wrong.
    #[tokio::test]
    async fn implement_patch_loop_rejects_a_worker_turn_with_no_tool_call_at_all() {
        let mut orchestrator = build_test_orchestrator(base_config(), vec![], None).await;
        let mut ctx = Context::new("goal".to_string());
        ctx.output = "### Identified First Warning\n\nAssuming cargo_clippy has been run, \
            here's what I would do next. If the warning is resolved, proceed with the next \
            steps."
            .to_string();
        let (tx, _rx) = mpsc::channel(100);

        let stage = orchestrator
            .run_implement_patch_loop(&mut ctx, &tx)
            .await
            .unwrap();

        assert_eq!(stage, Stage::Retry);
        let rejection = ctx
            .turns
            .iter()
            .find(|t| t.kind == TurnKind::Rejection)
            .expect("a no-tool-call Worker turn should be rejected");
        assert_eq!(rejection.source, "Worker");
        assert!(rejection.content.contains("no recognized tool_call"));
    }

    // Regression test for a second real failure, reported right after the one above: the Worker
    // called cargo_clippy, got its result, then called cargo_clippy *again* instead of switching
    // to apply_patch — repeated across rounds until the exact-match stall guard escalated the
    // whole run. `execute_and_verify` only handles ApplyPatch/Memorize; any other (valid, known)
    // tool reaching it at verify-time used to produce a cryptic "Unexpected tool at verify
    // stage: CargoClippy" debug dump. This exercises that same code path — CargoClippy isn't
    // handled specially in `execute_and_verify`, so no real subprocess ever runs here.
    #[tokio::test]
    async fn implement_patch_loop_rejects_a_second_read_only_tool_call_with_actionable_guidance() {
        let mut orchestrator = build_test_orchestrator(base_config(), vec![], None).await;
        let mut ctx = Context::new("goal".to_string());
        ctx.output = "```tool_call\n{\"tool\": \"cargo_clippy\"}\n```".to_string();
        let (tx, _rx) = mpsc::channel(100);

        let stage = orchestrator
            .run_implement_patch_loop(&mut ctx, &tx)
            .await
            .unwrap();

        assert_eq!(stage, Stage::Retry);
        let rejection = ctx
            .turns
            .iter()
            .find(|t| t.kind == TurnKind::Rejection)
            .expect("a repeated read-only tool call should be rejected");
        assert!(
            rejection
                .content
                .contains("already used this round's one information-lookup")
        );
        assert!(rejection.content.contains("apply_patch"));
    }

    // Regression: a live-verified run (see TODO.md's pinned reliability item) showed the Worker
    // calling cargo_clippy, seeing real warnings, then memorize-ing a note about them instead of
    // ever calling apply_patch — and the Validator (non-deterministic) approved that exact
    // substitution on one round after correctly rejecting the identical thing the round before.
    // `round_has_actionable_diagnostics` is the deterministic backstop for that; not testable
    // through `run_implement_patch_loop` itself (Memorize's `execute_and_verify` branch calls
    // `Agent::embed`, which builds a real Ollama/Chroma client — same "needs live infra" category
    // as `Stage::Test`'s real `cargo test` invocation, out of scope for `--lib` unit tests per
    // this file's own established precedent), so tested directly as the pure predicate it is.
    #[test]
    fn round_has_actionable_diagnostics_true_for_a_real_clippy_warning_this_round() {
        let mut ctx = Context::new("goal".to_string());
        ctx.round = 1;
        ctx.push_turn(
            TurnKind::Retrieval,
            "CargoClippy",
            "src/foo.rs:1:1: warning: field `x` is never read".to_string(),
        );
        assert!(round_has_actionable_diagnostics(&ctx));
    }

    #[test]
    fn round_has_actionable_diagnostics_true_for_a_real_check_error_this_round() {
        let mut ctx = Context::new("goal".to_string());
        ctx.round = 1;
        ctx.push_turn(
            TurnKind::Retrieval,
            "CargoCheck",
            "src/foo.rs:1:1: error: mismatched types".to_string(),
        );
        assert!(round_has_actionable_diagnostics(&ctx));
    }

    #[test]
    fn round_has_actionable_diagnostics_false_for_a_clean_clippy_run() {
        let mut ctx = Context::new("goal".to_string());
        ctx.round = 1;
        ctx.push_turn(
            TurnKind::Retrieval,
            "CargoClippy",
            "    Finished `dev` profile [unoptimized + debuginfo] target(s) in 3.11s".to_string(),
        );
        assert!(!round_has_actionable_diagnostics(&ctx));
    }

    #[test]
    fn round_has_actionable_diagnostics_false_when_the_diagnostics_are_from_an_earlier_round() {
        let mut ctx = Context::new("goal".to_string());
        ctx.round = 1;
        ctx.push_turn(
            TurnKind::Retrieval,
            "CargoClippy",
            "src/foo.rs:1:1: warning: field `x` is never read".to_string(),
        );
        ctx.round = 2;
        assert!(!round_has_actionable_diagnostics(&ctx));
    }

    #[test]
    fn round_has_actionable_diagnostics_false_with_no_diagnostic_turn_at_all() {
        let ctx = Context::new("goal".to_string());
        assert!(!round_has_actionable_diagnostics(&ctx));
    }

    // Regression: a live-verified run (see TODO.md's pinned reliability item) showed the
    // Architect — plan-only, explicitly forbidden by its own prompt from ever emitting a
    // tool_call — hallucinating a full apply_patch diff inside its plan anyway, which the Worker
    // then copied verbatim, unchanged, across 3 separate rounds. Fixture shaped exactly like the
    // real trace: PLAN/CHOICE/FILES text, an earlier unrelated fenced block (the clippy warning
    // quoted back), then an "IMPLEMENTATION:" label and a ```tool_call fence.
    #[test]
    fn strip_architect_tool_call_hallucination_removes_a_hallucinated_implementation_block() {
        let plan = "**PLAN:**\nFix the lint.\n\n**FILES:**\n- src/core/agent.rs\n\n\
            The first warning reported is:\n```\nsrc/core/agent.rs:36:5: warning: field \
            `options` is never read\n```\n\nRemove the unused field.\n\nIMPLEMENTATION:\n\
            ```tool_call\n{\n  \"tool\": \"apply_patch\",\n  \"diff\": \"...\"\n}\n```";
        let stripped = strip_architect_tool_call_hallucination(plan);
        assert!(
            !stripped.contains("tool_call"),
            "hallucinated tool_call should be removed, got: {stripped:?}"
        );
        assert!(
            !stripped.contains("IMPLEMENTATION:"),
            "the dangling label should be removed too, got: {stripped:?}"
        );
        assert!(
            stripped.contains("Remove the unused field."),
            "legitimate plan text before the hallucination should survive, got: {stripped:?}"
        );
        // The earlier, unrelated fenced block (quoting the clippy warning back) is real plan
        // content, not part of the hallucination — must not be touched.
        assert!(stripped.contains("field `options` is never read"));
    }

    #[test]
    fn strip_architect_tool_call_hallucination_leaves_a_real_plan_untouched() {
        let plan = "**PLAN:**\nFix the lint.\n\n**FILES:**\n- src/core/agent.rs";
        assert_eq!(strip_architect_tool_call_hallucination(plan), plan);
    }

    #[test]
    fn strip_architect_tool_call_hallucination_truncates_even_without_a_label_before_it() {
        let plan = "**PLAN:**\nFix the lint.\n\n```tool_call\n{\"tool\": \"apply_patch\"}\n```";
        let stripped = strip_architect_tool_call_hallucination(plan);
        assert_eq!(stripped, "**PLAN:**\nFix the lint.");
    }

    // Regression: two live-verified runs (see TODO.md's pinned reliability item) showed the
    // Worker writing a diff against a file it never actually read — in one case fabricating a
    // struct field (`pub client: LlmClient,`) that doesn't exist anywhere in the real file.
    // Absolute paths used throughout (not a real repo-relative path + `set_current_dir`) — this
    // codebase has a documented, previously-real flakiness lesson about `set_current_dir` being
    // process-global, not per-test-isolated (see `checkpoint.rs`'s tests) — `planned_files`'s own
    // `a/`/`./`-stripping only touches those specific prefixes, so an absolute tempdir path
    // passes through untouched and needs no CWD change at all.
    #[tokio::test]
    async fn auto_ground_planned_file_shows_real_line_numbered_content_when_unambiguous() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("target.rs");
        std::fs::write(&file, "fn a() {}\nfn b() {}\n").unwrap();

        let mut ctx = Context::new("goal".to_string());
        ctx.push_turn(
            TurnKind::Plan,
            "Architect",
            format!("PLAN: fix it\nFILES: {}", file.display()),
        );

        auto_ground_planned_file(&mut ctx).await;

        let turn = ctx
            .turns
            .iter()
            .find(|t| t.source == "AutoGroundedFile")
            .expect("should have pushed a grounding turn");
        assert!(turn.kind == TurnKind::Retrieval);
        assert!(
            turn.content.contains("1:fn a() {}"),
            "got: {}",
            turn.content
        );
        assert!(
            turn.content.contains("2:fn b() {}"),
            "got: {}",
            turn.content
        );
    }

    #[tokio::test]
    async fn auto_ground_planned_file_does_nothing_with_zero_planned_files() {
        let mut ctx = Context::new("goal".to_string());
        ctx.push_turn(TurnKind::Plan, "Architect", "PLAN: fix it".to_string());
        auto_ground_planned_file(&mut ctx).await;
        assert!(!ctx.turns.iter().any(|t| t.source == "AutoGroundedFile"));
    }

    #[tokio::test]
    async fn auto_ground_planned_file_does_nothing_with_multiple_planned_files_ambiguous() {
        let mut ctx = Context::new("goal".to_string());
        ctx.push_turn(
            TurnKind::Plan,
            "Architect",
            "PLAN: fix it\nFILES: a.rs, b.rs".to_string(),
        );
        auto_ground_planned_file(&mut ctx).await;
        assert!(!ctx.turns.iter().any(|t| t.source == "AutoGroundedFile"));
    }

    #[tokio::test]
    async fn auto_ground_planned_file_silently_skips_a_missing_file() {
        let mut ctx = Context::new("goal".to_string());
        ctx.push_turn(
            TurnKind::Plan,
            "Architect",
            "PLAN: fix it\nFILES: /nonexistent/path/that/does/not/exist_ruchat_test.rs".to_string(),
        );
        auto_ground_planned_file(&mut ctx).await;
        assert!(!ctx.turns.iter().any(|t| t.source == "AutoGroundedFile"));
    }

    // Regression: found while investigating whether later-round failures were really a model-
    // capability limit or something this codebase was contributing itself (see TODO.md's pinned
    // reliability item) — with no Summarizer configured (true of the real repro scripts) and
    // this function re-injecting a fresh grounding dump every round, a multi-round run repeatedly
    // targeting the same file used to accumulate one near-duplicate ~4000-char copy per round,
    // all still sitting in DOCUMENTS with nothing to ever compress them. Exactly one
    // AutoGroundedFile turn must exist at a time, no matter how many rounds re-ground the same
    // (or a different) file.
    #[tokio::test]
    async fn auto_ground_planned_file_replaces_the_previous_grounding_instead_of_accumulating() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("target.rs");
        std::fs::write(&file, "fn a() {}\n").unwrap();

        let mut ctx = Context::new("goal".to_string());
        ctx.push_turn(
            TurnKind::Plan,
            "Architect",
            format!("PLAN: fix it\nFILES: {}", file.display()),
        );

        // Simulate three rounds each re-grounding the same file.
        for round in 1..=3u64 {
            ctx.round = round;
            auto_ground_planned_file(&mut ctx).await;
        }

        let grounding_turns: Vec<_> = ctx
            .turns
            .iter()
            .filter(|t| t.source == "AutoGroundedFile")
            .collect();
        assert_eq!(
            grounding_turns.len(),
            1,
            "expected exactly one grounding turn after 3 rounds, got: {grounding_turns:?}"
        );
        assert_eq!(
            grounding_turns[0].round, 3,
            "the surviving turn should be the freshest one"
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

    // Regression canary for the HITL approval gate (maintainer: "keep on working on roadmap
    // entries, overnight"). `is_approval_yes` is the one piece of this feature with real
    // branching logic that doesn't need a live terminal — the actual pause (plan/diff shown via
    // `ctx.trace`, then a blocking stdin read via the same `Io` type breakpoints already use)
    // was instead verified live: `--approve` against a real agentic run, confirming the plan
    // and a real `git diff` render before the prompt, that typing 'y' proceeds to a real commit,
    // and that any other answer stops the run via `Stage::Escalate` without committing.
    #[test]
    fn is_approval_yes_accepts_only_an_exact_y_or_yes() {
        for accepted in ["y", "Y", "yes", "Yes", " y ", "y\n"] {
            assert!(
                is_approval_yes(accepted),
                "expected {accepted:?} to be approval"
            );
        }
    }

    #[test]
    fn is_approval_yes_rejects_everything_else_including_blank() {
        for rejected in ["n", "N", "no", "", "  ", "YES", "sure", "yep"] {
            assert!(
                !is_approval_yes(rejected),
                "expected {rejected:?} to be rejection"
            );
        }
    }

    #[tokio::test]
    async fn architect_only_completes() {
        let items = run_fixture(
            "architect_only.json",
            base_config(),
            vec!["Plan: refactor error handling to use `?`."],
            None,
        )
        .await;
        assert!(!items.is_empty());

        // Regression: `query_stream` used to send a "[Role's input] querying 'model'..." trace
        // on every single turn — redundant noise once each role already gets its own colored
        // banner (`ColorChange`). That per-turn announcement is gone now (`.ruchat_trace.md`
        // still gets refreshed, just silently); a model summary is only ever printed once, at
        // the start of a real (non-debug) run — `debug_stage_machine`, which this fixture runs
        // through, doesn't call `model_summary` at all, so no trace events are expected here.
        let has_querying_trace = items.iter().any(|item| {
            matches!(item, StreamItem::Event(AgentEvent::Trace(msg)) if msg.contains("querying"))
        });
        assert!(
            !has_querying_trace,
            "no turn should announce '...querying \\'model\\'...' anymore"
        );
    }

    #[tokio::test]
    async fn librarian_only_completes() {
        let mut config = base_config();
        config["Librarian"] = json!({ "model": "fake", "embed_model": "fake-embed" });
        let items = run_fixture(
            "librarian_only.json",
            config,
            vec!["{\"query\": \"error handling\", \"n_results\": 5, \"collection\": \"repo\"}"],
            Some(fake_query_response()),
        )
        .await;
        assert!(!items.is_empty());
    }

    #[tokio::test]
    async fn librarian_and_worker_completes() {
        let mut config = base_config();
        config["Librarian"] = json!({ "model": "fake", "embed_model": "fake-embed" });
        let items = run_fixture(
            "librarian_and_worker.json",
            config,
            vec![
                "{\"query\": \"error handling\", \"n_results\": 5, \"collection\": \"repo\"}",
                "Replaced unwrap() with `?` and anyhow::Context.",
            ],
            Some(fake_query_response()),
        )
        .await;
        assert!(!items.is_empty());
    }

    // The `agent_debug/*.json` fixture directory had two combinations (`architect_librarian_
    // and_worker.json`, `architect_librarian_worker_validator.json`) that existed on disk but
    // were never actually driven through `run_fixture`/`debug_stage_machine` by any test —
    // `cargo test --lib` compiling and passing gave no signal about whether these two specific
    // role sequences worked, since nothing exercised them. ROADMAP.md previously (incorrectly)
    // described the fixed-sequence debug mode as "not wired into cargo test" at all, which was
    // stale — 9 of the then-11 fixtures already were; these two were the only real gap.
    #[tokio::test]
    async fn architect_librarian_and_worker_completes() {
        let mut config = base_config();
        config["Librarian"] = json!({ "model": "fake", "embed_model": "fake-embed" });
        let items = run_fixture(
            "architect_librarian_and_worker.json",
            config,
            vec![
                "Plan: refactor error handling to use `?` and anyhow::Context.",
                "{\"query\": \"error handling\", \"n_results\": 5, \"collection\": \"repo\"}",
                "Replaced unwrap() with `?` and anyhow::Context.",
            ],
            Some(fake_query_response()),
        )
        .await;
        assert!(!items.is_empty());
    }

    #[tokio::test]
    async fn architect_librarian_worker_validator_completes() {
        let mut config = base_config();
        config["Librarian"] = json!({ "model": "fake", "embed_model": "fake-embed" });
        config["Validator"] = json!({ "model": "fake" });
        let items = run_fixture(
            "architect_librarian_worker_validator.json",
            config,
            vec![
                "Plan: refactor error handling to use `?` and anyhow::Context.",
                "{\"query\": \"error handling\", \"n_results\": 5, \"collection\": \"repo\"}",
                "Replaced unwrap() with `?` and anyhow::Context.",
                "{\"verdict\": \"VALIDATED\", \"reason\": \"\"}",
            ],
            Some(fake_query_response()),
        )
        .await;
        assert!(!items.is_empty());
    }

    // Regression test for graceful degradation when Chroma is unreachable during the
    // Librarian's on-demand retrieval (`Stage::Retrieve`). Before the fix, `run_librarian_
    // retrieval` propagated `retrieve_and_generate`'s error straight through `?`, and
    // `Stage::Retrieve` in `run_stage_machine` propagates that further via its own `?` —
    // killing the whole run even though Architect/Worker/Test/Commit don't need RAG context.
    // Confirmed this test fails against the pre-fix `?`-propagation code (reverted locally,
    // ran, saw the panic from the unwrapped `Err`, then restored the fix) before finalizing.
    #[tokio::test]
    async fn run_librarian_retrieval_degrades_gracefully_when_chroma_is_unreachable() {
        use crate::agent::llm_client::fake_vector_store::FailingVectorStore;

        let mut config = base_config();
        config["Librarian"] = json!({ "model": "fake", "embed_model": "fake-embed" });

        let architect = Agent::new(&mut config, "Architect", true, None, json!({}))
            .await
            .unwrap();
        let worker = Agent::new(&mut config, "Worker", true, None, json!({}))
            .await
            .unwrap();
        let librarian = Agent::new(&mut config, "Librarian", false, None, json!({}))
            .await
            .ok();

        let mut orchestrator = Orchestrator {
            scoper: None,
            architect,
            worker,
            librarian,
            critics: Vec::new(),
            summarizer: None,
            validator: None,
            orchestrator_config: config,
            chat: Arc::new(FakeLlmClient::new(vec![
                "{\"query\": \"error handling\", \"n_results\": 5, \"collection\": \"repo\"}",
            ])),
            embed: Arc::new(FakeLlmClient::new(vec![])),
            client: Some(Arc::new(FailingVectorStore)),
        };

        let mut ctx = Context::new("fix the flaky test".to_string());
        let (tx, _rx) = mpsc::channel(100);

        let result = orchestrator.run_librarian_retrieval(&mut ctx, &tx).await;

        assert!(
            result.is_ok(),
            "a Chroma outage during Librarian retrieval must not fail the whole run: {result:?}"
        );
        let skipped = ctx.turns.iter().find(|t| {
            t.kind == TurnKind::System && t.content.contains("RAG retrieval was skipped")
        });
        assert!(
            skipped.is_some(),
            "expected a System turn noting RAG retrieval was skipped due to the outage"
        );
    }

    // `recall_prior_memories` is tested directly rather than through a fixture: it's not part
    // of the fixed debug-sequence mechanism (`debug_stage_machine`), it runs unconditionally
    // once per real `run_stage_machine` call before any sequence starts. Unlike the Librarian's
    // own on-demand retrieval, it never calls `query_stream`, so no `responses` entries are
    // needed — the query is built deterministically from `ctx.goal`, not an LLM-authored spec.
    #[tokio::test]
    async fn recall_prior_memories_pushes_a_retrieval_turn_when_librarian_configured() {
        let mut config = base_config();
        config["Librarian"] = json!({ "model": "fake", "embed_model": "fake-embed" });
        let orchestrator =
            build_test_orchestrator(config, vec![], Some(fake_query_response())).await;
        let mut ctx = Context::new("fix the flaky test".to_string());
        let (tx, _rx) = mpsc::channel(100);

        orchestrator.recall_prior_memories(&mut ctx, &tx).await;

        let recalled = ctx
            .turns
            .iter()
            .find(|t| t.kind == TurnKind::Retrieval && t.source == "Memory")
            .expect("recall_prior_memories should push a Memory retrieval turn");
        assert!(recalled.content.contains("fake retrieved document"));
    }

    #[tokio::test]
    async fn recall_prior_memories_is_a_noop_without_a_librarian() {
        // `query_response: None` here means no Chroma client at all was resolved for this
        // Orchestrator — not just "no Librarian" but "nothing to query against, period" (in a
        // real run, `Orchestrator::new`'s Worker-`embed_args` fallback below would also have
        // had to fail for this to happen). See the next test for "no Librarian, but a client
        // still resolved via the Worker's `embed_args`" — that one does recall successfully.
        let orchestrator = build_test_orchestrator(base_config(), vec![], None).await;
        let mut ctx = Context::new("fix the flaky test".to_string());
        let (tx, _rx) = mpsc::channel(100);

        orchestrator.recall_prior_memories(&mut ctx, &tx).await;

        assert!(ctx.turns.is_empty());
    }

    // Regression: a memorize-only run (no Librarian configured at all) could write memories via
    // the Worker's `Memorize` tool call (`Agent::embed`, which already builds its own
    // independent client from the Worker's `embed_args`/`EmbedArgs::default()`) but could never
    // recall them — `recall_prior_memories` required `self.librarian` to be `Some`, unrelated to
    // whether anything was actually memorized. Fixed by resolving `self.client` independently in
    // `Orchestrator::new` from the Worker's `embed_args` whenever no Librarian client was built,
    // and having `recall_prior_memories` fall back to the Worker's own `embed_args` for the
    // collection name/embed model when no Librarian is configured to supply them.
    #[tokio::test]
    async fn recall_prior_memories_works_without_a_librarian_via_the_workers_embed_args() {
        let orchestrator =
            build_test_orchestrator(base_config(), vec![], Some(fake_query_response())).await;
        assert!(
            orchestrator.librarian.is_none(),
            "this scenario is specifically the no-Librarian case"
        );
        let mut ctx = Context::new("fix the flaky test".to_string());
        let (tx, _rx) = mpsc::channel(100);

        orchestrator.recall_prior_memories(&mut ctx, &tx).await;

        let memory_turn = ctx
            .turns
            .iter()
            .find(|t| t.kind == TurnKind::Retrieval && t.source == "Memory")
            .expect("recall_prior_memories should push a Memory retrieval turn even without a Librarian");
        assert!(memory_turn.content.contains("fake retrieved document"));
    }

    // Regression: a real run showed `recall_prior_memories` pulling in content that looked
    // unrelated to the task, traced to `Query::default()`'s `ChromaCollectionConfigArgs::
    // default()` falling back to the literal collection named "default" whenever no
    // "collection" key is set — which this ad-hoc pre-run recall never did, unlike
    // `run_librarian_retrieval`'s LLM-driven query (which picks a collection itself, guided by
    // `task_hint`). So a run configured with `--collection repo_src-all-minilm_l6-v2` (`ask.rs`,
    // which also now sets `memory_collection` on the Librarian's config for exactly this) was
    // silently querying an unrelated "default" collection for memory recall the whole time.
    #[tokio::test]
    async fn recall_prior_memories_queries_the_configured_memory_collection() {
        use crate::agent::llm_client::fake_vector_store::RecordingVectorStore;

        let mut config = base_config();
        config["Librarian"] = json!({
            "model": "fake",
            "embed_model": "fake-embed",
            "memory_collection": "repo_src-all-minilm_l6-v2",
        });

        let architect = Agent::new(&mut config, "Architect", true, None, json!({}))
            .await
            .unwrap();
        let worker = Agent::new(&mut config, "Worker", true, None, json!({}))
            .await
            .unwrap();
        let librarian = Agent::new(&mut config, "Librarian", false, None, json!({}))
            .await
            .ok();

        let store = Arc::new(RecordingVectorStore::new(fake_query_response()));
        let orchestrator = Orchestrator {
            scoper: None,
            architect,
            worker,
            librarian,
            critics: Vec::new(),
            summarizer: None,
            validator: None,
            orchestrator_config: config,
            chat: Arc::new(FakeLlmClient::new(vec![])),
            embed: Arc::new(FakeLlmClient::new(vec![])),
            client: Some(store.clone()),
        };

        let mut ctx = Context::new("fix the flaky test".to_string());
        let (tx, _rx) = mpsc::channel(100);

        orchestrator.recall_prior_memories(&mut ctx, &tx).await;

        let recorded = store.recorded_collections.lock().unwrap();
        assert_eq!(
            recorded.as_slice(),
            ["repo_src-all-minilm_l6-v2"],
            "expected the configured collection to be queried, not the literal \"default\""
        );
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

    #[tokio::test]
    async fn worker_and_validator_completes() {
        let mut config = base_config();
        config["Validator"] = json!({ "model": "fake" });
        let items = run_fixture(
            "worker_and_validator.json",
            config,
            vec![
                "fn foo() -> Result<()> { do_thing()?; Ok(()) }",
                "{\"verdict\": \"VALIDATED\", \"reason\": \"\"}",
            ],
            None,
        )
        .await;
        assert!(!items.is_empty());
    }

    #[tokio::test]
    async fn worker_and_validator_rejection_completes() {
        let mut config = base_config();
        config["Validator"] = json!({ "model": "fake" });
        let items = run_fixture(
            "worker_and_validator_rejection.json",
            config,
            vec![
                "fn foo() { do_thing().unwrap(); }",
                "{\"verdict\": \"REJECTED\", \"reason\": \"still uses unwrap()\"}",
            ],
            None,
        )
        .await;
        assert!(!items.is_empty());
    }

    #[tokio::test]
    async fn summarizer_completes() {
        let mut config = base_config();
        config["Summarizer"] = json!({ "model": "fake" });
        let items = run_fixture(
            "summarizer.json",
            config,
            vec!["Summary: several rejections over unwrap() usage; not yet resolved."],
            None,
        )
        .await;
        assert!(!items.is_empty());
    }

    // Regression canary for document summarization before the Worker (maintainer: "work on
    // roadmap 0.3 items" -> "Document summarization before the Worker"). Retrieved RAG content
    // used to always go straight into a Retrieval turn raw, however large — no compression step
    // existed between `Query::query`'s rendered output and `ctx.push_turn`.
    #[tokio::test]
    async fn maybe_summarize_retrieved_docs_is_a_noop_without_a_summarizer_configured() {
        let orchestrator = build_test_orchestrator(base_config(), vec![], None).await;
        let mut ctx = Context::new("goal".to_string());
        let (tx, _rx) = mpsc::channel(100);
        let large_docs = "x ".repeat(2000); // well over the summarization threshold

        let result = orchestrator
            .maybe_summarize_retrieved_docs(large_docs.clone(), &mut ctx, &tx)
            .await;

        assert_eq!(
            result, large_docs,
            "no Summarizer configured -> pass through unchanged"
        );
    }

    #[tokio::test]
    async fn maybe_summarize_retrieved_docs_passes_through_small_docs_unchanged() {
        let mut config = base_config();
        config["Summarizer"] = json!({ "model": "fake" });
        // A FakeLlmClient with zero scripted responses would panic if chat_stream were called —
        // proves small docs never trigger the summarization LLM call at all.
        let orchestrator = build_test_orchestrator(config, vec![], None).await;
        let mut ctx = Context::new("goal".to_string());
        let (tx, _rx) = mpsc::channel(100);
        let small_docs = "a short retrieved snippet".to_string();

        let result = orchestrator
            .maybe_summarize_retrieved_docs(small_docs.clone(), &mut ctx, &tx)
            .await;

        assert_eq!(result, small_docs);
    }

    #[tokio::test]
    async fn maybe_summarize_retrieved_docs_condenses_large_docs_when_a_summarizer_is_configured() {
        let mut config = base_config();
        config["Summarizer"] = json!({ "model": "fake" });
        let orchestrator = build_test_orchestrator(
            config,
            vec!["Condensed: fn foo() lives in src/lib.rs; rest was boilerplate metadata."],
            None,
        )
        .await;
        let mut ctx = Context::new("goal".to_string());
        let (tx, _rx) = mpsc::channel(100);
        let large_docs = "x ".repeat(2000);

        let result = orchestrator
            .maybe_summarize_retrieved_docs(large_docs.clone(), &mut ctx, &tx)
            .await;

        assert_eq!(
            result,
            "Condensed: fn foo() lives in src/lib.rs; rest was boilerplate metadata."
        );
        assert_ne!(result, large_docs);
    }

    #[tokio::test]
    async fn maybe_summarize_retrieved_docs_falls_back_to_raw_docs_if_summarization_fails() {
        let mut config = base_config();
        config["Summarizer"] = json!({ "model": "fake" });
        // An empty scripted response makes `summarize_retrieved_documents` return an error
        // ("LLM returned an empty document summary") — the retrieval must not be lost because
        // the compression step itself failed.
        let orchestrator = build_test_orchestrator(config, vec!["   "], None).await;
        let mut ctx = Context::new("goal".to_string());
        let (tx, _rx) = mpsc::channel(100);
        let large_docs = "x ".repeat(2000);

        let result = orchestrator
            .maybe_summarize_retrieved_docs(large_docs.clone(), &mut ctx, &tx)
            .await;

        assert_eq!(
            result, large_docs,
            "a failed summarization must fall back to the raw docs"
        );
    }

    #[tokio::test]
    async fn validator_only_completes() {
        let mut config = base_config();
        config["Validator"] = json!({ "model": "fake" });
        let items = run_fixture(
            "validator_only.json",
            config,
            vec!["{\"verdict\": \"REJECTED\", \"reason\": \"unwrap() on line 42\"}"],
            None,
        )
        .await;
        assert!(!items.is_empty());
    }

    /// Regression test for the `"Critic0"` vs `"Critic_0"` naming bug this
    /// fixture originally carried (see the module doc comment) — with two
    /// distinct scripted responses queued, if the sequence's second step
    /// silently dispatched to critic index 0 again instead of index 1, the
    /// `FakeLlmClient` queue would desync and either this call would consume
    /// the wrong response or the queue would run dry, panicking.
    #[tokio::test]
    async fn multiple_critics_dispatches_each_critic_once() {
        let mut config = base_config();
        config["Critics"] = json!([
            { "model": "fake", "task": "security review" },
            { "model": "fake", "task": "performance review" },
        ]);
        let items = run_fixture(
            "multiple_critics.json",
            config,
            vec!["No issues found.\nAPPROVED", "Looks efficient.\nAPPROVED"],
            None,
        )
        .await;
        assert!(!items.is_empty());
    }

    // Regression test for `run_critics_parallel` specifically — NOT via a fixture: fixtures run
    // through `debug_stage_machine`, whose "Critic_N" branch calls each critic's `query_stream`
    // directly and sequentially (see the `starts_with("Critic")` arm above), which never had
    // the bug and doesn't exercise `run_critics_parallel` at all (confirmed by temporarily
    // reverting the fix and finding `multiple_critics_dispatches_each_critic_once` above still
    // passed unchanged — it was testing the wrong code path). `run_critics_parallel` only runs
    // from the real `run_stage_machine`'s `Stage::Critique`, so it's called directly here.
    //
    // Before the fix: critics ran concurrently (`join_all`) but all streamed their responses
    // token-by-token onto the one shared `tx`, so two critics' output could interleave
    // character-by-character with no way for a renderer to tell them apart. Each critic's full
    // response must now arrive as one contiguous, clearly-labeled `Trace` block instead.
    #[tokio::test]
    async fn run_critics_parallel_emits_one_undamaged_labeled_trace_per_critic() {
        let mut config = base_config();
        config["Critics"] = json!([
            { "model": "fake", "name": "Security", "task": "security review" },
            { "model": "fake", "name": "Performance", "task": "performance review" },
        ]);
        let mut orchestrator = build_test_orchestrator(
            config,
            vec!["No issues found.\nAPPROVED", "Looks efficient.\nAPPROVED"],
            None,
        )
        .await;
        let mut ctx = Context::new("goal".to_string());
        ctx.output = "some implementation".to_string();
        let (tx, mut rx) = mpsc::channel(100);

        orchestrator
            .run_critics_parallel(&mut ctx, &tx)
            .await
            .unwrap();
        drop(tx);
        let mut traces = Vec::new();
        while let Some(item) = rx.recv().await {
            if let Ok(StreamItem::Event(AgentEvent::Trace(msg))) = item {
                traces.push(msg);
            }
        }

        // Exactly one trace block per critic — neither merged into the other nor dropped.
        assert_eq!(
            traces.len(),
            2,
            "expected exactly one trace per critic, got: {traces:?}"
        );

        // Deliberately NOT asserting which critic got which canned response: both critics race
        // on `FakeLlmClient`'s shared scripted-response queue (a plain `VecDeque`, popped in
        // call order), and since they now genuinely run concurrently (the fix this test
        // guards), which one's `chat_stream` call wins that race isn't deterministic — this
        // was a flaky test bug caught by running the full suite repeatedly, not a code bug.
        // What must hold regardless: each critic is clearly labeled, and each trace contains
        // exactly one critic's response, never both spliced together and never neither.
        assert!(traces.iter().any(|t| t.starts_with("[Critic 'Security']:")));
        assert!(
            traces
                .iter()
                .any(|t| t.starts_with("[Critic 'Performance']:"))
        );
        for t in &traces {
            let has_security_text = t.contains("No issues found.");
            let has_performance_text = t.contains("Looks efficient.");
            assert!(
                has_security_text ^ has_performance_text,
                "each trace should contain exactly one critic's response, not both or neither: {t:?}"
            );
        }

        // Both approved, so no rejection turns.
        assert!(!ctx.turns.iter().any(|t| t.kind == TurnKind::Rejection));
    }

    // Regression: maintainer feedback that the trace "only contains the agent output, not the
    // agent actions" — an approving critic's review used to push no turn at all, only the
    // ephemeral `ctx.trace(...)` call visible live on the event stream at the time, which is
    // never added to `ctx.turns` and so vanishes from the persisted trace file afterward. An
    // approving review is still an action the critic took and must be just as visible as a
    // rejecting one.
    #[tokio::test]
    async fn run_critics_parallel_records_an_approving_review_as_a_system_turn() {
        let mut config = base_config();
        config["Critics"] =
            json!([{ "model": "fake", "name": "Security", "task": "security review" }]);
        let mut orchestrator =
            build_test_orchestrator(config, vec!["No issues found.\nAPPROVED"], None).await;
        let mut ctx = Context::new("goal".to_string());
        ctx.output = "some implementation".to_string();
        let (tx, _rx) = mpsc::channel(100);

        orchestrator
            .run_critics_parallel(&mut ctx, &tx)
            .await
            .unwrap();

        let recorded = ctx.turns.iter().find(|t| {
            t.kind == TurnKind::System
                && t.source == "Critic 'Security'"
                && t.content.contains("No issues found.")
        });
        assert!(
            recorded.is_some(),
            "expected the approving critic's review recorded as a System turn, got: {:?}",
            ctx.turns
        );
    }

    // Regression: same class of bug as the critic-approval fix above — a Scoper round that
    // found nothing notable to say (goal already READY, empty notes) used to leave `ctx.turns`
    // completely untouched, even though the Scoper's own output was streamed live to the
    // console. `run_scope_stage` now records the raw output unconditionally, before any of the
    // selective notes/lookup-rejection turns that only fire in specific cases.
    #[tokio::test]
    async fn run_scope_stage_records_its_raw_output_even_with_empty_notes() {
        let mut config = base_config();
        config["Scoper"] = json!({ "model": "fake" });
        let mut orchestrator =
            build_test_orchestrator(config, vec![r#"{"verdict": "READY", "notes": ""}"#], None)
                .await;
        let mut ctx = Context::new("goal".to_string());
        let (tx, _rx) = mpsc::channel(100);

        let stage = orchestrator.run_scope_stage(&mut ctx, &tx).await.unwrap();

        assert_eq!(stage, Stage::Plan);
        let recorded = ctx.turns.iter().find(|t| {
            t.kind == TurnKind::System && t.source == "Scoper" && t.content.contains("READY")
        });
        assert!(
            recorded.is_some(),
            "expected the Scoper's raw output recorded even with empty notes, got: {:?}",
            ctx.turns
        );
    }

    #[tokio::test]
    async fn critic_only_completes() {
        let mut config = base_config();
        config["Critics"] = json!([{ "model": "fake", "task": "security review" }]);
        let items = run_fixture(
            "critic.json",
            config,
            vec!["No issues found.\nAPPROVED"],
            None,
        )
        .await;
        assert!(!items.is_empty());
    }
}
