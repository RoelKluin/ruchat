use crate::arg_utils::parse_key_val;
use anyhow::Result;
use chroma::types::{Cmek, Metadata, MetadataValue, Schema, ValueTypes};
use chroma::{ChromaCollection, ChromaHttpClient};
use clap::Parser;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Parser, Debug, Clone, PartialEq)]
pub struct ChromaCollectionConfigArgs {
    #[arg(short = 'c', long, default_value = "default")]
    pub collection: String,

    #[arg(short = 'm', long, value_name = "KEY:VALUE", value_parser = parse_key_val::<String, String>)]
    pub metadata: HashMap<String, String>,

    #[arg(short = 's', long, value_name = "KEY:VALUE", value_parser = parse_key_val::<String, String>)]
    pub schema: HashMap<String, String>,
}

impl ChromaCollectionConfigArgs {
    /// Create a collection in the chroma database
    pub async fn get_or_create_collection(
        &self,
        client: &ChromaHttpClient,
    ) -> Result<ChromaCollection> {
        let schema = match self.schema.is_empty() {
            true => None,
            false => {
                // FIXME: currently no defaults or keys support
                let defaults = ValueTypes {
                    string: None,
                    float_list: None,
                    sparse_vector: None,
                    int: None,
                    float: None,
                    boolean: None,
                };
                let keys: HashMap<String, ValueTypes> = HashMap::new();
                let cmek: Option<Cmek> = self
                    .schema
                    .get("cmek")
                    .cloned()
                    .map(|s| Cmek::Gcp(Arc::new(s)));
                let source_attached_function_id =
                    self.schema.get("source_attached_function_id").cloned();
                Some(Schema {
                    defaults,
                    keys,
                    cmek,
                    source_attached_function_id,
                })
            }
        };
        let metadata: Option<Metadata> = match self.metadata.is_empty() {
            true => None,
            false => {
                let mut md = Metadata::new();
                for (k, v) in self.metadata.iter() {
                    _ = md.insert(k.to_string(), MetadataValue::Str(v.to_string()));
                }
                Some(md)
            }
        };
        // Get or create a collection with the given name and no metadata.
        let collection = client
            .get_or_create_collection(self.collection.as_str(), schema, metadata)
            .await?;
        Ok(collection)
    }
}
