use crate::chroma::create_chroma_client;
use crate::error::RuChatError;
use crate::ollama::get_model_name;
use chromadb::collection::{ChromaCollection, CollectionEntries};
use chromadb::embeddings::EmbeddingFunction;
use clap::Parser;
use log::warn;
use ollama_rs::generation::embeddings::request::GenerateEmbeddingsRequest;
use ollama_rs::Ollama;
use serde_json::{Map, Value};

#[derive(Parser, Debug, Clone)]
pub struct EmbedArgs {
    #[clap(short, long, default_value = "nomic-embed-text:latest")]
    pub(crate) model: String,

    #[clap(short, long)]
    pub(crate) prompt: String,

    /// Chroma database server address and port
    #[clap(short = 'C', long, default_value = "http://localhost:8000")]
    pub(crate) chroma_server: String,

    /// Chroma database name
    #[clap(short = 'd', long, default_value = "default")]
    pub(crate) chroma_database: String,

    /// Chroma token for authentication
    #[clap(short = 't', long)]
    pub(crate) chroma_token: Option<String>,

    /// Chroma database collection name
    #[clap(short, long, default_value = "default")]
    pub(crate) collection: String,

    /// Chroma collection metadata, comma separated key:value pairs
    #[clap(short, long, default_value = "version:0.01")]
    pub(crate) collection_metadata: Option<String>,

    /// Chroma entries metadata, comma separated key:value pairs
    #[clap(short, long, default_value = "version:0.01")]
    pub(crate) entries_metadata: Option<String>,
}

fn get_metadata(arg_metadata: &Option<String>) -> Result<Option<Map<String, Value>>, RuChatError> {
    if arg_metadata.is_none() {
        return Ok(None);
    }
    let mut metadata = Map::new();
    if let Some(md) = arg_metadata {
        for s in md.split(',') {
            match s.split_once(':') {
                Some((k, v)) => _ = metadata.insert(k.to_string(), v.into()),
                None => return Err(RuChatError::InvalidMetadata(s.to_string())),
            }
        }
    }
    Ok(Some(metadata))
}

pub(crate) async fn embed(ollama: Ollama, args: &EmbedArgs) -> Result<(), RuChatError> {
    let model_name = get_model_name(&ollama, &args.model).await?;
    if !model_name.contains("embed") {
        warn!("Model {} might not be an embeddings model", model_name);
    }
    let entries_metadata = get_metadata(&args.entries_metadata)?;

    let request = GenerateEmbeddingsRequest::new(model_name, vec![args.prompt.as_str()].into());
    let client = create_chroma_client(
        args.chroma_token.as_deref(),
        &args.chroma_server,
        &args.chroma_database,
    )
    .await?;
    let res = ollama.generate_embeddings(request).await?;

    let collection_metadata = get_metadata(&args.collection_metadata)?;

    eprintln!("Collection name: {}", args.collection);
    // XXX error here.
    let collection: ChromaCollection = client
        .get_or_create_collection(&args.collection, collection_metadata)
        .await?;

    let id = collection.id().to_string();
    eprintln!("Collection Name: {}", collection.name());
    eprintln!("Collection ID: {}", id);
    eprintln!("Collection Metadata: {:?}", collection.metadata());
    eprintln!("Collection Count: {}", collection.count().await?);

    let collection_entries = CollectionEntries {
        ids: vec![id.as_str()],
        embeddings: Some(res.embeddings),
        metadatas: entries_metadata.map(|md| vec![md]),
        documents: Some(vec![&args.prompt]),
    };
    // The function to use to compute the embeddings. If None, embeddings must be provided.
    let embedding_function: Option<Box<dyn EmbeddingFunction>> = None;

    let result: Value = collection
        .upsert(collection_entries, embedding_function)
        .await?;
    eprintln!("{:?}", result);
    Ok(())
}
