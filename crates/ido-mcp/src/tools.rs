//! The seven read-only tools: their parameter shapes and their bodies.
//!
//! Every body is a plain function over `&str well` returning
//! `Result<String, String>` — markdown on success, a human-readable message on
//! failure. Nothing here knows about MCP: [`crate::server`] is the only module
//! that touches `rmcp`, so an SDK bump is a one-file change (see
//! `docs/mcp-server.md` §4.2).
//!
//! Every read goes through `ido_store`, so there is never a second, drifting
//! copy of "how a task file is parsed" — and **nothing here writes**: no
//! `migrate_well`, no `migrate_well` scaffold, no store mutation of any kind.
//! The one thing the process may write is the rebuildable search cache under
//! `<well>/.ido/index/`, and that happens in [`crate::semantic`], never here.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use ido_store::index::hybrid::{self, SearchMode};
use ido_store::model::{Section, Task, TreeNode};
use ido_store::{notes, tasks, wells, wiki};
use rmcp::schemars;

use crate::render::{
    cell, clamp_limit, content_block, dash, guard_id, header, iso_date, paging, row, window,
};
use crate::semantic::Semantic;

/// How deep the retrieval stack reaches before `section` scoping and `limit`
/// trim it. Matches the store's own cap, so scoping to one section can't starve
/// a result list that a broader query would have filled.
const MAX_SCAN: usize = 50;

// --- parameters -------------------------------------------------------------
//
// Doc comments become the JSON-Schema `description` of each field, so they are
// the model's only guide to what a parameter accepts — write them for that
// reader.

/// Parameters for [`search`].
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct SearchParams {
    /// What to look for. In the default "hybrid" mode a natural-language
    /// question works as well as literal words; in "keyword" mode this is a
    /// case-insensitive substring match over titles, bodies and task tags.
    pub query: String,
    /// Restrict to one section: "notes", "wiki", or "tasks". Omit to search all three.
    pub section: Option<String>,
    /// How to retrieve: "hybrid" (default — meaning and exact words fused),
    /// "semantic" (meaning only; finds a note that never uses your words), or
    /// "keyword" (exact substrings only; best for an identifier, a slug, a
    /// spelling). If this well has no semantic index, every mode answers with
    /// keyword results and the response says so.
    pub mode: Option<String>,
    /// Max hits to return (default 10, hard cap 50).
    pub limit: Option<u32>,
}

impl SearchParams {
    /// The requested retrieval mode, defaulting to hybrid (§5.2). An
    /// unrecognised value is an error, not a silent fallback.
    pub fn mode(&self) -> Result<SearchMode, String> {
        match self
            .mode
            .as_deref()
            .map(str::trim)
            .filter(|m| !m.is_empty())
        {
            Some(mode) => SearchMode::parse(mode),
            None => Ok(SearchMode::Hybrid),
        }
    }
}

/// Parameters for [`get_entry`].
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GetEntryParams {
    /// Which kind of entry: "note", "wiki", "task", or "goal". Use the `kind`
    /// exactly as `search` / `list_entries` reported it.
    pub kind: String,
    /// The entry's id, exactly as `search` / `list_entries` reported it: a
    /// note's well-relative path without `.md`, a wiki page's slug, or a
    /// task/goal's slug.
    pub id: String,
    /// Max characters of the body to return (default 8000, hard cap 40000).
    pub max_chars: Option<u32>,
    /// Character offset to start the body at (default 0) — page through a long
    /// entry by passing the offset the previous call's truncation note gave you.
    pub offset: Option<u32>,
}

/// Parameters for [`list_entries`].
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ListEntriesParams {
    /// Which section to list: "notes", "wiki", or "tasks" (tasks lists tasks and goals).
    pub section: String,
    /// Notes only: restrict to one folder, by its well-relative path (e.g.
    /// "projects/auth"). Omit for the whole section.
    pub prefix: Option<String>,
    /// Max entries to return (default 100, hard cap 500).
    pub limit: Option<u32>,
    /// How many entries to skip (default 0) — the ordering is stable, so paging is coherent.
    pub offset: Option<u32>,
}

/// Parameters for [`backlinks`].
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct BacklinksParams {
    /// The wiki page's slug (its id, not its title).
    pub slug: String,
}

