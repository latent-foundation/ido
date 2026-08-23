//! The four gated write tools: their parameter shapes and their bodies.
//!
//! Shaped exactly like [`crate::tools`] — plain functions over `&str well`
//! returning `Result<String, String>`, with every mutation delegated to
//! `ido_store` so there is never a second copy of "how a task file is written".
//! Nothing here knows about MCP; [`crate::server`] owns the annotations and the
//! descriptions.
//!
//! **The rules these tools are built to (`docs/mcp-server.md` §8):**
//!
//! - **Opt-in at the process level.** Absent `--allow-write` these tools are not
//!   advertised at all — [`crate::server::Ido::open`] never adds their routes.
//!   [`guard_writes`] is the second lock: a client that calls a name it never
//!   saw gets a refusal, not a write.
//! - **Never destructive.** No delete, no truncate, no overwrite of an existing
//!   body. A create that collides uniquifies (the store's own rule), an append
//!   only ever grows a body, and a task edit is one named field.
//! - **Path confinement on every id** — [`guard_write_id`] runs the lexical
//!   guard *and* the store's canonicalised containment check (§9).
//! - **The store's own invariants stay the store's.** `completed:` is stamped by
//!   `apply_status` and `archived:` by the auto-archive sweep, so neither is
//!   writable here ([`Field::parse`] explains why to the caller).

use std::fmt::Write as _;

use ido_store::model::Section;
use ido_store::{notes, paths, repeat, tasks, wiki};
use rmcp::schemars;

use crate::render::{dash, guard_id, parse_due_arg};
use crate::tools::wiki_pages;

/// The priorities the task drawer offers, in the order it offers them. An empty
/// value clears the field (the drawer's "none").
const PRIORITIES: [&str; 3] = ["low", "normal", "high"];

/// The `status` value that means "in no column at all" — the board's backlog.
/// The store spells it as an empty `status:` field; the tools spell it the way
/// `list_tasks` reports it, so a value read out of one tool goes into another.
const BACKLOG: &str = "backlog";

// --- parameters -------------------------------------------------------------
//
// Doc comments become the JSON-Schema `description` of each field, so they are
// the model's only guide to what a parameter accepts — write them for that
// reader.

/// Parameters for [`create_entry`].
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CreateEntryParams {
    /// Which kind to create: "note" (a markdown file in the notes tree) or
    /// "wiki" (a page in the [[link]]ed namespace). Tasks are not created here
    /// — use `create_task`, which fills in the board frontmatter.
    pub kind: String,
    /// The id to create at: for a note, its well-relative path without `.md`
    /// ("projects/auth-notes" — missing folders are created); for a wiki page,
    /// its slug ("auth-system"), which is globally unique across the section.
    /// If the id is already taken it is uniquified rather than overwritten, so
    /// read the id the response reports back — it may not be this one.
    pub id: String,
    /// The entry's full markdown body. Written verbatim; there is no frontmatter
    /// to set on a note or a wiki page, so a `# heading` first line is the
    /// convention for giving it a title.
    pub content: String,
}

/// Parameters for [`append_to_entry`].
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct AppendToEntryParams {
    /// Which kind of entry: "note", "wiki", "task", or "goal" — exactly as
    /// `search` / `list_entries` reported it.
    pub kind: String,
    /// The entry's id, exactly as another tool reported it. A missing note or
    /// wiki page is created; a missing task or goal is an error, because their
    /// ids are the slugs of real board entries.
    pub id: String,
    /// The markdown to add at the end of the body. Separated from what is
    /// already there by a blank line, so it lands as its own block.
    pub text: String,
}

/// Parameters for [`create_task`].
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct CreateTaskParams {
    /// The task's title, as free text ("Rotate the signing keys"). Its slug
    /// becomes the id, uniquified if that is taken — duplicate titles are fine.
    pub title: String,
    /// Which board column to create it in — a column id from `well_info`, or
    /// "backlog" for no column. Defaults to the first column.
    pub status: Option<String>,
    /// Due date, "YYYY-MM-DD", optionally with a 24-hour time
    /// ("2026-09-01 14:30"). Omit for no due date.
    pub due: Option<String>,
    /// Priority: "low", "normal", or "high". Omit for none.
    pub priority: Option<String>,
    /// Tags, comma-separated ("infra, security"). Omit for none.
    pub tags: Option<String>,
    /// The id of a goal to link this task to (from `list_goals`), which is what
    /// makes it count toward that goal's progress. Omit for none.
    pub goal: Option<String>,
    /// The task's markdown body — the detail behind the title, including any
    /// `- [ ]` checklist. Omit to create it with an empty body and add one later
    /// with `append_to_entry`.
    pub body: Option<String>,
}

