pub(crate) mod chroma;
pub(crate) mod sqlite_vec;

use clap::ValueEnum;
use serde::Deserialize;

/// Which backend a given `EmbedArgs`/Librarian config talks to — the same
/// opt-in-alternative shape `--chat-provider` uses for the LLM side
/// (`providers/llm/ollama/ask.rs`), applied to the vector store instead.
/// Chroma stays the default; `SqliteVec` is never picked implicitly.
#[derive(ValueEnum, Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum VectorProvider {
    #[default]
    Chroma,
    SqliteVec,
}