/// Parameters for [`list_tasks`].
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ListTasksParams {
    /// Only tasks in this board column (a column id from `well_info`), or
    /// "backlog" for tasks not in any column. Omit for every column.
    pub status: Option<String>,
    /// Only tasks carrying this tag (case-insensitive, exact tag match).
    pub tag: Option<String>,
    /// Only tasks linked to this goal, by goal id.
    pub goal: Option<String>,
    /// Only tasks due on or before this date, "YYYY-MM-DD". Undated tasks are excluded.
    pub due_before: Option<String>,
    /// Only tasks due on or after this date, "YYYY-MM-DD". Undated tasks are excluded.
    pub due_after: Option<String>,
    /// Include archived tasks (default false).
    pub include_archived: Option<bool>,
    /// Max tasks to return (default 100, hard cap 200).
    pub limit: Option<u32>,
}

/// Parameters for [`list_goals`].
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ListGoalsParams {
    /// Include archived goals (default false).
    pub include_archived: Option<bool>,
}

// --- shared helpers ---------------------------------------------------------

/// Which kind of entry an id names. Mirrors the `kind` strings the store's
/// `SearchHit` / `LinkRef` use, so ids round-trip verbatim.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Note,
    Wiki,
    Task,
    Goal,
}

impl Kind {
    /// Parse the `kind` a tool was handed, rejecting anything unknown.
    fn parse(kind: &str) -> Result<Self, String> {
        match kind.trim().to_lowercase().as_str() {
            "note" => Ok(Kind::Note),
            "wiki" => Ok(Kind::Wiki),
            "task" => Ok(Kind::Task),
            "goal" => Ok(Kind::Goal),
            other => Err(format!(
                "unknown kind `{other}` — use one of: note, wiki, task, goal"
            )),
        }
    }

    /// The wire name, as `search` / `list_entries` report it.
    fn as_str(self) -> &'static str {
        match self {
            Kind::Note => "note",
            Kind::Wiki => "wiki",
            Kind::Task => "task",
            Kind::Goal => "goal",
        }
    }
}

/// Parse a `section` argument into the store's own [`Section`].
fn parse_section(section: &str) -> Result<Section, String> {
    match section.trim().to_lowercase().as_str() {
        "notes" => Ok(Section::Notes),
        "wiki" => Ok(Section::Wiki),
        "tasks" => Ok(Section::Tasks),
        other => Err(format!(
            "unknown section `{other}` — use one of: notes, wiki, tasks"
        )),
    }
}

/// The 10-char date prefix of a `due` value — a due may carry an optional
/// " HH:MM", and every date comparison in ido works off the prefix.
fn due_date(due: &str) -> &str {
    due.get(..10).unwrap_or(due)
}

/// Validate a caller-supplied `YYYY-MM-DD` filter argument, returning the date
/// itself. Anything else is a tool error rather than a silently-empty result.
fn parse_date_arg(what: &str, value: &str) -> Result<String, String> {
    let date: String = value.trim().chars().take(10).collect();
    let shaped = date.chars().count() == 10
        && date.char_indices().all(|(i, c)| match i {
            4 | 7 => c == '-',
            _ => c.is_ascii_digit(),
        });
    if shaped {
        Ok(date)
    } else {
        Err(format!(
            "{what} must be a date like 2026-08-25 (got `{value}`)"
        ))
    }
}

/// Count the non-folder leaves of a tree.
fn count_files(nodes: &[TreeNode]) -> usize {
    nodes
        .iter()
        .map(|n| {
            if n.is_dir {
                count_files(&n.children)
            } else {
                1
            }
        })
        .sum()
}

/// Flatten a tree into `(id, name)` leaves, depth-first.
fn flatten(nodes: &[TreeNode], out: &mut Vec<(String, String)>) {
    for node in nodes {
        if node.is_dir {
            flatten(&node.children, out);
        } else {
            out.push((node.path.clone(), node.name.clone()));
        }
    }
}

/// Every wiki page as `(slug, wiki-relative path)` — folders are cosmetic, so
/// the slug is the id and the path is only shown for orientation.
fn wiki_pages(well: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    flatten(&wiki::list_wiki(well.to_string()), &mut out);
    // `flatten` yields (path, name); a page's identity is its name (the slug).
    out.into_iter().map(|(path, slug)| (slug, path)).collect()
}

/// A task's checklist rollup (`3/7`), or `—` when it has no checklist.
fn checks(task: &Task) -> String {
    if task.checks_total == 0 {
        "—".to_string()
    } else {
        format!("{}/{}", task.checks_done, task.checks_total)
    }
}

/// A task's column, naming the un-columned case the way the board does.
fn status_of(task: &Task) -> &str {
    if task.status.is_empty() {
        "backlog"
    } else {
        &task.status
    }
}

