//! MCP resources (§5.3): the well's entries made addressable by `ido://` uri,
//! so a client can jump straight to one it already has an id for — a `search`
//! hit, a `list_entries`/`list_tasks` row — without a tool round-trip.
//!
//! Four templates, one per [`Kind`]: `ido://note/{path}`, `ido://wiki/{slug}`,
//! `ido://task/{id}`, `ido://goal/{id}`. [`parse_uri`] is the only place a uri
//! is taken apart, and it hands back exactly the `(kind, id)` pair
//! [`GetEntryParams`] takes — `resources/read` ([`crate::server`]) then calls
//! [`tools::get_entry`] with it, **the same renderer `get_entry` the tool
//! uses**, so a resource read and a tool call of the same id return
//! byte-identical text (`docs/mcp-server.md` §5.3 says to do this literally;
//! there is no second renderer here).
//!
//! `resources/list` is deliberately **bounded** ([`LIST_LIMIT`]) rather than
//! walking the whole well — see the constant's own doc for why. Nothing here
//! writes: like every read tool, this only reads through `ido_store`.

use std::path::Path;

use ido_store::{notes, tasks};
use rmcp::model::{Resource, ResourceTemplate};

use crate::render::{guard_id, iso_date};
use crate::tools::{self, Kind};

/// The scheme every resource uri in this server uses.
const SCHEME: &str = "ido://";

/// How many entries `resources/list` returns: the N most-recently-modified
/// across all four kinds, never the whole well. A well can hold thousands of
/// notes; listing all of them on what is meant to be a cheap orientation call
/// would blow a client's context before a single body was read (§5.3's own
/// example: "a 5,000-note well would blow the client's context on a list
/// call"). 50 is generous for "what have I touched lately" — an agent that
/// needs more than that is doing a survey, which is exactly what `search` and
/// `list_entries` (unbounded-by-kind, paged) are for.
pub(crate) const LIST_LIMIT: usize = 50;

/// `resources/templates/list`'s cache TTL (SEP-2549, §4.1). The four
/// templates are fixed for the life of the process — they describe the *uri
/// shape* of each kind, not any well content — so there is nothing for a
/// client to miss by holding this an hour; it is a conservative floor, not a
/// measured number.
pub(crate) const TEMPLATES_TTL_MS: u64 = 60 * 60 * 1000;

/// `resources/list`'s cache TTL. Its ranking only moves when a file's mtime
/// moves, which happens on "someone edited a note" timescales, not "an agent
/// called this tool again" timescales. A minute means a client that lists,
/// reads a few entries, and lists again a moment later doesn't re-walk the
/// well, while still picking up an edit made a few calls ago on the next
/// listing.
pub(crate) const LIST_TTL_MS: u64 = 60 * 1000;

/// `resources/read`'s cache TTL. Unlike the two lists above this is an
/// entry's actual body — the one thing in this well that can change out from
/// under a client at literally any moment: the app, another agent, or (in
/// `--allow-write` mode) this very server's own write tools (§6.6). So this
/// stays near zero rather than merely "short": 5s only smooths over a client
/// re-reading the same uri twice within one turn, never a real staleness
/// window.
pub(crate) const READ_TTL_MS: u64 = 5 * 1000;

/// Parse an `ido://<kind>/<id>` uri back into the pair [`GetEntryParams`][ge]
/// takes. Rejects an unrecognised scheme, a missing id, an unknown kind (via
/// [`Kind::parse`]), and — because a uri is just another way to send a path —
/// runs the id through [`guard_id`] exactly as every other tool does (§9).
///
/// [ge]: crate::tools::GetEntryParams
pub(crate) fn parse_uri(uri: &str) -> Result<(Kind, String), String> {
    let rest = uri.strip_prefix(SCHEME).ok_or_else(|| {
        format!(
            "`{uri}` is not an ido resource uri — expected `ido://<kind>/<id>` (kind is note, \
             wiki, task, or goal)"
        )
    })?;
    let (kind, id) = rest
        .split_once('/')
        .ok_or_else(|| format!("`{uri}` is missing an id — expected `ido://<kind>/<id>`"))?;
    let kind = Kind::parse(kind)?;
    guard_id("id", id)?;
    Ok((kind, id.to_string()))
}

/// The uri for one entry — the exact inverse of [`parse_uri`].
pub(crate) fn format_uri(kind: Kind, id: &str) -> String {
    format!("{SCHEME}{}/{id}", kind.as_str())
}

