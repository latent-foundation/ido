//! The two offline commands: `--reindex` and `--eval` (docs/mcp-server.md
//! Appendix B). Both resolve a well, do their work, print to the terminal, and
//! exit — neither ever opens the JSON-RPC transport, which is why they may use
//! stdout freely.
//!
//! Neither is `#[cfg]`-gated. They go through [`Semantic`] like the server
//! does, so a keyword-only build (or a machine without the model) refuses them
//! with the same one honest sentence rather than not existing — a flag that
//! silently isn't there is a worse answer than a flag that says why it can't
//! run.

use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use ido_store::index::embed::Embedder;
use ido_store::index::eval::{self, EntryRef, EvalCase, Metrics, Report};
use ido_store::index::hybrid::{self, SearchMode};
use ido_store::index::store::{INDEX_DIR, update_index};

use crate::render::{cell, header, row};
use crate::semantic::Semantic;

/// Hits each mode is asked for per eval query. Recall is measured at 5;
/// asking for ten keeps MRR meaningful a little past the cutoff while staying
/// the size of answer a model actually reads.
const EVAL_LIMIT: usize = 10;

/// The three modes, scored in the order the table reports them.
const MODES: [SearchMode; 3] = [
    SearchMode::Keyword,
    SearchMode::Semantic,
    SearchMode::Hybrid,
];

/// A failed command: what to print, and what to exit with. Exit 2 means "this
/// build/machine can't do that" (the usage class); exit 1 means "it ran and the
/// answer is no".
pub struct CliError {
    /// The message, printed to stderr under an `ido-mcp:` prefix.
    pub message: String,
    /// Process exit code.
    pub code: i32,
}

impl CliError {
    fn usage(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: crate::EXIT_USAGE,
        }
    }

    fn failed(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: 1,
        }
    }
}

/// The loaded embedder, or a usage error naming exactly what is missing (no
/// `semantic` feature, no downloaded model, a model that wouldn't load).
fn embedder(well: &str) -> Result<&'static dyn Embedder, CliError> {
    let semantic = Semantic::new(well);
    semantic.embedder().ok_or_else(|| {
        CliError::usage(format!(
            "semantic search is unavailable: {}",
            semantic
                .unavailable()
                .unwrap_or_else(|| "the embedding model could not be loaded".to_string())
        ))
    })
}

// --- --reindex ---------------------------------------------------------------

/// Force a **full** rebuild of `<well>/.ido/index/`, then print the resulting
/// status to stderr.
///
/// The forcing move is deleting `manifest.json` — the index's commit point.
/// `update_index` reads it under the advisory lock to decide what to reuse, so
/// with it gone every file re-embeds; `chunks.jsonl` and `vectors.bin` are
/// unreadable without it and are rewritten wholesale a moment later. Deleting
/// only the commit point (rather than all three) also keeps the failure mode
/// small: if the rebuild can't run because another process holds the lock, that
/// process is itself mid-rebuild and will write a complete, consistent trio.
pub fn reindex(well: &Path) -> Result<(), CliError> {
    let well = well.to_string_lossy().into_owned();
    let embedder = embedder(&well)?;

    let manifest = Path::new(&well).join(INDEX_DIR).join("manifest.json");
    if manifest.exists()
        && let Err(e) = fs::remove_file(&manifest)
    {
        return Err(CliError::failed(format!(
            "clearing {}: {e}",
            manifest.display()
        )));
    }

    eprintln!(
        "ido-mcp: rebuilding the index from scratch with `{}`…",
        embedder.id()
    );
    let status = update_index(&well, embedder).map_err(CliError::failed)?;
    eprintln!(
        "ido-mcp: index rebuilt — {} chunks across {} files, {} ({}-dim)",
        status.chunks, status.files, status.model, status.dim
    );
    Ok(())
}

// --- --eval ------------------------------------------------------------------