/// Non-overlapping case-insensitive occurrences of an already-lowercased needle.
fn occurrences(hay: &str, needle: &str) -> usize {
    hay.to_lowercase().matches(needle).count()
}

/// Absolute path of the file backing an entry, for `stat`-only use (size +
/// mtime in [`list_entries`]). Never opened for reading.
fn entry_file(well: &str, kind: Kind, rel: &str) -> PathBuf {
    let root = Path::new(well);
    match kind {
        Kind::Note => root.join("notes").join(format!("{rel}.md")),
        Kind::Wiki => root.join("wiki").join(format!("{rel}.md")),
        Kind::Task => root.join("tasks").join(format!("{rel}.md")),
        Kind::Goal => root.join("tasks").join("goals").join(format!("{rel}.md")),
    }
}

/// A file's `(size in bytes, modified date)` — both `—` when it can't be stat'd.
fn file_stats(path: &Path) -> (String, String) {
    let Ok(meta) = std::fs::metadata(path) else {
        return ("—".to_string(), "—".to_string());
    };
    let modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64);
    (meta.len().to_string(), iso_date(modified))
}

// --- 1. well_info -----------------------------------------------------------

/// Orientation: what this well is, how its board is shaped, how much is in it,
/// and which search modes are actually live here (§5.2's index status).
pub fn well_info(well: &str, name: &str, semantic: &Semantic) -> String {
    let manifest = wells::read_manifest(well);
    let columns = tasks::task_columns(well.to_string());
    let all_tasks = tasks::list_tasks(well.to_string());
    let goals = tasks::list_goals(well.to_string());
    let archived_tasks = all_tasks.iter().filter(|t| t.archived).count();
    let archived_goals = goals.iter().filter(|g| g.archived).count();
    let note_count = count_files(&notes::list_tree(well.to_string()).unwrap_or_default());
    let page_count = wiki_pages(well).len();

    let sections = [
        ("notes", manifest.sections.notes),
        ("wiki", manifest.sections.wiki),
        ("tasks", manifest.sections.tasks),
    ];
    let enabled: Vec<&str> = sections
        .iter()
        .filter(|(_, on)| *on)
        .map(|(n, _)| *n)
        .collect();
    let disabled: Vec<&str> = sections
        .iter()
        .filter(|(_, on)| !*on)
        .map(|(n, _)| *n)
        .collect();

    let mut out = format!("# well: {name}\n\n");
    let _ = writeln!(out, "- path: `{well}`");
    let _ = writeln!(out, "- sections enabled: {}", enabled.join(", "));
    if !disabled.is_empty() {
        let _ = writeln!(out, "- sections disabled: {}", disabled.join(", "));
    }
    let _ = writeln!(
        out,
        "- board columns: {} (the last one, `{}`, is the done column)",
        columns
            .iter()
            .map(|c| format!("`{c}`"))
            .collect::<Vec<_>>()
            .join(" → "),
        columns.last().map(String::as_str).unwrap_or("—")
    );
    let _ = writeln!(
        out,
        "- auto-archive done tasks after: {}",
        match manifest.archive_done_after_days {
            Some(days) => format!("{days} days"),
            None => "off".to_string(),
        }
    );
    let _ = writeln!(out, "{}", semantic.status_lines());
    let _ = writeln!(
        out,
        "- access: read-only — this server never creates, edits or deletes any note, page, task \
         or goal{}",
        if semantic.unavailable().is_none() {
            "; the one thing it writes is the rebuildable search cache under `.ido/index`"
        } else {
            " and writes nothing at all"
        }
    );
    if !Path::new(well).join(".ido").join("well.toml").exists() {
        let _ = writeln!(
            out,
            "- note: no `.ido/well.toml` — this folder may not be an ido well; defaults assumed"
        );
    }

    let _ = writeln!(out, "\n## entries");
    let _ = writeln!(out, "- notes: {note_count} files");
    let _ = writeln!(out, "- wiki: {page_count} pages");
    let _ = writeln!(
        out,
        "- tasks: {} active, {archived_tasks} archived",
        all_tasks.len() - archived_tasks
    );
    let _ = writeln!(
        out,
        "- goals: {} active, {archived_goals} archived",
        goals.len() - archived_goals
    );
    let _ = writeln!(
        out,
        "\nStart with `search` for a topic, or `list_entries` to see what exists."
    );
    out
}

// --- 2. search --------------------------------------------------------------

