use crate::arg_utils::parse_key_val;
use crate::chroma::{ChromaClientConfigArgs, ChromaCollectionConfigArgs};
use crate::error::RuChatError;
use chroma::types::{UpdateMetadata, UpdateMetadataValue};
use clap::Parser;
use log::warn;
use ollama_rs::generation::embeddings::request::GenerateEmbeddingsRequest;
use uuid::Builder;
use md5;
use crate::ollama::OllamaArgs;

#[derive(Parser, Debug, Clone, PartialEq)]
pub struct EmbedPromptArgs {
    prompt: String,

    #[command(flatten)]
    embed_args: EmbedArgs,
}

impl EmbedPromptArgs {
    pub(crate) async fn embed(self) -> Result<(), RuChatError> {
        EmbedArgs::embed(self.prompt, self.embed_args).await
    }
}

/// Command-line arguments for embedding data into a Chroma database.
///
/// This struct defines the arguments required to perform an embedding
/// operation in a Chroma database, including model details, prompt,
/// and database connection information.
#[derive(Parser, Debug, Clone, PartialEq)]
pub struct EmbedArgs {
    /// Chroma update metadata, comma separated key:value pairs.
    #[arg(short, long, value_name = "KEY:VALUE", value_parser = parse_key_val::<String, String>)]
    update_metadata: Option<String>,

    /// ID associated with the embedding entry.
    #[arg(short, long)]
    id: Option<String>,

    /// URIs associated with the embedding entries.
    #[arg(short, long)]
    uris: Option<Vec<Option<String>>>,

    // FIXME: this is clashing with AskArgs ollama_args
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
    /// - `args`: The command-line arguments for the embedding operation.
    ///
    /// # Returns
    ///
    /// A `Result` indicating success or failure.
    pub(crate) async fn embed(prompt: String, args: EmbedArgs) -> Result<(), RuChatError> {
        let ollama = args.ollama_args.init()?;
        let model_name = args
            .ollama_args
            .get_model(&ollama, "all-minilm:l6-v2")
            .await?;
        if model_name != "all-minilm:l6-v2" && !model_name.contains("embed") {
            warn!("Model {} might not be an embeddings model", model_name);
        }

        let id = match args.id {
            Some(id) => id.to_string(),
            None => prompt.lines().next().ok_or(RuChatError::EmptyPrompt)?.to_string(),
        };
        let digest = md5::compute(format!("{model_name}:{id}"));
        let id = Builder::from_md5_bytes(digest.0).into_uuid().hyphenated().to_string();

        let client = args.client_config.create_client()?;

        eprintln!("Collection name: {}", args.collection_config.collection);
        // XXX: error here.
        let collection = args
            .collection_config
            .get_or_create_collection(&client)
            .await?;
        eprintln!("Connected to Chroma collection.");


        eprintln!("Collection Name: {}", collection.name());
        eprintln!("Collection ID: {}", id);
        eprintln!("Collection Metadata: {:?}", collection.metadata());
        eprintln!("Collection Count: {}", collection.count().await?);

        let ids = vec![id];
        let request = GenerateEmbeddingsRequest::new(model_name, vec![prompt.as_str()].into());
        let res = ollama.generate_embeddings(request).await?;
        let embeddings = res.embeddings;
        let documents = Some(vec![Some(prompt)]);
        let uris = args.uris.or_else(|| Some(vec![None]));
        let update_metadata = get_update_metadata(&args.update_metadata)?;

        collection
            .upsert(ids, embeddings, documents, uris, update_metadata)
            .await?;
        Ok(())
    }

}


/// Parses metadata from a string of comma-separated key:value pairs.
///
/// # Parameters
///
/// - `arg_metadata`: An optional string containing metadata.
///
/// # Returns
///
/// A `Result` containing an optional map of metadata or a `RuChatError`.
fn get_update_metadata(
    arg_metadata: &Option<String>,
) -> Result<Option<Vec<Option<UpdateMetadata>>>, RuChatError> {
    if arg_metadata.is_none() {
        return Ok(None);
    }
    let mut metadata = UpdateMetadata::new();
    if let Some(md) = arg_metadata {
        for s in md.split(',') {
            match s.split_once(':') {
                Some((k, v)) => {
                    _ = metadata.insert(k.to_string(), UpdateMetadataValue::Str(v.to_string()))
                }
                None => return Err(RuChatError::InvalidMetadata(s.to_string())),
            }
        }
    }
    Ok(Some(vec![Some(metadata)]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_metadata_valid() {
        let metadata_str = Some("key1:value1,key2:value2".to_string());
        let result = get_metadata(&metadata_str);
        assert!(result.is_ok());
        let metadata = result.unwrap().unwrap();
        assert_eq!(metadata["key1"], "value1".into());
        assert_eq!(metadata["key2"], "value2".into());
    }

    #[test]
    fn test_get_metadata_invalid() {
        let metadata_str = Some("key1value1".to_string());
        let result = get_metadata(&metadata_str);
        assert!(result.is_err());
    }
}
