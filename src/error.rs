use thiserror::Error;

#[derive(Error, Debug)]
pub enum RuChatError {
    #[error("Model Error: {0}")]
    ModelError(String),
    #[error("Invalid metadata: {0}")]
    InvalidMetadata(String),
    #[error("Input/output error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Serde error: {0}")]
    SerdeError(#[from] serde_json::error::Error),
    #[error("Ollama error: {0}")]
    OllamaError(#[from] ollama_rs::error::OllamaError),
    #[error("Unable to parse arg --server: '{0}'")]
    ArgServerError(String),
    #[error("Chroma error: {0}")]
    ChromaError(#[from] anyhow::Error),
    #[error("TryFromIntError: {0}")]
    TryFromIntError(#[from] std::num::TryFromIntError),
    #[error("Cursor.0 out of bounds")]
    Cursor0OutOfBounds,
    #[error("Cursor.1 out of bounds")]
    Cursor1OutOfBounds,
    #[error("Join error: {0}")]
    JoinError(#[from] tokio::task::JoinError),
    #[error("Request not found")]
    QuestionNotFound,
    #[error("Answer not found")]
    AnswerNotFound,
    #[error("Question already exists")]
    QuestionAlreadyExists,
    #[error("Answer already exists")]
    AnswerAlreadyExists,
}
