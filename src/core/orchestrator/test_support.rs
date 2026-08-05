use super::{DebugBreakpoints, Orchestrator, RunTaskOptions};
use crate::Result;
use crate::agent::Agent;
use crate::agent::event::StreamItem;
use crate::agent::llm_client::fake_vector_store::FakeVectorStore;
use crate::agent::llm_client::{FakeLlmClient, VectorStore};
use chroma::types::QueryResponse;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// Builds an `Orchestrator` by hand (not `Orchestrator::new`, which always
/// constructs a real `ChromaHttpClient` for the Librarian) so Librarian
/// fixtures can run against a `FakeVectorStore` instead of a live Chroma
/// server. Each role's `Agent` is still built through the real
/// `Agent::new`, so config parsing/merging is exercised exactly as in
/// production — only the network-facing clients are swapped.
pub(super) async fn build_test_orchestrator(
    mut config: Value,
    responses: Vec<&str>,
    query_response: Option<QueryResponse>,
) -> Orchestrator {
    let architect = Agent::new(&mut config, "Architect", true, None, json!({}))
        .await
        .unwrap();
    let worker = Agent::new(&mut config, "Worker", true, None, json!({}))
        .await
        .unwrap();
    let validator = Agent::new(&mut config, "Validator", false, None, json!({}))
        .await
        .ok();
    let summarizer = Agent::new(&mut config, "Summarizer", false, None, json!({}))
        .await
        .ok();
    let scoper = Agent::new(&mut config, "Scoper", false, None, json!({}))
        .await
        .ok();
    let librarian = Agent::new(&mut config, "Librarian", false, None, json!({}))
        .await
        .ok();

    let mut critics = Vec::new();
    if let Some(critic_list) = config.get("Critics").and_then(|v| v.as_array()) {
        for (i, c_val) in critic_list.iter().enumerate() {
            let role = format!("Critic_{i}");
            let mut c_config = json!({ role.clone(): c_val.clone() });
            if let Ok(agent) = Agent::new(&mut c_config, &role, true, None, json!({})).await {
                critics.push(agent);
            }
        }
    }

    let client: Option<Arc<dyn VectorStore>> = query_response
        .map(|response| Arc::new(FakeVectorStore { response }) as Arc<dyn VectorStore>);

    Orchestrator {
        scoper,
        architect,
        worker,
        librarian,
        critics,
        summarizer,
        validator,
        orchestrator_config: config,
        chat: Arc::new(FakeLlmClient::new(responses)),
        embed: Arc::new(FakeLlmClient::new(vec![])),
        client,
    }
}

pub(super) fn fake_query_response() -> QueryResponse {
    QueryResponse {
        ids: vec![vec!["doc1".to_string()]],
        embeddings: None,
        documents: Some(vec![vec![Some("fake retrieved document".to_string())]]),
        uris: None,
        metadatas: None,
        distances: None,
        include: vec![],
    }
}

/// Runs a fixture to completion and returns every streamed item —
/// panics (failing the test) if the stage machine ever surfaces an
/// `Err`, which is the thing debug-mode is meant to make crisp to catch.
pub(super) async fn run_fixture(
    fixture: &str,
    config: Value,
    responses: Vec<&str>,
    query_response: Option<QueryResponse>,
) -> Vec<StreamItem> {
    let orchestrator = build_test_orchestrator(config, responses, query_response).await;
    let path = format!("agent_debug/{fixture}");
    let stream = orchestrator.run_task_stream(
        "test goal".to_string(),
        RunTaskOptions {
            debug_sequence: Some(path),
            breakpoints: DebugBreakpoints::default(),
            resume: false,
            approve_commit: false,
            trace_timings: false,
        },
        CancellationToken::new(),
    );
    tokio_stream::StreamExt::collect::<Vec<Result<StreamItem>>>(stream)
        .await
        .into_iter()
        .map(|r| r.expect("debug sequence produced an error"))
        .collect()
}

pub(super) fn base_config() -> Value {
    json!({
        "Architect": { "model": "fake" },
        "Worker": { "model": "fake" },
    })
}
