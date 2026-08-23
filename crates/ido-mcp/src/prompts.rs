//! MCP prompts (§5.4): two read-only prompts, thin composites over the
//! existing tool bodies.
//!
//! `daily_review` (overdue tasks, tasks due today, and recently-touched
//! notes) and `weekly_digest` (what changed in the last 7 days, and which
//! goals moved) are, per §5.4, "cheap to add once the tools exist" — and this
//! takes that literally: each gathers the well's current state into a single
//! user message and hands it to the client's own model to reason over. No
//! second date/filter implementation lives here — just `tasks::list_tasks` /
//! `list_goals`, the date handling `tools.rs` / `render.rs` already have
//! ([`due_date`], [`iso_date`]), and [`crate::resources`]'s recency sweep for
//! "what changed". "Today" is `chrono::Local`, the same clock
//! `ido_store::tasks` stamps `completed:` with — reimplemented as [`today`]
//! here only because that helper is private to the store.
//!
//! Both are pure reads: like every tool in this crate, nothing here ever
//! writes.

use std::fmt::Write as _;

use chrono::{Duration, Local};
use ido_store::model::Task;
use ido_store::tasks;
use rmcp::model::{GetPromptResult, Prompt, PromptMessage, Role};

use crate::render::{cell, dash, header, iso_date, row};
use crate::resources;
use crate::tools::{due_date, status_of};

/// How many recently-touched notes `daily_review` lists, and how many
/// changed entries `weekly_digest` lists. Both prompts are "orient yourself"
/// summaries, not full audits, so this stays small — §5.1's "bound every
/// response" applies just as much to a prompt's embedded data as to a tool
/// result.
const RECENT_LIMIT: usize = 10;

/// Today's local date as `YYYY-MM-DD` — the same clock and format
/// `ido_store::tasks::today_ymd` uses to stamp `completed:`. That helper is
/// private to the store, so this is the crate's own copy of the one-liner
/// rather than a second *policy* (there is no other place "today" is decided
/// differently).
fn today() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

/// The two prompts this server advertises, for `prompts/list`. Neither takes
/// arguments: both operate on the whole well, which is already fixed for the
/// life of the process (§3.4 — one well per server).
pub(crate) fn list() -> Vec<Prompt> {
    vec![
        Prompt::new(
            "daily_review",
            Some(
                "Overdue tasks, tasks due today, and recently-touched notes — call this at the \
                 start of a session to see what needs attention right now.",
            ),
            None,
        ),
        Prompt::new(
            "weekly_digest",
            Some(
                "What changed across notes, wiki, tasks and goals in the last 7 days, and which \
                 goals gained progress — call this for a \"what happened this week\" summary.",
            ),
            None,
        ),
    ]
}

/// `prompts/get`'s body: dispatch by name. There is no third prompt to fall
/// back to, so anything else is a clear refusal rather than a default.
pub(crate) fn get(well: &str, name: &str) -> Result<GetPromptResult, String> {
    match name {
        "daily_review" => Ok(daily_review(well)),
        "weekly_digest" => Ok(weekly_digest(well)),
        other => Err(format!(
            "unknown prompt `{other}` — this server has two: daily_review, weekly_digest"
        )),
    }
}

/// Whether `task` counts as overdue against `today`: not archived, not sitting
/// in the done column (leaving the board isn't "overdue"), it has a due date,
/// and that date is before today. Mirrors the rail's own overdue-badge
/// convention (`components/tasks/logic.rs`'s `overdue_count`): unfiltered,
/// done-column and archived excluded, compared on the 10-char date prefix so
/// a due time never breaks the comparison.
fn is_overdue(task: &Task, done_column: &str, today: &str) -> bool {
    !task.archived
        && task.status != done_column
        && !task.due.is_empty()
        && due_date(&task.due) < today
}

/// Whether `task` is due today, by the same exclusions as [`is_overdue`].
fn is_due_today(task: &Task, done_column: &str, today: &str) -> bool {
    !task.archived
        && task.status != done_column
        && !task.due.is_empty()
        && due_date(&task.due) == today
}