/// Parameters for [`update_task_field`].
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct UpdateTaskFieldParams {
    /// The task's id (its slug), exactly as `list_tasks` reported it.
    pub id: String,
    /// Which single field to set: "title", "status", "priority", "due", "tags",
    /// "goal", or "repeat". Nothing else is writable — in particular
    /// "completed" and "archived" are the store's own bookkeeping, and the body
    /// is grown with `append_to_entry`, never replaced.
    pub field: String,
    /// The new value, in that field's own format: a column id (or "backlog")
    /// for status; "low" / "normal" / "high" for priority; "YYYY-MM-DD" with an
    /// optional " HH:MM" for due; comma-separated labels for tags; a goal id for
    /// goal; "daily" / "weekly" / "monthly" / "yearly" / "every N days|weeks|
    /// months|years" for repeat. An empty value clears the field (and for
    /// status means the backlog); a title cannot be cleared.
    pub value: String,
}

// --- gate + guards ----------------------------------------------------------

/// The process-level gate (§8), enforced a second time at the tool body.
///
/// The first enforcement is that [`crate::server::Ido::open`] never registers
/// these routes without `--allow-write`, so the tools are not advertised and the
/// SDK's router answers "tool not found" on its own. This is defence in depth:
/// nothing in the protocol stops a client calling a name it never saw, and a
/// future refactor that changes how routes are composed must not be able to
/// quietly open the door.
pub fn guard_writes(allow_write: bool) -> Result<(), String> {
    if allow_write {
        Ok(())
    } else {
        Err(
            "this server is read-only: it was started without --allow-write, so it cannot \
             create or modify anything in the well. Ask the user to restart it with \
             --allow-write if they want an agent to write here."
                .to_string(),
        )
    }
}

/// Both halves of §9's path-traversal rule for one id.
///
/// [`guard_id`] rejects lexically — `..`, absolute paths, drive letters,
/// backslashes — and `paths::confined` then canonicalises the path the store
/// would actually touch and proves it is still inside the well. Lexical
/// rejection alone can be walked around (a symlinked folder inside the well
/// resolves out of it); canonicalisation alone accepts ids that are nonsense
/// rather than malicious. A write deserves both.
fn guard_write_id(
    well: &str,
    what: &str,
    id: &str,
    section: Section,
    relative: &str,
) -> Result<(), String> {
    guard_id(what, id)?;
    paths::confined(well, section, relative)
        .map_err(|why| format!("invalid {what} `{id}`: {why}"))?;
    Ok(())
}

/// The kinds [`create_entry`] can create. Tasks and goals are deliberately
/// absent: they are frontmatter documents on a board, and `create_task` is the
/// tool that knows that.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum NewKind {
    Note,
    Wiki,
}

impl NewKind {
    fn parse(kind: &str) -> Result<Self, String> {
        match kind.trim().to_lowercase().as_str() {
            "note" => Ok(NewKind::Note),
            "wiki" => Ok(NewKind::Wiki),
            "task" | "goal" => Err(format!(
                "`create_entry` doesn't create {}s — call `create_task` instead, which sets the \
                 board frontmatter a task needs (goals are created in the app)",
                kind.trim().to_lowercase()
            )),
            other => Err(format!(
                "unknown kind `{other}` — `create_entry` takes: note, wiki"
            )),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            NewKind::Note => "note",
            NewKind::Wiki => "wiki",
        }
    }
}

/// The kinds [`append_to_entry`] can append to — all four, since appending to a
/// task or goal only touches its body.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum AppendKind {
    Note,
    Wiki,
    Task,
    Goal,
}

