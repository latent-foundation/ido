# calendar integration — design

*Status: proposal · 2026-07-08 · scope: the tasks section (tasks + goals)*

The tasks section stores dates (`due:` on tasks, `target:` on goals) but has no surface where
time is the axis — you can sort a column by due date, and cards flag `overdue` / `due-today`,
but there is no answer to "what does my week look like?" without scanning every column. This
document details how ido grows a calendar: an in-app calendar view first (pure frontend, no
new data), then optional due times, recurring tasks, an ICS feed out, and read-only external
calendars in. Each phase is independently shippable and useful on its own.

Conventions throughout (per the latent. canon and ido's existing choices): **Monday-first
weeks**, **day-first dates** ("7 Jul", via the existing `format_due`), **24-hour times**,
Lucide icons, tokens via `var(--…)` only.

---

## 1. Why a calendar, and why this shape

Three jobs, in priority order:

1. **See the load** — a month at a glance: which days are stacked, what's overdue, where the
   goal targets land. This is the #1 Notion-parity gap for task tracking.
2. **Reschedule by dragging** — moving a card between days should be as cheap as moving it
   between columns.
3. **Coexist with real calendars** — ido is local-first and single-user; it should *feed*
   the user's actual calendar (ICS out) and *show* it for context (ICS in), not try to become
   a groupware client. Two-way sync is explicitly deferred (§8).

Non-goals: time-blocking / day-planner UI, meeting scheduling, attendees/invites, full RRULE
authoring, Google OAuth sync.

---

## 2. What we build on (today's data model)

- `Task.due: String` — `YYYY-MM-DD` or empty; set via the drawer's `DatePicker`; compared
  as text against `today_ymd()` in `logic.rs::due_state`.
- `Goal.target: String` — same format.
- The whole board's data is already client-side: `state.tasks` / `state.goals` are loaded in
  full by `list_tasks` / `list_goals`. **Phase 1 therefore needs zero backend work.**
- Date math already exists in `components/datepicker.rs` (`weekday`, `days_in_month`,
  `parse_ymd`, Monday-first `WEEKDAYS`) — it just isn't shared.
- Per-well config lives in `.ido/well.toml` (`wells::read_manifest`), already the home of
  `columns`; rebuildable artifacts belong under `.ido/`.

---

## 3. Phase 1 — the in-app calendar view

### Placement

A **view toggle in the existing tasks toolbar**: `board | calendar` (Lucide `calendar-days`),
to the left of the archive button. Not a fourth rail section — it's the same data, a different
axis, and keeping it inside `TaskBoard`'s wrap means the existing `TaskDrawer` / `GoalDrawer`
overlays, the goal filter, the text filter, and `hide done` all keep working with **no
changes**. Sort and the backlog don't apply in calendar view (the toolbar hides the sort
control there; backlog tasks appear only if they have a due date).

State: `TaskView { Board, Calendar }` signal on `State`, plus `cal_month: RwSignal<(i32, u32)>`
(year, month0). Session-local; not persisted (opening on the current month is always right).

### Month grid

- 7 columns **Mo–Su**, 5–6 rows covering the month; leading/trailing days from adjacent
  months render dimmed but live (chips still show — a due date on the 1st shouldn't hide
  because the month started on a Wednesday).
