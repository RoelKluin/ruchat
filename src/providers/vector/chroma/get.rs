use crate::chroma::{ChromaClientConfigArgs, ChromaCollectionConfigArgs, IncludeArgs, WhereArgs};
use crate::{RuChatError, Result};
use clap::Parser;
use log::info;

/// Command-line arguments for geting a Chroma database.
///
/// This struct defines the arguments required to perform a get operation
/// in a Chroma database, including model details, get parameters,
/// and database connection information.
#[derive(Parser, Debug, Clone, PartialEq)]
pub(crate) struct GetArgs {
    /// Comma separated list of document IDs to retrieve.
    #[arg(short, long)]
    ids: Option<String>,

    /// The number of results to return.
    #[arg(short, long)]
    limit: Option<u32>,

    /// The number of results to skip before returning results.
    #[arg(short, long)]
    offset: Option<u32>,

    /// Chroma database metadata, comma separated key:value pairs.
    #[arg(short, long)]
    metadata: Option<String>,

    #[command(flatten)]
    collection: ChromaCollectionConfigArgs,

    #[command(flatten)]
    client: ChromaClientConfigArgs,

    #[command(flatten)]
    include: IncludeArgs,

    #[command(flatten)]
    r#where: WhereArgs,
}

impl GetArgs {
    pub(crate) async fn get(&self) -> Result<()> {
        let client = self.client.create_client().map_err(RuChatError::ChromaError)?;
        let collection = self.collection.get_collection(&client, "default").await?;

        let ids: Option<Vec<String>> = self.ids.as_ref()
            .map(|s| s.split(',').map(|id| id.trim().to_string()).collect());

        let r#where = self.r#where.parse()?;

        let include_list = self.include.parse()?;

        let get_result = collection.get(
            ids, r#where, self.limit, self.offset, include_list,
        ).await
            .map_err(RuChatError::ChromaHttpClientError)?;

        let res: Vec<_> = get_result.documents.unwrap_or_default();
        info!("Get result: {:?}", res);
        Ok(())
    }


}
