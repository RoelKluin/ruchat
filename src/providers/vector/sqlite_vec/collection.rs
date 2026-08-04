use crate::agent::llm_client::{VectorCollection, VectorStore};
use crate::chroma::r#where::metadata_matches;
use crate::{Result, RuChatError};
use async_trait::async_trait;
use chroma::types::{Include, IncludeList, Metadata, QueryResponse, UpdateMetadata, Where};
use rusqlite::Connection;
use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Mutex, Once};

static REGISTER_EXTENSION: Once = Once::new();

/// Registers the sqlite-vec extension's entry point once per process
/// (`sqlite3_auto_extension` applies to every connection opened afterwards,
/// including ones already open at registration time re-opening — see the
/// `sqlite-vec` crate's own test for this exact pattern). Idempotent by
/// construction via `Once`, so every `SqliteVecClient::open` call is safe to
/// call this unconditionally.
fn register_extension_once() {
    REGISTER_EXTENSION.call_once(|| unsafe {
        // The `sqlite-vec` crate's own FFI declaration lies about
        // `sqlite3_vec_init`'s signature (`fn()`, no args) — the real linked C
        // symbol matches `sqlite3_auto_extension`'s expected entry-point type.
        // This is the crate's own documented usage pattern (see its `lib.rs`
        // test), not a workaround invented here.
        rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(
            sqlite_vec::sqlite3_vec_init as *const (),
        )));
    });
}

/// A collection name becomes a literal SQL identifier (table name) below —
/// SQLite has no way to bind a table name as a query parameter. Restricting
/// to `[A-Za-z0-9_]+` (same character class Rust identifiers/most SQL
/// dialects allow unquoted) closes that off entirely rather than trying to
/// escape/quote an arbitrary string safely.
fn validate_collection_name(name: &str) -> Result<()> {
    if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        Ok(())
    } else {
        Err(RuChatError::SqliteVecError(format!(
            "invalid sqlite-vec collection name {name:?}: only ASCII letters, digits, and underscores are allowed"
        )))
    }
}

/// A local SQLite file (with the `sqlite-vec` extension loaded) standing in
/// for `ChromaHttpClient` — resolves a named collection instead of a network
/// round trip. One `Connection` per client, shared (behind a `Mutex`, since
/// `rusqlite::Connection` is `Send` but not `Sync`) across every collection
/// handle opened from it — SQLite itself serializes writers regardless, so
/// this doesn't add contention beyond what the database would enforce anyway.
pub(crate) struct SqliteVecClient {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteVecClient {
    /// Blocking — see `SqliteVecClientConfigArgs::create_client`'s doc comment
    /// for why callers run this via `spawn_blocking`.
    pub(crate) fn open(path: &Path) -> Result<Self> {
        register_extension_once();
        let conn = Connection::open(path).map_err(|e| {
            RuChatError::SqliteVecError(format!(
                "failed to open sqlite-vec database at {path:?}: {e}"
            ))
        })?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub(crate) fn collection(&self, name: &str) -> Result<SqliteVecCollection> {
        validate_collection_name(name)?;
        Ok(SqliteVecCollection {
            conn: Arc::clone(&self.conn),
            name: name.to_string(),
        })
    }
}

/// A named collection within a `SqliteVecClient`'s database — backed by two
/// tables: `vec_<name>` (a `vec0` virtual table, just `rowid`/`embedding`)
/// and `meta_<name>` (`rowid`/`id`/`document`/`metadata`, metadata stored as
/// a JSON blob since `chroma::types::{Metadata,UpdateMetadata}` already
/// round-trip through serde cleanly — no separate schema needed for
/// arbitrary metadata keys). Joined by `rowid`, which SQLite guarantees
/// stable and unique within a table.
///
/// `vec0` requires a fixed embedding dimension at table-creation time, unlike
/// Chroma (whose server infers it from the first write) — `ensure_schema`
/// defers that decision to the first `add`/`upsert` call, using that call's
/// own embedding length, so callers never need to declare a dimension
/// upfront either.
#[derive(Clone)]
pub(crate) struct SqliteVecCollection {
    conn: Arc<Mutex<Connection>>,
    name: String,
}

impl SqliteVecCollection {
    fn vec_table(&self) -> String {
        format!("vec_{}", self.name)
    }
    fn meta_table(&self) -> String {
        format!("meta_{}", self.name)
    }