impl AppendKind {
    fn parse(kind: &str) -> Result<Self, String> {
        match kind.trim().to_lowercase().as_str() {
            "note" => Ok(AppendKind::Note),
            "wiki" => Ok(AppendKind::Wiki),
            "task" => Ok(AppendKind::Task),
            "goal" => Ok(AppendKind::Goal),
            other => Err(format!(
                "unknown kind `{other}` — use one of: note, wiki, task, goal"
            )),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            AppendKind::Note => "note",
            AppendKind::Wiki => "wiki",
            AppendKind::Task => "task",
            AppendKind::Goal => "goal",
        }
    }
}

/// The task fields an agent may set — the allow-list, by name, from §8's
/// "edits to tasks are field-level".
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Field {
    Title,
    Status,
    Priority,
    Due,
    Tags,
    Goal,
    Repeat,
}

impl Field {
    /// Every writable field, in the order the drawer shows them — also the order
    /// a refusal lists them in.
    const ALL: [&'static str; 7] = [
        "title", "status", "priority", "due", "tags", "goal", "repeat",
    ];

    /// Parse a `field` argument, refusing anything outside [`Field::ALL`].
    ///
    /// The two interesting refusals are `completed` and `archived`: both are
    /// real frontmatter, and both are written by the store itself — `completed:`
    /// by the status choke point (`apply_status`) on a genuine transition into
    /// the done column, `archived:` by the auto-archive sweep. Letting an agent
    /// set them directly would put the file at odds with the board that derives
    /// from it, so the error says that rather than just "no".
    fn parse(field: &str) -> Result<Self, String> {
        let allowed = Field::ALL.join(", ");
        match field.trim().to_lowercase().as_str() {
            "title" => Ok(Field::Title),
            "status" => Ok(Field::Status),
            "priority" => Ok(Field::Priority),
            "due" => Ok(Field::Due),
            "tags" => Ok(Field::Tags),
            "goal" => Ok(Field::Goal),
            "repeat" => Ok(Field::Repeat),
            "completed" => Err(format!(
                "`completed` is not writable: ido stamps it itself when a task genuinely moves \
                 into the done column, and clears it when the task leaves. Setting \
                 field=\"status\" to the done column is the way to complete a task — that path \
                 stamps the date and spawns the next occurrence of a repeating task. Writable \
                 fields are: {allowed}."
            )),
            "archived" => Err(format!(
                "`archived` is not writable: it is set by the well's auto-archive sweep, which \
                 archives done tasks once their completion date is old enough (see `well_info`). \
                 Writing it by hand would desynchronise the board from its own rules. Writable \
                 fields are: {allowed}."
            )),
            "body" | "content" => Err(format!(
                "the body is not a field: add to it with `append_to_entry` (kind=\"task\"), which \
                 never overwrites what is already written. Writable fields are: {allowed}."
            )),
            "id" | "slug" => Err(format!(
                "a task's id is derived from its title — set field=\"title\" and the response \
                 tells you the new id. Writable fields are: {allowed}."
            )),
            other => Err(format!(
                "`{other}` is not a writable task field. Writable fields are: {allowed}."
            )),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Field::Title => "title",
            Field::Status => "status",
            Field::Priority => "priority",
            Field::Due => "due",
            Field::Tags => "tags",
            Field::Goal => "goal",
            Field::Repeat => "repeat",
        }
    }
}

/// Refuse a free-text frontmatter value that would break out of its own line.
///
/// Frontmatter is a flat `key: value` per line, written verbatim — so a newline
/// inside a value isn't an escaping wrinkle, it's a second field. `"urgent\n
/// completed: 2026-01-01"` as a task's tags would walk straight past
/// [`Field::parse`]'s allow-list, and a `---` on its own line would close the
/// header early and eat the body. The app can't type a newline into those
/// single-line inputs; a tool call can, so the check lives here. Rejected, not
/// stripped: a caller with a multi-line value meant something we shouldn't
/// guess at.
fn single_line(what: &str, value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.chars().any(|c| c.is_control()) {
        return Err(format!(
            "{what} must be a single line of plain text — no line breaks or control characters. \
             It is stored as one `{what}:` line of the task's frontmatter, so a break in it \
             would read back as a different field entirely. Put the detail in the body instead \
             (`append_to_entry` with kind=task)."
        ));
    }
    Ok(value.to_string())
}

