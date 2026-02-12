use crate::chroma::parse_metadata;
use crate::chroma::{ChromaClientConfigArgs, ChromaCollectionConfigArgs};
use crate::error::RuChatError;
use crate::ollama::OllamaArgs;
use chromadb::collection::CollectionEntries;
use chromadb::embeddings::EmbeddingFunction;
use clap::Parser;
use log::{info, warn};
use md5::{Digest, Md5};
use ollama_rs::generation::embeddings::request::GenerateEmbeddingsRequest;
use uuid::Builder;

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
    pub(crate) async fn embed(&self, prompt: String) -> Result<(), RuChatError> {
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
        let hasher = Md5::new_with_prefix(format!("{model}:{id}"));
        let digest = hasher.finalize();
        let id = Builder::from_md5_bytes(digest.into())
            .into_uuid()
            .hyphenated()
            .to_string();

        let client = self.client_config.create_client().await?;
        let collection = self.collection_config.get_collection(&client, "").await?;

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