/// Score keyword / semantic / hybrid retrieval over `path`'s query set against
/// `well`, print the table, and apply §6.7's gate.
///
/// The index is brought up to date first: scoring a stale index measures
/// yesterday's chunker.
pub fn run_eval(well: &Path, path: &Path) -> Result<(), CliError> {
    let well = well.to_string_lossy().into_owned();
    let embedder = embedder(&well)?;

    let raw = fs::read_to_string(path)
        .map_err(|e| CliError::usage(format!("reading {}: {e}", path.display())))?;
    let cases = eval::parse_jsonl(&raw).map_err(CliError::usage)?;

    eprintln!("ido-mcp: bringing the index up to date before scoring…");
    let status = update_index(&well, embedder).map_err(CliError::failed)?;
    eprintln!(
        "ido-mcp: {} chunks across {} files ({})",
        status.chunks, status.files, status.model
    );

    // The fixture's negative control: an archived task's vocabulary is unique
    // to it, so if *any* mode ever returns one, that mode is leaking archived
    // content and the run should fail loudly rather than merely score lower.
    let archived: Vec<String> = ido_store::tasks::list_tasks(well.clone())
        .into_iter()
        .filter(|t| t.archived)
        .map(|t| t.id)
        .collect();
    let mut leaked: Vec<String> = Vec::new();

    let mut reports: Vec<(SearchMode, Report)> = Vec::new();
    for mode in MODES {
        let mut degraded: Option<String> = None;
        let report = eval::score(&cases, |case| {
            let outcome = hybrid::search_with(&well, &case.query, mode, EVAL_LIMIT, Some(embedder));
            if let Some(reason) = outcome.degraded.clone() {
                degraded.get_or_insert(reason);
            }
            for hit in &outcome.hits {
                if hit.kind == "task" && archived.contains(&hit.id) {
                    leaked.push(format!(
                        "{} returned archived task `{}`",
                        mode.as_str(),
                        hit.id
                    ));
                }
            }
            outcome
                .hits
                .iter()
                .map(|hit| EntryRef::new(hit.kind.clone(), hit.id.clone()))
                .collect()
        });
        // A silently-degraded mode scores as keyword and would make the table
        // a lie — refuse rather than report it.
        if let Some(reason) = degraded {
            return Err(CliError::failed(format!(
                "`{}` degraded to keyword during the run ({reason}) — the scores would be \
                 meaningless",
                mode.as_str()
            )));
        }
        reports.push((mode, report));
    }

    print!("{}", table(&cases, &reports));

    if !leaked.is_empty() {
        return Err(CliError::failed(format!(
            "archived content leaked into results — {}",
            leaked.join("; ")
        )));
    }

    let keyword = &reports[0].1;
    let hybrid_report = &reports[2].1;
    let verdict = eval::gate(keyword, hybrid_report);
    println!("\n## gate (§6.7)\n");
    for finding in &verdict.findings {
        println!("- {finding}");
    }
    if verdict.passed {
        println!("\nGATE PASSED — hybrid earns its place.");
        Ok(())
    } else {
        println!(
            "\nGATE FAILED — per §6.7 the chunking or the query set is where to look. \
             Do not tune RRF_K."
        );
        Err(CliError::failed(
            "the retrieval gate failed (see the table above)",
        ))
    }
}

/// `R@5 / MRR`, to three places — enough to see a one-query difference in a
/// 31-query set without pretending to more precision than that.
fn score_cell(m: Option<&Metrics>) -> String {
    match m {
        Some(m) => format!("{:.3} / {:.3}", m.recall_at_5, m.mrr),
        None => "—".to_string(),
    }
}

/// The results table: one row per class plus an overall row, one column per
/// mode. Markdown, so it pastes straight into the design doc.
fn table(cases: &[EvalCase], reports: &[(SearchMode, Report)]) -> String {
    let mut out = format!("\n# retrieval eval — {} queries\n\n", cases.len());
    let _ = writeln!(
        out,
        "Cells are `recall@{} / MRR`, both means over the class's queries.\n",
        eval::RECALL_AT
    );

    let mut columns = vec!["class".to_string(), "queries".to_string()];
    columns.extend(reports.iter().map(|(mode, _)| mode.as_str().to_string()));
    let refs: Vec<&str> = columns.iter().map(String::as_str).collect();
    header(&mut out, &refs);

    // Class order comes from the first report's BTreeMap — stable, and every
    // mode saw the same cases, so the keys agree.
    let classes: Vec<&String> = reports
        .first()
        .map(|(_, r)| r.by_class.keys().collect())
        .unwrap_or_default();
    for class in classes {
        let queries = reports
            .first()
            .and_then(|(_, r)| r.by_class.get(class))
            .map_or(0, |m| m.queries);
        let mut cells = vec![cell(class), cell(&queries.to_string())];
        cells.extend(
            reports
                .iter()
                .map(|(_, r)| score_cell(r.by_class.get(class))),
        );
        row(&mut out, &cells);
    }

    let mut cells = vec!["**overall**".to_string(), cases.len().to_string()];
    cells.extend(reports.iter().map(|(_, r)| score_cell(Some(&r.overall))));
    row(&mut out, &cells);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn metrics(recall: f32, mrr: f32, queries: usize) -> Metrics {
        Metrics {
            queries,
            recall_at_5: recall,
            mrr,
        }
    }

    #[test]
    fn the_table_has_a_column_per_mode_and_a_row_per_class() {
        let cases = eval::parse_jsonl(
            "{\"query\": \"a\", \"expected\": [{\"kind\": \"note\", \"id\": \"x\"}], \
             \"class\": \"paraphrase\"}\n",
        )
        .unwrap();
        let by_class: BTreeMap<String, Metrics> =
            [("paraphrase".to_string(), metrics(0.5, 0.25, 1))]
                .into_iter()
                .collect();
        let report = Report {
            overall: metrics(0.5, 0.25, 1),
            by_class,
        };
        let reports: Vec<(SearchMode, Report)> =
            MODES.iter().map(|m| (*m, report.clone())).collect();

        let rendered = table(&cases, &reports);
        assert!(rendered.contains("| class | queries | keyword | semantic | hybrid |"));
        assert!(
            rendered.contains("| paraphrase | 1 | 0.500 / 0.250"),
            "{rendered}"
        );
        assert!(rendered.contains("**overall**"), "{rendered}");
    }

    #[test]
    fn a_missing_class_renders_as_a_dash_rather_than_a_zero() {
        assert_eq!(score_cell(None), "—", "absent is not the same as scoring 0");
        assert_eq!(score_cell(Some(&metrics(1.0, 1.0, 3))), "1.000 / 1.000");
    }
}
