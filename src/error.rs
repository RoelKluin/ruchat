use thiserror::Error;

/// An enumeration of possible errors in the RuChat application.
///
/// This enum defines various error types that can occur within the
/// RuChat application, each with a descriptive error message.
#[derive(Error, Debug)]
pub enum RuChatError {
    /// Error related to the model.
    #[error("Model Error: {0}")]
    ModelError(String),

    #[error("Invalid model name: {0}")]
    InvalidModelName(String),

    #[error("Model not found: {0}")]
    ModelNotFound(String),

    #[error("No model specified")]
    NoModelSpecified,

    /// Error due to invalid metadata.
    #[error("Invalid metadata: {0}")]
    InvalidMetadata(String),

    /// Input/output error.
    #[error("Input/output error: {0}")]
    Io(#[from] std::io::Error),

    /// Error during serialization or deserialization.
    #[error("Serde error: {0}")]
    SerdeError(#[from] serde_json::error::Error),

    /// Error from the Ollama library.
    #[error("Ollama error: {0}")]
    OllamaError(#[from] ollama_rs::error::OllamaError),

    /// Error when parsing the server argument.
    #[error("Unable to parse arg --server: '{0}'")]
    ArgServerError(String),

    /// Error from the Chroma library.
    #[error("Chroma error: {0}")]
    ChromaError(#[from] anyhow::Error),

    /// Error when converting from an integer.
    #[error("TryFromIntError: {0}")]
    TryFromIntError(#[from] std::num::TryFromIntError),

    /// Error when the cursor's first index is out of bounds.
    #[error("Cursor.0 out of bounds")]
    Cursor0OutOfBounds,

    /// Error when the cursor's second index is out of bounds.
    #[error("Cursor.1 out of bounds")]
    Cursor1OutOfBounds,

    /// Error when a task join operation fails.
    #[error("Join error: {0}")]
    JoinError(#[from] tokio::task::JoinError),

    /// Error when a request is not found.
    #[error("Request not found")]
    QuestionNotFound,

    /// Error when an answer is not found.
    #[error("Answer not found")]
    AnswerNotFound,

    /// Error when a question already exists.
    #[error("Question already exists")]
    QuestionAlreadyExists,

    /// Error when an answer already exists.
    #[error("Answer already exists")]
    AnswerAlreadyExists,

    /// Error when the cursor position is invalid.
    #[error("Invalid cursor position: {0}, {1}")]
    InvalidCursorPosition(usize, usize),

    /// Error from the Chroma HTTP client.
    #[error("Chroma HTTP client error: {0}")]
    ChromaHttpClientError(#[from] chroma::client::ChromaHttpClientError),
}

pub type Result<T> = std::result::Result<T, RuChatError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_error() {
        let error = RuChatError::ModelError("Test model error".to_string());
        assert_eq!(format!("{}", error), "Model Error: Test model error");
    }

    #[test]
    fn test_invalid_metadata_error() {
        let error = RuChatError::InvalidMetadata("Test invalid metadata".to_string());
        assert_eq!(
            format!("{}", error),
            "Invalid metadata: Test invalid metadata"
        );
    }

    #[test]
    fn test_cursor0_out_of_bounds_error() {
        let error = RuChatError::Cursor0OutOfBounds;
        assert_eq!(format!("{}", error), "Cursor.0 out of bounds");
    }

    #[test]
    fn test_cursor1_out_of_bounds_error() {
        let error = RuChatError::Cursor1OutOfBounds;
        assert_eq!(format!("{}", error), "Cursor.1 out of bounds");
    }
}