/// Validate a `status` argument against this well's columns, returning the
/// value the store wants (the backlog is an empty `status:`).
fn normalise_status(columns: &[String], value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() || value.eq_ignore_ascii_case(BACKLOG) {
        return Ok(String::new());
    }
    match columns.iter().find(|c| c.eq_ignore_ascii_case(value)) {
        Some(column) => Ok(column.clone()),
        None => Err(format!(
            "`{value}` is not one of this well's board columns. Use one of: {}, or \"{BACKLOG}\" \
             for no column (`well_info` lists them; the last one is the done column).",
            columns.join(", ")
        )),
    }
}

/// Validate a `priority` argument. Empty clears the field.
fn normalise_priority(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(String::new());
    }
    match PRIORITIES.iter().find(|p| p.eq_ignore_ascii_case(value)) {
        Some(p) => Ok((*p).to_string()),
        None => Err(format!(
            "`{value}` is not a priority — use one of: {}, or an empty value for none.",
            PRIORITIES.join(", ")
        )),
    }
}

/// Validate a `repeat` spec against the store's own recurrence parser, so a
/// rule that would silently never fire is refused at the door instead of
/// sitting in the file looking like it works. Empty clears the field.
fn normalise_repeat(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(String::new());
    }
    // `advance` returns None for exactly the specs the spawn path can't use;
    // the two dates are arbitrary — only the spec decides.
    if repeat::advance("2026-01-01", value, "2026-01-01").is_none() {
        return Err(format!(
            "`{value}` is not a recurrence ido understands. Use \"daily\", \"weekly\", \
             \"monthly\", \"yearly\", or \"every N days|weeks|months|years\" — anything else \
             would sit in the file and never spawn the next occurrence."
        ));
    }
    Ok(value.to_string())
}

/// Look a task up by id, with the same "here's how to find the real ids"
/// message the read tools use.
fn find_task(well: &str, id: &str) -> Result<ido_store::model::Task, String> {
    tasks::list_tasks(well.to_string())
        .into_iter()
        .find(|t| t.id == id)
        .ok_or_else(|| {
            format!(
                "no task `{id}` — use `list_tasks` to see the ids in this well (ids are \
                     slugs, not titles)"
            )
        })
}

/// Whether a goal id names a real goal.
fn goal_exists(well: &str, id: &str) -> bool {
    tasks::list_goals(well.to_string())
        .into_iter()
        .any(|g| g.id == id)
}

/// Validate a `goal` argument: empty clears the link, anything else must name a
/// goal that exists (a typo would otherwise link a task to nothing at all and
/// quietly not count toward any progress).
fn check_goal(well: &str, value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(String::new());
    }
    guard_write_id(
        well,
        "goal",
        value,
        Section::Tasks,
        &format!("goals/{value}.md"),
    )?;
    if goal_exists(well, value) {
        Ok(value.to_string())
    } else {
        Err(format!(
            "no goal `{value}` — use `list_goals` to see the goal ids in this well"
        ))
    }
}

/// The trailing line every write response ends with: what to call next, so the
/// id this tool just returned round-trips into a read.
fn round_trip(kind: &str, id: &str) -> String {
    format!("\nRead it back with `get_entry` kind={kind} id=`{id}`.")
}

// --- 1. create_entry --------------------------------------------------------

/// Create a note or a wiki page. Never overwrites: a taken id uniquifies, and
/// the response states the id actually written.
pub fn create_entry(well: &str, allow_write: bool, p: CreateEntryParams) -> Result<String, String> {
    guard_writes(allow_write)?;
    let kind = NewKind::parse(&p.kind)?;
    let id = p.id.trim().to_string();
    let written = match kind {
        NewKind::Note => {
            guard_write_id(well, "id", &id, Section::Notes, &format!("{id}.md"))?;
            notes::create_note_at(well.to_string(), id.clone(), p.content.clone())?
        }
        NewKind::Wiki => {
            guard_write_id(well, "id", &id, Section::Wiki, &format!("{id}.md"))?;
            wiki::create_page_at(well.to_string(), id.clone(), p.content.clone())?
        }
    };

    let mut out = format!("# created {} `{written}`\n\n", kind.as_str());
    let _ = writeln!(out, "- kind: {}", kind.as_str());
    let _ = writeln!(out, "- id: `{written}`");
    if written != id {
        let _ = writeln!(
            out,
            "- note: `{id}` was already taken, so the new entry got the id above — nothing was \
             overwritten. Use `{written}` from here on."
        );
    }
    let _ = writeln!(out, "- body: {} characters", p.content.chars().count());
    if matches!(kind, NewKind::Wiki) {
        let _ = writeln!(
            out,
            "- links: other entries can now reach it as `[[{written}]]`"
        );
    }
    let _ = writeln!(
        out,
        "{}\nAdd to it later with `append_to_entry` — this server never overwrites a body.",
        round_trip(kind.as_str(), &written)
    );
    Ok(out)
}

