use crate::arg_utils::parse_key_val;
use crate::chroma::{ChromaClientConfigArgs, ChromaCollectionConfigArgs};
use crate::error::RuChatError;
use crate::ollama::model::get_name;
use chroma::types::{Metadata, MetadataValue, UpdateMetadata, UpdateMetadataValue};
use clap::Parser;
use log::warn;
use ollama_rs::generation::embeddings::request::GenerateEmbeddingsRequest;
use ollama_rs::Ollama;

/// Command-line arguments for embedding data into a Chroma database.
///
/// This struct defines the arguments required to perform an embedding
/// operation in a Chroma database, including model details, prompt,
/// and database connection information.
#[derive(Parser, Debug, Clone, PartialEq)]
pub struct EmbedArgs {
    /// The model to use for generating embeddings.
    #[arg(short, long, default_value = "nomic-embed-text:latest")]
    pub(crate) model: String,

    /// The prompt to embed.
    #[arg(short, long)]
    pub(crate) prompt: String,

    /// Chroma update metadata, comma separated key:value pairs.
    #[arg(short, long, value_name = "KEY:VALUE", value_parser = parse_key_val::<String, String>)]
    pub(crate) update_metadata: Option<String>,

    /// URIs associated with the embedding entries.
    #[arg(short, long)]
    pub(crate) uris: Option<Vec<Option<String>>>,

    #[command(flatten)]
    pub client_config: ChromaClientConfigArgs,

    #[command(flatten)]
    pub collection_config: ChromaCollectionConfigArgs,
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
fn get_metadata(arg_metadata: &Option<String>) -> Result<Option<Metadata>, RuChatError> {
    if arg_metadata.is_none() {
        return Ok(None);
    }
    let mut metadata = Metadata::new();
    if let Some(md) = arg_metadata {
        for s in md.split(',') {
            match s.split_once(':') {
                Some((k, v)) => {
                    _ = metadata.insert(k.to_string(), MetadataValue::Str(v.to_string()))
                }
                None => return Err(RuChatError::InvalidMetadata(s.to_string())),
            }
        }
    }
    Ok(Some(metadata))
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
pub(crate) async fn embed(ollama: Ollama, args: EmbedArgs) -> Result<(), RuChatError> {
    let model_name = get_name(&ollama, &args.model).await?;
    if !model_name.contains("embed") {
        warn!("Model {} might not be an embeddings model", model_name);
    }
    let update_metadata = get_update_metadata(&args.update_metadata)?;

    let request = GenerateEmbeddingsRequest::new(model_name, vec![args.prompt.as_str()].into());
    let res = ollama.generate_embeddings(request).await?;
    let client = args.client_config.create_client()?;

    eprintln!("Collection name: {}", args.collection_config.collection);
    let collection = args
        .collection_config
        .get_or_create_collection(&client)
        .await?;

    let id = collection.id().to_string();
    eprintln!("Collection Name: {}", collection.name());
    eprintln!("Collection ID: {}", id);
    eprintln!("Collection Metadata: {:?}", collection.metadata());
    eprintln!("Collection Count: {}", collection.count().await?);

    let ids = vec![id];
    let embeddings = res.embeddings;
    let documents = Some(vec![Some(args.prompt)]);
    let uris = args.uris.or_else(|| Some(vec![None]));

    collection
        .upsert(ids, embeddings, documents, uris, update_metadata)
        .await?;
    Ok(())
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