/// One entry, as gathered for the recency sweep both `resources/list` and
/// [`crate::prompts`] read from — never sent over the wire directly.
struct Listed {
    kind: Kind,
    id: String,
    /// The human title, when the entry has one distinct from its id: a note's
    /// filename, a task/goal's free-text title. A wiki page's slug *is* its
    /// title, so this is `None` there.
    title: Option<String>,
    mtime_ms: Option<u64>,
    size: Option<u64>,
}

/// A file's `(modified, epoch ms; size, bytes)`, both `None` when it can't be
/// stat'd. The raw pair, not a display string: [`all_entries`] needs the
/// numbers themselves to rank by recency and cut off "the N most recent"
/// before anything is formatted for display.
fn mtime_and_size(path: &Path) -> (Option<u64>, Option<u64>) {
    let Ok(meta) = std::fs::metadata(path) else {
        return (None, None);
    };
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64);
    (mtime, Some(meta.len()))
}

/// Every non-archived entry across all four kinds, stat'd once each.
fn all_entries(well: &str) -> Vec<Listed> {
    let mut out = Vec::new();

    let mut notes = Vec::new();
    tools::flatten(
        &notes::list_tree(well.to_string()).unwrap_or_default(),
        &mut notes,
    );
    out.extend(notes.into_iter().map(|(id, name)| {
        let (mtime_ms, size) = mtime_and_size(&tools::entry_file(well, Kind::Note, &id));
        Listed {
            kind: Kind::Note,
            id,
            title: Some(name),
            mtime_ms,
            size,
        }
    }));

    out.extend(tools::wiki_pages(well).into_iter().map(|(slug, path)| {
        let (mtime_ms, size) = mtime_and_size(&tools::entry_file(well, Kind::Wiki, &path));
        Listed {
            kind: Kind::Wiki,
            id: slug,
            title: None,
            mtime_ms,
            size,
        }
    }));

    out.extend(
        tasks::list_tasks(well.to_string())
            .into_iter()
            .filter(|t| !t.archived)
            .map(|t| {
                let (mtime_ms, size) = mtime_and_size(&tools::entry_file(well, Kind::Task, &t.id));
                Listed {
                    kind: Kind::Task,
                    id: t.id,
                    title: Some(t.title),
                    mtime_ms,
                    size,
                }
            }),
    );

    out.extend(
        tasks::list_goals(well.to_string())
            .into_iter()
            .filter(|g| !g.archived)
            .map(|g| {
                let (mtime_ms, size) = mtime_and_size(&tools::entry_file(well, Kind::Goal, &g.id));
                Listed {
                    kind: Kind::Goal,
                    id: g.id,
                    title: Some(g.title),
                    mtime_ms,
                    size,
                }
            }),
    );

    out
}

/// Every entry, most-recently-modified first — an unstat'able file (`None`)
/// sorts last rather than panicking or crashing the ranking.
fn most_recent(well: &str) -> Vec<Listed> {
    let mut all = all_entries(well);
    all.sort_by_key(|e| std::cmp::Reverse(e.mtime_ms));
    all
}

/// `resources/list`'s body: the [`LIST_LIMIT`] most-recently-modified entries.
pub(crate) fn list(well: &str) -> Vec<Resource> {
    most_recent(well)
        .into_iter()
        .take(LIST_LIMIT)
        .map(|e| {
            let kind = e.kind.as_str();
            let modified = iso_date(e.mtime_ms);
            let mut r = Resource::new(format_uri(e.kind, &e.id), e.id.clone())
                .with_mime_type("text/markdown")
                .with_description(format!("{kind} — modified {modified}"));
            if let Some(title) = e.title {
                r = r.with_title(title);
            }
            if let Some(size) = e.size {
                r = r.with_size(size);
            }
            r
        })
        .collect()
}

/// The `(id, title)` of the `limit` most-recently-modified notes — the
/// "recently touched notes" half of [`crate::prompts::daily_review`]. Shares
/// the same recency sweep `resources/list` uses rather than re-walking the
/// notes tree a second way.
pub(crate) fn recent_notes(well: &str, limit: usize) -> Vec<(String, String)> {
    most_recent(well)
        .into_iter()
        .filter(|e| e.kind == Kind::Note)
        .take(limit)
        .map(|e| {
            let title = e.title.clone().unwrap_or_else(|| e.id.clone());
            (e.id, title)
        })
        .collect()
}