// --- 2. append_to_entry -----------------------------------------------------

/// Append to an entry's body, leaving everything already there — and, for a
/// task or goal, its whole frontmatter — untouched.
pub fn append_to_entry(
    well: &str,
    allow_write: bool,
    p: AppendToEntryParams,
) -> Result<String, String> {
    guard_writes(allow_write)?;
    let kind = AppendKind::parse(&p.kind)?;
    let id = p.id.trim().to_string();
    if p.text.trim().is_empty() {
        return Err("text is empty — there is nothing to append".to_string());
    }

    let (written, existed) = match kind {
        AppendKind::Note => {
            guard_write_id(well, "id", &id, Section::Notes, &format!("{id}.md"))?;
            let existed = notes::read_note(well.to_string(), id.clone()).is_ok();
            (
                notes::append_note(well.to_string(), id.clone(), p.text.clone())?,
                existed,
            )
        }
        AppendKind::Wiki => {
            guard_write_id(well, "id", &id, Section::Wiki, &format!("{id}.md"))?;
            let existed = wiki_pages(well).iter().any(|(slug, _)| *slug == id);
            (
                wiki::append_page(well.to_string(), id.clone(), p.text.clone())?,
                existed,
            )
        }
        AppendKind::Task => {
            guard_write_id(well, "id", &id, Section::Tasks, &format!("{id}.md"))?;
            // A task id is the slug of a real card: inventing one from a typo
            // would put a task on the board nobody asked for.
            find_task(well, &id)?;
            tasks::append_task_body(well.to_string(), id.clone(), p.text.clone())?;
            (id.clone(), true)
        }
        AppendKind::Goal => {
            guard_write_id(well, "id", &id, Section::Tasks, &format!("goals/{id}.md"))?;
            if !goal_exists(well, &id) {
                return Err(format!(
                    "no goal `{id}` — use `list_goals` to see the goal ids in this well"
                ));
            }
            tasks::append_goal_body(well.to_string(), id.clone(), p.text.clone())?;
            (id.clone(), true)
        }
    };

    let mut out = format!("# appended to {} `{written}`\n\n", kind.as_str());
    let _ = writeln!(out, "- kind: {}", kind.as_str());
    let _ = writeln!(out, "- id: `{written}`");
    let _ = writeln!(
        out,
        "- added: {} characters at the end of the body; nothing already there was changed",
        p.text.chars().count()
    );
    if !existed {
        let _ = writeln!(
            out,
            "- note: no {} `{id}` existed, so this call created it",
            kind.as_str()
        );
    }
    if matches!(kind, AppendKind::Task | AppendKind::Goal) {
        let _ = writeln!(
            out,
            "- frontmatter: untouched — status, due, tags and the rest are exactly as they were \
             (use `update_task_field` to change one)"
        );
    }
    let _ = writeln!(out, "{}", round_trip(kind.as_str(), &written));
    Ok(out)
}

// --- 3. create_task ---------------------------------------------------------

