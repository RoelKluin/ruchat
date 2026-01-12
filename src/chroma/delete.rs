use crate::chroma::{create_client, ChromaClientConfigArgs};
use crate::error::RuChatError;
use clap::Parser;

#[derive(Parser, Debug, Clone, PartialEq)]
pub struct ChromaDeleteArgs {
    #[arg(short, long)]
    pub collection: String,
    #[command(flatten)]
    pub client_config: ChromaClientConfigArgs,
}

pub async fn chroma_delete(args: &ChromaDeleteArgs) -> Result<(), RuChatError> {
    let client = create_client(&args.client_config)?;

    client.delete_collection(&args.collection).await?;
    println!("Deleted collection: {}", args.collection);
    Ok(())
}
