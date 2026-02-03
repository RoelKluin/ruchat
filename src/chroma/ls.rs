use crate::chroma::{ChromaClientConfigArgs, ChromaCollectionConfigArgs};
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
    #[command(flatten)]
    pub collection: ChromaCollectionConfigArgs,

    #[command(flatten)]
    pub client: ChromaClientConfigArgs,
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
pub(crate) async fn chroma_ls(args: ChromaLsArgs) -> Result<(), RuChatError> {
    // Instantiate a ChromaClient to connect to the Chroma database
    let client = args.client.create_client().await?;
    if args.collection.name().is_empty() {
        // List all collections in the database
        let collections = client.list_collections().await?;
        for collection in collections {
            eprintln!("Collection Name: {}", collection.name());
            eprintln!("Collection ID: {}", collection.id());
            eprintln!("Collection Metadata: {:?}", collection.metadata());
            eprintln!("Collection Count: {}", collection.count().await?);
            eprintln!("-----------------------------");
        }
    } else {
        // Instantiate a ChromaCollection to perform operations on a collection
        let collection = args.collection.get_collection(&client).await?;

        eprintln!("Collection Name: {}", collection.name());
        eprintln!("Collection ID: {}", collection.id());
        eprintln!("Collection Metadata: {:?}", collection.metadata());
        eprintln!("Collection Count: {}", collection.count().await?);
    }
    Ok(())
}
