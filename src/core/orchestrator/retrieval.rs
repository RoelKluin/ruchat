use super::{Orchestrator, OrchestratorResult, doc_summary};
use crate::Result;
use crate::RuChatError;
use crate::agent::json_extract::strip_json_fences;
use crate::agent::types::{Context, TurnKind};
use crate::providers::vector::chroma::query::Query;
use crate::retry_transient;
use serde_json::Value;
use tokio::sync::mpsc;

impl Orchestrator {
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
    pub(super) async fn maybe_summarize_retrieved_docs(
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

    pub(super) async fn run_librarian_retrieval(
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

        // One lump covering the whole retrieval: the Librarian's own query-construction LLM
        // call(s) below plus the actual Chroma round-trip and any doc summarization — pushed
        // with whichever of the two turns below actually lands.
        let librarian_start = std::time::Instant::now();
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
                ctx.push_turn_timed(TurnKind::Retrieval, "Librarian", docs, librarian_start);
            }
            Err(e) => {
                tracing::warn!(error = %e, "Librarian retrieval failed; continuing without RAG context");
                ctx.trace(
                    tx,
                    format!("Librarian retrieval skipped this round (retrieval failed): {e}"),
                )
                .await;
                ctx.push_turn_timed(
                    TurnKind::System,
                    "System",
                    format!(
                        "RAG retrieval was skipped this round because the retrieval lookup \
                         failed (Chroma may be unreachable): {e}. Continuing without retrieved \
                         context."
                    ),
                    librarian_start,
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
    pub(super) async fn recall_prior_memories(
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
        let recall_start = std::time::Instant::now();
        match q.query(client, &self.embed, &embed_model).await {
            Ok(docs) if !docs.trim().is_empty() => {
                let docs = self.maybe_summarize_retrieved_docs(docs, ctx, tx).await;
                ctx.push_turn_timed(TurnKind::Retrieval, "Memory", docs, recall_start);
            }
            Ok(_) => {}
            Err(e) => {
                ctx.trace(tx, format!("Memory recall skipped: {e}")).await;
            }
        }
    }

    pub(super) async fn handle_retrieve(
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

        let retrieve_start = std::time::Instant::now();
        let docs = q.query(client, &self.embed, &model).await?;
        let docs = self.maybe_summarize_retrieved_docs(docs, ctx, tx).await;
        ctx.push_turn_timed(TurnKind::Retrieval, "Retrieve", docs, retrieve_start);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::{base_config, build_test_orchestrator, fake_query_response};
    use super::Orchestrator;
    use crate::agent::Agent;
    use crate::agent::llm_client::FakeLlmClient;
    use crate::agent::types::{Context, TurnKind};
    use serde_json::json;
    use std::sync::Arc;
    use tokio::sync::mpsc;

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
}
