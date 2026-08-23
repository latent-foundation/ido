//! The retrieval eval: the query set's format, recall@5 / MRR, and the gate
//! (docs/mcp-server.md §6.7).
//!
//! Without an eval, "semantic search" is vibes. This module owns the *metric*
//! half — parsing `eval.jsonl`, scoring a ranking against a gold set, and
//! deciding whether hybrid earned its place — while the runner that actually
//! searches lives in `ido-mcp` (`--eval`), because only there is a real
//! embedder loaded. That split keeps the arithmetic unit-testable against
//! hand-built rankings, with no model and no index in sight.
//!
//! The gate, per §6.7: **hybrid must not do worse than keyword on recall@5
//! overall, and must not regress the `identifier` class.** Fusion is allowed to
//! be neutral (RRF can only reorder what the halves found), never negative. If
//! it fails, the chunking or the query set is where to look — not `RRF_K`.

use std::collections::BTreeMap;

/// The cutoff recall is measured at. §6.7 names 5; it is also roughly how many
/// hits a model actually reads before deciding.
pub const RECALL_AT: usize = 5;

/// One `{kind, id}` pair — an entry, in the same vocabulary
/// [`crate::model::SearchHit`] uses, so an expectation and a hit compare
/// directly.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct EntryRef {
    /// `"note"` | `"wiki"` | `"task"` | `"goal"`.
    pub kind: String,
    /// The entry's id, exactly as search reports it.
    pub id: String,
}

impl EntryRef {
    /// Build a reference — mostly for the runner, which turns hits into these.
    pub fn new(kind: impl Into<String>, id: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            id: id.into(),
        }
    }
}

/// One line of `eval.jsonl`: a query, its gold relevant set, and the class it
/// belongs to (`paraphrase` / `identifier` / `cross-section` — see the
/// fixture's README).
#[derive(Debug, Clone, serde::Deserialize)]
pub struct EvalCase {
    /// The search string, verbatim.
    pub query: String,
    /// Every entry that counts as relevant. Never empty in a valid file.
    pub expected: Vec<EntryRef>,
    /// The class this query exercises; scores are reported per class because
    /// the classes are testing different things.
    pub class: String,
}

/// Parse `eval.jsonl` — one JSON object per line, blank lines skipped. A bad
/// line names itself, since a silently-dropped query would inflate every score.
pub fn parse_jsonl(raw: &str) -> Result<Vec<EvalCase>, String> {
    let mut cases = Vec::new();
    for (i, line) in raw.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let case: EvalCase =
            serde_json::from_str(line).map_err(|e| format!("eval.jsonl line {}: {e}", i + 1))?;
        if case.expected.is_empty() {
            return Err(format!(
                "eval.jsonl line {}: `{}` expects nothing — a query with no gold set \
                 can only ever score zero",
                i + 1,
                case.query
            ));
        }
        cases.push(case);
    }
    if cases.is_empty() {
        return Err("eval.jsonl has no cases".to_string());
    }
    Ok(cases)
}

/// Scores for one group of queries (the whole set, or one class).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Metrics {
    /// How many queries went into these numbers.
    pub queries: usize,
    /// Mean per-query recall at [`RECALL_AT`]: of a query's gold entries, the
    /// fraction that appear in its top 5. A query with three expected entries
    /// and two of them in the top 5 scores 2/3 — partial credit, because the
    /// cross-section cases are deliberately multi-entry.
    pub recall_at_5: f32,
    /// Mean reciprocal rank of the **first** relevant hit, over the whole
    /// returned ranking (the runner asks every mode for the same depth, so the
    /// three columns stay comparable). No relevant hit at all scores 0.
    pub mrr: f32,
}

/// Overall + per-class [`Metrics`] for one retrieval mode.
#[derive(Debug, Clone, Default)]
pub struct Report {
    /// Every query in the set.
    pub overall: Metrics,
    /// Keyed by [`EvalCase::class`]; `BTreeMap` so the table's row order is
    /// stable across runs.
    pub by_class: BTreeMap<String, Metrics>,
}

