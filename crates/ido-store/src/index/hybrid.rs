//! The three retrieval modes and the fusion that joins them
//! (docs/mcp-server.md §6.1, §6.5).
//!
//! One entry point — [`search_with`] — answers in whichever of
//! [`SearchMode::Keyword`] / [`SearchMode::Semantic`] / [`SearchMode::Hybrid`]
//! is asked for, over the same [`SearchHit`] shape the lexical scan already
//! returns, so every caller (the MCP `search` tool now, the `Ctrl+K` palette in
//! P3) renders one result type.
//!
//! Two rules shape everything here:
//!
//! - **The two halves must see the same corpus.** [`crate::search`] excludes
//!   archived tasks and recurses wiki folders; [`super::store::collect_chunks`]
//!   does both too. A fused list is only meaningful if neither half can surface
//!   something the other would refuse to.
//! - **Degradation is reported, never silent** (§6.1). A mode that cannot run —
//!   no embedder loaded, no index built, an index from a different model — falls
//!   back to keyword and says why in [`SearchOutcome::degraded`]. It is never an
//!   error: keyword results with a note beat an empty error every time.
//!
//! Fusion is [`super::fusion::rrf`] over two **entry-level** rankings keyed
//! `"kind:id"`. The vector half ranks *chunks*, so it is deduped to entries
//! first, each entry keeping its best-scoring chunk — otherwise a long note
//! would occupy five of the fused list's slots with five views of itself.

use std::collections::{HashMap, HashSet};

use crate::model::SearchHit;
use crate::search::{self, SNIPPET_CHARS, truncate_chars};

use super::embed::{Embedder, Role};
use super::fusion::rrf;
use super::store::load_index;

/// Chunks the vector half pulls before entry-level dedupe (§6.5's "top-50 from
/// each list", counted in the unit the index actually ranks).
const SEMANTIC_CHUNK_K: usize = 50;

/// How deep each ranked list reaches into fusion. §6.5's constant; the lexical
/// scan already caps itself at the same number.
const FUSE_TOP_K: usize = 50;

/// Which retrieval stack answers a query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchMode {
    /// The lexical scan alone ([`crate::search::search`]) — exact substrings,
    /// always available, no index and no model needed.
    Keyword,
    /// The vector index alone — meaning over vocabulary, and blind to an
    /// identifier it has never seen in a similar context.
    Semantic,
    /// Both, fused with RRF. The default: neither half is sufficient alone.
    Hybrid,
}

impl SearchMode {
    /// Parse a caller-supplied mode name. Unknown values are an error rather
    /// than a silent fallback — a typo'd `mode` should say so, not quietly
    /// change what the caller asked for.
    pub fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_lowercase().as_str() {
            "keyword" | "lexical" => Ok(SearchMode::Keyword),
            "semantic" | "vector" => Ok(SearchMode::Semantic),
            "hybrid" => Ok(SearchMode::Hybrid),
            other => Err(format!(
                "unknown search mode `{other}` — use one of: hybrid, semantic, keyword"
            )),
        }
    }

    /// The wire name, as [`SearchMode::parse`] accepts it.
    pub fn as_str(self) -> &'static str {
        match self {
            SearchMode::Keyword => "keyword",
            SearchMode::Semantic => "semantic",
            SearchMode::Hybrid => "hybrid",
        }
    }
}

/// The result of a [`search_with`] call: the hits, **the mode that actually
/// ran**, and why it isn't the one that was asked for.
///
/// The effective mode is part of the answer rather than an afterthought: a
/// caller rendering "hybrid search" over keyword-only results is lying to the
/// model that reads them (§6.1).
pub struct SearchOutcome {
    /// Ranked best-first, at most the caller's `limit`.
    pub hits: Vec<SearchHit>,
    /// The mode that produced `hits` — equal to the requested mode unless
    /// `degraded` says otherwise.
    pub mode: SearchMode,
    /// Why the requested mode could not run. `None` when nothing degraded.
    pub degraded: Option<String>,
}