/// Hybrid / semantic / keyword search across notes, wiki pages, and tasks.
///
/// The mode that actually ran is reported in the heading, and a request that
/// had to fall back says why in the line under it (§6.1: degradation is never
/// silent — a model reading "hybrid" over keyword-only results would draw the
/// wrong conclusion from an empty answer).
pub fn search(well: &str, semantic: &Semantic, p: SearchParams) -> Result<String, String> {
    let query = p.query.trim().to_string();
    if query.is_empty() {
        return Err("query is empty — pass the words you expect to appear".to_string());
    }
    let requested = p.mode()?;
    let wanted = match p
        .section
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(section) => Some(match parse_section(section)? {
            Section::Notes => "note",
            Section::Wiki => "wiki",
            Section::Tasks => "task",
        }),
        None => None,
    };
    let limit = clamp_limit(p.limit, 10, 50);

    let outcome = hybrid::search_with(well, &query, requested, MAX_SCAN, semantic.embedder());
    let effective = outcome.mode;
    // The store knows *that* the index couldn't answer; this process often
    // knows the better *why* (no model on this machine, no semantic build), so
    // prefer it and fall back to the store's reason.
    let degraded = outcome
        .degraded
        .map(|store_reason| semantic.unavailable().unwrap_or(store_reason));

    let matching: Vec<_> = outcome
        .hits
        .into_iter()
        .filter(|h| wanted.is_none_or(|k| h.kind == k))
        .collect();
    let scanned = matching.len();
    let hits: Vec<_> = matching.into_iter().take(limit).collect();

    let needle = query.to_lowercase();
    // Only fetched when a task hit needs its body/tags counted.
    let mut task_rows: Option<Vec<Task>> = None;

    let scope = wanted.map_or_else(|| "all sections".to_string(), |k| format!("{k}s only"));
    let mut out = format!(
        "# search: \"{query}\" — {} hit(s), {scope} ({} search)\n\n",
        hits.len(),
        effective.as_str()
    );
    if let Some(reason) = &degraded {
        let _ = writeln!(
            out,
            "semantic index unavailable: {reason} — keyword results.\n"
        );
    }
    if hits.is_empty() {
        let _ = writeln!(out, "{}", nothing_matched(effective));
        return Ok(out);
    }

    for (i, hit) in hits.iter().enumerate() {
        let body = match hit.kind.as_str() {
            "note" => notes::read_note(well.to_string(), hit.id.clone()).unwrap_or_default(),
            "wiki" => wiki::read_page(well.to_string(), hit.id.clone()),
            _ => {
                let rows = task_rows.get_or_insert_with(|| tasks::list_tasks(well.to_string()));
                rows.iter()
                    .find(|t| t.id == hit.id)
                    .map(|t| format!("{}\n{}", t.body, t.tags.join(", ")))
                    .unwrap_or_default()
            }
        };
        let _ = writeln!(
            out,
            "{}. **{}** `{}` — {}",
            i + 1,
            hit.kind,
            hit.id,
            hit.title
        );
        if !hit.snippet.is_empty() {
            let _ = writeln!(out, "   > {}", hit.snippet);
        }
        let _ = writeln!(out, "   {}\n", why_it_matched(hit, &body, &needle));
    }

    let _ = writeln!(
        out,
        "Ids round-trip: pass a hit's `kind` and `id` straight to `get_entry` for the full text."
    );
    if scanned > hits.len() {
        let _ = writeln!(
            out,
            "{scanned} hit(s) matched in total; raise `limit` (cap 50) or narrow the query. \
             Several small, specific searches beat one broad one."
        );
    }
    Ok(out)
}

/// The advice to give when a search came back empty — which differs by mode:
/// under keyword the fix is usually a different literal word, under
/// semantic/hybrid it is usually that the well genuinely doesn't hold this.
fn nothing_matched(mode: SearchMode) -> &'static str {
    match mode {
        SearchMode::Keyword => {
            "Nothing matched. This is substring matching, not semantic: try a shorter or more \
             literal term, a different spelling, or `mode=\"hybrid\"` to search by meaning too. \
             `list_entries` shows what the well actually contains."
        }
        _ => {
            "Nothing matched, by wording or by meaning. Try a different angle on the topic, or \
             `list_entries` to see what the well actually contains — it may simply not be here."
        }
    }
}

