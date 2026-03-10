mod client;
mod collection;
pub(crate) mod create;
pub(crate) mod delete;
pub(crate) mod fork;
pub(crate) mod get;
pub(crate) mod include;
pub(crate) mod ls;
pub(crate) mod metadata;
pub(crate) mod modify;
pub(crate) mod query;
pub(crate) mod search;
pub(crate) mod r#where;

use crate::{Result, RuChatError};
use chroma::types;
pub(crate) use client::ChromaClientConfigArgs;
pub(crate) use collection::ChromaCollectionConfigArgs;
pub(crate) use include::IncludeArgs;
use log::{info, warn};
pub(crate) use metadata::{MetadataArgs, UpdateMetadataArrayArgs};
use serde::Serialize;
pub(crate) use r#where::WhereArgs;
use serde::Deserialize;

#[derive(clap::Args, Debug, Clone, PartialEq, Deserialize)]
pub(super) struct OutputArgs {
    /// Output in JSON format instead of a human-readable table.
    #[arg(short, long)]
    json: bool,

    /// Sort the results by ID before displaying.
    #[arg(short, long)]
    sort: bool,

    /// Specify which fields to display (comma-separated:
    /// id,doc,meta,embed,score,uri,distance,include,select).
    /// Defaults to "id,doc,meta".
    #[arg(short, long, value_delimiter = ',', default_value = "id,doc,meta")]
    fields: Vec<String>,

    /// Maximum width for the document column to prevent text wrapping issues.
    #[arg(long, default_value_t = 80)]
    max_width: usize,
}

impl OutputArgs {
    fn should_show(&self, field: &str) -> bool {
        self.fields.contains(&field.to_string())
    }
}

