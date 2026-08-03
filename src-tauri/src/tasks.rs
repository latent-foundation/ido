//! The tasks section: one markdown file per task under `tasks/`, its kanban
//! metadata held in frontmatter (`status` = column, plus `title` / `priority` /
//! `tags` / `order` / `due` / `goal` / `repeat` / `archived` / `completed`) and
//! its description in the body.
//!
//! A task's id is its file stem — the slug of its `title:`, the free-text
//! display name (duplicate titles are fine; slugs uniquify with `-N`). Editing
//! metadata reads, mutates, and re-writes the file via [`crate::frontmatter`],
//! so the body and any unknown keys survive.
//!
//! Every write path that can change a task's `status` — [`move_task`],
//! [`reorder_column`], and [`set_task_field`] — routes through [`apply_status`],
//! the single choke point for status transitions: it stamps `completed:` (today,
//! local date) on entering the done column and clears it on leaving, based on
//! the well's last board column (see [`done_column`]).
//!
//! Those same three paths also drive **recurrence**: on a genuine transition
//! into the done column, a task carrying a `repeat:` rule spawns its next
//! occurrence as a new file ([`prepare_spawn`] captures it inside the edit
//! closure, [`spawn_next`] writes it afterwards) and drops `repeat:` from the
//! completed file so the rule can't double-fire.
//!
//! [`sweep_archive`] auto-archives done tasks older than the well's
//! `archive_done_after_days` setting (`.ido/well.toml`, off by default): it
//! runs on every well open ([`crate::wells::migrate_well`]) and again the
//! moment the setting is written ([`crate::wells::set_archive_days`]).

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use pulldown_cmark::{Event, Options, Parser};

use crate::frontmatter;
use crate::model::{Goal, Section, Task};
use crate::paths::{section_dir, slugify, unique_name};

/// Absolute path of the markdown file backing task `id` in `well`.
fn task_path(well: &str, id: &str) -> PathBuf {
    section_dir(well, Section::Tasks).join(format!("{id}.md"))
}

/// Split a `tags` frontmatter value (`"a, b"`) into trimmed, non-empty labels.
fn split_tags(s: &str) -> Vec<String> {
    s.split(',')
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect()
}

/// Count `- [ ]` / `- [x]` task-list markers in a task body via pulldown-cmark
/// parser events (`Event::TaskListMarker`), not line scanning — so a fenced
/// code block that merely *contains* `- [ ]` text is never counted. This
/// mirrors the frontend's clickable-checkbox semantics (`markdown.rs`).
/// Nested/indented task-list items (under a parent list item) are walked by
/// the parser same as top-level ones, so they count too. Returns `(done, total)`.
fn count_checks(body: &str) -> (u32, u32) {
    let mut done = 0u32;
    let mut total = 0u32;
    for event in Parser::new_ext(body, Options::ENABLE_TASKLISTS) {
        if let Event::TaskListMarker(checked) = event {
            total += 1;
            if checked {
                done += 1;
            }
        }
    }
    (done, total)
}

/// Build a [`Task`] from a file's id and raw markdown.
fn task_from(id: String, src: &str) -> Task {
    let (fields, body) = frontmatter::parse(src);
    let (checks_done, checks_total) = count_checks(&body);
    Task {
        title: fields.get("title").cloned().unwrap_or_else(|| id.clone()),
        id,
        status: fields.get("status").cloned().unwrap_or_default(),
        priority: fields.get("priority").cloned().unwrap_or_default(),
        tags: fields
            .get("tags")
            .map(|t| split_tags(t))
            .unwrap_or_default(),
        order: fields
            .get("order")
            .and_then(|o| o.parse().ok())
            .unwrap_or(0),
        due: fields.get("due").cloned().unwrap_or_default(),
        goal: fields.get("goal").cloned().unwrap_or_default(),
        repeat: fields.get("repeat").cloned().unwrap_or_default(),
        archived: fields.get("archived").map(|v| v == "true").unwrap_or(false),
        completed: fields.get("completed").cloned().unwrap_or_default(),
        checks_done,
        checks_total,
        body,
    }
}

/// One past the largest `order` in `status`'s column — i.e. "append to the end".
fn next_order(tasks: &[Task], status: &str) -> i64 {
    tasks
        .iter()
        .filter(|t| t.status == status)
        .map(|t| t.order)
        .max()
        .map_or(0, |m| m + 1)
}

/// Set `key` to `value`, or remove it when `value` is blank (so we never emit an
/// empty `key:` line).
fn set_field(fields: &mut BTreeMap<String, String>, key: &str, value: &str) {
    let value = value.trim();
    if value.is_empty() {
        fields.remove(key);
    } else {
        fields.insert(key.to_string(), value.to_string());
    }
}

/// Today's local date as `YYYY-MM-DD`, for stamping `completed:`.
fn today_ymd() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

/// The well's done column — the last entry of `.ido/well.toml`'s `columns` —
/// mirroring how the frontend treats the last column as done.
fn done_column(well: &str) -> String {
    crate::wells::read_manifest(well)
        .columns
        .last()
        .cloned()
        .unwrap_or_default()
}

/// The well's first board column — where a spawned recurring occurrence lands.
fn first_column(well: &str) -> String {
    crate::wells::read_manifest(well)
        .columns
        .first()
        .cloned()
        .unwrap_or_default()
}

/// A task's effective completion date for the auto-archive sweep: the
/// `completed:` field when it parses as `YYYY-MM-DD`, else `mtime`'s date —
/// the fallback for tasks completed before the `completed:` stamp existed
/// (see `CLAUDE.md`). Pure and free of any filesystem access, so the
/// mtime-fallback path is table-testable without backdating a real file.
fn completion_date(completed: &str, mtime: chrono::NaiveDate) -> chrono::NaiveDate {
    chrono::NaiveDate::parse_from_str(completed, "%Y-%m-%d").unwrap_or(mtime)
}