/// One line explaining why a hit is in the list. The store ranks a title match
/// above any number of body matches, and a semantic hit may contain the query's
/// words nowhere at all — saying which is which keeps "0 occurrence(s)" at rank
/// 1 from reading like a bug.
fn why_it_matched(hit: &ido_store::model::SearchHit, body: &str, needle: &str) -> String {
    if hit.title.to_lowercase().contains(needle) {
        let count = occurrences(body, needle);
        return format!("title match; {count} occurrence(s) in the body");
    }
    match occurrences(body, needle) {
        0 => "no literal match — retrieved by meaning".to_string(),
        count => format!("{count} occurrence(s) in the body"),
    }
}

// --- 3. get_entry -----------------------------------------------------------

/// One entry's full markdown body plus its metadata.
pub fn get_entry(well: &str, p: GetEntryParams) -> Result<String, String> {
    let kind = Kind::parse(&p.kind)?;
    let id = p.id.trim().to_string();
    guard_id("id", &id)?;
    let max_chars = clamp_limit(p.max_chars, 8_000, 40_000);
    let offset = p.offset.unwrap_or(0) as usize;

    let (mut head, body) = match kind {
        Kind::Note => note_entry(well, &id)?,
        Kind::Wiki => wiki_entry(well, &id)?,
        Kind::Task => task_entry(well, &id)?,
        Kind::Goal => goal_entry(well, &id)?,
    };

    let view = window(&body, offset, max_chars);
    let _ = writeln!(head, "- length: {} characters\n", view.total);
    let _ = write!(head, "{}{}", content_block(&view.text), view.note());
    Ok(head)
}

/// The `# heading` + metadata block and body of a note.
fn note_entry(well: &str, id: &str) -> Result<(String, String), String> {
    let body = notes::read_note(well.to_string(), id.to_string())
        .map_err(|e| format!("no note `{id}` ({e}) — use `list_entries` with section=notes"))?;
    let meta = notes::note_meta(well.to_string(), id.to_string()).ok();
    let folder = id.rsplit_once('/').map(|(p, _)| p).unwrap_or("(root)");
    let mut head = format!("# note `{id}`\n\n");
    let _ = writeln!(head, "- kind: note");
    let _ = writeln!(head, "- folder: {folder}");
    let _ = writeln!(
        head,
        "- created: {}",
        iso_date(meta.as_ref().and_then(|m| m.created))
    );
    let _ = writeln!(
        head,
        "- modified: {}",
        iso_date(meta.as_ref().and_then(|m| m.modified))
    );
    Ok((head, body))
}

/// The metadata block and body of a wiki page (plus its inbound-link count).
fn wiki_entry(well: &str, slug: &str) -> Result<(String, String), String> {
    let pages = wiki_pages(well);
    let Some((_, path)) = pages.iter().find(|(s, _)| s == slug) else {
        return Err(format!(
            "no wiki page `{slug}` — use `list_entries` with section=wiki to see the slugs"
        ));
    };
    let folder = path.rsplit_once('/').map(|(p, _)| p).unwrap_or("(root)");
    let inbound = wiki::backlinks(well.to_string(), slug.to_string()).len();
    let body = wiki::read_page(well.to_string(), slug.to_string());
    let mut head = format!("# wiki `{slug}`\n\n");
    let _ = writeln!(head, "- kind: wiki");
    let _ = writeln!(
        head,
        "- folder: {folder} (cosmetic — the slug is the identity)"
    );
    let _ = writeln!(
        head,
        "- inbound links: {inbound} (call `backlinks` with slug=`{slug}` for the list)"
    );
    Ok((head, body))
}

/// The metadata block and body of a task.
fn task_entry(well: &str, id: &str) -> Result<(String, String), String> {
    let rows = tasks::list_tasks(well.to_string());
    let Some(task) = rows.into_iter().find(|t| t.id == id) else {
        return Err(format!(
            "no task `{id}` — use `list_tasks` to see the ids in this well"
        ));
    };
    let mut head = format!("# task `{id}` — {}\n\n", task.title);
    let _ = writeln!(head, "- kind: task");
    let _ = writeln!(head, "- title: {}", task.title);
    let _ = writeln!(head, "- status: {}", status_of(&task));
    let _ = writeln!(head, "- priority: {}", dash(&task.priority));
    let _ = writeln!(
        head,
        "- tags: {}",
        if task.tags.is_empty() {
            "—".to_string()
        } else {
            task.tags.join(", ")
        }
    );
    let _ = writeln!(head, "- due: {}", dash(&task.due));
    let _ = writeln!(head, "- goal: {}", dash(&task.goal));
    let _ = writeln!(head, "- repeat: {}", dash(&task.repeat));
    let _ = writeln!(head, "- completed: {}", dash(&task.completed));
    let _ = writeln!(
        head,
        "- archived: {}",
        if task.archived { "yes" } else { "no" }
    );
    let _ = writeln!(head, "- checklist: {}", checks(&task));
    Ok((head, task.body))
}

