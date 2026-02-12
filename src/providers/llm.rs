pub(crate) mod ollama;
use crate::Result;

trait LlmProvider {
    async fn generate(&self) -> Result<String>;
}
