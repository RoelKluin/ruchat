mod client;
mod collection;
pub(crate) mod create;
pub(crate) mod delete;
pub(crate) mod get;
pub(crate) mod search;
pub(crate) mod modify;
pub(crate) mod fork;
pub(crate) mod ls;
pub(crate) mod query;
pub(crate) mod r#where;
pub(crate) mod include;
pub(crate) mod metadata;

pub(crate) use client::ChromaClientConfigArgs;
pub(crate) use collection::ChromaCollectionConfigArgs;
pub(crate) use r#where::WhereArgs;
pub(crate) use include::IncludeArgs;
pub(crate) use metadata::{MetadataArgs, UpdateMetadataArrayArgs};
use serde::Serialize;
use chroma::types;
use crate::{Result, RuChatError};
use log::{info, warn};

#[derive(clap::Args, Debug, Clone, PartialEq)]
pub(crate) struct OutputArgs {
    /// Output in JSON format.
    #[arg(short, long)]
    pub json: bool,

    /// Sort the results by ID before displaying.
    #[arg(short, long)]
    pub sort: bool,

    /// Specify which fields to display (comma-separated:
    /// id,doc,meta,embed,score,uri,distance,include,select).
    /// Defaults to "id,doc,meta".
    #[arg(short, long, value_delimiter = ',', default_value = "id,doc,meta")]
    pub fields: Vec<String>,

    /// Maximum width for the document column to prevent text wrapping issues.
    #[arg(long, default_value_t = 80)]
    pub max_width: usize,
}

impl OutputArgs {
    pub fn should_show(&self, field: &str) -> bool {
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
    score: Option<f32>,     // for search results
    distance: Option<f32>,  // for query results
    uri: Option<String>,    // for query results
    include: Option<String>,// for query results and get results, json string of the include field
    select: Option<String>, // for search results, json string of the select field
}

impl AsMut<Self> for ChromaResponse<'_> {
    fn as_mut(&mut self) -> &mut Self {
        self
    }
}

