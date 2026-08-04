use crate::agent::llm_client::{VectorCollection, VectorStore};
use crate::chroma::{ChromaClientConfigArgs, ChromaCollectionConfigArgs, UpdateMetadataArrayArgs};
use crate::ollama::OllamaArgs;
use crate::sqlite_vec::SqliteVecClientConfigArgs;
use crate::{Result, RuChatError, VectorProvider};
use chroma::types::UpdateMetadataValue;
use chroma::types::{Metadata, MetadataValue, UpdateMetadata};
use chrono::Utc;
use clap::{Parser, ValueEnum};
use log::info;
use md5::{Digest, Md5};
use ollama_rs::Ollama;
use ollama_rs::generation::embeddings::request::{EmbeddingsInput, GenerateEmbeddingsRequest};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::result::Result as StdResult;
use std::sync::Arc;
use uuid::Builder;

/// The mode of operation for record synchronization.
#[derive(ValueEnum, Debug, Clone, PartialEq, Copy, Deserialize)]
pub(crate) enum UpsertMode {
    /// Only insert new records. Fails if IDs exist.
    Add,
    /// Only update existing records. Fails if IDs do not exist.
    Update,
    /// Insert new or update existing records. (Default)
    Upsert,
}

#[derive(Parser, Debug, Clone, PartialEq, Deserialize, Default)]
pub(crate) struct EmbedArgs {
    /// Optional prefix or base ID for the generated chunk IDs.
    #[arg(short, long)]
    id: Option<String>,

    #[command(flatten)]
    ollama_args: OllamaArgs,

    #[command(flatten)]
    client_config: ChromaClientConfigArgs,

    #[command(flatten)]
    sqlite_vec_client_config: SqliteVecClientConfigArgs,

    /// Which vector-store backend this `EmbedArgs` writes to/reads from —
    /// Chroma (default) or a local SQLite-vec file (`--sqlite-vec-path`).
    #[arg(long, value_enum, default_value_t = VectorProvider::Chroma, help_heading = "Vector Store")]
    vector_provider: VectorProvider,

    #[command(flatten)]
    collection_config: ChromaCollectionConfigArgs,

    #[command(flatten)]
    metadata: UpdateMetadataArrayArgs,
}

impl EmbedArgs {
    pub(crate) async fn embed(&self, prompt: &str, mode: UpsertMode, cfg: &Value) -> Result<()> {
        let raw_metadata = self.metadata.parse()?;
        let metadata_items: Vec<HashMap<String, _>> = raw_metadata
            .unwrap_or_default()
            .into_iter()
            .flatten()
            .collect();
        self.embed_with_metadata_items(prompt, mode, cfg, metadata_items)
            .await
    }

    pub(crate) fn set_id_prefix(&mut self, prefix: String) {
        self.id = Some(prefix);
    }

    /// The collection this `EmbedArgs` writes to (and so, the collection a caller must query
    /// to read back what it wrote). Used by `recall_prior_memories` (`orchestrator.rs`) so a
    /// memorize-only run (no Librarian configured) can recall via the same `EmbedArgs` the
    /// Worker's `Memorize` tool call already writes through (`Agent::embed`), rather than the
    /// literal `"default"` `ChromaCollectionConfigArgs::default()` would otherwise resolve to.
    pub(crate) fn collection_name(&self) -> &str {
        self.collection_config.name()
    }

    /// The embed model this `EmbedArgs` vectorizes with — see `collection_name`'s doc comment;
    /// same reasoning, recall must use the same embed model a write used or the vectors aren't
    /// comparable. Deliberately a synchronous config read (`OllamaArgs::model_name_or`), not
    /// `OllamaArgs::init`'s network-validated resolution: this only needs the configured name,
    /// not to confirm the model is pulled.
    pub(crate) fn embed_model_name(&self) -> String {
        self.ollama_args.model_name_or("all-minilm:l6-v2")
    }