/// The metadata block and body of a goal (with its derived task progress).
fn goal_entry(well: &str, id: &str) -> Result<(String, String), String> {
    let Some(goal) = tasks::list_goals(well.to_string())
        .into_iter()
        .find(|g| g.id == id)
    else {
        return Err(format!(
            "no goal `{id}` — use `list_goals` to see the ids in this well"
        ));
    };
    let (done, total) = goal_progress(well, id);
    let mut head = format!("# goal `{id}` — {}\n\n", goal.title);
    let _ = writeln!(head, "- kind: goal");
    let _ = writeln!(head, "- title: {}", goal.title);
    let _ = writeln!(head, "- target: {}", dash(&goal.target));
    let _ = writeln!(
        head,
        "- archived: {}",
        if goal.archived { "yes" } else { "no" }
    );
    let _ = writeln!(head, "- progress: {done}/{total} tasks done");
    Ok((head, goal.body))
}

/// A goal's derived progress — `(done, total)` over the **non-archived** tasks
/// pointing at it, "done" meaning the last board column. Mirrors the app's
/// goals bar exactly (`components/tasks/goals.rs`); progress is never stored.
fn goal_progress(well: &str, goal: &str) -> (usize, usize) {
    let columns = tasks::task_columns(well.to_string());
    let done_col = columns.last().cloned().unwrap_or_default();
    let rows = tasks::list_tasks(well.to_string());
    let linked: Vec<_> = rows
        .iter()
        .filter(|t| t.goal == goal && !t.archived)
        .collect();
    let done = linked.iter().filter(|t| t.status == done_col).count();
    (done, linked.len())
}

// --- 4. list_entries --------------------------------------------------------

/// One row of [`list_entries`]. `id` is what `get_entry` takes; `rel` is the
/// path the backing file actually sits at (they differ for a wiki page, whose
/// slug is its identity but whose file may live in a cosmetic folder).
struct Listing {
    kind: Kind,
    id: String,
    title: String,
    rel: String,
}

/// The cheap map of a section: id, title, modified date, size.
pub fn list_entries(well: &str, p: ListEntriesParams) -> Result<String, String> {
    let section = parse_section(&p.section)?;
    let prefix = p.prefix.as_deref().map(str::trim).unwrap_or("");
    if !prefix.is_empty() {
        if !matches!(section, Section::Notes) {
            return Err(
                "`prefix` only applies to section=notes (wiki folders are cosmetic, \
                        and tasks are flat)"
                    .to_string(),
            );
        }
        guard_id("prefix", prefix)?;
    }
    let limit = clamp_limit(p.limit, 100, 500);
    let offset = p.offset.unwrap_or(0) as usize;

    // Stable order (by id, per group) so `offset` paging stays coherent.
    let mut rows: Vec<Listing> = Vec::new();
    match section {
        Section::Notes => {
            let mut leaves = Vec::new();
            flatten(&notes::list_tree(well.to_string())?, &mut leaves);
            leaves.sort();
            let scope = format!("{}/", prefix.trim_end_matches('/'));
            rows.extend(leaves.into_iter().filter_map(|(id, name)| {
                (prefix.is_empty() || id.starts_with(&scope)).then(|| Listing {
                    kind: Kind::Note,
                    rel: id.clone(),
                    id,
                    title: name,
                })
            }));
        }
        Section::Wiki => {
            let mut pages = wiki_pages(well);
            pages.sort();
            rows.extend(pages.into_iter().map(|(slug, path)| Listing {
                kind: Kind::Wiki,
                title: format!(
                    "{slug} (in {})",
                    path.rsplit_once('/').map_or(".", |(p, _)| p)
                ),
                id: slug,
                rel: path,
            }));
        }
        Section::Tasks => {
            let mut task_rows: Vec<_> = tasks::list_tasks(well.to_string())
                .into_iter()
                .map(|t| Listing {
                    kind: Kind::Task,
                    rel: t.id.clone(),
                    id: t.id,
                    title: t.title,
                })
                .collect();
            task_rows.sort_by(|a, b| a.id.cmp(&b.id));
            let mut goal_rows: Vec<_> = tasks::list_goals(well.to_string())
                .into_iter()
                .map(|g| Listing {
                    kind: Kind::Goal,
                    rel: g.id.clone(),
                    id: g.id,
                    title: g.title,
                })
                .collect();
            goal_rows.sort_by(|a, b| a.id.cmp(&b.id));
            rows.extend(task_rows);
            rows.extend(goal_rows);
        }
    }

    let total = rows.len();
    let page: Vec<_> = rows.into_iter().skip(offset).take(limit).collect();

    let scope = if prefix.is_empty() {
        String::new()
    } else {
        format!(" under `{prefix}`")
    };
    let mut out = format!("# {}{scope} — {total} entr(ies)\n\n", p.section.trim());
    if page.is_empty() {
        let _ = writeln!(
            out,
            "Nothing here{}.",
            if offset > 0 {
                " at that offset"
            } else {
                " yet"
            }
        );
        return Ok(out);
    }
    header(&mut out, &["kind", "id", "title", "modified", "bytes"]);
    for entry in &page {
        let (bytes, modified) = file_stats(&entry_file(well, entry.kind, &entry.rel));
        row(
            &mut out,
            &[
                cell(entry.kind.as_str()),
                format!("`{}`", cell(&entry.id)),
                cell(&entry.title),
                cell(&modified),
                cell(&bytes),
            ],
        );
    }
    let _ = writeln!(out, "{}", paging(page.len(), offset, total));
    let _ = writeln!(
        out,
        "Pass any row's `kind` + `id` to `get_entry` for its full text."
    );
    Ok(out)
}