    /// Creates both backing tables if they don't exist yet — a no-op
    /// (`IF NOT EXISTS`) on every call after the first, so every write path
    /// can call this unconditionally instead of tracking whether it already
    /// ran. `dim` only matters for the very first call for a given
    /// collection name; a later call with a different `dim` has no effect on
    /// an already-created `vec0` table (a real dimension mismatch surfaces
    /// naturally as a `sqlite-vec` error on the following `INSERT`, not
    /// here).
    fn ensure_schema(
        conn: &Connection,
        vec_table: &str,
        meta_table: &str,
        dim: usize,
    ) -> Result<()> {
        conn.execute_batch(&format!(
            "CREATE VIRTUAL TABLE IF NOT EXISTS {vec_table} USING vec0(embedding float[{dim}]);
             CREATE TABLE IF NOT EXISTS {meta_table} (
                 rowid INTEGER PRIMARY KEY,
                 id TEXT UNIQUE NOT NULL,
                 document TEXT,
                 metadata TEXT
             );"
        ))
        .map_err(|e| {
            RuChatError::SqliteVecError(format!(
                "failed to create sqlite-vec collection {meta_table:?}: {e}"
            ))
        })
    }
}

fn embedding_to_json(embedding: &[f32]) -> String {
    // sqlite-vec accepts a JSON array of numbers directly for both INSERT and
    // MATCH against a `vec0` embedding column — see its own docs' "vectors as
    // JSON" section. Kept human-inspectable over a packed binary encoding,
    // which this crate has no independent need for elsewhere.
    serde_json::to_string(embedding).unwrap_or_else(|_| "[]".to_string())
}

fn metadata_to_json(metadata: &Option<Metadata>) -> Option<String> {
    metadata
        .as_ref()
        .map(|m| serde_json::to_string(m).unwrap_or_default())
}

fn update_metadata_to_json(metadata: &Option<UpdateMetadata>) -> Option<String> {
    metadata
        .as_ref()
        .map(|m| serde_json::to_string(m).unwrap_or_default())
}

fn json_to_metadata(json: Option<String>) -> Option<Metadata> {
    json.and_then(|s| serde_json::from_str(&s).ok())
}

#[async_trait]
impl VectorCollection for SqliteVecCollection {
    async fn existing_ids(&self, ids: Vec<String>) -> Result<HashSet<String>> {
        let this = self.clone();
        tokio::task::spawn_blocking(move || {
            let conn = this.conn.lock().unwrap();
            let meta_table = this.meta_table();
            // The meta table not existing yet (collection never written to) means
            // nothing exists — same "nothing exists yet" treatment `embed_chunks`
            // already gives a Chroma `.get()` error for an empty/absent collection.
            if conn
                .query_row(
                    "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1",
                    [&meta_table],
                    |_| Ok(()),
                )
                .is_err()
            {
                return Ok(HashSet::new());
            }
            let placeholders = ids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
            let sql = format!("SELECT id FROM {meta_table} WHERE id IN ({placeholders})");
            let mut stmt = conn.prepare(&sql).map_err(|e| {
                RuChatError::SqliteVecError(format!("sqlite-vec existing_ids query failed: {e}"))
            })?;
            let params = rusqlite::params_from_iter(ids.iter());
            let found = stmt
                .query_map(params, |row| row.get::<_, String>(0))
                .map_err(|e| {
                    RuChatError::SqliteVecError(format!(
                        "sqlite-vec existing_ids query failed: {e}"
                    ))
                })?
                .collect::<std::result::Result<HashSet<String>, _>>()
                .map_err(|e| {
                    RuChatError::SqliteVecError(format!(
                        "sqlite-vec existing_ids row read failed: {e}"
                    ))
                })?;
            Ok(found)
        })
        .await
        .map_err(|e| RuChatError::SqliteVecError(e.to_string()))?
    }