/// Auto-archive done tasks older than the well's `archive_done_after_days`
/// setting. A no-op when the setting is off (`None`) or the well has no done
/// column. For every non-archived task sitting in the done column (the last
/// manifest column) whose [`completion_date`] is strictly more than `days`
/// days before today (local), sets `archived: true` via the same
/// frontmatter-preserving edit every other field write uses. Already-archived
/// tasks are skipped, so a re-run is a no-op — safe to call on every well open
/// ([`crate::wells::migrate_well`]) and again whenever the setting is written
/// ([`crate::wells::set_archive_days`], so lowering the threshold takes effect
/// immediately rather than waiting for the next open).
pub(crate) fn sweep_archive(well: &str) {
    let manifest = crate::wells::read_manifest(well);
    let Some(days) = manifest.archive_done_after_days else {
        return;
    };
    let done_col = manifest.columns.last().cloned().unwrap_or_default();
    if done_col.is_empty() {
        return;
    }
    let today = chrono::Local::now().date_naive();
    for task in list_tasks(well.to_string()) {
        if task.archived || task.status != done_col {
            continue;
        }
        let mtime = fs::metadata(task_path(well, &task.id))
            .and_then(|m| m.modified())
            .map(|t| chrono::DateTime::<chrono::Local>::from(t).date_naive())
            .unwrap_or(today);
        let date = completion_date(&task.completed, mtime);
        if (today - date).num_days() > i64::from(days) {
            let _ = edit_task(well, &task.id, |fields, _| {
                fields.insert("archived".into(), "true".into());
            });
        }
    }
}

/// Apply a status transition to `fields`, the single choke point every status
/// write routes through. Sets `status` to `new_status`; when that actually
/// changes the value (an unchanged status, e.g. `reorder_column` re-writing a
/// card that stays in its column, is a no-op here), also stamps `completed:`
/// with today's local date on entering `done_col` or removes it on leaving.
fn apply_status(fields: &mut BTreeMap<String, String>, new_status: &str, done_col: &str) {
    let old_status = fields.get("status").cloned().unwrap_or_default();
    set_field(fields, "status", new_status);
    if old_status == new_status {
        return;
    }
    if new_status == done_col {
        fields.insert("completed".to_string(), today_ymd());
    } else if old_status == done_col {
        fields.remove("completed");
    }
}

/// The frontmatter + body of a recurring task's next occurrence, captured by
/// [`prepare_spawn`] inside an edit closure (which can't touch disk) and written
/// by [`spawn_next`] after the completing edit lands.
struct Spawn {
    /// The next occurrence's frontmatter: the completed task's fields minus
    /// `status` / `order` / `completed`, with `status` reset to the first column,
    /// `due` advanced, and `repeat` kept. `order` is filled in by [`spawn_next`].
    fields: BTreeMap<String, String>,
    /// The completed task's body, copied verbatim.
    body: String,
}

/// Decide whether completing a task should spawn its next occurrence, and if so
/// build it and strip `repeat:` from the completing file. Runs inside the edit
/// closure **before** [`apply_status`], so it reads the pre-transition status.
///
/// Spawns only on a *genuine* transition into `done_col` (old status differs and
/// the new status is the done column) of a task with a parseable `repeat:` rule.
/// A bad rule (or no due-advance) leaves `repeat:` in place and returns `None`,
/// so the task completes normally and nothing is silently lost.
fn prepare_spawn(
    fields: &mut BTreeMap<String, String>,
    body: &str,
    new_status: &str,
    done_col: &str,
    first_col: &str,
    today: &str,
) -> Option<Spawn> {
    let old_status = fields.get("status").cloned().unwrap_or_default();
    // Only entering the done column, and only as a change, can spawn.
    if new_status != done_col || old_status == new_status {
        return None;
    }
    let repeat = fields.get("repeat").cloned().unwrap_or_default();
    if repeat.trim().is_empty() {
        return None;
    }
    let due = fields.get("due").cloned().unwrap_or_default();
    // A rule that doesn't advance (bad spec) → no spawn, `repeat:` preserved.
    let next_due = crate::repeat::advance(&due, &repeat, today)?;

    // The next occurrence copies every field (incl. `repeat` + any unknowns),
    // then resets what a fresh occurrence must not inherit.
    let mut next = fields.clone();
    next.remove("status");
    next.remove("order");
    next.remove("completed");
    set_field(&mut next, "due", &next_due);
    next.insert("status".into(), first_col.to_string());

    // The completed card becomes plain history — drop the rule so re-dragging it
    // back into done can't spawn again.
    fields.remove("repeat");

    Some(Spawn {
        fields: next,
        body: body.to_string(),
    })
}

/// Write a captured [`Spawn`] as a new task file, appended at the end of its
/// (first) column and slugged from its title (uniquified, `untitled` when the
/// slug is empty) — the same naming machinery as [`create_task`]. Returns its id.
fn spawn_next(well: &str, mut spawn: Spawn) -> Result<String, String> {
    let dir = section_dir(well, Section::Tasks);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let status = spawn.fields.get("status").cloned().unwrap_or_default();
    let order = next_order(&list_tasks(well.to_string()), &status);
    spawn.fields.insert("order".into(), order.to_string());
    let title = spawn.fields.get("title").map(String::as_str).unwrap_or("");
    let stem = slugify(title);
    let stem = if stem.is_empty() { "untitled" } else { &stem };
    let id = unique_name(&dir, stem, "md");
    fs::write(
        dir.join(format!("{id}.md")),
        frontmatter::merge(&spawn.fields, &spawn.body),
    )
    .map_err(|e| e.to_string())?;
    Ok(id)
}

