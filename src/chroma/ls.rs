use crate::chroma::{create_client, ChromaClientConfigArgs};
use crate::error::RuChatError;
use anyhow::Result;
use clap::Parser;

/// Command-line arguments for listing Chroma database collections.
///
/// This struct defines the arguments required to list collections
/// in a Chroma database, including the collection name, server address,
/// database name, and an optional authentication token.
#[derive(Parser, Debug, Clone, PartialEq)]
pub struct ChromaLsArgs {
    /// Chroma database collection name.
    #[arg(short, long, default_value = "default")]
    pub(crate) collection: String,

    #[command(flatten)]
    pub client_config: ChromaClientConfigArgs,
}

/// Lists collections in a Chroma database.
///
/// This function connects to a Chroma database using the provided
/// arguments and lists the details of the specified collection.
///
/// # Parameters
///
/// - `args`: The command-line arguments for listing collections.
///
/// # Returns
///
/// A `Result` indicating success or failure.
pub(crate) async fn chroma_ls(args: &ChromaLsArgs) -> Result<(), RuChatError> {
    // Instantiate a ChromaClient to connect to the Chroma database
    let client = create_client(&args.client_config)?;

    // Instantiate a ChromaCollection to perform operations on a collection
    let collection = client
        .get_or_create_collection(&args.collection, None, None)
        .await?;
    eprintln!("Collection Name: {}", collection.name());
    eprintln!("Collection ID: {}", collection.id());
    eprintln!("Collection Metadata: {:?}", collection.metadata());
    eprintln!("Collection Count: {}", collection.count().await?);

    Ok(())
}