/// Search `well` in `mode`, returning at most `limit` hits.
///
/// `embedder` is the loaded model, or `None` when this build/machine has none
/// (no `semantic` feature, model not downloaded). A semantic or hybrid request
/// that cannot be served degrades to keyword and reports it; a keyword request
/// never touches the index or the model at all.
///
/// An empty query retrieves nothing, so no mode actually runs — the requested
/// mode comes back unchanged and undegraded.
pub fn search_with(
    well: &str,
    query: &str,
    mode: SearchMode,
    limit: usize,
    embedder: Option<&dyn Embedder>,
) -> SearchOutcome {
    let query = query.trim();
    if query.is_empty() {
        return SearchOutcome {
            hits: Vec::new(),
            mode,
            degraded: None,
        };
    }
    if mode == SearchMode::Keyword {
        return keyword_only(well, query, limit, None);
    }

    let semantic = match semantic_hits(well, query, embedder) {
        Ok(hits) => hits,
        Err(reason) => return keyword_only(well, query, limit, Some(reason)),
    };

    match mode {
        SearchMode::Semantic => SearchOutcome {
            hits: semantic.into_iter().take(limit).collect(),
            mode,
            degraded: None,
        },
        // Keyword is handled above; only Hybrid reaches here.
        _ => SearchOutcome {
            hits: fuse(
                search::search(well.to_string(), query.to_string()),
                semantic,
                limit,
            ),
            mode: SearchMode::Hybrid,
            degraded: None,
        },
    }
}

/// The lexical scan, bounded — and carrying `degraded` when it is standing in
/// for a mode that couldn't run.
fn keyword_only(well: &str, query: &str, limit: usize, degraded: Option<String>) -> SearchOutcome {
    SearchOutcome {
        hits: search::search(well.to_string(), query.to_string())
            .into_iter()
            .take(limit)
            .collect(),
        mode: SearchMode::Keyword,
        degraded,
    }
}

/// The fusion key: `kind` and `id` together are an entry's identity (a note and
/// a wiki page can share a name, and both halves report the same pair).
fn key(hit: &SearchHit) -> String {
    format!("{}:{}", hit.kind, hit.id)
}

/// RRF over the two entry-level rankings. A hit present in the lexical list
/// keeps **its** snippet (a literal match excerpt beats a chunk excerpt when
/// there is a literal match to show); a semantic-only hit keeps the chunk's.
fn fuse(lexical: Vec<SearchHit>, semantic: Vec<SearchHit>, limit: usize) -> Vec<SearchHit> {
    let lex_keys: Vec<String> = lexical.iter().take(FUSE_TOP_K).map(key).collect();
    let sem_keys: Vec<String> = semantic.iter().take(FUSE_TOP_K).map(key).collect();

    // Semantic first, lexical second: the later insert wins, so a lexical hit's
    // snippet replaces the semantic one for an entry both halves found.
    let mut by_key: HashMap<String, SearchHit> = HashMap::new();
    for hit in semantic.into_iter().chain(lexical) {
        by_key.insert(key(&hit), hit);
    }

    rrf(&[lex_keys, sem_keys])
        .into_iter()
        .filter_map(|(key, _score)| by_key.remove(&key))
        .take(limit)
        .collect()
}

/// The vector half: embed the query, sweep the index, dedupe chunk hits down to
/// entries (best chunk per entry — [`super::store::VectorIndex::query`] returns
/// best-first, so the first chunk seen for an entry *is* its best).
///
/// Every `Err` here is a degradation reason a human can act on, not a failure:
/// the caller turns it into keyword results plus an explanation.
fn semantic_hits(
    well: &str,
    query: &str,
    embedder: Option<&dyn Embedder>,
) -> Result<Vec<SearchHit>, String> {
    let embedder = embedder.ok_or("no embedding model is loaded")?;
    let index = load_index(well).ok_or("no semantic index has been built for this well")?;
    if index.manifest.model != embedder.id() {
        return Err(format!(
            "the index was built by `{}` but the loaded model is `{}` — the vectors are in \
             different spaces until it is rebuilt",
            index.manifest.model,
            embedder.id()
        ));
    }
    if index.manifest.dim != embedder.dim() {
        return Err(format!(
            "the index is {}-dim but the loaded model is {}-dim — it needs a rebuild",
            index.manifest.dim,
            embedder.dim()
        ));
    }

    let vector = embedder
        .embed(&[query.to_string()], Role::Query)?
        .into_iter()
        .next()
        .ok_or("the embedder returned no vector for the query")?;

    let mut seen: HashSet<(String, String)> = HashSet::new();
    let mut out = Vec::new();
    for (row, _score) in index.query(&vector, SEMANTIC_CHUNK_K) {
        let chunk = index.chunk(row);
        if !seen.insert((chunk.kind.clone(), chunk.entry_id.clone())) {
            continue; // a lower-scoring chunk of an entry we already have
        }
        out.push(SearchHit {
            kind: chunk.kind.clone(),
            id: chunk.entry_id.clone(),
            title: chunk_title(chunk),
            snippet: chunk_snippet(chunk),
        });
    }
    Ok(out)
}