/// One query's `(recall@5, reciprocal rank)` against its gold set.
fn case_scores(expected: &[EntryRef], ranked: &[EntryRef]) -> (f32, f32) {
    if expected.is_empty() {
        return (0.0, 0.0);
    }
    let found = expected
        .iter()
        .filter(|want| ranked.iter().take(RECALL_AT).any(|got| got == *want))
        .count();
    let recall = found as f32 / expected.len() as f32;
    let rr = ranked
        .iter()
        .position(|got| expected.contains(got))
        .map_or(0.0, |i| 1.0 / (i + 1) as f32);
    (recall, rr)
}

/// Mean of the accumulated per-query scores.
fn mean(sum_recall: f32, sum_rr: f32, queries: usize) -> Metrics {
    if queries == 0 {
        return Metrics::default();
    }
    Metrics {
        queries,
        recall_at_5: sum_recall / queries as f32,
        mrr: sum_rr / queries as f32,
    }
}

/// Run `search` over every case and score it. `search` returns one mode's
/// ranking for a query, best-first — the runner supplies it, so this module
/// never needs an embedder, an index, or a well.
pub fn score<F>(cases: &[EvalCase], mut search: F) -> Report
where
    F: FnMut(&EvalCase) -> Vec<EntryRef>,
{
    // (recall sum, rr sum, count) accumulators.
    let mut overall = (0.0f32, 0.0f32, 0usize);
    let mut classes: BTreeMap<String, (f32, f32, usize)> = BTreeMap::new();

    for case in cases {
        let ranked = search(case);
        let (recall, rr) = case_scores(&case.expected, &ranked);
        overall.0 += recall;
        overall.1 += rr;
        overall.2 += 1;
        let bucket = classes.entry(case.class.clone()).or_default();
        bucket.0 += recall;
        bucket.1 += rr;
        bucket.2 += 1;
    }

    Report {
        overall: mean(overall.0, overall.1, overall.2),
        by_class: classes
            .into_iter()
            .map(|(class, (r, rr, n))| (class, mean(r, rr, n)))
            .collect(),
    }
}

/// The class §6.7 singles out: hybrid must not make exact-token retrieval
/// worse than keyword-only.
pub const IDENTIFIER_CLASS: &str = "identifier";

/// Float slack for a "must not regress" comparison. Recall is a mean of exact
/// rationals, so equality is real — this only absorbs f32 division noise.
const EPSILON: f32 = 1e-6;

/// The §6.7 verdict on one eval run.
#[derive(Debug, Clone)]
pub struct Gate {
    /// Whether hybrid earned its place.
    pub passed: bool,
    /// One line per check, pass or fail — a gate that only speaks when it fails
    /// makes a passing run unreadable.
    pub findings: Vec<String>,
}

