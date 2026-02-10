use crate::chroma::get_options::ChromaGetOptions;
use crate::chroma::ChromaClientConfigArgs;
use crate::chroma::ChromaCollectionConfigArgs;
use crate::error::Result;
use clap::Parser;

#[derive(Parser, Debug, Clone, PartialEq)]
pub(crate) struct MetadataArgs {
    /// Options for getting metadata
    #[clap(flatten)]
    get_options: ChromaGetOptions,
    /// Chroma client configuration
    #[clap(flatten)]
    client: ChromaClientConfigArgs,
    /// Chroma collection configuration
    #[clap(flatten)]
    collection: ChromaCollectionConfigArgs,
}

impl MetadataArgs {
    pub(crate) async fn get_metadata(&self) -> Result<()> {
        let client = self.client.create_client().await?;
        let collection = self.collection.get_collection(&client, "default").await?;

        if let Some(get_options) = self.get_options.to_chroma_get_options()? {
            let res = collection.get(get_options).await?;
            if res.ids.is_empty() {
                println!("No records found in the collection.");
            } else {
                for (i, id) in res.ids.iter().enumerate() {
                    println!("ID: {}", id);
                    if let Some(metadata) = res.metadatas.as_ref().and_then(|m| m.get(i)) {
                        println!("Metadata: {}", serde_json::to_string_pretty(metadata)?);
                    } else {
                        println!("Metadata: None");
                    }
                    println!("-----------------------------");
                }
            }
        } else if let Some(map) = collection.metadata() {
            println!("{}", serde_json::to_string_pretty(&map)?);
        } else {
            println!("No metadata found for the collection.");
        }
        Ok(())
    }
}
