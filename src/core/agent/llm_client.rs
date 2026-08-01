use crate::retry_transient;
use crate::{Result, RuChatError};
use async_trait::async_trait;
use chroma::types::{IncludeList, QueryResponse, Where};
use chroma::ChromaHttpClient;
use ollama_rs::generation::chat::{request::ChatMessageRequest, ChatMessage, ChatMessageResponse};
use ollama_rs::generation::embeddings::request::GenerateEmbeddingsRequest;
use ollama_rs::generation::embeddings::GenerateEmbeddingsResponse;
use ollama_rs::Ollama;
use std::pin::Pin;
use std::sync::Arc;
use tokio_stream::Stream;
use tokio_stream::StreamExt;

/// A single streamed chat response chunk, normalized to this crate's
/// `Result` so callers never need to know `ollama-rs`'s internal stream
/// error representation.
pub(crate) type ChatStream = Pin<Box<dyn Stream<Item = Result<ChatMessageResponse>> + Send>>;

/// Everything `Agent::query_stream` / `generate_oneshot` need from an Ollama
/// client. Exists so both can take `Arc<dyn LlmClient>` (or be generic over
/// it) instead of a concrete `ollama_rs::Ollama`, enabling an in-memory fake
/// for stage-machine tests that currently require a live server.
#[async_trait]
pub(crate) trait LlmClient: Send + Sync {
    async fn chat_stream(&self, model: &str, messages: Vec<ChatMessage>) -> Result<ChatStream>;
    async fn generate_embeddings(
        &self,
        req: GenerateEmbeddingsRequest,
    ) -> Result<GenerateEmbeddingsResponse>;
}

#[async_trait]
impl LlmClient for Ollama {
    async fn chat_stream(&self, model: &str, messages: Vec<ChatMessage>) -> Result<ChatStream> {
        let request = ChatMessageRequest::new(model.to_string(), messages);
        let stream = self
            .send_chat_messages_stream(request)
            .await
            .map_err(RuChatError::OllamaError)?;
        let mapped =
            stream.map(|item| item.map_err(|_| RuChatError::Is("ollama stream error".into())));
        Ok(Box::pin(mapped))
    }

    async fn generate_embeddings(
        &self,
        req: GenerateEmbeddingsRequest,
    ) -> Result<GenerateEmbeddingsResponse> {
        Ollama::generate_embeddings(self, req)
            .await
            .map_err(RuChatError::OllamaError)
    }
}

#[async_trait]
impl LlmClient for Arc<dyn LlmClient> {
    async fn chat_stream(&self, model: &str, messages: Vec<ChatMessage>) -> Result<ChatStream> {
        (**self).chat_stream(model, messages).await
    }
    async fn generate_embeddings(
        &self,
        req: GenerateEmbeddingsRequest,
    ) -> Result<GenerateEmbeddingsResponse> {
        (**self).generate_embeddings(req).await
    }
}

#[async_trait]
impl VectorStore for Arc<dyn VectorStore> {
    async fn query_collection(
        &self,
        collection_name: &str,
        embeddings: Vec<Vec<f32>>,
        n_results: Option<u32>,
        r#where: Option<Where>,
        ids: Option<Vec<String>>,
        include: Option<IncludeList>,
    ) -> Result<QueryResponse> {
        (**self)
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

/// Narrow vector-store abstraction covering only what the agent pipeline's
/// Librarian/Retrieve-tool path needs — resolve a collection by name and run
/// a KNN query against it. NOT a general Chroma facade; the standalone
/// `chroma-*` CLI commands (get.rs, search.rs, etc.) keep using
/// `ChromaHttpClient`/`ChromaCollection` directly.
#[async_trait]
pub(crate) trait VectorStore: Send + Sync {
    async fn query_collection(
        &self,
        collection_name: &str,
        embeddings: Vec<Vec<f32>>,
        n_results: Option<u32>,
        r#where: Option<Where>,
        ids: Option<Vec<String>>,
        include: Option<IncludeList>,
    ) -> Result<QueryResponse>;
}

#[async_trait]
impl VectorStore for ChromaHttpClient {
    async fn query_collection(
        &self,
        collection_name: &str,
        embeddings: Vec<Vec<f32>>,
        n_results: Option<u32>,
        r#where: Option<Where>,
        ids: Option<Vec<String>>,
        include: Option<IncludeList>,
    ) -> Result<QueryResponse> {
        let collection = retry_transient!(async {
            self.get_collection(collection_name)
                .await
                .map_err(RuChatError::from)
        })?;
        retry_transient!(async {
            collection
                .query(
                    embeddings.clone(),
                    n_results,
                    r#where.clone(),
                    ids.clone(),
                    include.clone(),
                )
                .await
                .map_err(RuChatError::from)
        })
    }
}

#[async_trait]
pub(crate) trait EmbeddingsClient: Send + Sync {
    async fn generate_embeddings(
        &self,
        req: GenerateEmbeddingsRequest,
    ) -> Result<GenerateEmbeddingsResponse>;
}

#[async_trait]
impl EmbeddingsClient for ollama_rs::Ollama {
    async fn generate_embeddings(
        &self,
        req: GenerateEmbeddingsRequest,
    ) -> Result<GenerateEmbeddingsResponse> {
        ollama_rs::Ollama::generate_embeddings(self, req)
            .await
            .map_err(RuChatError::OllamaError)
    }
}

// A fake for tests:
#[cfg(test)]
pub(crate) struct FakeEmbeddingsClient {
    pub(crate) response: GenerateEmbeddingsResponse,
}
#[cfg(test)]
#[async_trait]
impl EmbeddingsClient for FakeEmbeddingsClient {
    async fn generate_embeddings(
        &self,
        _req: GenerateEmbeddingsRequest,
    ) -> Result<GenerateEmbeddingsResponse> {
        Ok(self.response.clone())
    }
}

#[cfg(test)]
pub(crate) mod fake_vector_store {
    use super::*;

    pub(crate) struct FakeVectorStore {
        pub(crate) response: QueryResponse,
    }

    #[async_trait]
    impl VectorStore for FakeVectorStore {
        async fn query_collection(
            &self,
            _collection_name: &str,
            _embeddings: Vec<Vec<f32>>,
            _n_results: Option<u32>,
            _where: Option<Where>,
            _ids: Option<Vec<String>>,
            _include: Option<IncludeList>,
        ) -> Result<QueryResponse> {
            Ok(self.response.clone())
        }
    }
}
