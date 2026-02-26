use crate::chroma::{ChromaClientConfigArgs, ChromaCollectionConfigArgs, UpdateMetadataArrayArgs};
use crate::ollama::OllamaArgs;
use crate::RuChatError;
use clap::Parser;
use md5::{Digest, Md5};
use ollama_rs::generation::embeddings::request::GenerateEmbeddingsRequest;
use uuid::Builder;
use std::collections::HashMap;
use ollama_rs::generation::embeddings::request::EmbeddingsInput;

#[derive(Parser, Debug, Clone, PartialEq)]
pub(crate) struct EmbedPromptArgs {
    prompt: String,

    #[command(flatten)]
    embed_args: EmbedArgs,
}

impl EmbedPromptArgs {
    pub(crate) async fn embed(self) -> Result<(), RuChatError> {
        self.embed_args.embed(self.prompt).await
    }
}

/// Command-line arguments for embedding data into a Chroma database.
///
/// This struct defines the arguments required to perform an embedding
/// operation in a Chroma database, including model details, prompt,
/// and database connection information.
#[derive(Parser, Debug, Clone, PartialEq)]
pub(crate) struct EmbedArgs {
    /// ID associated with the embedding entry.
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
    /// Embeds data into a Chroma database.
    ///
    /// This function connects to a Chroma database using the provided
    /// arguments, generates embeddings for the specified prompt, and
    /// stores the embeddings in the database.
    ///
    /// # Parameters
    ///
    /// - `ollama`: The Ollama client for generating embeddings.
    /// # Returns
    ///
    /// A `Result` indicating success or failure.
    pub(crate) async fn embed(&self, prompt: String) -> Result<(), RuChatError> {
        // 1. Flatten the nested Result<Option<Vec<Option<UpdateMetadata>>>>
        let raw_metadata = self.metadata.parse()?; 
        let metadata_items: Vec<HashMap<String, _>> = raw_metadata
            .unwrap_or_default()
            .into_iter()
            .flatten() // Removes the inner Option<UpdateMetadata>
            .collect();
        let line_pool: Vec<&str> = prompt.lines().collect();
        let (ollama, models) = self.ollama_args.init("all-minilm:l6-v2").await?;
        let model = models.last().unwrap().to_string();

        let mut chunk_texts: Vec<String> = Vec::new();
        let mut chunk_ids = Vec::new();

        let client = self.client_config.create_client()?;
        let collection = self.collection_config.get_collection(&client, "").await?;
        // 2. Process each metadata entry to create slices
        let chunk_metadatas = if metadata_items.len() < 2 {
            // Fallback: If no metadata provided, treat whole prompt as one chunk
            chunk_texts.push(prompt.clone());
            if metadata_items.is_empty() {
                None
            } else {
                Some(vec![Some(metadata_items[0].clone())]) // Use the single metadata for the whole prompt
            }
        } else {
            let mut chunk_metadatas = Vec::new();
            for meta in metadata_items {
                // Assuming UpdateMetadata has start/end fields or can be converted to a Map
                // If it's a struct, you might need to use serde_json::to_value()
                let meta_value = serde_json::to_value(&meta).unwrap_or_default();
                let start = meta_value.get("start").and_then(|v| v.as_u64()).unwrap_or(1) as usize;
                let end = meta_value.get("end").and_then(|v| v.as_u64()).unwrap_or(line_pool.len() as u64) as usize;

                // Slicing logic (1-based to 0-based index)
                let slice_start = start.saturating_sub(1);
                let slice_end = end.min(line_pool.len());
                let content = line_pool[slice_start..slice_end].join("\n");

                chunk_texts.push(content);
                chunk_metadatas.push(Some(meta)); 
            }
            Some(chunk_metadatas)
        };

        // 3. Generate IDs for each chunk
        for content in &chunk_texts {
            let hasher = Md5::new_with_prefix(format!("{model}:{}:{:?}", self.id.clone().unwrap_or_default(), content));
            let digest = hasher.finalize();
            let id = Builder::from_md5_bytes(digest.into()).into_uuid().hyphenated().to_string();
            chunk_ids.push(id);
        }
        let chunk_texts_copy = Some(chunk_texts.iter().map(|s| Some(s.to_string())).collect());
        // 4. Batch Embedding Request
        let request = GenerateEmbeddingsRequest::new(
            model, 
            EmbeddingsInput::Multiple(chunk_texts)
        );
        let res = ollama.generate_embeddings(request).await?;
        let embeddings = res.embeddings;

        // 5. Parallel Upsert (IDs, Embeddings, Documents, and Metadatas all match in length)
        collection
            .upsert(
                chunk_ids,
                embeddings,
                chunk_texts_copy,// Pass the slices as documents
                None,              // URIs
                chunk_metadatas
            )
            .await?;

        Ok(())
    }
}
