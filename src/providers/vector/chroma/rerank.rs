use chroma::types::QueryResponse;
use std::collections::HashSet;

/// Lightweight, dependency-free reranker combining vector distance with
/// lexical term-overlap between the query and each candidate document.
///
/// This is NOT a cross-encoder — it's a cheap second-pass signal that
/// corrects the common failure mode of small embedding models (like
/// all-minilm-l6-v2) on code/technical text: a document can land close in
/// embedding space while sharing almost no vocabulary with the query. Distance
/// still dominates the blended score; lexical overlap acts as a tie-breaker /
/// correction, not a replacement for the vector search itself.
pub(crate) struct RerankWeights {
    pub(crate) distance_weight: f32,
    pub(crate) lexical_weight: f32,
}

impl Default for RerankWeights {
    fn default() -> Self {
        Self {
            distance_weight: 0.7,
            lexical_weight: 0.3,
        }
    }
}

fn tokenize(s: &str) -> HashSet<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|t| t.len() > 2) // drop stopword-length noise (a, is, of, ..)
        .map(str::to_lowercase)
        .collect()
}

fn lexical_overlap(query_tokens: &HashSet<String>, doc: &str) -> f32 {
    if query_tokens.is_empty() {
        return 0.0;
    }
    let doc_tokens = tokenize(doc);
    let hits = query_tokens.intersection(&doc_tokens).count();
    hits as f32 / query_tokens.len() as f32
}

/// Reorders every parallel array within `result` in place, per query index,
/// by a blended distance+lexical score. No-ops (leaves Chroma's raw KNN
/// order untouched) when document text wasn't requested via `include` —
/// there is nothing to add over KNN order without it.
pub(crate) fn rerank_query_results(
    query_texts: &[String],
    result: &mut QueryResponse,
    weights: &RerankWeights,
) {
    if result.documents.is_none() {
        return;
    }

    for qi in 0..result.ids.len() {
        let n = result.ids[qi].len();
        if n == 0 {
            continue;
        }
        let query_tokens = query_texts.get(qi).map(|q| tokenize(q)).unwrap_or_default();

        let mut scored: Vec<(usize, f32)> = (0..n)
            .map(|ri| {
                let dist = result
                    .distances
                    .as_ref()
                    .and_then(|d| d.get(qi))
                    .and_then(|row| row.get(ri))
                    .copied()
                    .flatten()
                    .unwrap_or(1.0);
                // Chroma distance: lower is better — invert to a similarity so
                // higher score == better, consistent with lexical overlap.
                let dist_score = 1.0 - dist.clamp(0.0, 1.0);
                let doc = result
                    .documents
                    .as_ref()
                    .and_then(|d| d.get(qi))
                    .and_then(|row| row.get(ri))
                    .and_then(|o| o.as_deref())
                    .unwrap_or("");
                let lex_score = lexical_overlap(&query_tokens, doc);
                (
                    ri,
                    weights.distance_weight * dist_score + weights.lexical_weight * lex_score,
                )
            })
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        let order: Vec<usize> = scored.into_iter().map(|(ri, _)| ri).collect();

        permute_row(&mut result.ids[qi], &order);
        if let Some(d) = result.documents.as_mut() {
            permute_row(&mut d[qi], &order);
        }
        if let Some(m) = result.metadatas.as_mut() {
            permute_row(&mut m[qi], &order);
        }
        if let Some(e) = result.embeddings.as_mut() {
            permute_row(&mut e[qi], &order);
        }
        if let Some(u) = result.uris.as_mut() {
            permute_row(&mut u[qi], &order);
        }
        if let Some(ds) = result.distances.as_mut() {
            permute_row(&mut ds[qi], &order);
        }
    }
}

fn permute_row<T: Clone>(row: &mut [T], order: &[usize]) {
    let orig = row.to_vec();
    for (new_idx, &old_idx) in order.iter().enumerate() {
        row[new_idx] = orig[old_idx].clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexical_overlap_favors_matching_terms() {
        let q = tokenize("borrow checker lifetime");
        let high = lexical_overlap(&q, "the borrow checker enforces lifetime rules");
        let low = lexical_overlap(&q, "completely unrelated text about cooking");
        assert!(high > low);
    }
}