    async fn add(
        &self,
        ids: Vec<String>,
        embeddings: Vec<Vec<f32>>,
        documents: Option<Vec<Option<String>>>,
        metadatas: Option<Vec<Option<Metadata>>>,
    ) -> Result<()> {
        let this = self.clone();
        tokio::task::spawn_blocking(move || {
            let dim = embeddings.first().map(Vec::len).unwrap_or(0);
            let mut conn = this.conn.lock().unwrap();
            let vec_table = this.vec_table();
            let meta_table = this.meta_table();
            SqliteVecCollection::ensure_schema(&conn, &vec_table, &meta_table, dim)?;
            let tx = conn.transaction().map_err(|e| {
                RuChatError::SqliteVecError(format!("sqlite-vec transaction failed: {e}"))
            })?;
            for (i, id) in ids.iter().enumerate() {
                let doc = documents.as_ref().and_then(|d| d.get(i)).cloned().flatten();
                let meta_json = metadatas
                    .as_ref()
                    .and_then(|m| m.get(i))
                    .and_then(|m| metadata_to_json(m));
                tx.execute(
                    &format!(
                        "INSERT INTO {meta_table} (id, document, metadata) VALUES (?1, ?2, ?3)"
                    ),
                    rusqlite::params![id, doc, meta_json],
                )
                .map_err(|e| {
                    RuChatError::SqliteVecError(format!(
                        "sqlite-vec add failed for id {id:?} (already exists?): {e}"
                    ))
                })?;
                let rowid = tx.last_insert_rowid();
                tx.execute(
                    &format!("INSERT INTO {vec_table} (rowid, embedding) VALUES (?1, ?2)"),
                    rusqlite::params![rowid, embedding_to_json(&embeddings[i])],
                )
                .map_err(|e| {
                    RuChatError::SqliteVecError(format!("sqlite-vec add failed for id {id:?}: {e}"))
                })?;
            }
            tx.commit().map_err(|e| {
                RuChatError::SqliteVecError(format!("sqlite-vec add commit failed: {e}"))
            })?;
            Ok(())
        })
        .await
        .map_err(|e| RuChatError::SqliteVecError(e.to_string()))?
    }

    async fn update(
        &self,
        ids: Vec<String>,
        embeddings: Option<Vec<Option<Vec<f32>>>>,
        documents: Option<Vec<Option<String>>>,
        metadatas: Option<Vec<Option<UpdateMetadata>>>,
    ) -> Result<()> {
        let this = self.clone();
        tokio::task::spawn_blocking(move || {
            let conn = this.conn.lock().unwrap();
            let vec_table = this.vec_table();
            let meta_table = this.meta_table();
            for (i, id) in ids.iter().enumerate() {
                let rowid: i64 = conn
                    .query_row(
                        &format!("SELECT rowid FROM {meta_table} WHERE id = ?1"),
                        [id],
                        |row| row.get(0),
                    )
                    .map_err(|_| {
                        RuChatError::SqliteVecError(format!(
                            "sqlite-vec update failed: id {id:?} does not exist"
                        ))
                    })?;

                if let Some(doc) = documents.as_ref().and_then(|d| d.get(i)) {
                    conn.execute(
                        &format!("UPDATE {meta_table} SET document = ?1 WHERE rowid = ?2"),
                        rusqlite::params![doc, rowid],
                    )
                    .map_err(|e| {
                        RuChatError::SqliteVecError(format!(
                            "sqlite-vec update failed for id {id:?}: {e}"
                        ))
                    })?;
                }
                if let Some(meta) = metadatas.as_ref().and_then(|m| m.get(i)) {
                    conn.execute(
                        &format!("UPDATE {meta_table} SET metadata = ?1 WHERE rowid = ?2"),
                        rusqlite::params![update_metadata_to_json(meta), rowid],
                    )
                    .map_err(|e| {
                        RuChatError::SqliteVecError(format!(
                            "sqlite-vec update failed for id {id:?}: {e}"
                        ))
                    })?;
                }
                if let Some(Some(embedding)) = embeddings.as_ref().and_then(|e| e.get(i)) {
                    conn.execute(
                        &format!("UPDATE {vec_table} SET embedding = ?1 WHERE rowid = ?2"),
                        rusqlite::params![embedding_to_json(embedding), rowid],
                    )
                    .map_err(|e| {
                        RuChatError::SqliteVecError(format!(
                            "sqlite-vec update failed for id {id:?}: {e}"
                        ))
                    })?;
                }
            }
            Ok(())
        })
        .await
        .map_err(|e| RuChatError::SqliteVecError(e.to_string()))?
    }

