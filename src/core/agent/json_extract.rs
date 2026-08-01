/// Strips common ```json / ``` fences models wrap structured output in
/// despite being told to emit raw JSON. Shared by the Validator's verdict
/// parser and the Librarian's query-JSON parser so both get identical
/// leniency instead of two independently-drifting trim chains.
pub(crate) fn strip_json_fences(output: &str) -> &str {
    output
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim()
}