/// Create a board task: the title (whose slug is the id) plus the optional
/// frontmatter fields, each applied through the store's own field writer.
pub fn create_task(well: &str, allow_write: bool, p: CreateTaskParams) -> Result<String, String> {
    guard_writes(allow_write)?;
    let title = single_line("title", &p.title)?;
    if title.is_empty() {
        return Err("title is empty — a task needs a title, which becomes its id".to_string());
    }

    // Validate everything before writing anything, so a rejected field can't
    // leave a half-configured card on the board.
    let columns = tasks::task_columns(well.to_string());
    let status = match p.status.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(status) => normalise_status(&columns, status)?,
        // The board's own default for a new card, exactly like quick-add.
        None => columns.first().cloned().unwrap_or_default(),
    };
    let due = match p.due.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
        Some(due) => Some(parse_due_arg("due", due)?),
        None => None,
    };
    let priority = normalise_priority(p.priority.as_deref().unwrap_or_default())?;
    let tags = single_line("tags", p.tags.as_deref().unwrap_or_default())?;
    let goal = check_goal(well, p.goal.as_deref().unwrap_or_default())?;
    let body = p.body.unwrap_or_default();

    let id = tasks::create_task(well.to_string(), status.clone(), title.clone(), due.clone())?;
    for (key, value) in [("priority", &priority), ("tags", &tags), ("goal", &goal)] {
        if !value.is_empty() {
            tasks::set_task_field(well.to_string(), id.clone(), key.to_string(), value.clone())?;
        }
    }
    if !body.trim().is_empty() {
        // The task was created a moment ago with an empty body, so this writes
        // into nothing — §8's "never overwrite an existing body" holds.
        tasks::update_task_body(well.to_string(), id.clone(), body.clone())?;
    }

    let mut out = format!("# created task `{id}` — {title}\n\n");
    let _ = writeln!(
        out,
        "- id: `{id}` (the title's slug, uniquified if it was taken)"
    );
    let _ = writeln!(out, "- title: {title}");
    let _ = writeln!(
        out,
        "- status: {}",
        if status.is_empty() { BACKLOG } else { &status }
    );
    let _ = writeln!(out, "- due: {}", dash(due.as_deref().unwrap_or_default()));
    let _ = writeln!(out, "- priority: {}", dash(&priority));
    let _ = writeln!(out, "- tags: {}", dash(&tags));
    let _ = writeln!(out, "- goal: {}", dash(&goal));
    let _ = writeln!(out, "- body: {} characters", body.chars().count());
    let _ = writeln!(
        out,
        "{}\nChange one field with `update_task_field` id=`{id}`, or add to the body with \
         `append_to_entry` kind=task.",
        round_trip("task", &id)
    );
    Ok(out)
}

// --- 4. update_task_field ---------------------------------------------------