    async fn upsert(
        &self,
        ids: Vec<String>,
        embeddings: Vec<Vec<f32>>,
        documents: Option<Vec<Option<String>>>,
        metadatas: Option<Vec<Option<UpdateMetadata>>>,
    ) -> Result<()> {
        let this = self.clone();
        tokio::task::spawn_blocking(move || {
            let dim = embeddings.first().map(Vec::len).unwrap_or(0);
            let mut conn = this.conn.lock().unwrap();
            let vec_table = this.vec_table();
            let meta_table = this.meta_table();
            SqliteVecCollection::ensure_schema(&conn, &vec_table, &meta_table, dim)?;
            let tx = conn.transaction().map_err(|e| {
                RuChatError::SqliteVecError(format!("sqlite-vec transaction failed: {e}"))
            })?;
            for (i, id) in ids.iter().enumerate() {
                let doc = documents.as_ref().and_then(|d| d.get(i)).cloned().flatten();
                let meta_json = metadatas
                    .as_ref()
                    .and_then(|m| m.get(i))
                    .and_then(update_metadata_to_json);

                let existing_rowid: Option<i64> = tx
                    .query_row(
                        &format!("SELECT rowid FROM {meta_table} WHERE id = ?1"),
                        [id],
                        |row| row.get(0),
                    )
                    .ok();

                let rowid = if let Some(rowid) = existing_rowid {
                    tx.execute(
                        &format!(
                            "UPDATE {meta_table} SET document = ?1, metadata = ?2 WHERE rowid = ?3"
                        ),
                        rusqlite::params![doc, meta_json, rowid],
                    )
                    .map_err(|e| {
                        RuChatError::SqliteVecError(format!(
                            "sqlite-vec upsert failed for id {id:?}: {e}"
                        ))
                    })?;
                    rowid
                } else {
                    tx.execute(
                        &format!(
                            "INSERT INTO {meta_table} (id, document, metadata) VALUES (?1, ?2, ?3)"
                        ),
                        rusqlite::params![id, doc, meta_json],
                    )
                    .map_err(|e| {
                        RuChatError::SqliteVecError(format!(
                            "sqlite-vec upsert failed for id {id:?}: {e}"
                        ))
                    })?;
                    tx.last_insert_rowid()
                };

                // `vec0` virtual tables don't support `ON CONFLICT ... DO UPDATE` (confirmed
                // against the real extension while writing this: "UPSERT not implemented for
                // virtual table") — branch explicitly on whether this rowid already had a vec
                // row, same as the meta-table branch just above.
                if existing_rowid.is_some() {
                    tx.execute(
                        &format!("UPDATE {vec_table} SET embedding = ?1 WHERE rowid = ?2"),
                        rusqlite::params![embedding_to_json(&embeddings[i]), rowid],
                    )
                    .map_err(|e| {
                        RuChatError::SqliteVecError(format!(
                            "sqlite-vec upsert failed for id {id:?}: {e}"
                        ))
                    })?;
                } else {
                    tx.execute(
                        &format!("INSERT INTO {vec_table} (rowid, embedding) VALUES (?1, ?2)"),
                        rusqlite::params![rowid, embedding_to_json(&embeddings[i])],
                    )
                    .map_err(|e| {
                        RuChatError::SqliteVecError(format!(
                            "sqlite-vec upsert failed for id {id:?}: {e}"
                        ))
                    })?;
                }
            }
            tx.commit().map_err(|e| {
                RuChatError::SqliteVecError(format!("sqlite-vec upsert commit failed: {e}"))
            })?;
            Ok(())
        })
        .await
        .map_err(|e| RuChatError::SqliteVecError(e.to_string()))?
    }
}

#[async_trait]
impl VectorStore for SqliteVecCollection {
    async fn query_collection(
        &self,
        collection_name: &str,
        embeddings: Vec<Vec<f32>>,
        n_results: Option<u32>,
        r#where: Option<Where>,
        ids: Option<Vec<String>>,
        include: Option<IncludeList>,
    ) -> Result<QueryResponse> {
        // `collection_name` is trusted here to already match `self.name` — the
        // Orchestrator/Librarian retrieval path resolves one `SqliteVecCollection`
        // per configured collection name up front (mirroring how a `ChromaHttpClient`
        // is asked to resolve a *different* collection per query; a `SqliteVecClient`
        // resolves collections eagerly instead, so this parameter is unused here).
        let _ = collection_name;
        let this = self.clone();
        tokio::task::spawn_blocking(move || {
            let conn = this.conn.lock().unwrap();
            let vec_table = this.vec_table();
            let meta_table = this.meta_table();
            let n = n_results.unwrap_or(10) as usize;
            // Over-fetch when a `where`/`ids` filter needs client-side evaluation
            // (`vec0` has no metadata-filter pushdown of its own) — same strategy
            // `chroma/query.rs` already uses for filters Chroma's own server can't
            // apply, reusing that module's `metadata_matches` directly rather than
            // writing a second `Where` evaluator.
            let needs_filter = r#where.is_some() || ids.is_some();
            let fetch_k = if needs_filter { (n * 10).max(50) } else { n };

            let table_exists = conn
                .query_row(
                    "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1",
                    [&vec_table],
                    |_| Ok(()),
                )
                .is_ok();

            let mut out_ids = Vec::with_capacity(embeddings.len());
            let mut out_docs = Vec::with_capacity(embeddings.len());
            let mut out_metas = Vec::with_capacity(embeddings.len());
            let mut out_dists = Vec::with_capacity(embeddings.len());

            for query_embedding in &embeddings {
                if !table_exists {
                    out_ids.push(Vec::new());
                    out_docs.push(Vec::new());
                    out_metas.push(Vec::new());
                    out_dists.push(Vec::new());
                    continue;
                }
                let sql = format!(
                    "SELECT m.id, m.document, m.metadata, v.distance
                     FROM {vec_table} v JOIN {meta_table} m ON v.rowid = m.rowid
                     WHERE v.embedding MATCH ?1 AND k = ?2
                     ORDER BY v.distance"
                );
                let mut stmt = conn.prepare(&sql).map_err(|e| {
                    RuChatError::SqliteVecError(format!("sqlite-vec query failed: {e}"))
                })?;
                let rows = stmt
                    .query_map(
                        rusqlite::params![embedding_to_json(query_embedding), fetch_k as i64],
                        |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, Option<String>>(1)?,
                                row.get::<_, Option<String>>(2)?,
                                row.get::<_, f32>(3)?,
                            ))
                        },
                    )
                    .map_err(|e| {
                        RuChatError::SqliteVecError(format!("sqlite-vec query failed: {e}"))
                    })?
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .map_err(|e| {
                        RuChatError::SqliteVecError(format!(
                            "sqlite-vec query row read failed: {e}"
                        ))
                    })?;

                let mut q_ids = Vec::new();
                let mut q_docs = Vec::new();
                let mut q_metas = Vec::new();
                let mut q_dists = Vec::new();
                for (id, doc, meta_json, distance) in rows {
                    if let Some(allowed) = &ids
                        && !allowed.contains(&id)
                    {
                        continue;
                    }
                    let meta = json_to_metadata(meta_json);
                    if let Some(w) = &r#where {
                        let matches = meta.as_ref().is_some_and(|m| metadata_matches(w, m));
                        if !matches {
                            continue;
                        }
                    }
                    q_ids.push(id);
                    q_docs.push(doc);
                    q_metas.push(meta);
                    q_dists.push(Some(distance));
                    if q_ids.len() >= n {
                        break;
                    }
                }
                out_ids.push(q_ids);
                out_docs.push(q_docs);
                out_metas.push(q_metas);
                out_dists.push(q_dists);
            }