// --- 5. backlinks -----------------------------------------------------------

/// Everything that links to a wiki page, across every section.
pub fn backlinks(well: &str, p: BacklinksParams) -> Result<String, String> {
    let slug = p.slug.trim().to_string();
    guard_id("slug", &slug)?;
    let exists = wiki_pages(well).iter().any(|(s, _)| *s == slug);
    let refs = wiki::backlinks(well.to_string(), slug.clone());

    let mut out = format!("# backlinks → `{slug}` — {} referrer(s)\n\n", refs.len());
    if !exists {
        let _ = writeln!(
            out,
            "(No wiki page `{slug}` exists yet — these are links written ahead of the page.)\n"
        );
    }
    if refs.is_empty() {
        let _ = writeln!(
            out,
            "Nothing links here. `[[{slug}]]` references in notes, wiki pages, and task/goal \
             bodies would all show up in this list."
        );
        return Ok(out);
    }
    for link in &refs {
        let _ = writeln!(out, "- **{}** `{}` — {}", link.kind, link.id, link.title);
    }
    let _ = writeln!(
        out,
        "\nPass any row's `kind` + `id` to `get_entry` to read the referrer."
    );
    Ok(out)
}

// --- 6. list_tasks ----------------------------------------------------------

/// The board, filtered.
pub fn list_tasks(well: &str, p: ListTasksParams) -> Result<String, String> {
    let before = p
        .due_before
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .map(|d| parse_date_arg("due_before", d))
        .transpose()?;
    let after = p
        .due_after
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .map(|d| parse_date_arg("due_after", d))
        .transpose()?;
    let status = p.status.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let tag = p
        .tag
        .as_deref()
        .map(|t| t.trim().to_lowercase())
        .filter(|t| !t.is_empty());
    let goal = p.goal.as_deref().map(str::trim).filter(|g| !g.is_empty());
    let include_archived = p.include_archived.unwrap_or(false);
    let limit = clamp_limit(p.limit, 100, 200);

    let columns = tasks::task_columns(well.to_string());
    let matching: Vec<Task> = tasks::list_tasks(well.to_string())
        .into_iter()
        .filter(|t| include_archived || !t.archived)
        .filter(|t| status.is_none_or(|s| status_of(t).eq_ignore_ascii_case(s)))
        .filter(|t| {
            tag.as_deref()
                .is_none_or(|want| t.tags.iter().any(|have| have.to_lowercase() == want))
        })
        .filter(|t| goal.is_none_or(|g| t.goal == g))
        .filter(|t| {
            before
                .as_deref()
                .is_none_or(|d| !t.due.is_empty() && due_date(&t.due) <= d)
        })
        .filter(|t| {
            after
                .as_deref()
                .is_none_or(|d| !t.due.is_empty() && due_date(&t.due) >= d)
        })
        .collect();

    let total = matching.len();
    let page: Vec<_> = matching.into_iter().take(limit).collect();

    let mut out = format!("# tasks — {total} match(es)\n\n");
    let _ = writeln!(
        out,
        "Columns: {} (last = done). Archived tasks are {}.\n",
        columns.join(" → "),
        if include_archived {
            "included"
        } else {
            "excluded"
        }
    );
    if page.is_empty() {
        let _ = writeln!(out, "No task matches those filters.");
        if let Some(s) = status
            && !s.eq_ignore_ascii_case("backlog")
            && !columns.iter().any(|c| c.eq_ignore_ascii_case(s))
        {
            let _ = writeln!(
                out,
                "`status={s}` is not one of this well's columns — try one of {} or \"backlog\".",
                columns.join(", ")
            );
        }
        return Ok(out);
    }
    header(
        &mut out,
        &[
            "id",
            "title",
            "status",
            "priority",
            "tags",
            "due",
            "goal",
            "checks",
            "completed",
        ],
    );
    for task in &page {
        let status = if task.archived {
            format!("{} (archived)", status_of(task))
        } else {
            status_of(task).to_string()
        };
        row(
            &mut out,
            &[
                format!("`{}`", cell(&task.id)),
                cell(&task.title),
                cell(&status),
                cell(&task.priority),
                cell(&task.tags.join(", ")),
                cell(&task.due),
                cell(&task.goal),
                cell(&checks(task)),
                cell(&task.completed),
            ],
        );
    }
    if page.len() < total {
        let _ = writeln!(
            out,
            "\nShowing the first {} of {total}; raise `limit` (cap 200) or filter harder.",
            page.len()
        );
    }
    let _ = writeln!(
        out,
        "\nCall `get_entry` with kind=task and a row's id for that task's full body."
    );
    Ok(out)
}

