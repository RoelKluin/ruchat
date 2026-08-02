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
        assert_eq!(
            parse_include(r#"["distances","documents","embeddings","metadatas","uris"]"#).unwrap(),
            include_list
        );
    }

    #[test]
    fn test_parse_include_invalid_json() {
        let err = parse_include("not json").unwrap_err();
        assert!(matches!(err, RuChatError::InvalidIncludeList(_)));
    }

    #[test]
    fn test_parse_include_unknown_variant() {
        let err = parse_include(r#"["not_a_real_field"]"#).unwrap_err();
        assert!(matches!(err, RuChatError::InvalidIncludeList(_)));
    }

    #[test]
    fn test_include_args_parse_none() {
        let args = IncludeArgs { include: None };
        assert_eq!(args.parse().unwrap(), None);
    }

    #[test]
    fn test_include_args_parse_some() {
        let args = IncludeArgs {
            include: Some(r#"["distances"]"#.to_string()),
        };
        assert_eq!(
            args.parse().unwrap(),
            Some(IncludeList(vec![Include::Distance]))
        );
    }

    #[test]
    fn test_include_args_parse_propagates_error() {
        let args = IncludeArgs {
            include: Some("not json".to_string()),
        };
        assert!(args.parse().is_err());
    }

    #[test]
    fn test_update_from_json_sets_include() {
        let mut args = IncludeArgs::default();
        args.update_from_json(&serde_json::json!({ "include": "[\"distances\"]" }))
            .unwrap();
        assert_eq!(args.include, Some(r#"["distances"]"#.to_string()));
    }

    #[test]
    fn test_update_from_json_missing_key_is_noop() {
        let mut args = IncludeArgs::default();
        args.update_from_json(&serde_json::json!({})).unwrap();
        assert_eq!(args.include, None);
    }

    #[test]
    fn test_update_from_json_rejects_non_string() {
        let mut args = IncludeArgs::default();
        let err = args
            .update_from_json(&serde_json::json!({ "include": ["distances"] }))
            .unwrap_err();
        assert!(matches!(err, RuChatError::Is(_)));
    }
}