            let include = include.unwrap_or_else(IncludeList::default_query);
            Ok(QueryResponse {
                ids: out_ids,
                embeddings: None,
                documents: Some(out_docs),
                uris: None,
                metadatas: Some(out_metas),
                distances: Some(out_dists),
                include: include
                    .0
                    .into_iter()
                    .filter(|i| *i != Include::Embedding)
                    .collect(),
            })
        })
        .await
        .map_err(|e| RuChatError::SqliteVecError(e.to_string()))?
    }
}

#[async_trait]
impl VectorStore for SqliteVecClient {
    async fn query_collection(
        &self,
        collection_name: &str,
        embeddings: Vec<Vec<f32>>,
        n_results: Option<u32>,
        r#where: Option<Where>,
        ids: Option<Vec<String>>,
        include: Option<IncludeList>,
    ) -> Result<QueryResponse> {
        let collection = self.collection(collection_name)?;
        collection
            .query_collection(
                collection_name,
                embeddings,
                n_results,
                r#where,
                ids,
                include,
            )
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chroma::types::MetadataValue;
    use std::collections::HashMap;

    fn open_temp() -> (tempfile::TempDir, SqliteVecClient) {
        let dir = tempfile::tempdir().unwrap();
        let client = SqliteVecClient::open(&dir.path().join("test.sqlite3")).unwrap();
        (dir, client)
    }

    #[test]
    fn validate_collection_name_rejects_anything_but_alphanumeric_and_underscore() {
        assert!(validate_collection_name("valid_name_1").is_ok());
        assert!(validate_collection_name("").is_err());
        assert!(validate_collection_name("has space").is_err());
        assert!(validate_collection_name("has-dash").is_err());
        assert!(validate_collection_name("has;semicolon").is_err());
        assert!(validate_collection_name("'; DROP TABLE x; --").is_err());
    }

    #[tokio::test]
    async fn add_then_query_finds_the_nearest_neighbor() {
        let (_dir, client) = open_temp();
        let collection = client.collection("docs").unwrap();

        collection
            .add(
                vec!["a".into(), "b".into(), "c".into()],
                vec![
                    vec![1.0, 0.0, 0.0],
                    vec![0.0, 1.0, 0.0],
                    vec![0.0, 0.0, 1.0],
                ],
                Some(vec![
                    Some("doc a".into()),
                    Some("doc b".into()),
                    Some("doc c".into()),
                ]),
                None,
            )
            .await
            .unwrap();

        let response = collection
            .query_collection("docs", vec![vec![0.9, 0.1, 0.0]], Some(1), None, None, None)
            .await
            .unwrap();

        assert_eq!(response.ids.len(), 1);
        assert_eq!(response.ids[0], vec!["a".to_string()]);
        assert_eq!(
            response.documents.as_ref().unwrap()[0],
            vec![Some("doc a".to_string())]
        );
    }

    #[tokio::test]
    async fn existing_ids_reports_only_what_was_actually_written() {
        let (_dir, client) = open_temp();
        let collection = client.collection("docs").unwrap();
        collection
            .add(vec!["a".into()], vec![vec![1.0, 0.0]], None, None)
            .await
            .unwrap();

        let found = collection
            .existing_ids(vec!["a".into(), "b".into()])
            .await
            .unwrap();
        assert_eq!(found, HashSet::from(["a".to_string()]));
    }

    #[tokio::test]
    async fn existing_ids_on_a_never_written_collection_is_empty_not_an_error() {
        let (_dir, client) = open_temp();
        let collection = client.collection("brand_new").unwrap();
        let found = collection.existing_ids(vec!["a".into()]).await.unwrap();
        assert!(found.is_empty());
    }

    #[tokio::test]
    async fn add_rejects_a_duplicate_id() {
        let (_dir, client) = open_temp();
        let collection = client.collection("docs").unwrap();
        collection
            .add(vec!["a".into()], vec![vec![1.0, 0.0]], None, None)
            .await
            .unwrap();
        let err = collection
            .add(vec!["a".into()], vec![vec![0.0, 1.0]], None, None)
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("already exists"));
    }