/// Apply §6.7's gate: hybrid's overall recall@5 must be at least keyword's, and
/// its `identifier`-class recall@5 must not regress.
///
/// "At least", not "strictly greater": RRF only reorders what the two halves
/// already retrieved, so on a query set where keyword already finds everything
/// in the top 5, parity is the ceiling, not a failure. What would be a failure
/// is fusion *losing* something one half had.
pub fn gate(keyword: &Report, hybrid: &Report) -> Gate {
    let mut findings = Vec::new();
    let mut passed = true;

    let (k, h) = (keyword.overall.recall_at_5, hybrid.overall.recall_at_5);
    let ok = h + EPSILON >= k;
    passed &= ok;
    findings.push(format!(
        "{} overall recall@{RECALL_AT}: hybrid {h:.3} vs keyword {k:.3}",
        if ok { "PASS" } else { "FAIL" }
    ));

    let ki = keyword.by_class.get(IDENTIFIER_CLASS).copied();
    let hi = hybrid.by_class.get(IDENTIFIER_CLASS).copied();
    match (ki, hi) {
        (Some(k), Some(h)) => {
            let ok = h.recall_at_5 + EPSILON >= k.recall_at_5;
            passed &= ok;
            findings.push(format!(
                "{} `{IDENTIFIER_CLASS}` recall@{RECALL_AT}: hybrid {:.3} vs keyword {:.3}",
                if ok { "PASS" } else { "FAIL" },
                h.recall_at_5,
                k.recall_at_5
            ));
        }
        _ => {
            passed = false;
            findings.push(format!(
                "FAIL no `{IDENTIFIER_CLASS}` cases in the query set — §6.7's \
                 exact-token guard cannot be checked"
            ));
        }
    }

    Gate { passed, findings }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(kind: &str, id: &str) -> EntryRef {
        EntryRef::new(kind, id)
    }

    fn case(query: &str, class: &str, expected: Vec<EntryRef>) -> EvalCase {
        EvalCase {
            query: query.to_string(),
            expected,
            class: class.to_string(),
        }
    }

    // --- parsing ---------------------------------------------------------

    #[test]
    fn parses_the_fixture_shape() {
        let raw = "\
{\"query\": \"where are credentials\", \"expected\": [{\"kind\": \"note\", \"id\": \"pw\"}], \"class\": \"paraphrase\"}

{\"query\": \"SOW-2026-014\", \"expected\": [{\"kind\": \"note\", \"id\": \"a\"}, {\"kind\": \"task\", \"id\": \"b\"}], \"class\": \"cross-section\"}
";
        let cases = parse_jsonl(raw).unwrap();
        assert_eq!(cases.len(), 2, "blank lines are skipped, not counted");
        assert_eq!(cases[0].class, "paraphrase");
        assert_eq!(cases[1].expected, vec![r("note", "a"), r("task", "b")]);
    }

    #[test]
    fn a_malformed_line_names_itself() {
        let err = parse_jsonl("{\"query\": \"x\"}").unwrap_err();
        assert!(err.contains("line 1"), "got: {err}");
        let err = parse_jsonl("{\"query\": \"x\", \"expected\": [], \"class\": \"paraphrase\"}")
            .unwrap_err();
        assert!(err.contains("expects nothing"), "got: {err}");
        assert!(parse_jsonl("\n\n").is_err(), "an empty set is not a set");
    }

    // --- the metric math -------------------------------------------------

    #[test]
    fn a_hit_at_rank_one_is_perfect() {
        let (recall, rr) = case_scores(&[r("note", "a")], &[r("note", "a"), r("note", "b")]);
        assert_eq!((recall, rr), (1.0, 1.0));
    }

    #[test]
    fn reciprocal_rank_counts_the_first_relevant_hit() {
        let ranked = vec![r("note", "x"), r("note", "y"), r("note", "a")];
        let (recall, rr) = case_scores(&[r("note", "a")], &ranked);
        assert_eq!(recall, 1.0, "rank 3 is still inside the top 5");
        assert!((rr - 1.0 / 3.0).abs() < 1e-6, "got {rr}");
    }

    #[test]
    fn recall_stops_at_the_cutoff_but_mrr_does_not() {
        // The only relevant entry sits at rank 6 — outside recall@5, inside MRR.
        let mut ranked: Vec<EntryRef> = (0..5).map(|i| r("note", &format!("miss{i}"))).collect();
        ranked.push(r("note", "a"));
        let (recall, rr) = case_scores(&[r("note", "a")], &ranked);
        assert_eq!(recall, 0.0);
        assert!((rr - 1.0 / 6.0).abs() < 1e-6, "got {rr}");
    }

    #[test]
    fn a_multi_entry_case_gets_partial_credit() {
        let expected = vec![r("note", "a"), r("task", "b"), r("goal", "c")];
        let ranked = vec![r("note", "a"), r("wiki", "z"), r("task", "b")];
        let (recall, rr) = case_scores(&expected, &ranked);
        assert!((recall - 2.0 / 3.0).abs() < 1e-6, "two of three: {recall}");
        assert_eq!(rr, 1.0);
    }

    #[test]
    fn kind_is_part_of_identity() {
        // Same id, different kind — not a match.
        let (recall, rr) = case_scores(&[r("note", "a")], &[r("wiki", "a")]);
        assert_eq!((recall, rr), (0.0, 0.0));
    }

    #[test]
    fn nothing_retrieved_scores_zero_rather_than_dividing_by_zero() {
        let (recall, rr) = case_scores(&[r("note", "a")], &[]);
        assert_eq!((recall, rr), (0.0, 0.0));
        assert_eq!(case_scores(&[], &[r("note", "a")]), (0.0, 0.0));
    }

    // --- aggregation -----------------------------------------------------

    #[test]
    fn scores_average_over_queries_and_split_by_class() {
        let cases = vec![
            case("q1", "paraphrase", vec![r("note", "a")]),
            case("q2", "paraphrase", vec![r("note", "b")]),
            case("q3", "identifier", vec![r("note", "c")]),
        ];
        // q1 hits at rank 1, q2 misses entirely, q3 hits at rank 2.
        let report = score(&cases, |c| match c.query.as_str() {
            "q1" => vec![r("note", "a")],
            "q2" => vec![r("note", "zzz")],
            _ => vec![r("note", "zzz"), r("note", "c")],
        });

        assert_eq!(report.overall.queries, 3);
        assert!((report.overall.recall_at_5 - 2.0 / 3.0).abs() < 1e-6);
        // (1 + 0 + 0.5) / 3
        assert!(
            (report.overall.mrr - 0.5).abs() < 1e-6,
            "{}",
            report.overall.mrr
        );

        let para = report.by_class["paraphrase"];
        assert_eq!(para.queries, 2);
        assert!((para.recall_at_5 - 0.5).abs() < 1e-6);
        let ident = report.by_class["identifier"];
        assert_eq!(ident.queries, 1);
        assert!((ident.mrr - 0.5).abs() < 1e-6);
    }

    // --- the gate --------------------------------------------------------

    fn report(overall: f32, identifier: f32) -> Report {
        let m = |r: f32| Metrics {
            queries: 1,
            recall_at_5: r,
            mrr: r,
        };
        Report {
            overall: m(overall),
            by_class: [(IDENTIFIER_CLASS.to_string(), m(identifier))]
                .into_iter()
                .collect(),
        }
    }

    #[test]
    fn gate_passes_when_hybrid_wins_or_ties() {
        assert!(gate(&report(0.6, 1.0), &report(0.8, 1.0)).passed, "wins");
        assert!(
            gate(&report(0.6, 1.0), &report(0.6, 1.0)).passed,
            "parity is allowed — RRF can only reorder what was retrieved"
        );
    }

    #[test]
    fn gate_fails_on_an_overall_or_identifier_regression() {
        let overall = gate(&report(0.8, 1.0), &report(0.7, 1.0));
        assert!(!overall.passed);
        assert!(overall.findings.iter().any(|f| f.starts_with("FAIL")));

        let ident = gate(&report(0.6, 1.0), &report(0.9, 0.8));
        assert!(
            !ident.passed,
            "a paraphrase win can't buy an identifier loss"
        );
        assert!(
            ident.findings.iter().any(|f| f.contains(IDENTIFIER_CLASS)),
            "{:?}",
            ident.findings
        );
    }

    #[test]
    fn gate_fails_when_the_identifier_class_is_missing_entirely() {
        let empty = Report::default();
        let verdict = gate(&empty, &empty);
        assert!(!verdict.passed, "an unguardable gate is not a passing gate");
    }

    #[test]
    fn every_finding_is_reported_pass_or_fail() {
        let verdict = gate(&report(0.6, 1.0), &report(0.8, 1.0));
        assert_eq!(verdict.findings.len(), 2, "{:?}", verdict.findings);
        assert!(verdict.findings.iter().all(|f| f.starts_with("PASS")));
    }
}