/// Every entry modified at or after `since_ms` (epoch millis), most-recent
/// first and capped at `limit` — the "what changed" half of
/// [`crate::prompts::weekly_digest`].
pub(crate) fn recently_modified(
    well: &str,
    since_ms: u64,
    limit: usize,
) -> Vec<(Kind, String, String, u64)> {
    most_recent(well)
        .into_iter()
        .filter(|e| e.mtime_ms.is_some_and(|m| m >= since_ms))
        .take(limit)
        .map(|e| {
            let mtime = e.mtime_ms.unwrap_or_default();
            let title = e.title.unwrap_or_else(|| e.id.clone());
            (e.kind, e.id, title, mtime)
        })
        .collect()
}

/// The four resource templates (§5.3), each with a description written for a
/// model deciding whether a uri it already has (from a search hit, a task
/// row) is worth reading directly instead of calling a tool.
pub(crate) fn templates() -> Vec<ResourceTemplate> {
    [
        (
            "ido://note/{path}",
            "note",
            "A note, addressed by its well-relative path without `.md` — the same id `search` \
             and `list_entries` report for section=notes. Notes are foldered for real (unlike \
             wiki), so the path may have multiple `/`-separated segments, e.g. \
             `ido://note/projects/auth-notes`.",
        ),
        (
            "ido://wiki/{slug}",
            "wiki",
            "A wiki page, addressed by its slug — globally unique across the section and \
             independent of whatever cosmetic folder the file happens to sit in. e.g. \
             `ido://wiki/auth-system`.",
        ),
        (
            "ido://task/{id}",
            "task",
            "A task on the kanban board, addressed by its id (the title's slug), from a \
             `list_tasks` row or a `search` hit. e.g. `ido://task/rotate-the-signing-keys`.",
        ),
        (
            "ido://goal/{id}",
            "goal",
            "A goal (milestone), addressed by its id, from a `list_goals` row or a `search` hit. \
             e.g. `ido://goal/q3-launch`.",
        ),
    ]
    .into_iter()
    .map(|(uri, name, description)| {
        ResourceTemplate::new(uri, name)
            .with_description(description)
            .with_mime_type("text/markdown")
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_round_trips_for_every_kind() {
        for (kind, id) in [
            (Kind::Note, "projects/auth-notes"),
            (Kind::Wiki, "auth-system"),
            (Kind::Task, "rotate-the-keys"),
            (Kind::Goal, "q3-launch"),
        ] {
            let uri = format_uri(kind, id);
            assert_eq!(parse_uri(&uri).unwrap(), (kind, id.to_string()), "{uri}");
        }
    }

    #[test]
    fn parse_uri_rejects_a_bad_scheme() {
        for uri in [
            "file:///etc/passwd",
            "http://example.com/note/x",
            "ido:/note/x",
            "note/x",
        ] {
            let err = parse_uri(uri).unwrap_err();
            assert!(err.contains("ido://"), "`{uri}` gave: {err}");
        }
    }

    #[test]
    fn parse_uri_rejects_an_unknown_kind() {
        let err = parse_uri("ido://recipe/x").unwrap_err();
        assert!(err.contains("unknown kind"), "{err}");
    }

    #[test]
    fn parse_uri_rejects_a_missing_id() {
        assert!(parse_uri("ido://note").is_err());
        assert!(parse_uri("ido://note/").is_err());
    }

    #[test]
    fn parse_uri_rejects_traversal_ids() {
        for uri in [
            "ido://note/../../etc/passwd",
            "ido://note/a/../../etc/passwd",
            "ido://note//etc/passwd",
            "ido://note/C:/Windows",
            "ido://wiki/..\\x",
        ] {
            let err = parse_uri(uri).unwrap_err();
            assert!(err.contains("invalid id"), "`{uri}` gave: {err}");
        }
    }

    #[test]
    fn every_template_uri_starts_with_the_scheme_and_names_a_real_kind() {
        for template in templates() {
            assert!(template.uri_template.starts_with(SCHEME));
            assert!(
                Kind::parse(&template.name).is_ok(),
                "template name `{}` isn't one of the four kinds",
                template.name
            );
            assert_eq!(template.mime_type.as_deref(), Some("text/markdown"));
        }
        assert_eq!(templates().len(), 4);
    }
}