- Header: `‹ · month year · ›` plus a **today** button that jumps back and highlights today's
  cell (accent ring, same treatment as the datepicker's today).
- Day cell contents, top to bottom:
  - **goal targets** — a thin band with the `target` icon + goal title; `overdue` accent when
    `goal_overdue` (reuse). Click → `open_goal`.
  - **task chips** — title (single line, ellipsized) with the card's priority class for the
    left border / dot, `overdue` / `due-today` accents from `due_state` (done-column tasks
    render muted, like the board's done column).
  - **`+N more`** when a cell overflows (cap ~4 chips) → opens the **day popover**.
- **Day popover** (click a day number or `+N more`): the full list for that day, plus a
  quick-add input (same component pattern as `QuickAdd`) that creates a task **with that due
  date**, in the first column. Reuses `add_task_quick` + one `set_task_field(due)` — or a
  small `create_task` extension that takes an optional `due` to make it one write.

### Interactions

- **Click chip** → `open_task` (drawer slides in over the calendar, exactly as on the board).
- **Drag chip → another day cell** → `set_task_field(id, "due", ymd)`. HTML5 drag already
  works in-webview (`dragDropEnabled: false`); reuse the board's pattern — a `task_drag`
  signal, `dragover` sets a hovered-day signal for the highlight, `drop` commits. No
  within-day ordering (days are sets, not sequences).
- **Drag chip → an "unschedule" affordance** (small tray in the grid footer, mirroring the
  backlog's role) → clears `due`. Optional; cheap.
- Right-click chip → the existing `MenuTarget::Task` context menu.

### What renders, and the overdue question

Filter pipeline identical to the board: not archived, `matches(task, filter)`, `goal_ok`.
An **overdue task shows on its printed date** (in the past), which is honest but easy to
scroll past — so the header also shows a quiet `overdue N` chip (clock icon, accent) whenever
N > 0; clicking it opens a popover listing overdue tasks (same rows as the day popover).
This replaces scanning backward through months.

### Refactor that makes it clean

Move `weekday` / `days_in_month` / `is_leap` / `parse_ymd` / `WEEKDAYS` / month-name tables
out of `components/datepicker.rs` into a shared **`src/dates.rs`** (host-testable, tests move
with them; add `add_days` / `add_months` — needed by drag-reschedule display, recurrence in
§5, and the grid itself). `datepicker.rs` and `tasks/logic.rs` both import it. New component
file: `src/components/tasks/calendar.rs` (grid + day popover + overdue popover), wired in
`tasks/mod.rs` and rendered by `TaskBoard` when `task_view == Calendar`.

**Estimate: M.** One component file, one shared-module refactor, ~zero backend.

---

## 4. Phase 1.5 — optional due times

Date-only is right for most tasks, but ICS export (§6) and "due at 14:00" tasks want an
optional time. Smallest change that stays text-sortable:

- **Format:** `due: YYYY-MM-DD HH:MM` (24 h), the time part optional. Lexicographic order
  still equals chronological order, and mixed date-only/timed values sort sanely
  (`2026-07-08` < `2026-07-08 09:00` < `2026-07-08 14:00` < `2026-07-09`).
- **One trap to codify:** `due_state` compares the **first 10 chars** (the date prefix)
  against `today_ymd()` — comparing the full string would make a timed task today read as
  `overdue`/`""` incorrectly. Add this as a unit test before anything else.
- `format_due` renders "7 Jul 14:00" (time appended when present; year elision unchanged).
- `DatePicker` grows an optional time field (a plain `HH:MM` input under the grid — don't
  build a clock widget).
- No timezone stored: values are **floating local time**, correct for a single-machine,
  single-user app. (§6 notes what that means for export.)

**Estimate: S.** Touches `dates.rs`, `logic.rs` (+tests), `datepicker.rs`.

---

## 5. Phase 2 — recurring tasks (`repeat:`)

A calendar without recurrence stays half-useful — chores and reviews are the highest-volume
personal tasks. Keep the vocabulary deliberately small (no RRULE authoring):

```
repeat: daily | weekly | monthly | yearly | every N days | every N weeks | every N months
```

**Semantics — spawn-on-done:**

- When a task with `repeat:` **enters the done column** (any path: `move_task`,
  `reorder_column`, `set_task_field("status", …)` — centralize in one backend helper called
  after any status write), the backend:
  1. computes `next_due = advance(due, repeat)` — from the *printed* due, not the completion
     date, so a weekly task stays anchored to its weekday; if `due` is empty, from today;
  2. creates the next occurrence: same title (slug uniquifies to `-N`), same frontmatter
     (priority/tags/goal/`repeat`), body copied, `status` = first column, `due` = next_due,
     order appended;
  3. **removes `repeat:` from the completed file.** The done task becomes plain history, and
     the rule can't double-fire — re-dragging the old card to done is inert. This also makes
     the operation idempotent without any bookkeeping.
- Month math clamps (31 Jan + 1 month → 28/29 Feb) — lives in `dates.rs`… backend needs it
  too, so the canonical copy goes in `src-tauri` (`paths`-style pure module with tests) and
  the frontend `dates.rs` mirrors only what the grid needs. (The two crates don't share code
  today; mirroring 30 lines of date math with the same test table is cheaper than a shared
  crate — note it in both files.)
- The drawer grows a `repeat` field (a `select` like priority). Cards and calendar chips get
  a tiny `repeat` icon (Lucide `repeat`, 10px) after the title.
- Undo: the spawn rides the normal task-creation path; deleting the spawned task is the
  existing soft-delete.

**Estimate: M.** Backend helper + `advance` (+ table tests: daily/weekly/every-N/month-clamp/
yearly/leap), drawer field, two icon call-sites.

---

## 6. Phase 3 — ICS export (ido → real calendars)

Generate a standards-correct `.ics` so the user's actual calendar shows ido's dated work.

- **Output:** `<well>/.ido/calendar.ics`, regenerated **after every task/goal write** when
  `calendar.export = true` in `well.toml` (writing ~hundreds of VEVENTs is sub-millisecond;
  no debounce needed). A settings row shows the toggle + the file path with a copy button.
- **What exports:** non-archived tasks **with a due date** and status ≠ done (a
  `calendar.export_done` toggle can include completed ones later), plus goals with a
  `target`. Backlog counts — a dated backlog task is still dated.
- **VEVENT, not VTODO.** Google and most calendar UIs hide or cripple VTODOs; events render
  everywhere. Date-only dues become **all-day events** (`DTSTART;VALUE=DATE:20260708`,
  `DTEND` = next day, exclusive per RFC 5545); timed dues (§4) become 1-hour floating-local
  events (no `TZID` — matches the floating semantics of the store; if this ever syncs across
  machines in different zones, revisit with `TZID` from the OS).
- **Field mapping:** `SUMMARY` = title · `DESCRIPTION` = body · `CATEGORIES` = tags ·
  `PRIORITY` = high→1, normal→5, low→9 · goal exports as `CATEGORIES:goal:<slug>` on the
  task and its own all-day VEVENT for the target.
- **UID stability — the one real gotcha.** A task's file id **re-slugs on rename**
  (`rename_task`), so ids can't be UIDs or every rename duplicates the event in subscribed
  calendars. On first export, any exported task/goal missing a `uid:` frontmatter key gets
  one **written back into its file** (8 random hex + `@ido`); `frontmatter::merge` already
  preserves unknown keys everywhere else, so the only code that learns about `uid:` is the
  exporter.
- **Byte-stable output:** `DTSTAMP` / `LAST-MODIFIED` from the file's mtime (not "now"), so
  regenerating without changes produces an identical file — keeps file-watching subscribers
  (and the user's diff of `.ido/`) quiet.
- Use the **`icalendar` crate** (generation now, parsing for §7 via its `parser` feature) —
  it handles escaping, CRLF, and 75-octet line folding, the three things hand-rolled ICS
  always gets wrong. Backend module `src-tauri/src/ics.rs` + `export_calendar` command +
  regenerate hooks in `tasks.rs` write paths.
- **Honest consumer notes** (belongs in the settings UI copy, one line): Thunderbird and
  most desktop apps subscribe to a file path and pick up changes; Outlook opens/imports it;
  **Google Calendar only subscribes to URLs**, so for Google this is a manual re-import
  unless/until a tiny localhost feed server is added (deferred — one `GET /calendar.ics`
  endpoint, still local-only; cheap, but wait for demand).

**Estimate: M.** The crate does the format; the work is UID write-back + regenerate hooks +
tempdir tests (uid persistence, all-day boundaries, escaping round-trip, mtime stability).

---

## 7. Phase 4 — external calendars in (read-only)

Show the user's real calendar *inside* the month view, as context around the tasks — never
as files, never editable.

- **Config:** `calendar.feeds = ["https://…ics", …]` in `well.toml`; settings gets a
  feeds editor (same pattern as the columns editor).
- **Backend:** async `fetch_feeds` command — `reqwest` GET per URL, parse with `icalendar`'s
  parser, expand recurring events with the **`rrule` crate** over a bounded window (shown
  month ± 1 year — never unbounded; feeds love `FREQ=YEARLY` with no `UNTIL`), and return
  flat `ExternalEvent { title, date, time?, feed }` rows. Cache the parsed result under
  `.ido/feeds/<url-hash>.json` with a fetched-at stamp; serve cache when offline or fresher
  than the TTL (default 6 h), refetch on the calendar view's manual refresh button.
- **Rendering:** a distinct quiet chip style (muted, dashed left border, `calendar` icon),
  sorted above task chips, not draggable, no context menu; click → a small read-only popover
  (title, time, source feed). They never enter `state.tasks`, search, or export (no loops:
  events ido exported and re-imported via a user's aggregated feed are deduped by dropping
  incoming UIDs ending `@ido`).
- **Privacy note for the README:** this is the first feature that makes network requests —
  only to user-entered URLs, only on open/refresh, cached locally. Say so explicitly; it's
  an ecosystem-values point, not a technicality.

**Estimate: M–L** (the long tail is feed weirdness; the `rrule`+`icalendar` pairing and a
strict expansion window contain it).

---

## 8. Explicitly deferred

- **Two-way sync (CalDAV / Google API).** Requires conflict resolution, etag bookkeeping,
  and OAuth token storage — all three cut against the local-first, no-accounts posture. The
  right vehicle is the planned **web sync** bet; revisit there. ICS out + ICS in covers the
  daily need (see your work in your calendar; see your calendar in your work) without any of it.
- **OS notifications** ("due in 30 min") via `tauri-plugin-notification` — adjacent but a
  separate design (quiet hours, snooze, per-task opt-in). The calendar view + an overdue
  chip must not depend on it.
- **Cross-well aggregate calendar** — wells are isolated by design; an aggregate surface is
  MCP/web-sync territory.
- **Week/day view with an hour grid** — only earns its keep once timed dues (§4) see real
  use; month + day-popover covers the jobs in §1.

---

## 9. Implementation inventory

| Piece | Where | New/changed |
|---|---|---|
| `TaskView` toggle, `cal_month`, drag-day signals | `src/state/types.rs` + `tasks.rs` | changed |
| Calendar grid + day/overdue popovers | `src/components/tasks/calendar.rs` | new |
| Shared date math (Mo-first, add_days/months) | `src/dates.rs` (from `datepicker.rs`) | new (moved) |
| Backend date math + `advance(due, repeat)` | `src-tauri/src/dates.rs` | new |
| Recurrence spawn-on-done hook | `src-tauri/src/tasks.rs` | changed |
| ICS generation + UID write-back | `src-tauri/src/ics.rs` (`icalendar` crate) | new |
| Feed fetch/parse/cache | `src-tauri/src/feeds.rs` (`reqwest`, `rrule`) | new |
| Commands: `export_calendar`, `fetch_feeds`, `set_calendar_config` | `lib.rs` `generate_handler!` + `src/ipc.rs` | changed |
| `well.toml`: `[calendar] export / export_done / feeds` | `src-tauri/src/wells.rs` manifest | changed |
| Settings rows (toggle, path, feeds editor) | `src/components/settings.rs` | changed |
| Toolbar view toggle | `src/components/tasks/board.rs` | changed |
| Drawer `repeat` field + repeat icons | `drawers.rs`, `board.rs`, `calendar.rs` | changed |

Test surface (all host-runnable, matching the existing suites): date-math tables (both
crates), `due_state` date-prefix rule with timed dues, `advance` recurrence table
(incl. month-clamp + leap), chip bucketing by day, ICS tempdir round-trips (uid write-back,
all-day boundaries, byte-stability), feed cache TTL.

Suggested order: **§3 → §5 → §6 → §4 → §7** — the view proves the surface, recurrence makes
it load-bearing, export makes it visible outside, times and feeds are refinements.

## 10. Open questions

1. Day-popover quick-add: first column (proposed — it's triaged work) or backlog?
2. Should `hide done` also hide done chips on the calendar (proposed: yes, same toggle)?
3. Export horizon: everything dated, or clamp to ±1 year to keep feeds lean (proposed: everything; personal scale is small)?
4. Does the goals bar render in calendar view (proposed: yes — scoping the month by goal is exactly the "see the load" job)?
