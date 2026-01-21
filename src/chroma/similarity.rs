use crate::chroma::{ChromaClientConfigArgs, ChromaCollectionConfigArgs};
use crate::error::RuChatError;
use anyhow::Result;
use chroma::types::{
    BooleanOperator, CompositeExpression, DocumentExpression, DocumentOperator, IncludeList,
    MetadataComparison, MetadataExpression, MetadataValue, PrimitiveOperator, Where,
};
use clap::Parser;

/// Chroma database similarity search command line arguments.
///
/// This struct defines the arguments required to perform a similarity
/// search in a Chroma database, including query parameters and database
/// connection information.
#[derive(Parser, Debug, Clone, PartialEq)]
pub struct SimilarityArgs {
    /// Query string to search for similar embeddings.
    #[arg(short, long)]
    pub(crate) query: String,

    /// Number of embeddings to return.
    #[arg(short, long, default_value = "1")]
    pub(crate) count: u32,

    /// Number of similar embeddings to return.
    #[arg(short, long, default_value = "5")]
    pub(crate) similarity_count: u32,

    /// Chroma database metadata, comma separated key:value pairs.
    #[arg(short, long)]
    pub(crate) metadata: Option<String>,

    #[command(flatten)]
    pub client_config: ChromaClientConfigArgs,

    #[command(flatten)]
    pub collection_config: ChromaCollectionConfigArgs,
}

/// Subcommand to find similar embeddings in a Chroma database.
///
/// This function connects to a Chroma database using the provided
/// arguments, performs a similarity search, and returns the results.
///
/// # Parameters
///
/// - `args`: The command-line arguments for the similarity search.
///
/// # Returns
///
/// A `Result` indicating success or failure.
pub(crate) async fn similarity_search(args: &SimilarityArgs) -> Result<(), RuChatError> {
    // Instantiate a ChromaClient to connect to the Chroma database
    let client = args.client_config.create_client()?;

    // Instantiate a ChromaCollection to perform operations on a collection
    let collection = args
        .collection_config
        .get_or_create_collection(&client)
        .await?;

    let ids: Option<Vec<String>> = None;
    let mut children = vec![];
    children.push(Where::Document(DocumentExpression {
        operator: DocumentOperator::Contains,
        pattern: args.query.clone(),
    }));
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
    let where_metadata = Some(Where::Composite(composite_expression));

    let limit = Some(args.count);
    let offset = None;
    let include = Some(IncludeList::default_get());
    let get_result = collection
        .get(
            ids.clone(),
            where_metadata.clone(),
            limit,
            offset,
            include.clone(),
        )
        .await?;

    if let Some(query_embeddings) = get_result.embeddings {
        let n_results = Some(args.similarity_count);
        let query_result = collection
            .query(query_embeddings, n_results, where_metadata, ids, include)
            .await?;

        // FIXME: This is a placeholder for the actual embeddings
        // Instantiate QueryOptions to perform a similarity search on the collection
        // Alternatively, an embedding_function can also be provided with query_texts to perform the search
        println!("Query result: {:?}", query_result);
    }

    Ok(())
}
