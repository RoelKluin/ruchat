use ollama_rs::error::OllamaError;
use std::fmt::{self, Display, Formatter};

#[derive(Debug)]
pub enum Error {
    InvalidModelName(String),
    ModelNotFound(String),
    FileReadError(std::io::Error),
    ConfigSerializationError(serde_json::Error),
    ConfigDeserializationError(serde_json::Error),
    ModelPullError(String),
    OllamaServerError(String),
    ReadError(String, std::io::Error),
    StreamWriteError(std::io::Error),
}

impl From<std::io::Error> for Error {
    fn from(err: std::io::Error) -> Self {
        Error::StreamWriteError(err)
    }
}

impl From<serde_json::Error> for Error {
    fn from(err: serde_json::Error) -> Self {
        if err.is_data() || err.is_syntax() {
            Error::ConfigDeserializationError(err)
        } else {
            Error::ConfigSerializationError(err)
        }
    }
}

impl From<OllamaError> for Error {
    fn from(err: OllamaError) -> Self {
        match err {
            _ => todo!(),
        }
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            Error::InvalidModelName(name) => write!(f, "Invalid model name: {}", name),
            Error::ModelNotFound(name) => write!(f, "Model not found: {}", name),
            Error::FileReadError(e) => write!(f, "Failed to read file: {}", e),
            Error::ConfigSerializationError(e) => write!(f, "Failed to serialize config: {}", e),
            Error::ConfigDeserializationError(e) => {
                write!(f, "Failed to deserialize config: {}", e)
            }
            Error::ModelPullError(name) => write!(f, "Failed to pull model: {}", name),
            Error::OllamaServerError(server) => write!(f, "Invalid Ollama server: {}", server),
            Error::ReadError(file, e) => write!(f, "Failed to read {}: {}", file, e),
            Error::StreamWriteError(e) => write!(f, "Failed to write to stream: {}", e),
        }
    }
}
