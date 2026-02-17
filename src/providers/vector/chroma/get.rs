use crate::chroma::{ChromaClientConfigArgs, ChromaCollectionConfigArgs, parse_where};
use crate::RuChatError;
use anyhow::Result;
use clap::Parser;
use chroma::types::IncludeList;
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

    /// Optionally include documents in the get response, comma separated values.
    #[arg(short, long)]
    include: Option<String>,

    #[command(flatten)]
    collection: ChromaCollectionConfigArgs,

    #[command(flatten)]
    client: ChromaClientConfigArgs,
}

impl GetArgs {
    pub(crate) async fn get(&self) -> Result<(), RuChatError> {
        let client = self.client.create_client()?;
        let collection = self.collection.get_collection(&client, "default").await?;

        let ids: Option<Vec<String>> = self.ids.as_ref()
            .map(|s| s.split(',').map(|id| id.trim().to_string()).collect());

        let owhere = self.metadata.as_ref()
            .map(|md| parse_where(md))
            .transpose()?;

        let include_list = self.include.as_ref()
            .map(|inc| serde_json::from_str::<IncludeList>(inc))
            .transpose()?;

        let get_result = collection.get(
            ids, owhere, self.limit, self.offset, include_list,
        ).await?;

        let res: Vec<_> = get_result.documents.unwrap_or_default();
        info!("Get result: {}", serde_json::to_string_pretty(&res)?);
        Ok(())
    }


}
