use ollama_rs::error::OllamaError;
use serde_json::error::Error as SerdeError;
use std::io::Error as IoError;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum RuChatError {
    #[error("Invalid model name: {0}")]
    InvalidModelName(String),
    #[error("Model not found: {0}")]
    ModelNotFound(String),
    #[error("Failed to read file: {0}")]
    FileReadError(IoError),
    #[error("Serde error: {0}")]
    SerdeError(SerdeError),
    #[error("Failed to read file: {0}")]
    ModelPullError(String),
    #[error("Ollama error: {0}")]
    OllamaError(OllamaError),
    #[error("Unable to parse arg --server: '{0}'")]
    ArgServerError(String),
    #[error("Failed to read {0}: {1}")]
    ReadError(String, IoError),
    #[error("Failed to write to stream: {0}")]
    StreamWriteError(IoError),
}

impl From<std::io::Error> for RuChatError {
    fn from(err: IoError) -> Self {
        RuChatError::StreamWriteError(err)
    }
}

impl From<OllamaError> for RuChatError {
    fn from(err: OllamaError) -> Self {
        RuChatError::OllamaError(err)
    }
}

impl From<SerdeError> for RuChatError {
    fn from(err: SerdeError) -> Self {
        RuChatError::SerdeError(err)
    }
}