    /// An independent vector-store read client for this `EmbedArgs`'s own configured backend —
    /// mirrors `Orchestrator::new`'s Librarian client construction, but for the memorize-only
    /// path that has no Librarian to borrow one from. Respects `vector_provider` the same way
    /// `resolve_collection` (the write side) does, so a memorize-only run configured for
    /// SQLite-vec can actually recall what it wrote.
    pub(crate) async fn client(&self, cfg: &Value) -> Result<Arc<dyn VectorStore>> {
        match self.vector_provider {
            VectorProvider::Chroma => {
                Ok(Arc::new(self.client_config.create_client(cfg).await?) as Arc<dyn VectorStore>)
            }
            VectorProvider::SqliteVec => Ok(Arc::new(
                self.sqlite_vec_client_config.create_client().await?,
            ) as Arc<dyn VectorStore>),
        }
    }

    /// Resolves the collection this `EmbedArgs` writes to, as a `Box<dyn VectorCollection>` —
    /// the one place `vector_provider` is branched on for the write path, so
    /// `embed_with_metadata_items`/`embed_raw_items`/`embed_chunks` stay backend-agnostic below
    /// this point. Chroma needs an explicit get-or-create-free `get_collection` (unchanged
    /// behavior); SQLite-vec has no separate "collection must already exist" concept — tables
    /// are created lazily on first write (see `SqliteVecCollection::ensure_schema`), so opening
    /// one here never fails just because it's new.
    async fn resolve_collection(&self, cfg: &Value) -> Result<Box<dyn VectorCollection>> {
        match self.vector_provider {
            VectorProvider::Chroma => {
                let client = self.client_config.create_client(cfg).await?;
                let collection = self
                    .collection_config
                    .get_collection(&client, "default")
                    .await?;
                Ok(Box::new(collection))
            }
            VectorProvider::SqliteVec => {
                let name = self.collection_config.name();
                let name = if name.is_empty() { "default" } else { name };
                let client = self.sqlite_vec_client_config.create_client().await?;
                let collection = client.collection(name)?;
                Ok(Box::new(collection))
            }
        }
    }

    /// Same as `embed`, but takes pre-built metadata items directly instead
    /// of parsing them from `self.metadata`'s CLI string. Lets callers like
    /// the ctags indexer (`core::index`) construct metadata programmatically
    /// while reusing all the existing chunk-slicing/upsert/dedup logic below
    /// unchanged.
    ///
    /// Unlike `embed_raw_items` below, this treats `prompt` as ONE shared
    /// text and uses each metadata item's `start`/`end` fields to slice a
    /// distinct chunk out of it by line range — the ctags indexer's model
    /// (one file, many symbol-scoped sub-chunks). Signature is unchanged
    /// from upstream; `core/index.rs` depends on it as-is.
    pub(crate) async fn embed_with_metadata_items(
        &self,
        prompt: &str,
        mode: UpsertMode,
        cfg: &Value,
        metadata_items: Vec<HashMap<String, UpdateMetadataValue>>,
    ) -> Result<()> {
        let (ollama, models) = self.ollama_args.init("all-minilm:l6-v2", cfg).await?;
        let model = models
            .last()
            .ok_or_else(|| RuChatError::InternalError("No model found".into()))?
            .to_string();

        let collection = self.resolve_collection(cfg).await?;

        // 1. Processing and Slicing
        let line_pool: Vec<&str> = prompt.lines().collect();
        let mut chunk_texts: Vec<String> = Vec::new();
        let mut chunk_metadatas: Vec<Option<UpdateMetadata>> = Vec::new();

        if metadata_items.len() < 2 {
            chunk_texts.push(prompt.to_string());
            if !metadata_items.is_empty() {
                chunk_metadatas.push(Some(metadata_items[0].clone()));
            }
        } else {
            for meta in metadata_items {
                // NOTE (pre-existing, unrelated to this rebase): the
                // created_at/model_origin overwrite below via `meta_value`
                // is discarded — `chunk_metadatas.push(Some(meta))` a few
                // lines down pushes the ORIGINAL `meta`, not `meta_value`.
                // Both fields get correctly (re)written later in
                // `embed_chunks`'s per-chunk loop, so this is dead code
                // rather than a bug with an observable effect. Left as-is
                // to keep this rebase to the two actual conflicts.
                let mut meta_value = serde_json::to_value(&meta).unwrap_or_default();
                if let Some(v) = meta_value.get_mut("created_at") {
                    *v = serde_json::json!(Utc::now().to_rfc3339());
                }
                if let Some(v) = meta_value.get_mut("model_origin") {
                    *v = serde_json::json!(model.clone());
                }

                let start = meta_value
                    .get("start")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(1) as u32;
                let end = meta_value
                    .get("end")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(line_pool.len() as u64) as u32;

                let slice_start = (start.saturating_sub(1)) as usize;
                let slice_end = (end as usize).min(line_pool.len());

                chunk_texts.push(line_pool[slice_start..slice_end].join("\n"));
                chunk_metadatas.push(Some(meta));
            }
        }

        self.embed_chunks(
            chunk_texts,
            chunk_metadatas,
            mode,
            &ollama,
            collection.as_ref(),
            &model,
        )
        .await
    }

