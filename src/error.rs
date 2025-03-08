use thiserror::Error;

#[derive(Error, Debug)]
pub enum RuChatError {
    #[error("Invalid model name: {0}")]
    InvalidModelName(String),
    #[error("Invalid metadata: {0}")]
    InvalidMetadata(String),
    #[error("Model not found: {0}")]
    ModelNotFound(String),
    #[error("Input/output error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Serde error: {0}")]
    SerdeError(#[from] serde_json::error::Error),
    #[error("Failed to read file: {0}")]
    ModelPullError(String),
    #[error("Ollama error: {0}")]
    OllamaError(#[from] ollama_rs::error::OllamaError),
    #[error("Unable to parse arg --server: '{0}'")]
    ArgServerError(String),
    #[error("Chroma error: {0}")]
    ChromaError(#[from] anyhow::Error),
}
