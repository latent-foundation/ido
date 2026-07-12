# tasks roadmap — P1 / P2 working document

*Status: living document · 2026-07-09 · companion to [calendar-integration.md](calendar-integration.md)*

The P0 batch (calendar view, recurrence, `completed:` stamp, rail overdue badge) has landed.
This document is the working reference for the next two batches. Each item is sized to be **one
subagent run**: scoped files, tests, acceptance criteria, and a suggested model tier
(haiku = trivial/mechanical, sonnet = standard, opus = large or correctness-critical). Runs are
sequential — parallel agents race each other through `just fmt`, which formats the whole workspace.

**Definition of done for every item:** `just verify` passes (fmt-check + clippy `-D warnings` +
backend tests), `cargo test -p ido-ui` passes when frontend logic changed, formatted only via
`just fmt` (never bare `cargo fmt`), and UI items get a visual pass in the running app (`just dev`
/ the `run` skill) before the batch is called done. Tick the checkbox when an item lands.

Suggested order within each batch is the listed order (dependencies noted per item).

---

## P1 — daily-driver ergonomics

### P1.1 — [x] goal-chip date formatting *(XS · haiku)*

**Problem.** Goal chips render `target` as raw ISO (`2026-07-08`) while cards show day-first
("8 Jul") — [goals.rs](../src/components/tasks/goals.rs) interpolates `{target}` directly.

**Design.** Render via `logic::format_due(&target, dates::this_year())`; keep the full ISO string
as the `title=` tooltip. Check the calendar view's goal bands (P0) render targets the same way.

**Files.** `src/components/tasks/goals.rs` only (format_due already has tests).
**Acceptance.** A goal targeted `2026-07-20` shows "20 Jul" on its chip; next-year targets keep the year.

### P1.2 — [x] tag-click filtering *(S · sonnet)*

**Problem.** Tags render as inert spans; the only filter is the free-text box. Filtering by tag —
the most common board-scoping gesture — means typing the tag.

**Design.** A dedicated `tag_filter: RwSignal<Option<String>>` on `State` (like `goal_filter`),
with a pure `tag_ok(task, &Option<String>)` in `logic.rs` (+ test). Apply it everywhere `matches`
+ `goal_ok` are applied today: column cards **and counts**, backlog, the calendar's chips and
overdue count, the goals-bar progress numbers stay unfiltered (they describe the goal, not the
view). Clicking a tag chip on a card/backlog row toggles the filter (`stop_propagation` — a card
click otherwise opens the drawer); the active tag gets an `active` class, and the toolbar shows a
dismissible `tag: ui ×` chip next to the filter input. Clear on well switch (follow what
`goal_filter` does).

**Files.** `src/state/types.rs`, `src/components/tasks/logic.rs` (+test), `board.rs`,
`calendar.rs`, `style/app.css`.
**Acceptance.** Click a tag → board/backlog/calendar all narrow, column counts follow; click
again or dismiss the toolbar chip → cleared; card click still opens the drawer.

### P1.3 — [x] checklist rollup on cards *(S · sonnet)*

**Problem.** Task bodies carry clickable `- [ ]` checklists, but cards show nothing — the cheapest
"subtasks" signal there is.

**Design.** Backend: add `pulldown-cmark` to `src-tauri` and count `TaskListMarker(bool)` events
in `task_from` → `checks_done: u32` / `checks_total: u32` on `Task` (parser events, not line
scanning, so fenced-code `- [ ]` is never counted — the exact semantics of the frontend's
clickable checkboxes in `markdown.rs`). Frontend: mirror the two fields on the frontend `Task`
(serde `#[serde(default)]` so it can land either side first), and render a quiet `3/7` chip
(Lucide `square-check`, 10px) in the card meta row and backlog row when `checks_total > 0`;
muted styling when complete.

**Files.** `src-tauri/Cargo.toml`, `src-tauri/src/model.rs`, `src-tauri/src/tasks.rs` (+tests:
counts, fenced-code exclusion, zero-checklist), `src/model.rs`, `src/components/tasks/board.rs`,
`style/app.css`.
**Acceptance.** A body with 3 of 7 boxes ticked shows `3/7` on the card; toggling a box in the
drawer and closing it updates the card; a code fence containing `- [ ]` doesn't count.

### P1.4 — [x] Esc-close + basic board keyboard nav *(M · sonnet)*