/// Whether file stem `id` already carries `slug` — either exactly, or as a
/// uniquified variant (`slug-2`, `slug-17`, …). Used by the renames so
/// re-committing an unchanged title doesn't walk a `-N` suffix upward.
fn id_matches_slug(id: &str, slug: &str) -> bool {
    id == slug
        || id
            .strip_prefix(slug)
            .and_then(|rest| rest.strip_prefix('-'))
            .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// Read the markdown at `path`, let `edit` mutate its fields + body, write back.
fn edit_md(
    path: PathBuf,
    edit: impl FnOnce(&mut BTreeMap<String, String>, &mut String),
) -> Result<(), String> {
    let src = fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let (mut fields, mut body) = frontmatter::parse(&src);
    edit(&mut fields, &mut body);
    fs::write(path, frontmatter::merge(&fields, &body)).map_err(|e| e.to_string())
}

/// Edit task `id` in place (frontmatter + body).
fn edit_task(
    well: &str,
    id: &str,
    edit: impl FnOnce(&mut BTreeMap<String, String>, &mut String),
) -> Result<(), String> {
    edit_md(task_path(well, id), edit)
}

/// The board columns for this well (from `.ido/well.toml`).
#[tauri::command]
pub fn task_columns(well: String) -> Vec<String> {
    crate::wells::read_manifest(&well).columns
}

/// The well's auto-archive-done-after-N-days setting (`.ido/well.toml`);
/// `None` means off. Mirrors [`task_columns`]'s read-only manifest access —
/// the settings UI's other half of [`crate::wells::set_archive_days`].
#[tauri::command]
pub fn archive_days(well: String) -> Option<u32> {
    crate::wells::read_manifest(&well).archive_done_after_days
}

/// Every task in the well, sorted by `order` then id. Subfolders (e.g. future
/// goals) and hidden / non-`.md` files are skipped.
#[tauri::command]
pub fn list_tasks(well: String) -> Vec<Task> {
    let Ok(entries) = fs::read_dir(section_dir(&well, Section::Tasks)) else {
        return Vec::new();
    };
    let mut tasks: Vec<Task> = entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let name = path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or_default();
            if name.starts_with('.') || path.extension().and_then(|x| x.to_str()) != Some("md") {
                return None;
            }
            let id = path.file_stem().and_then(|s| s.to_str())?.to_string();
            let src = fs::read_to_string(&path).ok()?;
            Some(task_from(id, &src))
        })
        .collect();
    tasks.sort_by(|a, b| a.order.cmp(&b.order).then_with(|| a.id.cmp(&b.id)));
    tasks
}

/// Create an empty task in column `status`, at the column's end, titled `title`
/// (free text; blank = untitled), optionally due `due` (`YYYY-MM-DD`; `None` or
/// blank sets no due date). The file stem is the title's slug, uniquified
/// (`fix-login`, `fix-login-2`, …) — duplicate titles are fine. Returns its id.
#[tauri::command]
pub fn create_task(
    well: String,
    status: String,
    title: String,
    due: Option<String>,
) -> Result<String, String> {
    let dir = section_dir(&well, Section::Tasks);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let title = title.trim();
    let stem = slugify(title);
    let stem = if stem.is_empty() { "untitled" } else { &stem };
    let id = unique_name(&dir, stem, "md");
    let order = next_order(&list_tasks(well.clone()), &status);
    let mut fields = BTreeMap::new();
    set_field(&mut fields, "status", &status);
    set_field(&mut fields, "title", title);
    fields.insert("order".to_string(), order.to_string());
    if let Some(due) = due {
        set_field(&mut fields, "due", &due);
    }
    fs::write(
        dir.join(format!("{id}.md")),
        frontmatter::merge(&fields, ""),
    )
    .map_err(|e| e.to_string())?;
    Ok(id)
}

/// Move task `id` into column `status`, appended at its end. An empty `status`
/// clears the field — the task drops to the backlog (un-columned). Routes
/// through [`apply_status`], so entering/leaving the done column stamps or
/// clears `completed:`; a recurring task entering done also spawns its next
/// occurrence ([`prepare_spawn`] / [`spawn_next`]).
#[tauri::command]
pub fn move_task(well: String, id: String, status: String) -> Result<(), String> {
    let order = next_order(&list_tasks(well.clone()), &status).to_string();
    let done_col = done_column(&well);
    let first_col = first_column(&well);
    let today = today_ymd();
    let mut spawn = None;
    edit_task(&well, &id, |fields, body| {
        spawn = prepare_spawn(fields, body, &status, &done_col, &first_col, &today);
        apply_status(fields, &status, &done_col);
        fields.insert("order".into(), order);
    })?;
    if let Some(spawn) = spawn {
        spawn_next(&well, spawn)?;
    }
    Ok(())
}

/// Reorder a column: set every id in `ids` to `status` with `order = index`, so
/// the column matches `ids` exactly (a manual drag-to-position). An empty
/// `status` clears the field (the backlog). Tasks not listed are left
/// untouched. Routes through [`apply_status`] — a card newly pulled into the
/// done column gets stamped; one merely reordered within it (status unchanged)
/// is left alone. A recurring card pulled into done spawns its next occurrence
/// after the loop (into the first column, so it never disturbs this reorder;
/// even in a single-column well it just appends past the reordered ids).
#[tauri::command]
pub fn reorder_column(well: String, status: String, ids: Vec<String>) -> Result<(), String> {
    let done_col = done_column(&well);
    let first_col = first_column(&well);
    let today = today_ymd();
    let mut spawns = Vec::new();
    for (i, id) in ids.iter().enumerate() {
        let order = i.to_string();
        let mut spawn = None;
        let _ = edit_task(&well, id, |fields, body| {
            spawn = prepare_spawn(fields, body, &status, &done_col, &first_col, &today);
            apply_status(fields, &status, &done_col);
            fields.insert("order".into(), order);
        });
        if let Some(spawn) = spawn {
            spawns.push(spawn);
        }
    }
    for spawn in spawns {
        let _ = spawn_next(&well, spawn);
    }
    Ok(())
}

/// Set a single metadata field (`priority` / `tags` / `due` / `repeat` /
/// `status`), or clear it when `value` is blank. A `status` write (the drawer's
/// status picker) routes through [`apply_status`] like every other status
/// change, and — like the other status paths — spawns the next occurrence of a
/// recurring task on entering done.
#[tauri::command]
pub fn set_task_field(well: String, id: String, key: String, value: String) -> Result<(), String> {
    if key == "status" {
        let done_col = done_column(&well);
        let first_col = first_column(&well);
        let today = today_ymd();
        let mut spawn = None;
        edit_task(&well, &id, |fields, body| {
            spawn = prepare_spawn(fields, body, &value, &done_col, &first_col, &today);
            apply_status(fields, &value, &done_col);
        })?;
        if let Some(spawn) = spawn {
            spawn_next(&well, spawn)?;
        }
        return Ok(());
    }
    edit_task(&well, &id, |fields, _| set_field(fields, &key, &value))
}

/// Replace a task's markdown body, preserving its frontmatter.
#[tauri::command]
pub fn update_task_body(well: String, id: String, body: String) -> Result<(), String> {
    edit_task(&well, &id, |_, b| *b = body)
}