impl ChromaResponse<'_> {
    pub(super) fn render(&mut self, options: &OutputArgs) -> Result<()> {
        // 1. Handle Sorting
        if options.sort {
            match self {
                ChromaResponse::Get(r) => r.sort_by_ids(),
                ChromaResponse::Query(r) => r.sort_by_ids(),
                ChromaResponse::Search(_) => warn!("Search results are not sortable by ID"),
            }
        }

        // 2. Handle JSON (Granular control usually happens via `jq` or similar,
        // but we respect the flags by logging what we're about to show)
        if options.json {
            let json = serde_json::to_string_pretty(&self)
                .map_err(|e| RuChatError::InternalError(e.to_string()))?;
            info!("{json}");
            return Ok(());
        }

        // 3. Tabular rendering logic
        match self {
            ChromaResponse::Get(r) => {
                let rows = flatten_get(r);
                print_table(rows, options, false, false);
            },
            ChromaResponse::Search(r) => {
                for (i, _) in r.ids.iter().enumerate() {
                    info!("\nSearch Result Set #{}", i);
                    let rows = flatten_search(r, i);
                    print_table(rows, options, true, false);
                }
            },
            ChromaResponse::Query(r) => {
                for (i, _) in r.ids.iter().enumerate() {
                    info!("\nQuery Result Set #{}", i);
                    let rows = flatten_query(r, i);
                    print_table(rows, options, false, true);
                }
            }
        }
        Ok(())
    }
}
fn print_table(rows: Vec<OutputRow>, options: &OutputArgs, is_search: bool, is_query: bool) {

    // 1. Build Dynamic Header
    let mut header = String::new();
    if options.should_show("id")    { header.push_str(&format!("{:<36} ", "ID")); }
    if options.should_show("doc")   { header.push_str(&format!("{:<30} ", "DOCUMENT")); }
    if options.should_show("embed") { header.push_str(&format!("{:<12} ", "EMBEDDING")); }

    if is_search && options.should_show("score") {
        header.push_str(&format!("{:<10} ", "SCORE"));
    }
    if is_query && options.should_show("distance") {
        header.push_str(&format!("{:<10} ", "DISTANCE"));
    }
    if options.should_show("uri")     { header.push_str(&format!("{:<20} ", "URI")); }
    if options.should_show("meta")    { header.push_str("METADATA"); }

    info!("{:-<120}", "");
    info!("{}", header);
    info!("{:-<120}", "");

    // 2. Render Rows
    for row in rows {
        let mut line = String::new();

        if options.should_show("id") {
            line.push_str(&format!("{:<36} ", row.id));
        }

        if options.should_show("doc") {
            let doc = row.document.unwrap_or_else(|| "-".to_string());
            let truncated = if doc.len() > 27 { format!("{}...", &doc[..24]) } else { format!("{:<27}", doc) };
            line.push_str(&format!("{} ", truncated));
        }

        if options.should_show("embed") {
            let emb_str = row.embedding.map_or("None".to_string(), |e| format!("[dim: {}]", e.len()));
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
            line.push_str(&format!("{:<20} ", if uri.len() > 17 { format!("{}...", &uri[..17]) } else { uri }));
        }

        if options.should_show("meta") {
            if let Some(m) = row.metadata {
                let truncated = if m.len() > 40 { format!("{}...", &m[..37]) } else { m };
                line.push_str(&truncated);
            }
        }
        if options.should_show("select") {
            let sel = row.select.unwrap_or_else(|| "-".to_string());
            let trunc = if sel.len() > 12 { format!("{}...", &sel[..12]) } else { format!("{:<15}", sel) };
            line.push_str(&format!("{} ", trunc));
        }

        if options.should_show("include") {
            let inc = row.include.unwrap_or_else(|| "-".to_string());
            let trunc = if inc.len() > 12 { format!("{}...", &inc[..12]) } else { format!("{:<15}", inc) };
            line.push_str(&format!("{} ", trunc));
        }
        info!("{}", line);
    }
}
fn flatten_get(r: &types::GetResponse) -> Vec<OutputRow> {
    (0..r.ids.len()).map(|i| OutputRow {
        id: r.ids[i].clone(),
        document: r.documents.as_ref().and_then(|d| d[i].clone()),
        metadata: r.metadatas.as_ref().and_then(|m| m[i].as_ref().map(|map| format!("{:?}", map))),
        embedding: r.embeddings.as_ref().and_then(|e| e.get(i).cloned()),
        score: None,
        distance: None,
        select: None,
        uri: r.uris.as_ref().and_then(|u| u[i].clone()),
        include: r.include.get(i).map(|inc| format!("{:?}", inc)),
    }).collect()
}

fn flatten_search(r: &types::SearchResponse, index: usize) -> Vec<OutputRow> {
    let ids = &r.ids[index];
    (0..ids.len()).map(|i| OutputRow {
        id: ids[i].clone(),
        document: r.documents.get(index).and_then(|d| d.as_ref().and_then(|docs| docs[i].clone())),
        metadata: r.metadatas.get(index).and_then(|m| m.as_ref().and_then(|metas| metas[i].as_ref().map(|m| format!("{:?}", m)))),
        embedding: r.embeddings.get(index).and_then(|e| e.as_ref().and_then(|embs| embs[i].clone())),
        score: r.scores.get(index).and_then(|s| s.as_ref().and_then(|sv| sv[i])),
        select: r.select.get(index).and_then(|s| serde_json::to_string(&s).ok()),
        distance: None,
        uri: None,
        include: None,
    }).collect()
}

fn flatten_query(r: &types::QueryResponse, index: usize) -> Vec<OutputRow> {
    let ids = &r.ids[index];
    (0..ids.len()).map(|i| OutputRow {
        id: ids[i].clone(),
        document: r.documents.as_ref().and_then(|d| d.get(index)).and_then(|docs| docs[i].clone()),
        metadata: r.metadatas.as_ref().and_then(|m| m.get(index)).and_then(|metas| metas[i].as_ref().map(|m| format!("{:?}", m))),
        embedding: r.embeddings.as_ref().and_then(|e| e.get(index)).and_then(|embs| embs[i].clone()),
        uri: r.uris.as_ref().and_then(|u| u.get(index)).and_then(|uris| uris[i].clone()),
        distance: r.distances.as_ref().and_then(|d| d.get(index)).and_then(|dist| dist[i]),
        include: r.include.get(index).map(|inc| format!("{:?}", inc)), // Adjusted per types
        score: None,
        select: None,
    }).collect()
}