/// One task row: id, title, status, priority, due.
fn task_row(out: &mut String, task: &Task) {
    row(
        out,
        &[
            format!("`{}`", cell(&task.id)),
            cell(&task.title),
            cell(status_of(task)),
            cell(&task.priority),
            cell(&task.due),
        ],
    );
}

/// `daily_review`: overdue tasks, tasks due today, and recently-touched
/// notes, framed as a request for the client's own model to triage.
fn daily_review(well: &str) -> GetPromptResult {
    let today = today();
    let columns = tasks::task_columns(well.to_string());
    let done_column = columns.last().cloned().unwrap_or_default();
    let all_tasks = tasks::list_tasks(well.to_string());
    let overdue: Vec<&Task> = all_tasks
        .iter()
        .filter(|t| is_overdue(t, &done_column, &today))
        .collect();
    let due_today: Vec<&Task> = all_tasks
        .iter()
        .filter(|t| is_due_today(t, &done_column, &today))
        .collect();
    let recent_notes = resources::recent_notes(well, RECENT_LIMIT);

    let mut out = format!(
        "Give me my daily review for {today}. Here is the well's current state — triage it: \
         call out anything overdue first, then what's due today, then anything in the \
         recently-touched notes worth following up on.\n\n"
    );

    let _ = writeln!(out, "## Overdue ({})\n", overdue.len());
    if overdue.is_empty() {
        let _ = writeln!(out, "Nothing overdue.\n");
    } else {
        header(&mut out, &["id", "title", "status", "priority", "due"]);
        for task in &overdue {
            task_row(&mut out, task);
        }
        let _ = writeln!(out);
    }

    let _ = writeln!(out, "## Due today ({})\n", due_today.len());
    if due_today.is_empty() {
        let _ = writeln!(out, "Nothing due today.\n");
    } else {
        header(&mut out, &["id", "title", "status", "priority", "due"]);
        for task in &due_today {
            task_row(&mut out, task);
        }
        let _ = writeln!(out);
    }

    let _ = writeln!(out, "## Recently touched notes ({})\n", recent_notes.len());
    if recent_notes.is_empty() {
        let _ = writeln!(out, "No notes to show.\n");
    } else {
        for (id, title) in &recent_notes {
            let _ = writeln!(out, "- **{title}** (`{id}`)");
        }
        let _ = writeln!(out);
    }

    let _ = writeln!(
        out,
        "Call get_entry (kind=task or kind=note) on any id above for its full detail."
    );

    GetPromptResult::new(vec![PromptMessage::new_text(Role::User, out)])
        .with_description("Overdue tasks, tasks due today, and recently-touched notes.")
}

