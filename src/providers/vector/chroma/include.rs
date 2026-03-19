// src/chroma/parser.rs
use crate::{Result, RuChatError};
use chroma::types::IncludeList;
use clap::Parser;
use serde::Deserialize;
use serde_json::Value;

#[derive(Parser, Debug, Clone, PartialEq, Deserialize, Default)]
pub(crate) struct IncludeArgs {
    /// comma seperated string of include fields: "distance,document,embedding,metadata,uri"
    #[arg(short, long, help_heading = "Output Control")]
    include: Option<String>,
}

fn parse_include(include: &str) -> Result<IncludeList> {
    serde_json::from_str(include).map_err(|e| {
        RuChatError::InvalidIncludeList(format!("Error {e} while parsing '{include}'"))
    })
}

impl IncludeArgs {
    pub(crate) fn parse(&self) -> Result<Option<IncludeList>> {
        self.include.as_ref().map(|s| parse_include(s)).transpose()
    }
    pub(crate) fn update_from_json(&mut self, json: &Value) -> Result<()> {
        if let Some(include) = json.get("include") {
            if include.is_string() {
                self.include = Some(include.as_str().unwrap().to_string());
                Ok(())
            } else {
                Err(RuChatError::Is(format!(
                    "Expected 'include' to be a string in JSON, got {:?}",
                    include
                )))
            }
        } else {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chroma::types::Include;

    #[test]
    fn test_parse_include() {
        let include_list = IncludeList(vec![
            Include::Distance,
            Include::Document,
            Include::Embedding,
            Include::Metadata,
            Include::Uri,
        ]);
        eprintln!(
            "Testing valid include list {}",
            serde_json::to_string(&include_list).unwrap()
        );
        assert_eq!(
            parse_include(r#"["distances","documents","embeddings","metadatas","uris"]"#).unwrap(),
            include_list
        );
    }
}
