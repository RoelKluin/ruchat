use crate::args::EmbedArgs;
use crate::chroma::create_chroma_client;
use crate::error::RuChatError;
use crate::ollama::get_model_name;
use chromadb::collection::{ChromaCollection, CollectionEntries};
use log::warn;
use ollama_rs::generation::embeddings::request::GenerateEmbeddingsRequest;
use ollama_rs::Ollama;
use serde_json::{Map, Value};

pub(crate) async fn embed(ollama: Ollama, args: &EmbedArgs) -> Result<(), RuChatError> {
    let model_name = get_model_name(&ollama, &args.model).await?;
    if !model_name.contains("embed") {
        warn!("Model {} might not be an embeddings model", model_name);
    }
    let mut metadata = Map::new();
    if let Some(md) = &args.metadata {
        for s in md.split(',') {
            match s.split_once(':') {
                Some((k, v)) => _ = metadata.insert(k.to_string(), v.into()),
                None => return Err(RuChatError::InvalidMetadata(s.to_string())),
            }
        }
    }

    let request = GenerateEmbeddingsRequest::new(model_name, vec![args.prompt.as_str()].into());
    let client = create_chroma_client(
        args.chroma_token.as_deref(),
        &args.chroma_server,
        &args.chroma_database,
    )
    .await?;
    let res = ollama.generate_embeddings(request).await?;

    let collection: ChromaCollection = client
        .get_or_create_collection(&args.collection, None)
        .await?;
    let count_str = collection.count().await?.to_string();

    let collection_entries = CollectionEntries {
        ids: vec![count_str.as_str()],
        embeddings: Some(res.embeddings),
        metadatas: Some(vec![metadata]),
        documents: Some(vec![&args.prompt]),
    };

    let result: Value = collection.upsert(collection_entries, None).await?;
    eprintln!("{:?}", result);
    Ok(())
}
