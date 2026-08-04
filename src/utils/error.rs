use crate::agent::event::StreamItem;
use std::result::Result as StdResult;
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

    /// A general error from the `anyhow` crate.
    #[error("Anyhow error: {0}")]
    AnyhowError(#[from] anyhow::Error),

    /// Error when the model name is invalid.
    #[error("Invalid model name: {0}")]
    InvalidModelName(String),

    /// Error when the model is not found.
    #[error("Model not found: {0}")]
    ModelNotFound(String),

    /// Error when no model is specified.
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

    /// Error from the Anthropic Messages API client (`providers/llm/anthropic`) — request
    /// building, HTTP transport, or SSE stream parsing failures.
    #[error("Anthropic API error: {0}")]
    AnthropicError(String),

    /// Error from the SQLite-vec vector-store backend (`providers/vector/sqlite_vec`) — file
    /// open, schema creation, or query/write failures against the local database. Same
    /// String-carrying shape as `AnthropicError`: distinguishable from the generic `Is`/
    /// `InternalError` catch-alls without a full per-failure-mode enum, matching what was
    /// judged proportionate for a single external-integration surface.
    #[error("SQLite-vec error: {0}")]
    SqliteVecError(String),

    /// Error when parsing metadata.
    #[error("Metadata parse error for input '{0}': {1}")]
    MetadataFileReadError(String, std::io::Error),

    /// Error when parsing metadata.
    #[error("Metadata parse error for input '{0}': {1}")]
    MetadataParseError(String, serde_json::error::Error),

    /// Error when parsing the server argument.
    #[error("Unable to parse arg --server: '{0}'")]
    ArgServerError(String),

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
    #[error("Invalid cursor position {0}: {1}, {2}")]
    InvalidCursorPosition(String, usize, usize),

    /// Error from the Chroma HTTP client.
    #[error("Chroma HTTP client error: {0}")]
    ChromaHttpClientError(#[from] chroma::client::ChromaHttpClientError),

    /// Error when the active team index is out of bounds.
    #[error("Active team index out of bounds")]
    ActiveTeamIndexOutOfBounds,

    /// Error when the prompt is empty.
    #[error("Prompt is empty")]
    EmptyPrompt,

    /// Error when the exit code format is invalid.
    #[error("Invalid exit code format: {0}")]
    InvalidExitCodeFormat(String),

    /// Error when there are conflicting prompts.
    #[error("Conflicting prompts provided")]
    ConflictingPrompts,

    /// Error when parsing an integer fails.
    #[error("Parse int error: {0}")]
    ParseIntError(#[from] std::num::ParseIntError),

    /// Error when reading a file fails.
    #[error("File read error: {0}")]
    FileReadError(String),

    /// Error when a command exits with a non-zero status.
    #[error("Command `{0}` exited with status: {1}")]
    CommandExitError(String, String),

    /// Error when multiple commands exit with errors.
    #[error("Multiple command exit errors: {0}")]
    MultipleCommandExitErrors(String),

    /// Error when no prompt is provided.
    #[error("No prompt provided")]
    NoPromptProvided,

    /// Error when no collection is specified.
    #[error("No collection specified. See `ruchat chroma-ls` for available collections.")]
    NoCollectionSpecified,

    /// Error when a collection is not found.
    #[error("No answer found for question_id {0} and answer_id {1}")]
    HistoryError(usize, usize),

    /// Error when the provided string is neither a file nor parseable metadata.
    #[error("Provided string is neither a file or parseable '{0}'")]
    MetadataFileOrParseError(String),

    /// A catch-all error for unexpected internal issues.
    #[error("Internal error: {0}")]
    InternalError(String),

    /// Error when the include list is invalid.
    #[error("Invalid include list: {0}")]
    InvalidIncludeList(String),

    /// Error when an include field is invalid.
    #[error("Invalid include field: {0}")]
    InvalidIncludeField(String),

    /// Error when metadata conversion fails.
    #[error("Metadata conversion error: {0}")]
    MetadataConversionError(String),

    /// Error when an agent is missing.
    #[error("Missing agent: {0}")]
    MissingAgent(String),

    /// Error when sending a message through a channel fails. Boxed (not `#[from]`, since that
    /// only auto-derives when the field type matches the source type exactly) because the
    /// unboxed `SendError<StdResult<StreamItem, Box<RuChatError>>>` embeds a whole `StreamItem`,
    /// making this by far the largest variant (≥240 bytes) and tripping `clippy::
    /// result_large_err` on every fallible function in the crate, since `RuChatError` is the
    /// crate-wide `Result<T>` error type. Manual `From` impl below does the boxing `#[from]`
    /// can't.
    #[error("Channel send error: {0}")]
    ChannelError(Box<tokio::sync::mpsc::error::SendError<StdResult<StreamItem, Box<RuChatError>>>>),

    /// Error when a role is invalid.
    #[error("Invalid role: {0}")]
    InvalidRole(String),

    /// A general error for any other issues that don't fit into the above categories.
    #[error("Error: {0}")]
    Is(String),

    /// Orchestration was cancelled (receiver dropped / explicit shutdown) —
    /// not a failure, just an early-exit signal distinct from semantic
    /// rejections and transient infra errors.
    #[error("Orchestration cancelled")]
    Cancelled,
}

/// Manual, not `#[from]`: see `ChannelError`'s doc comment for why the field is boxed.
impl From<tokio::sync::mpsc::error::SendError<StdResult<StreamItem, Box<RuChatError>>>>
    for RuChatError
{
    fn from(
        e: tokio::sync::mpsc::error::SendError<StdResult<StreamItem, Box<RuChatError>>>,
    ) -> Self {
        RuChatError::ChannelError(Box::new(e))
    }
}

/// A type alias for `Result` that uses `RuChatError` as the error type.
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
