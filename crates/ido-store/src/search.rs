//! Cross-section full-text search over a well.
//!
//! A brute-force scan of every markdown file in `notes/`, `wiki/`, and `tasks/`
//! on each query — there is no persisted index. Wells are small and local, so a
//! scan is fast and keeps `.ido/` free of a cache to invalidate. The same scan
//! is the natural backing for a future MCP `search` tool.

use std::fs;
use std::path::Path;

use crate::frontmatter;
use crate::model::{SearchHit, Section};
use crate::paths::{rel_path, section_dir};

/// Cap on returned results — enough for a palette, bounded for huge wells.
const MAX_HITS: usize = 50;
/// Cap on a snippet's length, in characters. `pub(crate)` so the semantic half
/// ([`crate::index::hybrid`]) can cut its snippets to the same length — a fused
/// result list where the two halves truncate differently reads as a bug.
pub(crate) const SNIPPET_CHARS: usize = 120;

/// Search a well's notes, wiki, and tasks for `query` (case-insensitive,
/// substring). Ranked by relevance — a title match dominates, then the number of
/// occurrences in the body / tags; ties keep section order (notes → wiki →
/// tasks). Archived tasks are excluded. Empty / whitespace queries return nothing.
pub fn search(well: String, query: String) -> Vec<SearchHit> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Vec::new();
    }
    let mut hits: Vec<(i64, SearchHit)> = Vec::new();
    search_notes(&well, &q, &mut hits);
    search_wiki(&well, &q, &mut hits);
    search_tasks(&well, &q, &mut hits);
    // Highest score first; the sort is stable, so equal-score hits keep the
    // section order they were collected in.
    hits.sort_by_key(|h| std::cmp::Reverse(h.0));
    hits.into_iter().take(MAX_HITS).map(|(_, h)| h).collect()
}

/// Count of non-overlapping case-insensitive occurrences of `q` in `hay`.
fn count_matches(hay: &str, q: &str) -> usize {
    hay.to_lowercase().matches(q).count()
}

/// Recursively scan `notes/`. A note's id is its well-relative path (no `.md`).
fn search_notes(well: &str, q: &str, out: &mut Vec<(i64, SearchHit)>) {
    let root = section_dir(well, Section::Notes);
    walk_notes(&root, &root, q, out);
}

fn walk_notes(dir: &Path, root: &Path, q: &str, out: &mut Vec<(i64, SearchHit)>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            walk_notes(&path, root, q, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
            let title = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_string();
            let body = fs::read_to_string(&path).unwrap_or_default();
            let id = rel_path(root, &path.with_extension(""));
            push_hit(out, "note", id, title, &body, "", q);
        }
    }
}

/// Recursively scan the `wiki/` namespace. Folders under `wiki/` are purely
/// cosmetic — a page's identity is its slug (the file stem) wherever the file
/// sits, exactly as `wiki::find_page` resolves it — so the walk recurses but
/// the id never carries the folder.
///
/// The recursion is not optional: a foldered page that only the flat top-level
/// scan could see would be invisible to the palette and to MCP `search`, and
/// the semantic half (`index::store::collect_wiki`) already recurses — the two
/// halves of hybrid retrieval must see the same corpus.
fn search_wiki(well: &str, q: &str, out: &mut Vec<(i64, SearchHit)>) {
    walk_wiki(&section_dir(well, Section::Wiki), q, out);
}

fn walk_wiki(dir: &Path, q: &str, out: &mut Vec<(i64, SearchHit)>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            walk_wiki(&path, q, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
            let Some(slug) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let body = fs::read_to_string(&path).unwrap_or_default();
            push_hit(
                out,
                "wiki",
                slug.to_string(),
                slug.to_string(),
                &body,
                "",
                q,
            );
        }
    }
}

/// Scan the flat top level of `tasks/` (the `goals/` subdir is skipped — it's a
/// directory, not a `.md`). A task's id is its file stem; its display `title:`
/// (falling back to the stem), body, *and* tags are searched. Archived tasks
/// are skipped.
fn search_tasks(well: &str, q: &str, out: &mut Vec<(i64, SearchHit)>) {
    for_each_md(&section_dir(well, Section::Tasks), |stem, src| {
        let (fields, body) = frontmatter::parse(src);
        if fields.get("archived").map(|v| v == "true").unwrap_or(false) {
            return;
        }
        let title = fields
            .get("title")
            .cloned()
            .unwrap_or_else(|| stem.to_string());
        let tags = fields.get("tags").map(String::as_str).unwrap_or("");
        push_hit(out, "task", stem.to_string(), title, &body, tags, q);
    });
}

/// Call `f(file_stem, contents)` for every non-hidden `.md` directly in `dir`.
fn for_each_md(dir: &Path, mut f: impl FnMut(&str, &str)) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        if name.starts_with('.') || path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if let Ok(contents) = fs::read_to_string(&path) {
            f(stem, &contents);
        }
    }
}

/// Push a scored hit when `q` matches the title, body, or `extra` (tags). The
/// score floats title matches to the top, then ranks by occurrence count.
fn push_hit(
    out: &mut Vec<(i64, SearchHit)>,
    kind: &str,
    id: String,
    title: String,
    body: &str,
    extra: &str,
    q: &str,
) {
    let in_title = title.to_lowercase().contains(q);
    let occurrences = count_matches(body, q) + count_matches(extra, q);
    if !in_title && occurrences == 0 {
        return;
    }
    let score = i64::from(in_title) * 1000 + occurrences as i64;
    out.push((
        score,
        SearchHit {
            kind: kind.to_string(),
            id,
            title,
            snippet: snippet(body, q),
        },
    ));
}