/// `weekly_digest`: everything modified in the last 7 days across every kind,
/// and which goals gained progress in that window.
fn weekly_digest(well: &str) -> GetPromptResult {
    let today = today();
    let week_ago = Local::now() - Duration::days(7);
    let cutoff_date = week_ago.format("%Y-%m-%d").to_string();
    let cutoff_ms = week_ago.timestamp_millis().max(0) as u64;

    let changed = resources::recently_modified(well, cutoff_ms, RECENT_LIMIT);

    let columns = tasks::task_columns(well.to_string());
    let done_column = columns.last().cloned().unwrap_or_default();
    let all_tasks = tasks::list_tasks(well.to_string());
    let goals: Vec<_> = tasks::list_goals(well.to_string())
        .into_iter()
        .filter(|g| !g.archived)
        .collect();

    let mut out = format!(
        "Give me my weekly digest for the 7 days ending {today}. Here is the well's current \
         state — summarize what changed and call out any goal that moved.\n\n"
    );

    let _ = writeln!(out, "## Changed since {cutoff_date} ({})\n", changed.len());
    if changed.is_empty() {
        let _ = writeln!(out, "Nothing changed in the last 7 days.\n");
    } else {
        header(&mut out, &["kind", "id", "title", "modified"]);
        for (kind, id, title, mtime_ms) in &changed {
            row(
                &mut out,
                &[
                    cell(kind.as_str()),
                    format!("`{}`", cell(id)),
                    cell(title),
                    cell(&iso_date(Some(*mtime_ms))),
                ],
            );
        }
        let _ = writeln!(out);
    }

    let _ = writeln!(out, "## Goals ({})\n", goals.len());
    if goals.is_empty() {
        let _ = writeln!(out, "This well has no goals.\n");
    } else {
        header(
            &mut out,
            &["id", "title", "target", "progress", "done this week"],
        );
        for goal in &goals {
            let linked: Vec<&Task> = all_tasks
                .iter()
                .filter(|t| t.goal == goal.id && !t.archived)
                .collect();
            let done = linked.iter().filter(|t| t.status == done_column).count();
            let moved = linked
                .iter()
                .filter(|t| {
                    !t.completed.is_empty() && due_date(&t.completed) >= cutoff_date.as_str()
                })
                .count();
            row(
                &mut out,
                &[
                    format!("`{}`", cell(&goal.id)),
                    cell(&goal.title),
                    cell(dash(&goal.target)),
                    format!("{done}/{}", linked.len()),
                    moved.to_string(),
                ],
            );
        }
        let _ = writeln!(out);
    }

    let _ = writeln!(
        out,
        "Call list_tasks goal=<id> or get_entry kind=goal for detail on any goal above."
    );

    GetPromptResult::new(vec![PromptMessage::new_text(Role::User, out)])
        .with_description("What changed in the last 7 days, and which goals moved.")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A task with just the fields these filters look at; everything else is
    /// a placeholder so the struct literal stays short at every call site.
    fn task(id: &str, status: &str, due: &str, archived: bool, completed: &str) -> Task {
        Task {
            id: id.to_string(),
            title: id.to_string(),
            status: status.to_string(),
            priority: String::new(),
            tags: Vec::new(),
            order: 0,
            due: due.to_string(),
            goal: String::new(),
            repeat: String::new(),
            archived,
            completed: completed.to_string(),
            checks_done: 0,
            checks_total: 0,
            body: String::new(),
        }
    }

    #[test]
    fn overdue_excludes_archived_done_and_undated() {
        let (done, today) = ("done", "2026-08-23");
        assert!(is_overdue(
            &task("a", "todo", "2026-08-01", false, ""),
            done,
            today
        ));
        assert!(
            !is_overdue(&task("a", "todo", "2026-08-01", true, ""), done, today),
            "archived tasks are never overdue"
        );
        assert!(
            !is_overdue(
                &task("a", "done", "2026-08-01", false, "2026-08-20"),
                done,
                today
            ),
            "the done column is never overdue"
        );
        assert!(
            !is_overdue(&task("a", "todo", "", false, ""), done, today),
            "an undated task has nothing to be overdue against"
        );
        assert!(
            !is_overdue(&task("a", "todo", "2026-08-23", false, ""), done, today),
            "due today isn't overdue yet"
        );
        assert!(
            !is_overdue(&task("a", "todo", "2026-08-24", false, ""), done, today),
            "a future due date isn't overdue"
        );
    }

    #[test]
    fn due_today_ignores_the_time_of_day_and_excludes_the_done_column() {
        let (done, today) = ("done", "2026-08-23");
        assert!(is_due_today(
            &task("a", "todo", "2026-08-23 09:00", false, ""),
            done,
            today
        ));
        assert!(
            !is_due_today(
                &task("a", "done", "2026-08-23", false, "2026-08-23"),
                done,
                today
            ),
            "the done column doesn't need chasing"
        );
        assert!(!is_due_today(
            &task("a", "todo", "2026-08-22", false, ""),
            done,
            today
        ));
    }

    #[test]
    fn the_two_prompts_are_named_and_described_without_arguments() {
        let prompts = list();
        assert_eq!(prompts.len(), 2);
        assert_eq!(prompts[0].name, "daily_review");
        assert_eq!(prompts[1].name, "weekly_digest");
        for p in &prompts {
            assert!(
                p.description.as_deref().unwrap_or_default().len() > 20,
                "`{}` needs a real description",
                p.name
            );
            assert!(p.arguments.is_none(), "neither prompt takes arguments");
        }
    }

    #[test]
    fn get_refuses_an_unknown_prompt_name() {
        // Dispatch happens before any disk access, so a nonexistent well
        // still proves the refusal (mirrors `write::tests`' same trick).
        let err = get("./definitely-not-a-well-xyz", "nope").unwrap_err();
        assert!(err.contains("daily_review"));
        assert!(err.contains("weekly_digest"));
    }
}
