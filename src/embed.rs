use crate::chroma::parse_metadata;
use crate::chroma::{ChromaClientConfigArgs, ChromaCollectionConfigArgs};
use crate::error::RuChatError;
use crate::ollama::OllamaArgs;
use chromadb::collection::CollectionEntries;
use chromadb::embeddings::EmbeddingFunction;
use clap::Parser;
use log::{info, warn};
use ollama_rs::generation::embeddings::request::GenerateEmbeddingsRequest;
use serde_json::{Map, Value};
use uuid::Builder;

#[derive(Parser, Debug, Clone, PartialEq)]
pub(super) struct EmbedPromptArgs {
    prompt: String,

    #[command(flatten)]
    embed_args: EmbedArgs,
}

impl EmbedPromptArgs {
    pub(super) async fn embed(self) -> Result<(), RuChatError> {
        self.embed_args.embed(self.prompt).await
    }
}

/// Command-line arguments for embedding data into a Chroma database.
///
/// This struct defines the arguments required to perform an embedding
/// operation in a Chroma database, including model details, prompt,
/// and database connection information.
#[derive(Parser, Debug, Clone, PartialEq)]
pub(super) struct EmbedArgs {
    /// Chroma update metadata, comma separated key:value pairs.
    #[arg(short, long)]
    metadata: Option<String>,

    /// ID associated with the embedding entry.
    #[arg(short, long)]
    id: Option<String>,

    #[command(flatten)]
    ollama_args: OllamaArgs,

    #[command(flatten)]
    client_config: ChromaClientConfigArgs,

    #[command(flatten)]
    collection_config: ChromaCollectionConfigArgs,
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
    pub(super) async fn embed(&self, prompt: String) -> Result<(), RuChatError> {
        let (ollama, models) = self.ollama_args.init("all-minilm:l6-v2").await?;
        let model = models.last().unwrap().to_string();
        if model != "all-minilm:l6-v2" && !model.contains("embed") {
            warn!("Model {model} might not be an embeddings model");
        }

        let id = match &self.id {
            Some(id) => id.to_string(),
            None => prompt
                .lines()
                .next()
                .ok_or(RuChatError::EmptyPrompt)?
                .to_string(),
        };
        let digest = md5::compute(format!("{model}:{id}"));
        let id = Builder::from_md5_bytes(digest.0)
            .into_uuid()
            .hyphenated()
            .to_string();

        let client = self.client_config.create_client().await?;
        let mut collection_metadata: Map<String, Value> = Map::new();
        collection_metadata.insert("model".to_string(), Value::String(model.clone()));

        let collection = self
            .collection_config
            .get_or_create_collection(&client, Some(collection_metadata))
            .await?;

        info!(
            "Targeting Collection: {} (ID: {})",
            collection.name(),
            collection.id()
        );

        let request = GenerateEmbeddingsRequest::new(model, vec![prompt.as_str()].into());
        let res = ollama.generate_embeddings(request).await?;

        let embeddings = res.embeddings;
        if !embeddings.is_empty() {
            info!("Generated embedding dimension: {}", embeddings[0].len());
        }

        let ids = vec![id.as_str()];
        let documents = None; //Some(vec![prompt.as_str()]);
        let metadata = parse_metadata(&self.metadata)?;
        let collection_entries = CollectionEntries {
            ids,
            metadatas: metadata.map(|md| vec![md]),
            documents,
            embeddings: Some(embeddings),
        };
        // The function to use to compute the embeddings. If None, embeddings must be provided.
        let embedding_function: Option<Box<dyn EmbeddingFunction>> = None;

        let result = collection
            .upsert(collection_entries, embedding_function)
            .await?;
        info!("Upserted {}", result);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_metadata_valid() {
        let metadata_str = Some("key1:value1,key2:value2".to_string());
        let result = parse_metadata(&metadata_str);
        assert!(result.is_ok());
        let metadata = result.unwrap().unwrap();
        assert_eq!(metadata["key1"], "value1".into());
        assert_eq!(metadata["key2"], "value2".into());
    }

    #[test]
    fn test_get_metadata_invalid() {
        let metadata_str = Some("key1value1".to_string());
        let result = parse_metadata(&metadata_str);
        assert!(result.is_err());
    }
}