// --- 7. list_goals ----------------------------------------------------------

/// The goals (milestones), each with progress derived from its linked tasks.
pub fn list_goals(well: &str, p: ListGoalsParams) -> Result<String, String> {
    let include_archived = p.include_archived.unwrap_or(false);
    let columns = tasks::task_columns(well.to_string());
    let done_col = columns.last().cloned().unwrap_or_default();
    let rows = tasks::list_tasks(well.to_string());
    let goals: Vec<_> = tasks::list_goals(well.to_string())
        .into_iter()
        .filter(|g| include_archived || !g.archived)
        .collect();

    let mut out = format!("# goals — {} goal(s)\n\n", goals.len());
    let _ = writeln!(
        out,
        "Progress counts non-archived tasks pointing at the goal; \"done\" means the last board \
         column (`{}`). Archived goals are {}.\n",
        dash(&done_col),
        if include_archived {
            "included"
        } else {
            "excluded"
        }
    );
    if goals.is_empty() {
        let _ = writeln!(out, "This well has no goals.");
        return Ok(out);
    }
    header(&mut out, &["id", "title", "target", "progress", "archived"]);
    for goal in &goals {
        let linked: Vec<_> = rows
            .iter()
            .filter(|t| t.goal == goal.id && !t.archived)
            .collect();
        let done = linked.iter().filter(|t| t.status == done_col).count();
        row(
            &mut out,
            &[
                format!("`{}`", cell(&goal.id)),
                cell(&goal.title),
                cell(&goal.target),
                format!("{done}/{}", linked.len()),
                cell(if goal.archived { "yes" } else { "no" }),
            ],
        );
    }
    let _ = writeln!(
        out,
        "\nCall `list_tasks` with goal=<id> for a goal's tasks, or `get_entry` with kind=goal \
         for its description."
    );
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_and_sections_reject_junk() {
        assert!(Kind::parse("note").is_ok());
        assert_eq!(Kind::parse(" Wiki ").unwrap().as_str(), "wiki");
        assert!(Kind::parse("notes").is_err(), "section name, not a kind");
        assert!(parse_section("notes").is_ok());
        assert!(parse_section("note").is_err(), "kind name, not a section");
    }

    #[test]
    fn date_args_take_the_ten_char_prefix() {
        assert_eq!(
            parse_date_arg("due_before", "2026-08-25").unwrap(),
            "2026-08-25"
        );
        assert_eq!(
            parse_date_arg("due_before", "2026-08-25 14:30").unwrap(),
            "2026-08-25"
        );
        assert!(parse_date_arg("due_before", "next tuesday").is_err());
        assert!(parse_date_arg("due_before", "2026-8-5").is_err());
    }

    #[test]
    fn due_comparisons_ignore_the_time() {
        assert_eq!(due_date("2026-08-25 14:30"), "2026-08-25");
        assert_eq!(due_date("2026-08-25"), "2026-08-25");
        assert_eq!(due_date(""), "");
    }
}