**Problem.** The drawers close only via the X button; board cards aren't focusable — capture is
keyboarded (quick-add, palette) but triage isn't.

**Design.**
- **Esc, layered:** extend the global keydown in `workspace.rs` (where the Ctrl-gated shortcuts
  live): on bare Escape — if the search palette or a popover/menu is open, leave it to them
  (they already self-handle; skip when `search_open`/menu signals are set); else if
  `active_task`/`active_goal` is set → close the drawer and consume. **First verify** the block
  editor's textarea Escape (deactivate-without-commit) stops propagation — if not, add
  `stop_propagation` there so escaping a block edit takes one Esc and closing the drawer takes a
  second. Same two-step feel as the rest of the app.
- **Board nav (deliberately basic):** cards get `tabindex="0"` + a `:focus-visible` accent
  outline (tokens). On a focused card: `Enter`/`Space` → `open_task`; `ArrowUp/Down` → previous/
  next card in the same column; `ArrowLeft/Right` → the same index in the adjacent column. Roving
  focus via the DOM (`query_selector_all` over `.ido-task-card` scoped to columns, using the
  existing `data-task-id`) — no new reactive state, mirroring how `drop_before_at` measures the
  DOM rather than re-rendering. No delete/move chords yet (that's saved-views-era polish).

**Files.** `src/components/workspace.rs`, `src/components/tasks/board.rs`,
`src/components/mainpane.rs` (Escape propagation check), `style/app.css`.
**Acceptance.** Esc closes the drawer (two Escs from inside an active block); Tab reaches a card,
arrows walk the board in both axes, Enter opens; nothing regresses the editor's bare-key handling.

### P1.5 — [x] fix drawer remount focus loss *(M · opus — reactive-graph surgery, easy to regress)*

**Problem.** `TaskDrawer` finds its task inside a reactive closure over `state.tasks`, so **every**
board refresh (which follows every `set_task_field`) recreates `DrawerInner` — dropping focus,
selection, and scroll after each field commit. Same pattern in `GoalDrawer`.

**Design.** Key the drawer on the **id**, not the task snapshot: the outer closure reads only
`active_task` (so the inner view is recreated on open/switch, and on rename — fine, rename
already re-points `active_task`), and `DrawerInner(id)` derives its data via a
`Memo<Option<Task>>` over `state.tasks`. Field inputs stay **uncontrolled** (seeded once per
mount, exactly as today's destructured `prop:value=title` behaves) — a mid-typing refresh must
never clobber the input; the reactive memo is for existence (drawer auto-closes if the task
vanishes) and for read-only bits. The body `Buffer` is likewise created once per id — field
commits then leave in-progress body editing completely untouched (today this survives only by
lucky blur-ordering). Goal drawer: same restructure. Watch: the goal `<select>` options list may
stay reactive (new goals appear) while the selected value stays uncontrolled.

**Files.** `src/components/tasks/drawers.rs` (both drawers), no state changes expected.
**Acceptance.** Open drawer → change priority → focus/scroll intact, no visual remount flicker;
type in tags, Tab to due, pick a date → tags committed, focus flow natural; edit body, change a
field mid-edit → body content intact; rename → drawer stays open on the new id; deleting the task
elsewhere (context menu) closes the drawer.

### P1.6 — ~~Notion import script~~ *(SKIPPED 2026-07-09 — not wanted; migration will be manual)*

**Problem.** "Completely replace Notion" needs the existing Notion tasks database to move over
without manual re-entry.

**Design.** A workspace-member CLI crate `tools/notion-import` (kept out of the app binary),
depending on the `ido` lib for `paths::slugify` / `paths::unique_name` / `frontmatter::merge` so
imported files are byte-consistent with app-created ones.

- **Input:** a Notion CSV export (or the export folder — when per-row `.md` page files exist,
  their content becomes task bodies, with Notion's property-header block stripped).
- **Mapping (flags, with sane defaults):** `Name` → title (slugged file, uniquified) ·
  `--status-col Status` + `--column-map "Not started=todo,In progress=in-progress,Done=done"`
  (unmapped statuses import verbatim → they surface in the backlog as orphans, which is exactly
  ido's model for "column I don't have") · `--priority-map` onto low/normal/high · multi-select
  tags → `tags:` · date column → `due:` (accept ISO and Notion's "July 8, 2026"; ranges take the
  start) · optional `--goal-col` creates `tasks/goals/` files for each distinct value and points
  `goal:` at them. Done-mapped rows get `completed:` from a `--completed-col` when present.
- **Safety:** `--dry-run` prints the plan (row → file path + fields); refuses a non-empty
  `tasks/` without `--force` (re-running would duplicate via `-N` slugs — say so in the error).
- **Tests:** CSV fixtures → tempdir well → assert files/frontmatter; date-format table.

**Files.** `Cargo.toml` (workspace member), `tools/notion-import/` (new), a usage section
appended to this doc when it lands.
**Acceptance.** A real export of your Notion tasks DB lands in a fresh well with statuses,
priorities, tags, dues, goals and bodies intact, and the board opens it cleanly.

---

## P2 — power features

### P2.1 — [x] due times *(S · sonnet · spec already written — [calendar doc §4](calendar-integration.md))*

Implement calendar-integration.md §4 verbatim: optional `due: YYYY-MM-DD HH:MM` (24 h, text-sortable);
**first** the `due_state` date-prefix rule + test (compare first 10 chars — full-string comparison
misflags timed tasks); `format_due` appends the time ("7 Jul 14:00"); `DatePicker` gains a plain
`HH:MM` input under the grid; card/chip/backlog display follows. Backend untouched (`due` is an
opaque trimmed string). Do this before the table view so its due column sorts timed values correctly.

**Files.** `src/dates.rs` or `tasks/logic.rs` (+tests), `src/components/datepicker.rs`, display call sites.
**Acceptance.** A task due `2026-07-09 09:00` flags `due-today` on the 9th, sorts before an
`2026-07-09 14:00` one, renders "9 Jul 09:00"; date-only tasks unchanged.

### P2.2 — [x] table view *(M · opus — third `TaskView`, touches model both sides)*

**Problem.** No flat surface for review — "everything, sortable, dense" is what boards are bad at.

**Design.** `TaskView::Table` as the third toolbar view state (P0 added the enum + toggle).
Columns: title · status (col_label; "(backlog)" when empty/orphan) · priority · tags · due
(format_due + due_state accents) · goal (title, not slug) · **completed**. Surfacing `completed`
means adding it to `Task` on both sides (backend `task_from` + frontend mirror, `#[serde(default)]`)
— it's already stamped on disk by P0. Row click → drawer; header click → sort by that column
(asc/desc toggle, extending `logic::sorted`'s comparators — keep them pure + tested); the
text/tag/goal filters and hide-done apply; archived stay excluded. Read-only v1 — no inline cell
editing (the drawer is one click away and P1.5 made it cheap). Table scrolls inside its own
container (CLAUDE.md: wide content never scrolls the page).

**Files.** `src/state/types.rs`, `src-tauri/src/model.rs` + `tasks.rs` (+test), `src/model.rs`,
new `src/components/tasks/table.rs` (+ `mod.rs`), `board.rs` toolbar, `logic.rs` (+tests), `style/app.css`.
**Acceptance.** Toggle to table → every non-archived task listed; sort by due groups empties last;
sort by completed answers "what did I finish this week"; row click opens the drawer.

### P2.3 — [x] backlog reorder *(S · sonnet)*

**Problem.** Backlog drops always append (`move_task_to(id, "")`); rows can't be arranged.
The backend already supports it — `reorder_column` accepts the empty status.

**Design.** Give the backlog the board's own drop mechanics: `data-task-id` on rows, a
`dragover` computing the drop-before row (generalize `logic::drop_before_at` to take the item
selector, or a sibling helper measuring `.ido-backlog-row` midpoints — vertical list, same math),
the same insertion-line reactive class, drop → `commit_card_drop` with `col = ""` (verify its
`t.status == col` filter handles the empty string; orphan-status rows shown in the backlog keep
their own `order` and simply aren't renumbered — document that in the module doc).

**Files.** `src/components/tasks/board.rs`, `logic.rs` (+test), possibly `state/tasks.rs`, `style/app.css`.
**Acceptance.** Dragging within the backlog shows the insertion line and persists the new order
across a reload; dropping a column card at a specific backlog position lands there.

### P2.4 — [x] auto-archive done *(S–M · sonnet)*

**Problem.** The done column grows forever; archiving is manual per-task.

**Design.** Per-well setting `archive_done_after_days: Option<u32>` in `.ido/well.toml` (manifest
struct + the `set_task_columns`-style write command; settings UI: a small number input with
"off" as empty). Enforcement: an idempotent backend sweep on well open (call it where
`migrate_well` runs): tasks in the done column, not archived, whose `completed:` (fallback: file
mtime for pre-P0 tasks) is older than N days → set `archived: true`. Silent — they land in the
archive view, which is already restorable.

**Files.** `src-tauri/src/wells.rs` (manifest + command + call site), `src-tauri/src/tasks.rs`
(sweep + tests: stamped-old archived, stamped-recent kept, mtime fallback, idempotency),
`src/ipc.rs`, `src/state/`, `src/components/settings.rs`.
**Acceptance.** Set 30 days → reopen the well → month-old done tasks are in the archive, this
week's remain; toggle off → nothing moves.

### P2.5 — [x] tag autocomplete *(S–M · sonnet)*

**Problem.** Tags are free text; vocabulary fragments (`ui` / `UI` / `interface`).

**Design.** The vocabulary is derived, not stored: a `Memo<Vec<String>>` of distinct tags across
non-archived tasks (case-preserving, first-seen casing wins for display). Extract the drawer's
tags input into a `TagsInput` component: typing after the last comma opens a suggestion popover
(custom, datepicker-style — native `<datalist>` is un-themeable) filtered by prefix,
case-insensitive; Enter/click inserts the suggestion's canonical casing; Esc closes the popover
only (P1.4's layered-Esc rule). Suggestion ranking pure + host-tested.

**Files.** `src/components/tasks/drawers.rs` (or a new `tagsinput.rs`), `logic.rs` (+test), `style/app.css`.
**Acceptance.** With `bug` on some task, typing `b` in another task's tags offers `bug`; accepting
yields `bug` not `Bug`; commas/blur behavior identical to today's input.

### P2.6 — [x] saved views *(M · opus — cross-cutting state + persistence; do last)*

**Problem.** Filter + sort + view + scope combinations are rebuilt by hand every time; Notion's
per-view filters are its core power feature.

**Design.** A `SavedView { name, view, filter, tag, goal, hide_done, sort }` snapshot of the
toolbar state. Persist in `.ido/well.toml` (`views: Vec<SavedView>` on the manifest — it's durable
per-well config, like columns; whole-list `set_views` command mirroring `set_task_columns`).
UI: a toolbar dropdown — current view name (or "views"), the saved list (click applies, `×`
deletes), and "save current…" with an inline name input. Applying sets the six signals; any
manual change after that just means the live state drifted from the preset (no dirty tracking —
keep it dumb). Goal/tag references that no longer exist apply as filters that match nothing;
that's visible and self-explanatory, don't validate.

**Files.** `src-tauri/src/wells.rs` (manifest + command + tests), `src/ipc.rs`,
`src/state/` (apply/save/delete methods), `src/components/tasks/board.rs`, `style/app.css`.
**Depends on** P2.1/P2.2 (a view worth saving includes the table + timed sorts) and P1.2 (tag filter).
**Acceptance.** Save "this week" (calendar + tag + hide-done) → restart the app → apply it from
the dropdown → identical board state; delete removes it from `well.toml`.

---

## Batch order at a glance

| # | Item | Size | Model | Depends on |
|---|---|---|---|---|
| P1.1 | goal-chip date formatting | XS | haiku | — |
| P1.2 | tag-click filtering | S | sonnet | — |
| P1.3 | checklist rollup | S | sonnet | — |
| P1.4 | Esc + board keyboard nav | M | sonnet | — |
| P1.5 | drawer remount focus fix | M | opus | — |
| P1.6 | ~~Notion import script~~ | — | — | skipped per user, 2026-07-09 |
| P2.1 | due times | S | sonnet | calendar view (P0) |
| P2.2 | table view | M | opus | P2.1 |
| P2.3 | backlog reorder | S | sonnet | — |
| P2.4 | auto-archive done | S–M | sonnet | completed: stamp (P0) |
| P2.5 | tag autocomplete | S–M | sonnet | P1.2 (shared tag vocabulary) |
| P2.6 | saved views | M | opus | P2.1, P2.2, P1.2 |

After each batch: update CLAUDE.md's **Current state** section, tick the boxes here, and do one
visual pass over the running app before moving on.
