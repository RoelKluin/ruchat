use thiserror::Error;

/// An enumeration of possible errors in the RuChat application.
///
/// This enum defines various error types that can occur within the
/// RuChat application, each with a descriptive error message.
#[derive(Error, Debug)]
pub enum RuChatError {
    /// Represents an error related to the model.
    #[error("Model Error: {0}")]
    ModelError(String),

    /// Represents an error due to invalid metadata.
    #[error("Invalid metadata: {0}")]
    InvalidMetadata(String),

    /// Represents an input/output error.
    #[error("Input/output error: {0}")]
    Io(#[from] std::io::Error),

    /// Represents an error during serialization or deserialization.
    #[error("Serde error: {0}")]
    SerdeError(#[from] serde_json::error::Error),

    /// Represents an error from the Ollama library.
    #[error("Ollama error: {0}")]
    OllamaError(#[from] ollama_rs::error::OllamaError),

    /// Represents an error when parsing the server argument.
    #[error("Unable to parse arg --server: '{0}'")]
    ArgServerError(String),

    /// Represents an error from the Chroma library.
    #[error("Chroma error: {0}")]
    ChromaError(#[from] anyhow::Error),

    /// Represents an error when converting from an integer.
    #[error("TryFromIntError: {0}")]
    TryFromIntError(#[from] std::num::TryFromIntError),

    /// Represents an error when the cursor's first index is out of bounds.
    #[error("Cursor.0 out of bounds")]
    Cursor0OutOfBounds,

    /// Represents an error when the cursor's second index is out of bounds.
    #[error("Cursor.1 out of bounds")]
    Cursor1OutOfBounds,

    /// Represents an error when a task join operation fails.
    #[error("Join error: {0}")]
    JoinError(#[from] tokio::task::JoinError),

    /// Represents an error when a request is not found.
    #[error("Request not found")]
    QuestionNotFound,

    /// Represents an error when an answer is not found.
    #[error("Answer not found")]
    AnswerNotFound,

    /// Represents an error when a question already exists.
    #[error("Question already exists")]
    QuestionAlreadyExists,

    /// Represents an error when an answer already exists.
    #[error("Answer already exists")]
    AnswerAlreadyExists,
}
