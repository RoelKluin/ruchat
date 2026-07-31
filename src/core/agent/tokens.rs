use std::sync::OnceLock;
use tiktoken_rs::CoreBPE;

static BPE: OnceLock<CoreBPE> = OnceLock::new();

/// Approximate token count using the `cl100k_base` BPE (GPT-3.5/4 family).
///
/// **This is an approximation, not the exact tokenizer for arbitrary
/// Ollama-served models** — Llama/Qwen/etc. use their own SentencePiece/BPE
/// vocabularies, and Ollama's REST API doesn't expose a standalone tokenize
/// endpoint to query the real one. This replaces the previous `len() / 4`
/// heuristic, which ignored subword/whitespace/punctuation structure
/// entirely; `cl100k_base` at least tokenizes in a shape similar to what
/// modern subword tokenizers produce, which is a meaningfully better proxy
/// even though it isn't exact per-model.
pub(crate) fn count_tokens(text: &str) -> u64 {
    let bpe = BPE.get_or_init(|| {
        tiktoken_rs::cl100k_base().expect("bundled cl100k_base ranks failed to load")
    });
    bpe.encode_ordinary(text).len() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_roughly_reasonable_tokens() {
        // Sanity bound, not an exact-match assertion — different tokenizers
        // will disagree on exact counts; this just guards against a
        // regression back to the old `len()/4` behavior (which would give
        // ~5 for this string) or a totally broken 0/huge output.
        let n = count_tokens("The quick brown fox jumps over the lazy dog.");
        assert!((5..=15).contains(&n), "unexpected token count: {n}");
    }
}