/// Retitle task `id` to `name` (free text, stored as `title:`), re-slugging the
/// file to match. A colliding slug is uniquified (duplicate titles are fine), and
/// a punctuation-only title keeps the current file name. Returns the (new) id.
#[tauri::command]
pub fn rename_task(well: String, id: String, name: String) -> Result<String, String> {
    let title = name.trim().to_string();
    if title.is_empty() {
        return Err("name is empty".into());
    }
    let dir = section_dir(&well, Section::Tasks);
    let slug = slugify(&title);
    let new_id = if slug.is_empty() || id_matches_slug(&id, &slug) {
        id
    } else {
        let new_id = unique_name(&dir, &slug, "md");
        fs::rename(
            dir.join(format!("{id}.md")),
            dir.join(format!("{new_id}.md")),
        )
        .map_err(|e| e.to_string())?;
        new_id
    };
    edit_task(&well, &new_id, |fields, _| {
        fields.insert("title".into(), title);
    })?;
    Ok(new_id)
}

/// Delete task `id`, returning its raw file content so the delete can be undone
/// (see [`restore_task`]).
#[tauri::command]
pub fn delete_task(well: String, id: String) -> Result<String, String> {
    let path = task_path(&well, &id);
    let content = fs::read_to_string(&path).unwrap_or_default();
    fs::remove_file(&path).map_err(|e| e.to_string())?;
    Ok(content)
}

/// Recreate task `id` from raw markdown (undo of [`delete_task`]).
#[tauri::command]
pub fn restore_task(well: String, id: String, content: String) -> Result<(), String> {
    let dir = section_dir(&well, Section::Tasks);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    fs::write(dir.join(format!("{id}.md")), content).map_err(|e| e.to_string())
}

// --- goals (milestones) ----------------------------------------------------

/// The `tasks/goals/` folder for `well`.
fn goals_dir(well: &str) -> PathBuf {
    section_dir(well, Section::Tasks).join("goals")
}

/// Absolute path of the markdown file backing goal `id`.
fn goal_path(well: &str, id: &str) -> PathBuf {
    goals_dir(well).join(format!("{id}.md"))
}

/// Re-point (or, with `new = None`, clear) the `goal:` field of every task that
/// targets `old` — after a goal rename / delete, so tasks never dangle.
fn retarget_goal(well: &str, old: &str, new: Option<&str>) {
    for task in list_tasks(well.to_string()) {
        if task.goal == old {
            let _ = edit_task(well, &task.id, |fields, _| match new {
                Some(n) => {
                    fields.insert("goal".into(), n.to_string());
                }
                None => {
                    fields.remove("goal");
                }
            });
        }
    }
}

/// Every goal in the well, alphabetical.
#[tauri::command]
pub fn list_goals(well: String) -> Vec<Goal> {
    let Ok(entries) = fs::read_dir(goals_dir(&well)) else {
        return Vec::new();
    };
    let mut goals: Vec<Goal> = entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let name = path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or_default();
            if name.starts_with('.') || path.extension().and_then(|x| x.to_str()) != Some("md") {
                return None;
            }
            let id = path.file_stem().and_then(|s| s.to_str())?.to_string();
            let (fields, body) = frontmatter::parse(&fs::read_to_string(&path).ok()?);
            Some(Goal {
                title: fields.get("title").cloned().unwrap_or_else(|| id.clone()),
                id,
                target: fields.get("target").cloned().unwrap_or_default(),
                order: fields
                    .get("order")
                    .and_then(|o| o.parse().ok())
                    .unwrap_or(0),
                archived: fields.get("archived").map(|v| v == "true").unwrap_or(false),
                body,
            })
        })
        .collect();
    // Manual order (drag-to-reorder), with id as a stable tie-break.
    goals.sort_by(|a, b| a.order.cmp(&b.order).then_with(|| a.id.cmp(&b.id)));
    goals
}

/// Create a uniquely-named empty goal, appended at the end. Returns its id.
#[tauri::command]
pub fn create_goal(well: String) -> Result<String, String> {
    let dir = goals_dir(&well);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let id = unique_name(&dir, "untitled", "md");
    let order = list_goals(well.clone())
        .iter()
        .map(|g| g.order)
        .max()
        .map_or(0, |m| m + 1);
    let mut fields = BTreeMap::new();
    fields.insert("order".to_string(), order.to_string());
    fs::write(
        dir.join(format!("{id}.md")),
        frontmatter::merge(&fields, ""),
    )
    .map_err(|e| e.to_string())?;
    Ok(id)
}

/// Renumber the goals bar to match `ids` (drag-to-reorder); `order` = index.
#[tauri::command]
pub fn reorder_goals(well: String, ids: Vec<String>) -> Result<(), String> {
    for (i, id) in ids.iter().enumerate() {
        let order = i.to_string();
        let _ = edit_md(goal_path(&well, id), |fields, _| {
            fields.insert("order".into(), order);
        });
    }
    Ok(())
}

/// Set (or clear, when blank) a goal field (`target`), preserving the body.
#[tauri::command]
pub fn set_goal_field(well: String, id: String, key: String, value: String) -> Result<(), String> {
    edit_md(goal_path(&well, &id), |fields, _| {
        set_field(fields, &key, &value)
    })
}

/// Replace a goal's markdown body, preserving its frontmatter.
#[tauri::command]
pub fn update_goal_body(well: String, id: String, body: String) -> Result<(), String> {
    edit_md(goal_path(&well, &id), |_, b| *b = body)
}

/// Retitle goal `id` to `name` (free text, stored as `title:`), re-slugging the
/// file to match and re-pointing tasks that target it. Same slug rules as
/// [`rename_task`]. Returns the (new) id.
#[tauri::command]
pub fn rename_goal(well: String, id: String, name: String) -> Result<String, String> {
    let title = name.trim().to_string();
    if title.is_empty() {
        return Err("name is empty".into());
    }
    let dir = goals_dir(&well);
    let slug = slugify(&title);
    let new_id = if slug.is_empty() || id_matches_slug(&id, &slug) {
        id
    } else {
        let new_id = unique_name(&dir, &slug, "md");
        fs::rename(
            dir.join(format!("{id}.md")),
            dir.join(format!("{new_id}.md")),
        )
        .map_err(|e| e.to_string())?;
        retarget_goal(&well, &id, Some(&new_id));
        new_id
    };
    edit_md(goal_path(&well, &new_id), |fields, _| {
        fields.insert("title".into(), title);
    })?;
    Ok(new_id)
}

