//! Reciprocal rank fusion (docs/mcp-server.md §6.5).
//!
//! Neither retrieval half suffices alone: keyword nails exact identifiers,
//! vectors nail paraphrase. RRF fuses their rankings using ranks only, which
//! is exactly why it fits here — the lexical scorer's `in_title * 1000 +
//! occurrences` is not comparable to cosine similarity in any principled way,
//! and RRF never has to compare them.

/// The paper's constant. It holds up across corpora — **don't tune it**: any
/// gain is smaller than the noise in a hand-built eval set.
pub const RRF_K: f32 = 60.0;

/// Fuse ranked id lists into one ranking:
/// `score(id) = Σ_lists 1 / (RRF_K + rank)`, rank 1-based. Highest score
/// first; ties keep first-appearance order across the lists as given (so the
/// caller's list order is the deterministic tiebreak).
pub fn rrf(lists: &[Vec<String>]) -> Vec<(String, f32)> {
    // First-seen order doubles as the tie-break, so accumulate in a Vec — the
    // fused lists are top-50 each, far too small for a map to matter.
    let mut scores: Vec<(String, f32)> = Vec::new();
    for list in lists {
        for (i, id) in list.iter().enumerate() {
            let add = 1.0 / (RRF_K + (i + 1) as f32);
            match scores.iter_mut().find(|(seen, _)| seen == id) {
                Some((_, score)) => *score += add,
                None => scores.push((id.clone(), add)),
            }
        }
    }
    // Stable sort: equal scores keep first-seen order.
    scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scores
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn appearing_in_both_lists_beats_topping_one() {
        let fused = rrf(&[
            ids(&["only-lexical", "both"]),
            ids(&["only-vector", "both"]),
        ]);
        assert_eq!(fused[0].0, "both", "two mid ranks outweigh one top rank");
        let expected = 2.0 / (RRF_K + 2.0);
        assert!((fused[0].1 - expected).abs() < 1e-6, "the paper's formula");
    }

    #[test]
    fn single_list_order_is_preserved() {
        let fused = rrf(&[ids(&["a", "b", "c"])]);
        let order: Vec<&str> = fused.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(order, vec!["a", "b", "c"]);
    }

    #[test]
    fn ties_keep_first_seen_order() {
        // Symmetric ranks → identical scores; the first list's order wins.
        let fused = rrf(&[ids(&["x", "y"]), ids(&["y", "x"])]);
        let order: Vec<&str> = fused.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(order, vec!["x", "y"]);
    }

    #[test]
    fn empty_input_fuses_to_nothing() {
        assert!(rrf(&[]).is_empty());
        assert!(rrf(&[Vec::new(), Vec::new()]).is_empty());
    }
}
