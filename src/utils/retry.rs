use crate::RuChatError;
use std::time::Duration;

pub(crate) const MAX_ATTEMPTS: u32 = 3;
const BASE_DELAY_MS: u64 = 250;

/// True for errors plausibly transient (dropped connection, request
/// timeout) rather than semantic (bad prompt, malformed tool call,
/// validation rejection). Conservative on purpose: only the network-shaped
/// `OllamaError` variant is retried. Retrying semantic errors would just
/// reproduce the same deterministic failure at the cost of another model
/// call — those should surface immediately to the stage machine instead.
pub(crate) fn is_transient(err: &RuChatError) -> bool {
    match err {
        RuChatError::OllamaError(_) => true,
        // NEEDS VERIFICATION: `chroma::client::ChromaHttpClientError`'s
        // variant set isn't in the provided excerpts. Ideally this should
        // match specific transient variants (connection refused, timeout)
        // the same way `OllamaError` is matched wholesale-transient. Until
        // that enum is inspected, fall back to a conservative substring
        // check on Display output — replace with proper variant matching
        // once the crate's error type is available.
        RuChatError::ChromaHttpClientError(e) => {
            let msg = e.to_string().to_lowercase();
            msg.contains("connection") || msg.contains("timed out") || msg.contains("timeout")
        }
        _ => false,
    }
}

pub(crate) fn backoff_delay(attempt: u32) -> Duration {
    Duration::from_millis(BASE_DELAY_MS * 2u64.pow(attempt.saturating_sub(1)))
}