/// An entry's display title, recovered from the chunk that matched.
/// `heading_path` is `"Entry title › H2 › H3"` by construction
/// ([`super::chunk`]), so its root **is** the title the lexical half reports —
/// which is what keeps one entry from showing two different names depending on
/// which half found it.
fn chunk_title(chunk: &super::chunk::Chunk) -> String {
    let root = chunk
        .heading_path
        .split(" › ")
        .next()
        .unwrap_or_default()
        .trim();
    if root.is_empty() {
        chunk.entry_id.clone()
    } else {
        root.to_string()
    }
}

/// A snippet for a semantic hit: the heading trail below the entry title (so an
/// isolated chunk says *where* in the entry it came from), then the chunk's
/// first real line, cut to [`SNIPPET_CHARS`] like every lexical snippet.
fn chunk_snippet(chunk: &super::chunk::Chunk) -> String {
    let excerpt = chunk
        .text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default();
    let excerpt = truncate_chars(excerpt, SNIPPET_CHARS);
    match chunk.heading_path.split_once(" › ") {
        Some((_title, trail)) if !trail.trim().is_empty() => format!("{trail} — {excerpt}"),
        _ => excerpt,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::embed::FakeEmbedder;
    use crate::index::store::update_index;
    use std::fs;
    use std::path::Path;
    use tempfile::{TempDir, tempdir};

    /// A well with the three section folders scaffolded (mirrors `search.rs`'s
    /// and `index::store`'s helpers).
    fn well() -> (TempDir, String) {
        let dir = tempdir().unwrap();
        for s in ["notes", "wiki", "tasks"] {
            fs::create_dir_all(dir.path().join(s)).unwrap();
        }
        let path = dir.path().to_string_lossy().into_owned();
        (dir, path)
    }

    fn write(well: &str, rel: &str, body: &str) {
        let path = Path::new(well).join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
    }

    fn ids(outcome: &SearchOutcome) -> Vec<&str> {
        outcome.hits.iter().map(|h| h.id.as_str()).collect()
    }

    /// The corpus every retrieval test below shares:
    ///
    /// - `credentials` — reachable only by *meaning* from "keychain storage":
    ///   the body says "storage in the keychain", so a substring search for the
    ///   whole phrase misses while bag-of-words overlap wins.
    /// - `frame-strip` — carries an exact identifier no embedder would relate
    ///   to anything.
    /// - `board` — an unrelated distractor.
    fn seeded() -> (TempDir, String) {
        let (dir, w) = well();
        write(
            &w,
            "notes/credentials.md",
            "# Credentials\n\nWe settled on storage in the keychain, per platform.\n",
        );
        write(
            &w,
            "notes/frame-strip.md",
            "# Frame strip\n\nSerial WSBC-4417-982 stamped under the bottom bracket.\n",
        );
        write(
            &w,
            "notes/board.md",
            "# Board\n\nThe kanban drag indicator uses card midpoints.\n",
        );
        update_index(&w, &FakeEmbedder::default()).unwrap();
        (dir, w)
    }

    // --- mode parsing ---------------------------------------------------

    #[test]
    fn modes_parse_and_round_trip() {
        assert_eq!(SearchMode::parse("hybrid").unwrap(), SearchMode::Hybrid);
        assert_eq!(
            SearchMode::parse(" Semantic ").unwrap(),
            SearchMode::Semantic
        );
        assert_eq!(SearchMode::parse("KEYWORD").unwrap(), SearchMode::Keyword);
        assert!(
            SearchMode::parse("fuzzy").is_err(),
            "a typo is not a default"
        );
        for mode in [
            SearchMode::Keyword,
            SearchMode::Semantic,
            SearchMode::Hybrid,
        ] {
            assert_eq!(SearchMode::parse(mode.as_str()).unwrap(), mode);
        }
    }

    // --- what each half is for ------------------------------------------

    #[test]
    fn semantic_finds_what_substring_matching_cannot() {
        let (_d, w) = seeded();
        let fake = FakeEmbedder::default();
        let query = "keychain storage";

        let keyword = search_with(&w, query, SearchMode::Keyword, 10, Some(&fake));
        assert!(
            !ids(&keyword).contains(&"credentials"),
            "the phrase never appears literally — keyword must miss it: {:?}",
            ids(&keyword)
        );

        let semantic = search_with(&w, query, SearchMode::Semantic, 10, Some(&fake));
        assert_eq!(semantic.mode, SearchMode::Semantic);
        assert!(semantic.degraded.is_none());
        assert_eq!(
            semantic.hits.first().map(|h| h.id.as_str()),
            Some("credentials"),
            "shared vocabulary, different word order: {:?}",
            ids(&semantic)
        );
    }

    #[test]
    fn keyword_still_nails_an_exact_identifier() {
        let (_d, w) = seeded();
        let fake = FakeEmbedder::default();
        let hits = search_with(&w, "WSBC-4417-982", SearchMode::Keyword, 10, Some(&fake));
        assert_eq!(ids(&hits), vec!["frame-strip"]);
    }

    #[test]
    fn hybrid_carries_both_halves_findings() {
        let (_d, w) = seeded();
        let fake = FakeEmbedder::default();

        let fused = search_with(&w, "keychain storage", SearchMode::Hybrid, 10, Some(&fake));
        assert_eq!(fused.mode, SearchMode::Hybrid);
        assert!(fused.degraded.is_none());
        assert!(
            ids(&fused).contains(&"credentials"),
            "the semantic-only hit must survive fusion: {:?}",
            ids(&fused)
        );

        let ident = search_with(&w, "WSBC-4417-982", SearchMode::Hybrid, 10, Some(&fake));
        assert_eq!(
            ident.hits.first().map(|h| h.id.as_str()),
            Some("frame-strip"),
            "and the lexical-only hit must not be demoted: {:?}",
            ids(&ident)
        );
    }

    #[test]
    fn a_lexical_hit_keeps_its_own_snippet_after_fusion() {
        let (_d, w) = seeded();
        let fake = FakeEmbedder::default();
        let fused = search_with(&w, "midpoints", SearchMode::Hybrid, 10, Some(&fake));
        let hit = fused
            .hits
            .iter()
            .find(|h| h.id == "board")
            .expect("the distractor is the literal match here");
        assert!(
            hit.snippet.contains("midpoints"),
            "the match excerpt beats a chunk excerpt when there is one: {}",
            hit.snippet
        );
    }

    #[test]
    fn a_semantic_hit_carries_its_heading_trail() {
        let (_d, w) = well();
        write(
            &w,
            "notes/auth.md",
            "# Auth\n\n## Session tokens\n\nrotated hourly by the daemon\n",
        );
        update_index(&w, &FakeEmbedder::default()).unwrap();
        let fake = FakeEmbedder::default();
        let hits = search_with(
            &w,
            "rotated hourly daemon",
            SearchMode::Semantic,
            5,
            Some(&fake),
        );
        let hit = hits.hits.first().expect("one note, one match");
        assert_eq!(hit.kind, "note");
        assert_eq!(hit.id, "auth");
        assert_eq!(hit.title, "auth", "the entry title, not the heading path");
        assert!(
            hit.snippet.starts_with("Auth › Session tokens — "),
            "an isolated chunk says where it came from: {}",
            hit.snippet
        );
    }

    // --- the corpus contract --------------------------------------------

    #[test]
    fn archived_tasks_never_surface_in_any_mode() {
        let (_d, w) = well();
        let id = crate::tasks::create_task(
            w.clone(),
            "todo".into(),
            "Recalibrate the oscilloscope".into(),
            None,
        )
        .unwrap();
        crate::tasks::update_task_body(
            w.clone(),
            id.clone(),
            "Tektronix graticule and beam-finder trimmer pots.\n".into(),
        )
        .unwrap();
        crate::tasks::set_task_field(w.clone(), id, "archived".into(), "true".into()).unwrap();
        update_index(&w, &FakeEmbedder::default()).unwrap();

        let fake = FakeEmbedder::default();
        for mode in [
            SearchMode::Keyword,
            SearchMode::Semantic,
            SearchMode::Hybrid,
        ] {
            for query in ["oscilloscope", "Tektronix graticule beam-finder"] {
                let out = search_with(&w, query, mode, 10, Some(&fake));
                assert!(
                    out.hits.is_empty(),
                    "{mode:?} leaked an archived task for `{query}`: {:?}",
                    ids(&out)
                );
            }
        }
    }

    #[test]
    fn both_halves_see_foldered_wiki_pages() {
        let (_d, w) = well();
        write(
            &w,
            "wiki/concepts/hydration.md",
            "Dough hydration is water weight over flour weight.\n",
        );
        update_index(&w, &FakeEmbedder::default()).unwrap();
        let fake = FakeEmbedder::default();
        for mode in [
            SearchMode::Keyword,
            SearchMode::Semantic,
            SearchMode::Hybrid,
        ] {
            let out = search_with(&w, "hydration", mode, 10, Some(&fake));
            assert_eq!(ids(&out), vec!["hydration"], "{mode:?} missed it");
            assert_eq!(out.hits[0].kind, "wiki");
        }
    }

    // --- degradation ------------------------------------------------------

    #[test]
    fn no_index_degrades_to_keyword_and_says_so() {
        let (_d, w) = well();
        write(&w, "notes/board.md", "the kanban drag indicator");
        let fake = FakeEmbedder::default();

        for mode in [SearchMode::Semantic, SearchMode::Hybrid] {
            let out = search_with(&w, "kanban", mode, 10, Some(&fake));
            assert_eq!(out.mode, SearchMode::Keyword, "degraded, not errored");
            let reason = out
                .degraded
                .clone()
                .expect("degradation is reported, never silent");
            assert!(reason.contains("no semantic index"), "got: {reason}");
            assert_eq!(ids(&out), vec!["board"], "and still answers");
        }
    }

    #[test]
    fn no_embedder_degrades_to_keyword_and_says_so() {
        let (_d, w) = seeded();
        let out = search_with(&w, "kanban", SearchMode::Hybrid, 10, None);
        assert_eq!(out.mode, SearchMode::Keyword);
        assert!(
            out.degraded
                .as_deref()
                .unwrap_or_default()
                .contains("model"),
            "the reason must name the missing piece: {:?}",
            out.degraded
        );
    }

    #[test]
    fn an_index_from_another_model_degrades_rather_than_mixing_spaces() {
        let (_d, w) = seeded(); // built with `fake-bow-v1`

        struct OtherModel(FakeEmbedder);
        impl Embedder for OtherModel {
            fn id(&self) -> &str {
                "some-other-model"
            }
            fn dim(&self) -> usize {
                self.0.dim()
            }
            fn embed(&self, texts: &[String], role: Role) -> Result<Vec<Vec<f32>>, String> {
                self.0.embed(texts, role)
            }
        }

        let out = search_with(
            &w,
            "keychain",
            SearchMode::Semantic,
            10,
            Some(&OtherModel(FakeEmbedder::default())),
        );
        assert_eq!(out.mode, SearchMode::Keyword);
        let reason = out.degraded.expect("a model mismatch is a degradation");
        assert!(reason.contains("different spaces"), "got: {reason}");
    }

    #[test]
    fn keyword_mode_never_touches_the_index_or_the_model() {
        // No index, no embedder — a keyword request must be entirely unbothered.
        let (_d, w) = well();
        write(&w, "notes/board.md", "the kanban drag indicator");
        let out = search_with(&w, "kanban", SearchMode::Keyword, 10, None);
        assert_eq!(out.mode, SearchMode::Keyword);
        assert!(out.degraded.is_none(), "nothing was asked for that failed");
        assert_eq!(ids(&out), vec!["board"]);
    }

    #[test]
    fn limit_bounds_every_mode() {
        let (_d, w) = seeded();
        let fake = FakeEmbedder::default();
        for mode in [
            SearchMode::Keyword,
            SearchMode::Semantic,
            SearchMode::Hybrid,
        ] {
            let out = search_with(&w, "the", mode, 2, Some(&fake));
            assert!(out.hits.len() <= 2, "{mode:?} returned {}", out.hits.len());
        }
    }

    #[test]
    fn an_empty_query_retrieves_nothing_in_every_mode() {
        let (_d, w) = seeded();
        let fake = FakeEmbedder::default();
        for mode in [
            SearchMode::Keyword,
            SearchMode::Semantic,
            SearchMode::Hybrid,
        ] {
            let out = search_with(&w, "   ", mode, 10, Some(&fake));
            assert!(out.hits.is_empty());
            assert!(out.degraded.is_none());
        }
    }
}