    #[tokio::test]
    async fn add_with_a_dimension_mismatch_returns_a_legible_error() {
        let (_dir, client) = open_temp();
        let collection = client.collection("docs").unwrap();
        collection
            .add(vec!["a".into()], vec![vec![1.0, 0.0]], None, None)
            .await
            .unwrap();
        let err = collection
            .add(vec!["b".into()], vec![vec![1.0, 0.0, 0.0]], None, None)
            .await
            .unwrap_err();
        // Not asserting exact wording (that's `vec0`'s own error text, not ours to pin down) —
        // just that it surfaces as our own error type, not a panic or a silently-wrong write.
        assert!(!format!("{err}").is_empty());
    }

    #[tokio::test]
    async fn update_rejects_an_id_that_does_not_exist() {
        let (_dir, client) = open_temp();
        let collection = client.collection("docs").unwrap();
        let err = collection
            .update(
                vec!["missing".into()],
                None,
                Some(vec![Some("x".into())]),
                None,
            )
            .await
            .unwrap_err();
        assert!(format!("{err}").contains("does not exist"));
    }

    #[tokio::test]
    async fn upsert_inserts_new_and_updates_existing_in_the_same_call() {
        let (_dir, client) = open_temp();
        let collection = client.collection("docs").unwrap();
        collection
            .add(
                vec!["a".into()],
                vec![vec![1.0, 0.0]],
                Some(vec![Some("old".into())]),
                None,
            )
            .await
            .unwrap();

        collection
            .upsert(
                vec!["a".into(), "b".into()],
                vec![vec![1.0, 0.0], vec![0.0, 1.0]],
                Some(vec![Some("new".into()), Some("brand new".into())]),
                None,
            )
            .await
            .unwrap();

        let response = collection
            .query_collection("docs", vec![vec![1.0, 0.0]], Some(2), None, None, None)
            .await
            .unwrap();
        let docs = response.documents.as_ref().unwrap()[0].clone();
        assert!(docs.contains(&Some("new".to_string())));
        assert!(docs.contains(&Some("brand new".to_string())));
    }

