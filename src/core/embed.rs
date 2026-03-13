use crate::chroma::{ChromaClientConfigArgs, ChromaCollectionConfigArgs, UpdateMetadataArrayArgs};
use crate::ollama::OllamaArgs;
use crate::{Result, RuChatError};
use chroma::types::UpdateMetadataValue;
use chroma::types::{Metadata, MetadataValue, UpdateMetadata};
use chrono::Utc;
use clap::{Parser, ValueEnum};
use log::info;
use md5::{Digest, Md5};
use ollama_rs::generation::embeddings::request::{EmbeddingsInput, GenerateEmbeddingsRequest};
use serde::Deserialize;
use std::collections::HashMap;
use std::result::Result as StdResult;
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
    collection_config: ChromaCollectionConfigArgs,

    #[command(flatten)]
    metadata: UpdateMetadataArrayArgs,
}

impl EmbedArgs {
    pub(crate) async fn embed(&self, prompt: &str, mode: UpsertMode) -> Result<()> {
        let (ollama, models) = self.ollama_args.init("all-minilm:l6-v2").await?;
        let model = models
            .last()
            .ok_or_else(|| RuChatError::InternalError("No model found".into()))?
            .to_string();

        let client = self.client_config.create_client()?;
        let collection = self
            .collection_config
            .get_collection(&client, "default")
            .await?;

        // 1. Processing and Slicing (Your existing logic)
        let raw_metadata = self.metadata.parse()?;
        let metadata_items: Vec<HashMap<String, _>> = raw_metadata
            .unwrap_or_default()
            .into_iter()
            .flatten()
            .collect();

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
            model.clone(),
            EmbeddingsInput::Multiple(chunk_texts.clone()),
        );
        let embed_res = ollama.generate_embeddings(request).await?;
        let embeddings = embed_res.embeddings;

        let mut final_ids = Vec::new();
        let mut final_embeddings = Vec::new();
        let mut final_docs = Vec::new();
        let mut final_metadatas = Vec::new();

        for (i, id) in chunk_ids.iter().enumerate() {
            // Deduplication check: only act if record is missing or we are in Upsert mode
            let exists = collection
                .get(Some(vec![id.clone()]), None, None, None, None)
                .await
                .is_ok();

            if mode == UpsertMode::Upsert || !exists {
                let mut meta = chunk_metadatas
                    .get(i)
                    .and_then(|m| m.clone())
                    .unwrap_or_default();

                // FIX 2: Correct Metadata value types
                meta.insert(
                    "created_at".to_string(),
                    UpdateMetadataValue::Str(chrono::Utc::now().to_rfc3339()),
                );
                meta.insert(
                    "model_origin".to_string(),
                    UpdateMetadataValue::Str(model.clone()),
                );

                final_ids.push(id.to_string());
                final_embeddings.push(embeddings[i].clone());
                final_docs.push(Some(chunk_texts[i].clone()));
                final_metadatas.push(Some(meta));
            }
        }

        if !final_ids.is_empty() {
            collection
                .upsert(
                    final_ids,
                    final_embeddings,
                    Some(final_docs),
                    None,
                    Some(final_metadatas),
                )
                .await?;
        }

        let docs_to_send: Option<Vec<Option<String>>> =
            Some(chunk_texts.into_iter().map(Some).collect());
        let metadatas_to_send: Option<Vec<Option<UpdateMetadata>>> = if chunk_metadatas.is_empty() {
            None
        } else {
            Some(chunk_metadatas)
        };

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
                    .add(chunk_ids, embeddings, docs_to_send, None, metadatas_to_send)
                    .await
                    .map_err(RuChatError::ChromaHttpClientError)?;
                info!("Added records");
            }
            UpsertMode::Update => {
                // Map embeddings for Update (Update accepts Option<Vec<Option<Vec<f32>>>>)
                let update_embeddings = Some(embeddings.into_iter().map(Some).collect());
                collection
                    .update(
                        chunk_ids,
                        update_embeddings,
                        docs_to_send,
                        None,
                        metadatas_to_send,
                    )
                    .await
                    .map_err(RuChatError::ChromaHttpClientError)?;
                info!("Updated Records");
            }
            UpsertMode::Upsert => {
                collection
                    .upsert(chunk_ids, embeddings, docs_to_send, None, metadatas_to_send)
                    .await
                    .map_err(RuChatError::ChromaHttpClientError)?;
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
    #[arg(short, long, value_enum, default_value = "upsert")]
    mode: UpsertMode,

    #[command(flatten)]
    args: EmbedArgs,
}

impl EmbedPromptArgs {
    pub(crate) async fn embed(&self) -> Result<()> {
        self.args.embed(self.prompt.as_str(), self.mode).await
    }
}
