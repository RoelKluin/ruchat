use crate::chroma::parse_metadata;
use crate::error::Result;
use chromadb::collection::GetOptions;
use clap::Parser;
use serde_json::Value;

#[derive(Parser, Debug, Clone, PartialEq)]
pub struct ChromaGetOptions {
    /// Common Chroma client configuration arguments
    #[arg(short, long)]
    pub ids: Vec<String>,
    /// Common Chroma collection configuration arguments
    #[arg(short, long)]
    pub where_metadata: Option<String>,
    /// Limit the number of results returned
    #[arg(short, long)]
    pub limit: Option<usize>,
    /// Offset for the results returned
    #[arg(short, long)]
    pub offset: Option<usize>,
    /// Filter based on document content
    #[arg(short, long)]
    pub where_document: Option<String>,
    /// Include specific fields in the results
    #[arg(short, long)]
    pub include: Vec<String>,
}

impl ChromaGetOptions {
    pub fn to_chroma_get_options(&self) -> Result<Option<GetOptions>> {
        let where_metadata = match parse_metadata(&self.where_metadata)? {
            Some(map) => Some(Value::Object(map)),
            None => None,
        };
        let where_document = match &self.where_document {
            Some(doc_str) => Some(Value::String(doc_str.clone())),
            None => None,
        };
        let include = if self.include.is_empty() {
            None
        } else {
            Some(self.include.clone())
        };
        if self.ids.is_empty()
            && where_metadata.is_none()
            && where_document.is_none()
            && include.is_none()
            && self.limit.is_none()
            && self.offset.is_none()
        {
            return Ok(None);
        }
        Ok(Some(GetOptions {
            ids: self.ids.clone(),
            where_metadata,
            limit: self.limit,
            offset: self.offset,
            where_document,
            include,
        }))
    }
}