#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum ChromaResponse<'a> {
    Get(&'a mut types::GetResponse),
    Search(&'a mut types::SearchResponse),
    Query(&'a mut types::QueryResponse),
}

struct OutputRow {
    id: String,
    document: Option<String>,
    metadata: Option<String>,
    embedding: Option<Vec<f32>>,
    score: Option<f32>,      // for search results
    distance: Option<f32>,   // for query results
    uri: Option<String>,     // for query results
    include: Option<String>, // for query results and get results, json string of the include field
    select: Option<String>,  // for search results, json string of the select field
}

impl AsMut<Self> for ChromaResponse<'_> {
    fn as_mut(&mut self) -> &mut Self {
        self
    }
}

impl ChromaResponse<'_> {
    pub(super) fn render(&mut self, options: &OutputArgs) -> Result<()> {
        info!("{}", self.as_string(options)?);
        Ok(())
    }
    pub(crate) fn as_string(&mut self, options: &OutputArgs) -> Result<String> {
        if options.sort {
            match self {
                ChromaResponse::Get(r) => r.sort_by_ids(),
                ChromaResponse::Query(r) => r.sort_by_ids(),
                ChromaResponse::Search(_) => warn!("Search results are not sortable by ID"),
            }
        }

        if options.json {
            serde_json::to_string_pretty(&self)
                .map_err(|e| RuChatError::InternalError(e.to_string()))
        } else {
            let mut table = String::new();
            match self {
                ChromaResponse::Get(r) => {
                    let rows = flatten_get(r);
                    table.push_str(&create_table(rows, options, false, false)?);
                }
                ChromaResponse::Search(r) => {
                    for (i, _) in r.ids.iter().enumerate() {
                        info!("\nSearch Result Set #{}", i);
                        let rows = flatten_search(r, i);
                        table.push_str(&create_table(rows, options, true, false)?);
                    }
                }
                ChromaResponse::Query(r) => {
                    for (i, _) in r.ids.iter().enumerate() {
                        info!("\nQuery Result Set #{}", i);
                        let rows = flatten_query(r, i);
                        table.push_str(&create_table(rows, options, false, true)?);
                    }
                }
            }
            Ok(table)
        }
    }
}
fn create_table(
    rows: Vec<OutputRow>,
    options: &OutputArgs,
    is_search: bool,
    is_query: bool,
) -> Result<String> {
    // 1. Build Dynamic Header
    let mut header = String::new();
    if options.should_show("id") {
        header.push_str(&format!("{:<36} ", "ID"));
    }
    if options.should_show("doc") {
        header.push_str(&format!("{:<30} ", "DOCUMENT"));
    }
    if options.should_show("embed") {
        header.push_str(&format!("{:<12} ", "EMBEDDING"));
    }

    if is_search && options.should_show("score") {
        header.push_str(&format!("{:<10} ", "SCORE"));
    }
    if is_query && options.should_show("distance") {
        header.push_str(&format!("{:<10} ", "DISTANCE"));
    }
    if options.should_show("uri") {
        header.push_str(&format!("{:<20} ", "URI"));
    }
    if options.should_show("meta") {
        header.push_str("METADATA");
    }

    let mut table = String::new();
    table.push_str(&format!("{:-<120}{header}{:-<120}\n", "", ""));

    // 2. Render Rows
    for row in rows {
        let mut line = String::new();

        if options.should_show("id") {
            line.push_str(&format!("{:<36} ", row.id));
        }

        if options.should_show("doc") {
            let doc = row.document.unwrap_or_else(|| "-".to_string());
            let truncated = if doc.len() > 27 {
                format!("{}...", &doc[..24])
            } else {
                format!("{:<27}", doc)
            };
            line.push_str(&format!("{} ", truncated));
        }

        if options.should_show("embed") {
            let emb_str = row
                .embedding
                .map_or("None".to_string(), |e| format!("[dim: {}]", e.len()));
            line.push_str(&format!("{:<12} ", emb_str));
        }

        if is_search && options.should_show("score") {
            line.push_str(&format!("{:<10.4} ", row.score.unwrap_or(0.0)));
        }

        if is_query && options.should_show("distance") {
            line.push_str(&format!("{:<10.4} ", row.distance.unwrap_or(0.0)));
        }

        if options.should_show("uri") {
            let uri = row.uri.unwrap_or_else(|| "-".to_string());
            line.push_str(&format!(
                "{:<20} ",
                if uri.len() > 17 {
                    format!("{}...", &uri[..17])
                } else {
                    uri
                }
            ));
        }

        if options.should_show("meta")
            && let Some(m) = row.metadata
        {
            let truncated = if m.len() > 40 {
                format!("{}...", &m[..37])
            } else {
                m
            };
            line.push_str(&truncated);
        }
        if options.should_show("select") {
            let sel = row.select.unwrap_or_else(|| "-".to_string());
            let trunc = if sel.len() > 12 {
                format!("{}...", &sel[..12])
            } else {
                format!("{:<15}", sel)
            };
            line.push_str(&format!("{} ", trunc));
        }

        if options.should_show("include") {
            let inc = row.include.unwrap_or_else(|| "-".to_string());
            let trunc = if inc.len() > 12 {
                format!("{}...", &inc[..12])
            } else {
                format!("{:<15}", inc)
            };
            line.push_str(&format!("{} ", trunc));
        }
        table.push_str(&line);
    }
    Ok(table)
}
fn flatten_get(r: &types::GetResponse) -> Vec<OutputRow> {
    (0..r.ids.len())
        .map(|i| OutputRow {
            id: r.ids[i].clone(),
            document: r.documents.as_ref().and_then(|d| d[i].clone()),
            metadata: r
                .metadatas
                .as_ref()
                .and_then(|m| m[i].as_ref().map(|map| format!("{:?}", map))),
            embedding: r.embeddings.as_ref().and_then(|e| e.get(i).cloned()),
            score: None,
            distance: None,
            select: None,
            uri: r.uris.as_ref().and_then(|u| u[i].clone()),
            include: r.include.get(i).map(|inc| format!("{:?}", inc)),
        })
        .collect()
}