    /// Embeds pre-chunked `(text, metadata)` pairs verbatim — each pair is
    /// its own complete chunk, with NO line-range slicing against a shared
    /// prompt. Complements `embed_with_metadata_items` (which owns the
    /// slicing model for the ctags indexer) rather than replacing it; used
    /// by ingestion pipelines like `hist::HistIngestArgs` where each item
    /// (a commit message, a diff hunk) is already independent, fully-formed
    /// text.
    pub(crate) async fn embed_raw_items(
        &self,
        items: Vec<(String, UpdateMetadata)>,
        mode: UpsertMode,
        cfg: &Value,
    ) -> Result<()> {
        let (ollama, models) = self.ollama_args.init("all-minilm:l6-v2", cfg).await?;
        let model = models
            .last()
            .ok_or_else(|| RuChatError::InternalError("No model found".into()))?
            .to_string();

        let collection = self.resolve_collection(cfg).await?;

        let (chunk_texts, chunk_metadatas): (Vec<String>, Vec<Option<UpdateMetadata>>) = items
            .into_iter()
            .map(|(text, meta)| (text, Some(meta)))
            .unzip();

        self.embed_chunks(
            chunk_texts,
            chunk_metadatas,
            mode,
            &ollama,
            collection.as_ref(),
            &model,
        )
        .await
    }

