use crate::agent::llm_client::LlmClient;
use crate::{Result, RuChatError};
use ollama_rs::generation::chat::ChatMessage;
use std::time::Duration;
use tokio_stream::StreamExt;

/// Compresses raw retrieved RAG content (a Chroma query's rendered results — always includes
/// verbose per-row metadata: timestamps, embedding model name, long reference lists) before it
/// reaches the Worker's prompt as a `TurnKind::Retrieval` turn. A distinct one-shot prompt from
/// `run_summary.rs`'s trace summaries, not a reuse of `agent_role/summarizer.md` (that template
/// is specifically about compressing round *history* — "what was tried and rejected," "current
/// implementation state" — concepts that don't apply to retrieved documents, and would actively
/// mislead the model if reused as-is here). Same "bypass the Agent/Role/Context turn-log
/// machinery for a one-shot analysis" shape as `run_summary.rs`, since this doesn't participate
/// in the run as a role, it just transforms one string before it's recorded.
///
/// Deliberately instructs the model to keep code/identifiers verbatim: unlike a history
/// summary (safe to paraphrase — it's a narrative of what happened), retrieved RAG content can
/// be literal source code or exact identifiers the Worker needs unchanged to produce a correct
/// patch. The compression opportunity here is the metadata/formatting overhead and cross-row
/// duplication, not the actual retrieved text.
pub(crate) async fn summarize_retrieved_documents(
    ollama: &dyn LlmClient,
    model: &str,
    goal: &str,
    docs: &str,
) -> Result<String> {
    if docs.trim().is_empty() {
        return Err(RuChatError::Is(
            "empty retrieved documents, nothing to summarize".into(),
        ));
    }
    let system = "You are compressing retrieved search results from a code/documentation \
        vector database before they reach a coding agent's prompt, to save tokens. The results \
        below are relevant to the stated goal. Condense them: remove redundant or duplicate \
        information across rows, drop verbose bookkeeping metadata that doesn't help the goal \
        (timestamps, embedding model names, long reference-id lists), and merge near-duplicate \
        entries. You MUST NOT paraphrase, shorten, or alter any actual code, file path, \
        function/struct/symbol name, or other verbatim identifier that appears in the results — \
        keep every one of those exactly as written, character for character; a coding agent may \
        need to match them exactly to produce a correct change. Only the surrounding \
        explanation/metadata is safe to compress. If nothing is safe to remove, say so briefly \
        rather than inventing compression.\n\n\
        This is a static index, not a live command: it can only ever tell you what happens to be \
        indexed, never the current, real-time state of the repository or its build. Do NOT \
        answer the goal from your own knowledge, and never state a conclusion the retrieved rows \
        don't literally contain — most importantly, never claim something does not exist (\"no \
        warnings found\", \"nothing matches\") just because the retrieved rows don't happen to \
        show it; that is a false negative, not an absence. If the retrieved rows don't actually \
        answer the goal — for example the goal asks for a live tool's current output and these \
        rows are old source/test content that merely mentions the same topic — say plainly that \
        the retrieved results don't show that, instead of inventing an answer. Output ONLY the \
        condensed results (or that one plain statement), no preamble, no meta-commentary about \
        what you removed.";
    let user = format!("GOAL: {goal}\n\nRETRIEVED RESULTS TO CONDENSE:\n{docs}");
    let messages = vec![
        ChatMessage::system(system.to_string()),
        ChatMessage::user(user),
    ];
    let generated = tokio::time::timeout(Duration::from_secs(30), async {
        let mut stream = ollama.chat_stream(model, messages).await?;
        let mut message = String::new();
        while let Some(chunk) = stream.next().await {
            message.push_str(&chunk?.message.content);
        }
        Ok::<String, RuChatError>(message)
    })
    .await
    .map_err(|_| RuChatError::Is("document summarization timed out after 30s".into()))??;

    let generated = generated.trim().to_string();
    if generated.is_empty() {
        Err(RuChatError::Is(
            "LLM returned an empty document summary".into(),
        ))
    } else {
        Ok(generated)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::llm_client::FakeLlmClient;

    #[tokio::test]
    async fn summarize_retrieved_documents_returns_the_trimmed_llm_response() {
        let ollama = FakeLlmClient::new(vec![
            "  Condensed: fn foo() -> Result<()> lives in \
            src/lib.rs.  ",
        ]);
        let summary = summarize_retrieved_documents(&ollama, "any-model", "find foo", "raw docs")
            .await
            .unwrap();
        assert_eq!(
            summary,
            "Condensed: fn foo() -> Result<()> lives in src/lib.rs."
        );
    }

    #[tokio::test]
    async fn summarize_retrieved_documents_rejects_empty_input_without_calling_the_llm() {
        let ollama = FakeLlmClient::new(vec![]); // would panic if chat_stream were called
        let result = summarize_retrieved_documents(&ollama, "any-model", "find foo", "   ").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn summarize_retrieved_documents_errors_on_an_empty_llm_response() {
        let ollama = FakeLlmClient::new(vec!["   "]);
        let result =
            summarize_retrieved_documents(&ollama, "any-model", "find foo", "raw docs").await;
        assert!(result.is_err());
    }
}