fn flatten_search(r: &types::SearchResponse, index: usize) -> Vec<OutputRow> {
    let ids = &r.ids[index];
    (0..ids.len())
        .map(|i| OutputRow {
            id: ids[i].clone(),
            document: r
                .documents
                .get(index)
                .and_then(|d| d.as_ref().and_then(|docs| docs[i].clone())),
            metadata: r.metadatas.get(index).and_then(|m| {
                m.as_ref()
                    .and_then(|metas| metas[i].as_ref().map(|m| format!("{:?}", m)))
            }),
            embedding: r
                .embeddings
                .get(index)
                .and_then(|e| e.as_ref().and_then(|embs| embs[i].clone())),
            score: r
                .scores
                .get(index)
                .and_then(|s| s.as_ref().and_then(|sv| sv[i])),
            select: r
                .select
                .get(index)
                .and_then(|s| serde_json::to_string(&s).ok()),
            distance: None,
            uri: None,
            include: None,
        })
        .collect()
}

fn flatten_query(r: &types::QueryResponse, index: usize) -> Vec<OutputRow> {
    let ids = &r.ids[index];
    (0..ids.len())
        .map(|i| OutputRow {
            id: ids[i].clone(),
            document: r
                .documents
                .as_ref()
                .and_then(|d| d.get(index))
                .and_then(|docs| docs[i].clone()),
            metadata: r
                .metadatas
                .as_ref()
                .and_then(|m| m.get(index))
                .and_then(|metas| metas[i].as_ref().map(|m| format!("{:?}", m))),
            embedding: r
                .embeddings
                .as_ref()
                .and_then(|e| e.get(index))
                .and_then(|embs| embs[i].clone()),
            uri: r
                .uris
                .as_ref()
                .and_then(|u| u.get(index))
                .and_then(|uris| uris[i].clone()),
            distance: r
                .distances
                .as_ref()
                .and_then(|d| d.get(index))
                .and_then(|dist| dist[i]),
            include: r.include.get(index).map(|inc| format!("{:?}", inc)), // Adjusted per types
            score: None,
            select: None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chroma::types::Include;
    use chroma::types::MetadataValue;
    use std::collections::HashMap;

    #[test]
    fn test_output_row_creation() {
        let row = OutputRow {
            id: "123".to_string(),
            document: Some("This is a test document.".to_string()),
            metadata: Some("{\"key\": \"value\"}".to_string()),
            embedding: Some(vec![0.1, 0.2, 0.3]),
            score: Some(0.95),
            distance: Some(0.05),
            uri: Some("http://example.com".to_string()),
            include: Some("{\"extra\": \"info\"}".to_string()),
            select: Some("{\"field\": \"data\"}".to_string()),
        };

        assert_eq!(row.id, "123");
        assert_eq!(row.document.unwrap(), "This is a test document.");
        assert_eq!(row.metadata.unwrap(), "{\"key\": \"value\"}");
        assert_eq!(row.embedding.unwrap(), vec![0.1, 0.2, 0.3]);
        assert_eq!(row.score.unwrap(), 0.95);
        assert_eq!(row.distance.unwrap(), 0.05);
        assert_eq!(row.uri.unwrap(), "http://example.com");
        assert_eq!(row.include.unwrap(), "{\"extra\": \"info\"}");
        assert_eq!(row.select.unwrap(), "{\"field\": \"data\"}");
    }

    #[test]
    fn test_output_args_should_show() {
        let options = OutputArgs {
            json: false,
            sort: false,
            fields: vec!["id".to_string(), "doc".to_string()],
            max_width: 80,
        };

        assert!(options.should_show("id"));
        assert!(options.should_show("doc"));
        assert!(!options.should_show("meta"));
        assert!(!options.should_show("embed"));
    }
    #[test]
    fn test_create_table() {
        let rows = vec![OutputRow {
            id: "123".to_string(),
            document: Some("This is a test document.".to_string()),
            metadata: Some("{\"key\": \"value\"}".to_string()),
            embedding: Some(vec![0.1, 0.2, 0.3]),
            score: Some(0.95),
            distance: Some(0.05),
            uri: Some("http://example.com".to_string()),
            include: Some("{\"extra\": \"info\"}".to_string()),
            select: Some("{\"field\": \"data\"}".to_string()),
        }];

        let options = OutputArgs {
            json: false,
            sort: false,
            fields: vec!["id".to_string(), "doc".to_string(), "meta".to_string()],
            max_width: 80,
        };

        let table = create_table(rows, &options, false, false).unwrap();
        assert!(table.contains("ID"));
        assert!(table.contains("DOCUMENT"));
        assert!(table.contains("METADATA"));
        assert!(table.contains("123"));
        assert!(table.contains("This is a test document."));
        assert!(table.contains("{\"key\": \"value\"}"));
    }
    #[test]
    fn test_create_table_with_score() {
        let rows = vec![OutputRow {
            id: "123".to_string(),
            document: Some("This is a test document.".to_string()),
            metadata: None,
            embedding: None,
            score: Some(0.95),
            distance: None,
            uri: None,
            include: None,
            select: None,
        }];

        let options = OutputArgs {
            json: false,
            sort: false,
            fields: vec!["id".to_string(), "doc".to_string(), "score".to_string()],
            max_width: 80,
        };

        let table = create_table(rows, &options, true, false).unwrap();
        assert!(table.contains("SCORE"));
        assert!(table.contains("0.9500"));
    }
    #[test]
    fn test_create_table_with_distance() {
        let rows = vec![OutputRow {
            id: "123".to_string(),
            document: Some("This is a test document.".to_string()),
            metadata: None,
            embedding: None,
            score: None,
            distance: Some(0.05),
            uri: None,
            include: None,
            select: None,
        }];

        let options = OutputArgs {
            json: false,
            sort: false,
            fields: vec!["id".to_string(), "doc".to_string(), "distance".to_string()],
            max_width: 80,
        };

        let table = create_table(rows, &options, false, true).unwrap();
        assert!(table.contains("DISTANCE"));
        assert!(table.contains("0.0500"));
    }
    #[test]
    fn test_create_table_with_uri() {
        let rows = vec![OutputRow {
            id: "123".to_string(),
            document: Some("This is a test document.".to_string()),
            metadata: None,
            embedding: None,
            score: None,
            distance: None,
            uri: Some("http://example.com".to_string()),
            include: None,
            select: None,
        }];

        let options = OutputArgs {
            json: false,
            sort: false,
            fields: vec!["id".to_string(), "doc".to_string(), "uri".to_string()],
            max_width: 80,
        };

        let table = create_table(rows, &options, false, false).unwrap();
        assert!(table.contains("URI"));
        assert!(table.contains("http://example.com"));
    }
    #[test]
    fn test_json_output() {
        let meta = serde_json::json!({"key": "value"});
        let meta_v: HashMap<String, MetadataValue> = serde_json::from_value(meta.clone()).unwrap();
        let include = serde_json::json!({"extra": "info"});
        let include_v: Include = serde_json::from_value(include.clone()).unwrap();
        let mut response = ChromaResponse::Get(&mut types::GetResponse {
            ids: vec!["123".to_string()],
            documents: Some(vec![Some("This is a test document.".to_string())]),
            metadatas: Some(vec![Some(meta_v)]),
            embeddings: Some(vec![vec![0.1, 0.2, 0.3]]),
            uris: Some(vec![Some("http://example.com".to_string())]),
            include: vec![include_v],
        });

        let options = OutputArgs {
            json: true,
            sort: false,
            fields: vec!["id".to_string(), "doc".to_string(), "meta".to_string()],
            max_width: 80,
        };

        let json_str = response.as_string(&options).unwrap();
        assert!(json_str.contains("\"ids\": [\"123\"]"));
        assert!(json_str.contains("\"documents\": [[\"This is a test document.\"]]"));
        assert!(json_str.contains("\"metadatas\": [[{\"key\": \"value\"}]]"));
    }
}
