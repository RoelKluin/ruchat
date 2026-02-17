// src/chroma/parser.rs
use crate::{RuChatError, Result};
use chroma::types::{IncludeList, Include};
use clap::Parser;

#[derive(Parser, Debug, Clone, PartialEq)]
pub(crate) struct IncludeArgs {
    /// comma seperated string of include fields: "distance,document,embedding,metadata,uri"
    #[arg(short, long)]
    include: Option<String>,
}

fn parse_include(include: &str) -> Result<IncludeList> {
    let mut inc_vec = Vec::new();
    for field in include.split(',') {
        let entry = match field.trim() {
            "distance" => Include::Distance,
            "document" => Include::Document,
            "embedding" => Include::Embedding,
            "metadata" => Include::Metadata,
            "uri" => Include::Uri,
            _ => {
                return Err(RuChatError::InvalidIncludeField(field.trim().to_string()));
            }
        };
        inc_vec.push(entry);
    }
    Ok(IncludeList(inc_vec))
}

impl IncludeArgs {
    pub(crate) fn parse(&self) -> Result<Option<IncludeList>> {
        self.include.as_ref().map(|s| parse_include(s)).transpose()
    }
}