    /// Shared tail of `embed_with_metadata_items` and `embed_raw_items`:
    /// given already-chunked texts + optional per-chunk metadata, generates
    /// embeddings, checks existing IDs, and dispatches add/update/upsert per
    /// `mode`. Factored out of the original "2. Generate IDs and
    /// Embeddings" / "3. Unified Dispatch" sections, which were previously
    /// duplicated wholesale between callers — now both bottom out here.
    async fn embed_chunks(
        &self,
        chunk_texts: Vec<String>,
        chunk_metadatas: Vec<Option<UpdateMetadata>>,
        mode: UpsertMode,
        ollama: &Ollama,
        collection: &dyn VectorCollection,
        model: &str,
    ) -> Result<()> {
        // 2. Generate IDs and Embeddings
        let mut chunk_ids = Vec::new();
        for content in &chunk_texts {
            let hasher = Md5::new_with_prefix(format!(
                "{model}:{}:{}",
                self.id.as_deref().unwrap_or_default(),
                content
            ));
            let digest = hasher.finalize();
            let id = Builder::from_md5_bytes(digest.into())
                .into_uuid()
                .hyphenated()
                .to_string();
            chunk_ids.push(id);
        }
        let request = GenerateEmbeddingsRequest::new(
            model.to_string(),
            EmbeddingsInput::Multiple(chunk_texts.clone()),
        );
        let embed_res = ollama.generate_embeddings(request).await?;
        let embeddings = embed_res.embeddings;

        let mut final_ids = Vec::new();
        let mut final_embeddings = Vec::new();
        let mut final_docs = Vec::new();
        let mut final_metadatas = Vec::new();

        // Batched existence check: one round trip for all chunk IDs instead of
        // one existence check per chunk. An error (e.g. collection empty/not
        // yet created) is treated the same as "nothing exists yet".
        let existing_ids: HashSet<String> = collection
            .existing_ids(chunk_ids.clone())
            .await
            .unwrap_or_default();

        for (i, id) in chunk_ids.iter().enumerate() {
            let exists = existing_ids.contains(id);

            if mode == UpsertMode::Upsert || !exists {
                let mut meta = chunk_metadatas
                    .get(i)
                    .and_then(|m| m.clone())
                    .unwrap_or_default();

                meta.insert(
                    "created_at".to_string(),
                    UpdateMetadataValue::Str(chrono::Utc::now().to_rfc3339()),
                );
                meta.insert(
                    "model_origin".to_string(),
                    UpdateMetadataValue::Str(model.to_string()),
                );

                final_ids.push(id.to_string());
                final_embeddings.push(embeddings[i].clone());
                final_docs.push(Some(chunk_texts[i].clone()));
                final_metadatas.push(Some(meta));
            }
        }

        // Nothing new/changed to write — avoids calling add/update/upsert
        // with empty vectors below.
        if final_ids.is_empty() {
            return Ok(());
        }

        // `final_docs`/`final_metadatas` are already the deduped set this
        // function should actually write; the mode-dispatch match below now
        // operates on them directly instead of re-sending the full,
        // undeduped `chunk_ids`/`chunk_texts` — that was the source of the
        // double-write for `UpsertMode::Upsert` (once here via an
        // unconditional `.upsert()`, once again in the match arm below).
        let docs_to_send: Option<Vec<Option<String>>> = Some(final_docs);
        let metadatas_to_send: Option<Vec<Option<UpdateMetadata>>> = Some(final_metadatas);

        // 3. Unified Dispatch
        match mode {
            UpsertMode::Add => {
                let metadatas_to_send: Option<Vec<Option<Metadata>>> = metadatas_to_send
                    .map(|vec| {
                        vec.into_iter()
                            .map(|meta_opt| {
                                meta_opt
                                    .map(|meta| {
                                        meta.into_iter()
                                            .map(|(k, v)| {
                                                MetadataValue::try_from(&v).map(|mv| (k, mv))
                                            })
                                            .collect::<StdResult<Metadata, _>>()
                                    })
                                    .transpose()
                            })
                            .collect::<StdResult<Vec<Option<Metadata>>, _>>()
                    })
                    .transpose()
                    .map_err(|e| RuChatError::MetadataConversionError(e.to_string()))?;
                collection
                    .add(final_ids, final_embeddings, docs_to_send, metadatas_to_send)
                    .await?;
                info!("Added records");
            }
            UpsertMode::Update => {
                let update_embeddings = Some(final_embeddings.into_iter().map(Some).collect());
                collection
                    .update(
                        final_ids,
                        update_embeddings,
                        docs_to_send,
                        metadatas_to_send,
                    )
                    .await?;
                info!("Updated Records");
            }
            UpsertMode::Upsert => {
                collection
                    .upsert(final_ids, final_embeddings, docs_to_send, metadatas_to_send)
                    .await?;
                info!("Upserted records");
            }
        }

        Ok(())
    }
}

#[derive(Parser, Debug, Clone, PartialEq, Deserialize)]
pub(crate) struct EmbedPromptArgs {
    /// The text content to be embedded.
    prompt: String,

    /// The operation to perform.
    // Long-only, deliberately: collides with `EmbedArgs`'s flattened `UpdateMetadataArrayArgs::
    // metadata`, which also deliberately claims `-M` (see that field's own comment) — both
    // ending up on `EmbedPromptArgs` at once made every invocation of `ruchat embed` panic at
    // startup in debug builds, found while auditing for the analogous collision in query.rs.
    #[arg(long, value_enum, default_value = "upsert")]
    mode: UpsertMode,

    #[command(flatten)]
    args: EmbedArgs,
}

impl EmbedPromptArgs {
    pub(crate) async fn embed(&self, cfg: &Value) -> Result<()> {
        self.args.embed(self.prompt.as_str(), self.mode, cfg).await
    }
}