    #[tokio::test]
    async fn query_applies_a_where_filter_client_side() {
        let (_dir, client) = open_temp();
        let collection = client.collection("docs").unwrap();

        let mut meta_a: Metadata = HashMap::new();
        meta_a.insert("lang".into(), MetadataValue::Str("rust".into()));
        let mut meta_b: Metadata = HashMap::new();
        meta_b.insert("lang".into(), MetadataValue::Str("python".into()));

        collection
            .add(
                vec!["a".into(), "b".into()],
                vec![vec![1.0, 0.0], vec![0.9, 0.1]],
                Some(vec![Some("doc a".into()), Some("doc b".into())]),
                Some(vec![Some(meta_a), Some(meta_b)]),
            )
            .await
            .unwrap();

        let where_clause = chroma::types::Where::Metadata(chroma::types::MetadataExpression {
            key: "lang".to_string(),
            comparison: chroma::types::MetadataComparison::Primitive(
                chroma::types::PrimitiveOperator::Equal,
                MetadataValue::Str("python".into()),
            ),
        });

        let response = collection
            .query_collection(
                "docs",
                vec![vec![1.0, 0.0]],
                Some(5),
                Some(where_clause),
                None,
                None,
            )
            .await
            .unwrap();

        assert_eq!(response.ids[0], vec!["b".to_string()]);
    }
}