/// Delete goal `id`, clearing the `goal:` field of any task that targeted it.
/// Returns its raw file content so the delete can be undone (see [`restore_goal`];
/// undo restores the goal file only — tasks' cleared `goal:` refs are not re-pointed).
#[tauri::command]
pub fn delete_goal(well: String, id: String) -> Result<String, String> {
    let path = goal_path(&well, &id);
    let content = fs::read_to_string(&path).unwrap_or_default();
    fs::remove_file(&path).map_err(|e| e.to_string())?;
    retarget_goal(&well, &id, None);
    Ok(content)
}

/// Recreate goal `id` from raw markdown (undo of [`delete_goal`]).
#[tauri::command]
pub fn restore_goal(well: String, id: String, content: String) -> Result<(), String> {
    let dir = goals_dir(&well);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    fs::write(dir.join(format!("{id}.md")), content).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::{TempDir, tempdir};

    fn well() -> (TempDir, String) {
        let dir = tempdir().unwrap();
        let path = dir.path().to_string_lossy().into_owned();
        fs::create_dir_all(dir.path().join("tasks")).unwrap();
        (dir, path)
    }

    fn find<'a>(tasks: &'a [Task], id: &str) -> &'a Task {
        tasks.iter().find(|t| t.id == id).unwrap()
    }

    /// Shorthand for an untitled, due-less create (most tests don't care about
    /// titles or due dates).
    fn create(w: &str, status: &str) -> String {
        create_task(w.to_string(), status.to_string(), String::new(), None).unwrap()
    }

    /// Read back a task's raw frontmatter fields, for asserting on keys not
    /// carried by [`Task`] (like `completed`).
    fn fields_of(w: &str, id: &str) -> BTreeMap<String, String> {
        let src = fs::read_to_string(task_path(w, id)).unwrap();
        frontmatter::parse(&src).0
    }

    /// Today's local date as `YYYY-MM-DD`, computed independently of
    /// `today_ymd` so the tests aren't tautological against it.
    fn today() -> String {
        chrono::Local::now().format("%Y-%m-%d").to_string()
    }

    #[test]
    fn create_list_and_order_within_column() {
        let (_d, w) = well();
        let a = create(&w, "todo");
        let b = create(&w, "todo");
        assert_eq!(a, "untitled");
        assert_eq!(b, "untitled-2");
        let tasks = list_tasks(w);
        assert_eq!(tasks.len(), 2);
        // Both in todo, appended in order; an untitled task displays its id.
        assert_eq!(find(&tasks, "untitled").status, "todo");
        assert_eq!(find(&tasks, "untitled").title, "untitled");
        assert!(find(&tasks, "untitled-2").order > find(&tasks, "untitled").order);
    }

    #[test]
    fn create_with_title_slugs_the_file_and_keeps_the_text() {
        let (_d, w) = well();
        let a = create_task(
            w.clone(),
            "todo".into(),
            "Fix login: OAuth expiry".into(),
            None,
        )
        .unwrap();
        assert_eq!(a, "fix-login-oauth-expiry");
        assert_eq!(
            find(&list_tasks(w.clone()), &a).title,
            "Fix login: OAuth expiry"
        );
        // A duplicate title is fine — the slug uniquifies.
        let b = create_task(
            w.clone(),
            "todo".into(),
            "Fix login: OAuth expiry".into(),
            None,
        )
        .unwrap();
        assert_eq!(b, "fix-login-oauth-expiry-2");
        assert_eq!(find(&list_tasks(w), &b).title, "Fix login: OAuth expiry");
    }

    #[test]
    fn move_sets_status_and_appends() {
        let (_d, w) = well();
        let a = create(&w, "todo");
        move_task(w.clone(), a.clone(), "doing".into()).unwrap();
        assert_eq!(find(&list_tasks(w), &a).status, "doing");
    }

    #[test]
    fn reorder_column_sets_order_and_status() {
        let (_d, w) = well();
        let a = create(&w, "todo");
        let b = create(&w, "doing");
        let c = create(&w, "todo");
        // Lay `todo` out as c, a, b — pulling b in from another column.
        reorder_column(
            w.clone(),
            "todo".into(),
            vec![c.clone(), a.clone(), b.clone()],
        )
        .unwrap();
        let tasks = list_tasks(w);
        assert_eq!(find(&tasks, &c).order, 0);
        assert_eq!(find(&tasks, &a).order, 1);
        assert_eq!(find(&tasks, &b).order, 2);
        assert_eq!(find(&tasks, &b).status, "todo");
    }

    #[test]
    fn fields_and_body_round_trip() {
        let (_d, w) = well();
        let a = create(&w, "todo");
        set_task_field(w.clone(), a.clone(), "priority".into(), "high".into()).unwrap();
        set_task_field(w.clone(), a.clone(), "tags".into(), "bug, ui".into()).unwrap();
        update_task_body(w.clone(), a.clone(), "## details\n\nrepro".into()).unwrap();
        let t = find(&list_tasks(w.clone()), &a).clone();
        assert_eq!(t.priority, "high");
        assert_eq!(t.tags, vec!["bug".to_string(), "ui".into()]);
        assert_eq!(t.body, "## details\n\nrepro");
        // Clearing a field removes it.
        set_task_field(w.clone(), a.clone(), "priority".into(), "".into()).unwrap();
        assert_eq!(find(&list_tasks(w), &a).priority, "");
    }

    #[test]
    fn rename_and_delete() {
        let (_d, w) = well();
        let a = create(&w, "todo");
        let new = rename_task(w.clone(), a, "Fix Login".into()).unwrap();
        assert_eq!(new, "fix-login");
        assert_eq!(find(&list_tasks(w.clone()), &new).title, "Fix Login");
        delete_task(w.clone(), new).unwrap();
        assert!(list_tasks(w).is_empty());
    }

    #[test]
    fn rename_uniquifies_and_keeps_suffixed_ids_stable() {
        let (_d, w) = well();
        let a = create(&w, "todo");
        let b = create(&w, "todo");
        rename_task(w.clone(), a, "Review PR".into()).unwrap();
        // Same title on a second task → the slug uniquifies instead of erroring.
        let b = rename_task(w.clone(), b, "Review PR".into()).unwrap();
        assert_eq!(b, "review-pr-2");
        // Re-committing the same title must not walk the -N suffix upward.
        let b2 = rename_task(w.clone(), b.clone(), "Review PR".into()).unwrap();
        assert_eq!(b2, b);
        // A punctuation-only title keeps the file name but stores the title.
        let b3 = rename_task(w.clone(), b2.clone(), "!!!".into()).unwrap();
        assert_eq!(b3, b2);
        assert_eq!(find(&list_tasks(w), &b3).title, "!!!");
    }

    #[test]
    fn goal_rename_and_delete_retarget_tasks() {
        let (_d, w) = well();
        fs::create_dir_all(goals_dir(&w)).unwrap();
        let g = create_goal(w.clone()).unwrap();
        let t = create(&w, "todo");
        set_task_field(w.clone(), t.clone(), "goal".into(), g.clone()).unwrap();
        assert_eq!(find(&list_tasks(w.clone()), &t).goal, "untitled");
        // Rename re-points the task's goal and stores the display title.
        let g2 = rename_goal(w.clone(), g, "Ship v1".into()).unwrap();
        assert_eq!(g2, "ship-v1");
        assert_eq!(list_goals(w.clone())[0].title, "Ship v1");
        assert_eq!(find(&list_tasks(w.clone()), &t).goal, "ship-v1");
        // Delete clears it.
        delete_goal(w.clone(), g2).unwrap();
        assert_eq!(find(&list_tasks(w.clone()), &t).goal, "");
        assert!(list_goals(w).is_empty());
    }

    #[test]
    fn delete_returns_content_and_restore_round_trips() {
        let (_d, w) = well();
        let a = create(&w, "todo");
        set_task_field(w.clone(), a.clone(), "priority".into(), "high".into()).unwrap();
        update_task_body(w.clone(), a.clone(), "keep me".into()).unwrap();
        // Delete hands back the raw file so the UI can offer undo.
        let content = delete_task(w.clone(), a.clone()).unwrap();
        assert!(list_tasks(w.clone()).is_empty());
        assert!(content.contains("priority: high"));
        assert!(content.contains("keep me"));
        // Restore brings it back verbatim (frontmatter + body).
        restore_task(w.clone(), a.clone(), content).unwrap();
        let t = find(&list_tasks(w), &a).clone();
        assert_eq!(t.priority, "high");
        assert_eq!(t.body, "keep me");
    }

    #[test]
    fn archived_flag_round_trips() {
        let (_d, w) = well();
        let t = create(&w, "todo");
        assert!(!find(&list_tasks(w.clone()), &t).archived);
        // Archive, then restore — via the generic field setter the UI uses.
        set_task_field(w.clone(), t.clone(), "archived".into(), "true".into()).unwrap();
        assert!(find(&list_tasks(w.clone()), &t).archived);
        set_task_field(w.clone(), t.clone(), "archived".into(), String::new()).unwrap();
        assert!(!find(&list_tasks(w), &t).archived);
    }

    #[test]
    fn reorder_goals_sets_order() {
        let (_d, w) = well();
        fs::create_dir_all(goals_dir(&w)).unwrap();
        let a = create_goal(w.clone()).unwrap();
        let b = create_goal(w.clone()).unwrap();
        let c = create_goal(w.clone()).unwrap();
        // Created goals append in order.
        let ids: Vec<String> = list_goals(w.clone()).iter().map(|g| g.id.clone()).collect();
        assert_eq!(ids, vec![a.clone(), b.clone(), c.clone()]);
        // Drag puts c first.
        reorder_goals(w.clone(), vec![c.clone(), a.clone(), b.clone()]).unwrap();
        let ids: Vec<String> = list_goals(w).iter().map(|g| g.id.clone()).collect();
        assert_eq!(ids, vec![c, a, b]);
    }

    #[test]
    fn move_into_done_stamps_completed() {
        let (_d, w) = well();
        let a = create(&w, "todo");
        // A not-yet-done task carries no completed date on its `Task`.
        assert_eq!(find(&list_tasks(w.clone()), &a).completed, "");
        move_task(w.clone(), a.clone(), "done".into()).unwrap();
        assert_eq!(find(&list_tasks(w.clone()), &a).status, "done");
        assert_eq!(fields_of(&w, &a).get("completed"), Some(&today()));
        // The stamp is now surfaced on the `Task` struct (not just the raw
        // frontmatter), for the table view's completed column.
        assert_eq!(find(&list_tasks(w), &a).completed, today());
    }

    #[test]
    fn moving_out_of_done_clears_completed() {
        let (_d, w) = well();
        let a = create(&w, "todo");
        move_task(w.clone(), a.clone(), "done".into()).unwrap();
        assert!(fields_of(&w, &a).contains_key("completed"));

        // Back to a named column.
        move_task(w.clone(), a.clone(), "todo".into()).unwrap();
        assert!(!fields_of(&w, &a).contains_key("completed"));

        // And out to the backlog (empty status).
        move_task(w.clone(), a.clone(), "done".into()).unwrap();
        assert!(fields_of(&w, &a).contains_key("completed"));
        move_task(w.clone(), a.clone(), "".into()).unwrap();
        assert!(!fields_of(&w, &a).contains_key("completed"));
    }

    #[test]
    fn reorder_column_stamps_on_transition_but_not_on_reorder_within_done() {
        let (_d, w) = well();
        let a = create(&w, "todo");
        let b = create(&w, "done");
        // Pretend b finished a while ago.
        set_task_field(
            w.clone(),
            b.clone(),
            "completed".into(),
            "2020-01-01".into(),
        )
        .unwrap();

        // Pull a into done (a transition) while merely reordering b within it
        // (no transition — b's status stays "done").
        reorder_column(w.clone(), "done".into(), vec![b.clone(), a.clone()]).unwrap();

        assert_eq!(fields_of(&w, &a).get("completed"), Some(&today()));
        // b's pre-existing stamp survives untouched.
        assert_eq!(
            fields_of(&w, &b).get("completed"),
            Some(&"2020-01-01".to_string())
        );
    }

    #[test]
    fn set_task_field_status_routes_through_apply_status() {
        let (_d, w) = well();
        let a = create(&w, "todo");
        set_task_field(w.clone(), a.clone(), "status".into(), "done".into()).unwrap();
        assert_eq!(find(&list_tasks(w.clone()), &a).status, "done");
        assert_eq!(fields_of(&w, &a).get("completed"), Some(&today()));

        set_task_field(w.clone(), a.clone(), "status".into(), "todo".into()).unwrap();
        assert!(!fields_of(&w, &a).contains_key("completed"));
    }

    #[test]
    fn create_task_optional_due() {
        let (_d, w) = well();
        let with_due = create_task(
            w.clone(),
            "todo".into(),
            "Ship it".into(),
            Some("2026-12-01".into()),
        )
        .unwrap();
        assert_eq!(find(&list_tasks(w.clone()), &with_due).due, "2026-12-01");

        let without_due = create_task(w.clone(), "todo".into(), "No due yet".into(), None).unwrap();
        assert_eq!(find(&list_tasks(w), &without_due).due, "");
    }

    /// Set a task's `repeat:` rule (the drawer's recurrence field).
    fn set_repeat(w: &str, id: &str, spec: &str) {
        set_task_field(w.to_string(), id.to_string(), "repeat".into(), spec.into()).unwrap();
    }

    #[test]
    fn move_into_done_spawns_next_occurrence() {
        let (_d, w) = well();
        let a = create_task(
            w.clone(),
            "todo".into(),
            "Water plants".into(),
            Some("2026-07-01".into()),
        )
        .unwrap();
        set_repeat(&w, &a, "weekly");
        set_task_field(w.clone(), a.clone(), "priority".into(), "high".into()).unwrap();
        set_task_field(w.clone(), a.clone(), "tags".into(), "home, chore".into()).unwrap();
        set_task_field(w.clone(), a.clone(), "goal".into(), "garden".into()).unwrap();
        update_task_body(w.clone(), a.clone(), "remember the balcony".into()).unwrap();

        move_task(w.clone(), a.clone(), "done".into()).unwrap();

        let tasks = list_tasks(w.clone());
        assert_eq!(tasks.len(), 2, "the next occurrence was spawned");

        // The completed file: done, stamped, and no longer recurring.
        let done = find(&tasks, &a);
        assert_eq!(done.status, "done");
        let done_fields = fields_of(&w, &a);
        assert_eq!(done_fields.get("completed"), Some(&today()));
        assert!(
            !done_fields.contains_key("repeat"),
            "repeat stripped on done"
        );

        // The spawned next occurrence.
        let next = tasks.iter().find(|t| t.id != a).unwrap();
        // Same title → the slug uniquifies against the completed file.
        assert_eq!(next.id, "water-plants-2");
        assert_eq!(next.status, "todo");
        assert_eq!(next.due, "2026-07-08"); // 2026-07-01 + 7 days
        assert_eq!(next.title, "Water plants");
        assert_eq!(next.priority, "high");
        assert_eq!(next.tags, vec!["home".to_string(), "chore".into()]);
        assert_eq!(next.goal, "garden");
        assert_eq!(next.repeat, "weekly");
        assert_eq!(next.body, "remember the balcony");
        // Appended to the (empty, now that `a` left) first column.
        assert_eq!(next.order, 0);
        let next_fields = fields_of(&w, &next.id);
        assert!(!next_fields.contains_key("completed"));
    }

    #[test]
    fn repeat_without_due_spawns_from_today() {
        let (_d, w) = well();
        let a = create_task(w.clone(), "todo".into(), "Standup".into(), None).unwrap();
        set_repeat(&w, &a, "daily");
        move_task(w.clone(), a.clone(), "done".into()).unwrap();

        let tasks = list_tasks(w.clone());
        let next = tasks.iter().find(|t| t.id != a).unwrap();
        // Empty due anchors to today; daily → today + 1.
        let expected = chrono::Local::now()
            .date_naive()
            .checked_add_days(chrono::Days::new(1))
            .unwrap()
            .format("%Y-%m-%d")
            .to_string();
        assert_eq!(next.due, expected);
    }

    #[test]
    fn non_done_transition_does_not_spawn() {
        let (_d, w) = well();
        let a = create_task(
            w.clone(),
            "todo".into(),
            "Chore".into(),
            Some("2026-07-01".into()),
        )
        .unwrap();
        set_repeat(&w, &a, "weekly");
        move_task(w.clone(), a.clone(), "planning".into()).unwrap();
        assert_eq!(list_tasks(w).len(), 1, "todo → planning must not spawn");
    }

    #[test]
    fn completing_twice_spawns_only_once() {
        let (_d, w) = well();
        let a = create_task(
            w.clone(),
            "todo".into(),
            "Chore".into(),
            Some("2026-07-01".into()),
        )
        .unwrap();
        set_repeat(&w, &a, "weekly");
        move_task(w.clone(), a.clone(), "done".into()).unwrap();
        assert_eq!(list_tasks(w.clone()).len(), 2);
        // Drag the completed card back out and in again — repeat was stripped,
        // so the second completion is inert.
        move_task(w.clone(), a.clone(), "todo".into()).unwrap();
        move_task(w.clone(), a.clone(), "done".into()).unwrap();
        assert_eq!(list_tasks(w).len(), 2, "no second spawn");
    }

    #[test]
    fn set_task_field_status_into_done_spawns() {
        let (_d, w) = well();
        let a = create_task(
            w.clone(),
            "todo".into(),
            "Chore".into(),
            Some("2026-07-01".into()),
        )
        .unwrap();
        set_repeat(&w, &a, "weekly");
        set_task_field(w.clone(), a.clone(), "status".into(), "done".into()).unwrap();
        let tasks = list_tasks(w.clone());
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks.iter().find(|t| t.id != a).unwrap().due, "2026-07-08");
    }

    #[test]
    fn reorder_column_into_done_spawns() {
        let (_d, w) = well();
        let a = create_task(
            w.clone(),
            "todo".into(),
            "Chore".into(),
            Some("2026-07-01".into()),
        )
        .unwrap();
        set_repeat(&w, &a, "weekly");
        // Pull `a` into the done column via a reorder (a genuine transition).
        reorder_column(w.clone(), "done".into(), vec![a.clone()]).unwrap();
        let tasks = list_tasks(w.clone());
        assert_eq!(tasks.len(), 2);
        assert_eq!(find(&tasks, &a).status, "done");
        let next = tasks.iter().find(|t| t.id != a).unwrap();
        assert_eq!(next.status, "todo");
        assert_eq!(next.due, "2026-07-08");
    }

    #[test]
    fn non_repeat_task_into_done_does_not_spawn() {
        let (_d, w) = well();
        let a = create_task(
            w.clone(),
            "todo".into(),
            "One off".into(),
            Some("2026-07-01".into()),
        )
        .unwrap();
        move_task(w.clone(), a.clone(), "done".into()).unwrap();
        assert_eq!(list_tasks(w).len(), 1, "no repeat → no spawn");
    }

    #[test]
    fn count_checks_counts_ticked_and_total() {
        let body = "\
- [x] one
- [ ] two
- [x] three
- [ ] four
- [x] five
- [ ] six
- [ ] seven";
        assert_eq!(count_checks(body), (3, 7));
    }

    #[test]
    fn count_checks_ignores_fenced_code() {
        let body = "\
```
- [ ] not a task
- [x] also not a task
```

- [x] real task";
        assert_eq!(count_checks(body), (1, 1));
    }

    #[test]
    fn count_checks_zero_when_no_checklists() {
        assert_eq!(count_checks("just some prose\n\nand a paragraph"), (0, 0));
        assert_eq!(count_checks(""), (0, 0));
    }

    #[test]
    fn count_checks_counts_nested_items() {
        let body = "\
- [x] parent
  - [ ] nested child
  - [x] another nested child
- [ ] sibling";
        // 4 markers total; parent + "another nested child" are ticked.
        assert_eq!(count_checks(body), (2, 4));
    }

    #[test]
    fn list_tasks_populates_checks_from_body() {
        let (_d, w) = well();
        let a = create(&w, "todo");
        update_task_body(
            w.clone(),
            a.clone(),
            "- [x] one\n- [ ] two\n- [x] three".into(),
        )
        .unwrap();
        let t = find(&list_tasks(w), &a).clone();
        assert_eq!((t.checks_done, t.checks_total), (2, 3));
    }

    #[test]
    fn garbage_repeat_into_done_does_not_spawn_and_keeps_repeat() {
        let (_d, w) = well();
        let a = create_task(
            w.clone(),
            "todo".into(),
            "Chore".into(),
            Some("2026-07-01".into()),
        )
        .unwrap();
        set_repeat(&w, &a, "whenever-i-feel-like-it");
        move_task(w.clone(), a.clone(), "done".into()).unwrap();
        // No spawn, and nothing lost: the rule survives on the completed file.
        assert_eq!(list_tasks(w.clone()).len(), 1);
        let fields = fields_of(&w, &a);
        assert_eq!(
            fields.get("repeat"),
            Some(&"whenever-i-feel-like-it".to_string())
        );
        // Still completes normally.
        assert_eq!(find(&list_tasks(w.clone()), &a).status, "done");
        assert_eq!(fields.get("completed"), Some(&today()));
    }

    // --- auto-archive sweep (P2.4) ------------------------------------

    /// Set the well's `archive_done_after_days`, bypassing
    /// `wells::set_archive_days`'s own immediate sweep — so these tests
    /// exercise [`sweep_archive`] directly, in isolation.
    fn set_archive_setting(w: &str, days: Option<u32>) {
        let mut manifest = crate::wells::read_manifest(w);
        manifest.archive_done_after_days = days;
        let path = std::path::Path::new(w).join(".ido").join("well.toml");
        fs::write(path, toml::to_string(&manifest).unwrap()).unwrap();
    }

    #[test]
    fn sweep_archives_stale_completed_done_tasks() {
        let (_d, w) = well();
        crate::wells::migrate_well(w.clone()).unwrap();
        let a = create(&w, "done");
        set_task_field(
            w.clone(),
            a.clone(),
            "completed".into(),
            "2020-01-01".into(),
        )
        .unwrap();
        set_archive_setting(&w, Some(30));

        sweep_archive(&w);

        assert!(
            find(&list_tasks(w), &a).archived,
            "a month-old done task is archived"
        );
    }

    #[test]
    fn sweep_keeps_recently_completed_tasks() {
        let (_d, w) = well();
        crate::wells::migrate_well(w.clone()).unwrap();
        let a = create(&w, "done");
        set_task_field(w.clone(), a.clone(), "completed".into(), today()).unwrap();
        set_archive_setting(&w, Some(30));

        sweep_archive(&w);

        assert!(
            !find(&list_tasks(w), &a).archived,
            "a task completed today is kept"
        );
    }

    #[test]
    fn sweep_leaves_already_archived_tasks_untouched() {
        let (_d, w) = well();
        crate::wells::migrate_well(w.clone()).unwrap();
        let a = create(&w, "done");
        set_task_field(
            w.clone(),
            a.clone(),
            "completed".into(),
            "2020-01-01".into(),
        )
        .unwrap();
        set_task_field(w.clone(), a.clone(), "archived".into(), "true".into()).unwrap();
        set_archive_setting(&w, Some(30));

        sweep_archive(&w);
        sweep_archive(&w); // idempotent by construction — re-running changes nothing

        assert!(find(&list_tasks(w), &a).archived);
    }

    #[test]
    fn sweep_disabled_moves_nothing() {
        let (_d, w) = well();
        crate::wells::migrate_well(w.clone()).unwrap();
        let a = create(&w, "done");
        set_task_field(
            w.clone(),
            a.clone(),
            "completed".into(),
            "2020-01-01".into(),
        )
        .unwrap();
        // `archive_done_after_days` left at its default `None` — off.

        sweep_archive(&w);

        assert!(
            !find(&list_tasks(w), &a).archived,
            "the setting being off means nothing moves"
        );
    }

    #[test]
    fn sweep_keeps_unstamped_task_with_fresh_mtime() {
        let (_d, w) = well();
        crate::wells::migrate_well(w.clone()).unwrap();
        // No `completed:` stamp (a pre-P0 task) — falls back to the file's
        // mtime, which is "now" since it was just created.
        let a = create(&w, "done");
        set_archive_setting(&w, Some(30));

        sweep_archive(&w);

        assert!(
            !find(&list_tasks(w), &a).archived,
            "a fresh mtime keeps the conservative fallback from archiving"
        );
    }

    #[test]
    fn completion_date_prefers_completed_field_then_falls_back_to_mtime() {
        let mtime = chrono::NaiveDate::from_ymd_opt(2020, 6, 15).unwrap();
        assert_eq!(
            completion_date("2026-01-02", mtime),
            chrono::NaiveDate::from_ymd_opt(2026, 1, 2).unwrap()
        );
        // Missing or unparseable `completed:` falls back to `mtime`.
        assert_eq!(completion_date("", mtime), mtime);
        assert_eq!(completion_date("not-a-date", mtime), mtime);
    }
}
