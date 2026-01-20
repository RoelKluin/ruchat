use crate::chroma::{
    create_client, get_or_create_chroma_collection, ChromaClientConfigArgs,
    ChromaCollectionConfigArgs,
};
use crate::error::RuChatError;
use crate::io::Io;
use crate::ollama::OllamaArgs;
use anyhow::Result;
use chroma::types::{
    BooleanOperator, CompositeExpression, DocumentExpression, DocumentOperator, IncludeList,
    MetadataComparison, MetadataExpression, MetadataValue, PrimitiveOperator, Where,
};
use clap::Parser;
use ollama_rs::generation::completion::request::GenerationRequest;
use ollama_rs::Ollama;
use tokio_stream::StreamExt;

/// Command-line arguments for querying a Chroma database.
///
/// This struct defines the arguments required to perform a query
/// in a Chroma database, including model details, query parameters,
/// and database connection information.
#[derive(Parser, Debug, Clone, PartialEq)]
pub struct QueryArgs {
    /// The query string to search for in the database.
    #[arg(short, long)]
    pub(crate) query: String,

    /// The prompt to use for generating a response.
    #[arg(short, long)]
    pub(crate) prompt: String,

    /// The number of results to return.
    #[arg(short, long, default_value = "1")]
    pub(crate) count: u32,

    /// Chroma database collection name.
    #[arg(short, long, default_value = "default")]
    pub(crate) collection: String,

    /// Chroma database metadata, comma separated key:value pairs.
    #[arg(short, long)]
    pub(crate) metadata: Option<String>,

    #[command(flatten)]
    pub collection_config: ChromaCollectionConfigArgs,

    #[command(flatten)]
    pub client_config: ChromaClientConfigArgs,

    #[command(flatten)]
    pub(crate) ollama_args: OllamaArgs,
}

pub async fn query_chroma(args: &QueryArgs) -> Result<Vec<Vec<f32>>, RuChatError> {
    let mut children = vec![Where::Document(DocumentExpression {
        operator: DocumentOperator::Contains,
        pattern: args.query.clone(),
    })];

    if let Some(metadata) = args.metadata.as_ref() {
        metadata.split(',').for_each(|s| {
            if let Some((k, v)) = s.split_once(':') {
                children.push(Where::Metadata(MetadataExpression {
                    key: k.to_string(),
                    comparison: MetadataComparison::Primitive(
                        PrimitiveOperator::Equal,
                        MetadataValue::Str(v.to_string()),
                    ),
                }));
            }
        });
    }

    let composite_expression = CompositeExpression {
        operator: BooleanOperator::And,
        children,
    };

    let where_metadata = Some(Where::Composite(composite_expression));

    let client = create_client(&args.client_config)?;
    // Perform the query
    let collection_uuid =
        get_or_create_chroma_collection(&client, &args.collection, &args.collection_config).await?;
    let collection = client.get_collection(&collection_uuid).await?;
    let query_embeddings: Vec<Vec<f32>> = vec![];
    let n_results: Option<u32> = Some(args.count);
    let where_metadata: Option<Where> = where_metadata;
    let ids: Option<Vec<String>> = None;
    let include: Option<IncludeList> = Some(IncludeList::default_get());

    let result = collection
        .query(query_embeddings, n_results, where_metadata, ids, include)
        .await?;
    let embeddings: Option<Vec<Vec<Option<Vec<f32>>>>> = result.embeddings;

    match embeddings {
        Some(embeddings) => Ok(embeddings
            .into_iter()
            .map(|e| e.into_iter().filter_map(|v| v).flatten().collect())
            .collect()),
        None => Ok(vec![]),
    }
}

/// Performs a query on a Chroma database and generates a response.
///
/// This function connects to a Chroma database using the provided
/// arguments, performs a query, and generates a response using the
/// specified model.
///
/// # Parameters
///
/// - `ollama`: The Ollama client for generating responses.
/// - `args`: The command-line arguments for the query.
///
/// # Returns
///
/// A `Result` indicating success or failure.
pub(crate) async fn query(ollama: Ollama, args: &QueryArgs) -> Result<(), RuChatError> {
    let client = create_client(&args.client_config)?;

    // Get embeddings from a collection with filters and limit set to 1.
    // An empty IDs vec will return all embeddings.
    let ids: Option<Vec<String>> = None;
    let mut children = vec![Where::Document(DocumentExpression {
        operator: DocumentOperator::Contains,
        pattern: args.query.clone(),
    })];

    if let Some(md) = args.metadata.as_ref() {
        md.split(',').for_each(|s| {
            if let Some((k, v)) = s.split_once(':') {
                children.push(Where::Metadata(MetadataExpression {
                    key: k.to_string(),
                    comparison: MetadataComparison::Primitive(
                        PrimitiveOperator::Equal,
                        MetadataValue::Str(v.to_string()),
                    ),
                }))
            }
        });
    }
    let children = vec![Where::Composite(CompositeExpression {
        operator: BooleanOperator::And,
        children,
    })];
    let composite_expression = CompositeExpression {
        operator: BooleanOperator::And,
        children,
    };
    let where_metadata = Where::Composite(composite_expression);

    let limit = Some(args.count);
    let offset = None;
    let include = Some(IncludeList::default_get());
    let collection = client
        .get_or_create_collection(&args.collection, None, None)
        .await?;
    let get_result = collection
        .get(ids, Some(where_metadata), limit, offset, include)
        .await?;
    let res: Vec<_> = get_result
        .embeddings
        .map(|embeddings| embeddings.into_iter().flatten().collect())
        .unwrap_or_default();
    eprintln!("Get result: {:?}", res);
    let prompt = format!(
        "Using this data: {:?}, respond to this prompt: {}",
        res, args.prompt
    );

    let mut cio = Io::new();
    let model_name = args.ollama_args.get_model(&ollama).await?;
    let options = args.ollama_args.get_options().await?;
    let request = GenerationRequest::new(model_name, prompt).options(options);
    let mut stream = ollama.generate_stream(request).await?;
    while let Some(res) = stream.next().await {
        let responses = res?;
        for resp in responses {
            cio.write_line(&resp.response).await?;
        }
    }
    Ok(())
}