/// Set one field of one task, through the store's own writers — so completing a
/// task stamps `completed:` and spawns a repeat exactly as the board does.
pub fn update_task_field(
    well: &str,
    allow_write: bool,
    p: UpdateTaskFieldParams,
) -> Result<String, String> {
    guard_writes(allow_write)?;
    let id = p.id.trim().to_string();
    guard_write_id(well, "id", &id, Section::Tasks, &format!("{id}.md"))?;
    let field = Field::parse(&p.field)?;
    let task = find_task(well, &id)?;

    let columns = tasks::task_columns(well.to_string());
    let done_column = columns.last().cloned().unwrap_or_default();
    let raw = p.value.trim();

    let (was, value) = match field {
        Field::Title => (task.title.clone(), single_line("title", raw)?),
        Field::Status => (
            if task.status.is_empty() {
                BACKLOG.to_string()
            } else {
                task.status.clone()
            },
            normalise_status(&columns, raw)?,
        ),
        Field::Priority => (task.priority.clone(), normalise_priority(raw)?),
        Field::Due => (
            task.due.clone(),
            if raw.is_empty() {
                String::new()
            } else {
                parse_due_arg("due", raw)?
            },
        ),
        Field::Tags => (task.tags.join(", "), single_line("tags", raw)?),
        Field::Goal => (task.goal.clone(), check_goal(well, raw)?),
        Field::Repeat => (task.repeat.clone(), normalise_repeat(raw)?),
    };

    // A rename re-slugs the file, so it is the one edit that can change the id.
    let new_id = if matches!(field, Field::Title) {
        if value.is_empty() {
            return Err(
                "a task's title can't be cleared — it is what the card and its id are made of. \
                 Pass the new title as `value`."
                    .to_string(),
            );
        }
        tasks::rename_task(well.to_string(), id.clone(), value.clone())?
    } else {
        tasks::set_task_field(
            well.to_string(),
            id.clone(),
            field.as_str().to_string(),
            value.clone(),
        )?;
        id.clone()
    };

    let shown = |v: &str| {
        if v.is_empty() {
            "(none)".to_string()
        } else {
            v.to_string()
        }
    };
    let after = match field {
        Field::Status if value.is_empty() => BACKLOG.to_string(),
        _ => shown(&value),
    };
    let mut out = format!("# task `{new_id}` — {} updated\n\n", field.as_str());
    let _ = writeln!(out, "- id: `{new_id}`");
    let _ = writeln!(out, "- field: {}", field.as_str());
    let _ = writeln!(out, "- was: {}", shown(&was));
    let _ = writeln!(out, "- now: {after}");
    if new_id != id {
        let _ = writeln!(
            out,
            "- note: the title's slug changed, so the task's id is now `{new_id}` (it was \
             `{id}`) — use the new one from here on. Its body and every other field are unchanged."
        );
    }
    if matches!(field, Field::Status) {
        if value == done_column && was != done_column {
            let _ = writeln!(
                out,
                "- side effects: moving into the done column stamped `completed:` with today's \
                 date{}",
                if task.repeat.trim().is_empty() {
                    ""
                } else {
                    ", and this task repeats, so ido also created its next occurrence in the \
                     first column"
                }
            );
        } else if was == done_column && value != done_column {
            let _ = writeln!(
                out,
                "- side effects: leaving the done column cleared this task's `completed:` stamp"
            );
        }
    }
    let _ = writeln!(
        out,
        "{}\nOnly this field changed; the body is untouched.",
        round_trip("task", &new_id)
    );
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The gate has to be shut by default and say what would open it, because
    /// this message is what the model reports back to the user.
    #[test]
    fn writes_are_refused_without_the_flag() {
        assert!(guard_writes(true).is_ok());
        let err = guard_writes(false).unwrap_err();
        assert!(err.contains("--allow-write"), "got: {err}");
        assert!(err.contains("read-only"), "got: {err}");
    }

    /// Every closed door in the tool bodies, and the fact that it is closed
    /// before any store call can run.
    #[test]
    fn every_write_tool_is_shut_when_the_gate_is() {
        // A well path that does not exist: if the gate leaked, these would fail
        // with a filesystem error instead of the refusal.
        let well = "./definitely-not-a-well-xyz";
        let refusal = |e: String| assert!(e.contains("--allow-write"), "got: {e}");
        refusal(
            create_entry(
                well,
                false,
                CreateEntryParams {
                    kind: "note".into(),
                    id: "x".into(),
                    content: String::new(),
                },
            )
            .unwrap_err(),
        );
        refusal(
            append_to_entry(
                well,
                false,
                AppendToEntryParams {
                    kind: "note".into(),
                    id: "x".into(),
                    text: "hi".into(),
                },
            )
            .unwrap_err(),
        );
        refusal(
            create_task(
                well,
                false,
                CreateTaskParams {
                    title: "x".into(),
                    status: None,
                    due: None,
                    priority: None,
                    tags: None,
                    goal: None,
                    body: None,
                },
            )
            .unwrap_err(),
        );
        refusal(
            update_task_field(
                well,
                false,
                UpdateTaskFieldParams {
                    id: "x".into(),
                    field: "status".into(),
                    value: "done".into(),
                },
            )
            .unwrap_err(),
        );
    }

    /// An id that could address anything outside the well is refused before the
    /// store sees it — the lexical half of §9, provable without a real well.
    #[test]
    fn traversal_ids_are_refused_with_the_gate_open() {
        for id in ["../x", "/etc/passwd", "a/../../etc/passwd", "C:/Windows"] {
            let err = create_entry(
                "./definitely-not-a-well-xyz",
                true,
                CreateEntryParams {
                    kind: "note".into(),
                    id: id.to_string(),
                    content: "x".into(),
                },
            )
            .unwrap_err();
            assert!(err.contains("invalid id"), "`{id}` gave: {err}");
        }
    }

    #[test]
    fn create_entry_takes_notes_and_pages_and_redirects_tasks() {
        assert_eq!(NewKind::parse("note").unwrap(), NewKind::Note);
        assert_eq!(NewKind::parse(" Wiki ").unwrap(), NewKind::Wiki);
        let err = NewKind::parse("task").unwrap_err();
        assert!(err.contains("create_task"), "got: {err}");
        assert!(NewKind::parse("notes").is_err(), "section name, not a kind");
    }

    #[test]
    fn append_takes_all_four_kinds() {
        for (kind, parsed) in [
            ("note", AppendKind::Note),
            ("wiki", AppendKind::Wiki),
            ("task", AppendKind::Task),
            ("GOAL", AppendKind::Goal),
        ] {
            assert_eq!(AppendKind::parse(kind).unwrap(), parsed);
        }
        assert!(AppendKind::parse("recipe").is_err());
    }

    /// The allow-list is the security surface of `update_task_field`, and the
    /// refusals have to teach rather than stonewall.
    #[test]
    fn the_writable_fields_are_exactly_the_allow_list() {
        for name in Field::ALL {
            assert_eq!(Field::parse(name).unwrap().as_str(), name);
        }
        assert_eq!(Field::parse(" Status ").unwrap(), Field::Status);

        let completed = Field::parse("completed").unwrap_err();
        assert!(
            completed.contains("done column"),
            "the refusal must explain who owns the stamp: {completed}"
        );
        let archived = Field::parse("archived").unwrap_err();
        assert!(
            archived.contains("auto-archive"),
            "the refusal must explain who owns the flag: {archived}"
        );
        for refusal in [&completed, &archived] {
            for name in Field::ALL {
                assert!(
                    refusal.contains(name),
                    "a refusal lists what *is* allowed, but `{name}` is missing: {refusal}"
                );
            }
        }
        assert!(
            Field::parse("body")
                .unwrap_err()
                .contains("append_to_entry")
        );
        assert!(
            Field::parse("order")
                .unwrap_err()
                .contains("not a writable")
        );
        assert!(Field::parse("").unwrap_err().contains("not a writable"));
    }

    /// The allow-list is only as strong as the file format under it:
    /// frontmatter is one `key: value` per line, written verbatim, so a
    /// newline in a free-text value would *be* a second field — including one
    /// [`Field::parse`] refuses by name.
    #[test]
    fn frontmatter_values_cant_smuggle_a_second_field() {
        assert_eq!(single_line("title", "  Ship P4  ").unwrap(), "Ship P4");
        assert_eq!(single_line("tags", "mcp, p4").unwrap(), "mcp, p4");
        for smuggled in [
            "urgent\ncompleted: 2026-01-01",
            "urgent\r\narchived: true",
            "Ship\n---\nnot the body",
            "tab\tseparated\u{7f}",
        ] {
            let err = single_line("tags", smuggled).unwrap_err();
            assert!(err.contains("single line"), "`{smuggled}` gave: {err}");
        }
    }

    #[test]
    fn statuses_are_checked_against_the_wells_own_columns() {
        let columns = ["todo", "doing", "done"].map(String::from);
        assert_eq!(normalise_status(&columns, "doing").unwrap(), "doing");
        assert_eq!(
            normalise_status(&columns, " DONE ").unwrap(),
            "done",
            "matched case-insensitively, stored in the column's own casing"
        );
        assert_eq!(normalise_status(&columns, "backlog").unwrap(), "");
        assert_eq!(normalise_status(&columns, "").unwrap(), "");
        let err = normalise_status(&columns, "in-review").unwrap_err();
        assert!(err.contains("todo, doing, done"), "got: {err}");
        assert!(err.contains("backlog"), "got: {err}");
    }

    #[test]
    fn priorities_are_the_three_the_drawer_offers() {
        assert_eq!(normalise_priority("high").unwrap(), "high");
        assert_eq!(normalise_priority(" Low ").unwrap(), "low");
        assert_eq!(normalise_priority("").unwrap(), "", "empty clears it");
        let err = normalise_priority("urgent").unwrap_err();
        assert!(err.contains("low, normal, high"), "got: {err}");
    }

    #[test]
    fn repeat_specs_must_actually_spawn() {
        for spec in ["daily", "weekly", "Monthly", "yearly", "every 3 weeks"] {
            assert_eq!(normalise_repeat(spec).unwrap(), spec.trim());
        }
        assert_eq!(normalise_repeat("  ").unwrap(), "", "empty clears it");
        let err = normalise_repeat("every other tuesday").unwrap_err();
        assert!(err.contains("every N days"), "got: {err}");
    }
}