/// A short context excerpt: the first body line containing `q`, else the first
/// non-empty line (a title-only match still gets a preview). Truncated to
/// [`SNIPPET_CHARS`] characters (char-safe).
fn snippet(body: &str, q: &str) -> String {
    let line = body
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && l.to_lowercase().contains(q))
        .or_else(|| body.lines().map(str::trim).find(|l| !l.is_empty()))
        .unwrap_or_default();
    truncate_chars(line, SNIPPET_CHARS)
}

/// `text` cut to at most `max` **characters** (never bytes — this runs on
/// arbitrary user prose), with an ellipsis marking the cut. `pub(crate)` so the
/// semantic half builds its snippets the same way.
pub(crate) fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::{TempDir, tempdir};

    /// A well with the three section folders scaffolded.
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

    #[test]
    fn empty_query_returns_nothing() {
        let (_d, w) = well();
        write(&w, "notes/x.md", "hello");
        assert!(search(w, "   ".into()).is_empty());
    }

    #[test]
    fn matches_across_sections_with_ids() {
        let (_d, w) = well();
        write(&w, "notes/folder/auth.md", "# Auth\n\nlogin flow");
        write(&w, "wiki/login.md", "the auth hub");
        write(
            &w,
            "tasks/fix-auth.md",
            "---\nstatus: todo\n---\nbody about auth",
        );
        write(&w, "tasks/goals/m1.md", "ignored milestone auth");

        let hits = search(w, "auth".into());
        let ids: Vec<_> = hits
            .iter()
            .map(|h| (h.kind.as_str(), h.id.as_str()))
            .collect();
        // Note id keeps its folder path; goals/ is not scanned as a task.
        assert!(ids.contains(&("note", "folder/auth")));
        assert!(ids.contains(&("wiki", "login")));
        assert!(ids.contains(&("task", "fix-auth")));
        assert_eq!(hits.len(), 3, "goals/ subdir is skipped");
    }

    #[test]
    fn wiki_pages_in_cosmetic_folders_are_found_by_slug() {
        // Regression: the wiki scan used to read only the flat top level of
        // `wiki/`, so every page in an organisational folder was invisible to
        // the palette and to MCP search — while the semantic half indexed them.
        let (_d, w) = well();
        write(&w, "wiki/flat.md", "a flat page about kombucha");
        write(
            &w,
            "wiki/concepts/nested.md",
            "a nested page about kombucha",
        );
        write(
            &w,
            "wiki/concepts/deeper/very-nested.md",
            "a deeply nested page about kombucha",
        );

        let hits = search(w.clone(), "kombucha".into());
        let ids: Vec<&str> = hits.iter().map(|h| h.id.as_str()).collect();
        assert!(ids.contains(&"flat"));
        assert!(ids.contains(&"nested"), "folders are cosmetic: {ids:?}");
        assert!(ids.contains(&"very-nested"), "and they nest: {ids:?}");
        assert!(
            hits.iter().all(|h| h.kind == "wiki" && !h.id.contains('/')),
            "a page's id is its slug, never its folder path: {ids:?}"
        );

        // The slug is also matchable as a title, wherever the file sits.
        let by_title = search(w, "very-nested".into());
        assert_eq!(by_title.len(), 1);
        assert_eq!(by_title[0].id, "very-nested");
    }

    #[test]
    fn task_display_title_is_searched_and_shown() {
        let (_d, w) = well();
        write(
            &w,
            "tasks/fix-login.md",
            "---\nstatus: todo\ntitle: Fix login: OAuth expiry\n---\nbody",
        );
        let hits = search(w, "oauth".into());
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "fix-login");
        assert_eq!(hits[0].title, "Fix login: OAuth expiry");
    }

    #[test]
    fn skips_archived_and_searches_tags() {
        let (_d, w) = well();
        write(
            &w,
            "tasks/active.md",
            "---\nstatus: todo\ntags: backend, urgent\n---\nplain body",
        );
        write(
            &w,
            "tasks/old.md",
            "---\nstatus: done\narchived: true\n---\nplain body",
        );
        // A tag match surfaces the active task...
        let hits = search(w.clone(), "backend".into());
        assert_eq!(
            hits.iter().map(|h| h.id.as_str()).collect::<Vec<_>>(),
            vec!["active"]
        );
        // ...and the archived task is excluded even on a body match.
        let ids: Vec<String> = search(w, "plain".into())
            .into_iter()
            .map(|h| h.id)
            .collect();
        assert_eq!(ids, vec!["active".to_string()]);
    }

    #[test]
    fn title_matches_rank_first() {
        let (_d, w) = well();
        write(&w, "wiki/onboarding.md", "nothing relevant here");
        write(&w, "wiki/guide.md", "see onboarding steps"); // body-only match
        let hits = search(w, "onboarding".into());
        assert_eq!(hits.len(), 2);
        assert_eq!(
            hits[0].title, "onboarding",
            "title hit sorts before body hit"
        );
    }

    #[test]
    fn snippet_is_the_matching_line() {
        let (_d, w) = well();
        write(
            &w,
            "notes/n.md",
            "intro line\n\nthe MATCH is on this line\ntail",
        );
        let hits = search(w, "match".into());
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].snippet, "the MATCH is on this line");
    }
}
