use crate::chroma::r#where::{
    filter_get_response, where_needs_client_side_eval, with_metadata_included,
};
use crate::chroma::{
    ChromaClientConfigArgs, ChromaCollectionConfigArgs, ChromaResponse, IncludeArgs, OutputArgs,
    WhereArgs,
};
use crate::{Result, RuChatError, retry_transient};
use clap::Parser;
use serde_json::Value;

/// Command-line arguments for geting a Chroma database.
///
/// This struct defines the arguments required to perform a get operation
/// in a Chroma database, including model details, get parameters,
/// and database connection information.
#[derive(Parser, Debug, Clone, PartialEq)]
pub(crate) struct GetArgs {
    /// Comma separated list of document IDs to retrieve.
    // Long-only, deliberately: an auto-derived `-i` collides with `IncludeArgs`'s own `-i`
    // (`--include`), both flattened into this same command — see `query.rs`'s identical fix.
    #[arg(long)]
    ids: Option<String>,

    /// The number of results to return.
    #[arg(short, long)]
    limit: Option<u32>,

    /// The number of results to skip before returning results.
    #[arg(short, long)]
    offset: Option<u32>,

    #[command(flatten)]
    collection: ChromaCollectionConfigArgs,

    #[command(flatten)]
    client: ChromaClientConfigArgs,

    #[command(flatten)]
    include: IncludeArgs,

    #[command(flatten)]
    r#where: WhereArgs,

    #[command(flatten)]
    output: OutputArgs,
}

impl GetArgs {
    pub(crate) async fn get(&self, cfg: &Value) -> Result<()> {
        let client = self
            .client
            .create_client(cfg)
            .await
            .map_err(RuChatError::AnyhowError)?;
        let collection = self.collection.get_collection(&client, "default").await?;

        let ids = super::parse_ids(&self.ids);

        let r#where = self.r#where.parse()?;

        let mut include_list = self.include.parse()?;

        // See `where_needs_client_side_eval`'s doc comment: Chroma's metadata filter has no
        // scalar-substring operator, so a CONTAINS anywhere in `where` must be evaluated
        // ourselves rather than sent to Chroma (which would just silently filter everything
        // out against a plain string field). In that case, fetch with no server-side where/
        // limit/offset at all — a `get` isn't similarity-ranked, so unlike `query.rs` there's
        // no need to over-fetch a candidate pool, just fetch everything (matching `ids` if
        // given) and apply the real filter, then offset/limit, ourselves afterward.
        let needs_client_filter = r#where.as_ref().is_some_and(where_needs_client_side_eval);
        let (query_where, fetch_limit, fetch_offset) = if needs_client_filter {
            include_list = Some(with_metadata_included(include_list));
            (None, None, None)
        } else {
            (r#where.clone(), self.limit, self.offset)
        };

        let mut get_result = retry_transient!(async {
            collection
                .get(
                    ids.clone(),
                    query_where.clone(),
                    fetch_limit,
                    fetch_offset,
                    include_list.clone(),
                )
                .await
                .map_err(RuChatError::from)
        })?;

        if let Some(w) = r#where.as_ref().filter(|_| needs_client_filter) {
            filter_get_response(
                &mut get_result,
                w,
                self.offset.unwrap_or(0) as usize,
                self.limit.map(|l| l as usize),
            );
        }

        ChromaResponse::Get(&mut get_result).render(&self.output)
    }
}
