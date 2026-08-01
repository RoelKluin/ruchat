use crate::{Result, RuChatError};
use async_trait::async_trait;
use ollama_rs::generation::embeddings::request::GenerateEmbeddingsRequest;
use ollama_rs::generation::embeddings::response::GenerateEmbeddingsResponse;

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
    async fn generate_embeddings(&self, _req: GenerateEmbeddingsRequest) -> Result<GenerateEmbeddingsResponse> {
        Ok(self.response.clone())
    }
}
