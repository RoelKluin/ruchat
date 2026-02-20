// src/chroma/parser.rs
use crate::{RuChatError, Result};
use chroma::types::IncludeList;
use clap::Parser;

#[derive(Parser, Debug, Clone, PartialEq)]
pub(crate) struct IncludeArgs {
    /// comma seperated string of include fields: "distance,document,embedding,metadata,uri"
    #[arg(short, long)]
    include: Option<String>,
}

impl IncludeArgs {
    pub(crate) fn parse(&self) -> Result<Option<IncludeList>> {
        self.include.as_ref().map(|include|
            serde_json::from_str(include)
                .map_err(|e| {
                    let err_msg = format!("Error {e} while parsing '{include}'");
                    RuChatError::InvalidIncludeList(err_msg)
                })
            ).transpose()
    }
}

#[cfg(test)]
mod tests{
    use super::*;

    #[test]
    fn test_parse_include() {
        let include_list = IncludeList(vec![
            Include::Distance,
            Include::Document,
            Include::Embedding,
            Include::Metadata,
            Include::Uri,
        ]);
        eprintln!("Testing valid include list {}", serde_json::to_string(&include_list).unwrap());
        assert_eq!(parse_include(r#"["distances","documents","embeddings","metadatas","uris"]"#).unwrap(), include_list);
    }
}
