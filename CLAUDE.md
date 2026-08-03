# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`ido` (井戸, "a well") is a local-first, markdown-based knowledge app in the **latent.**
ecosystem. It is a Tauri desktop app with a Leptos (CSR → WASM) frontend. A **well** is any
folder the user picks; inside it, three **sections** — **notes** (foldered markdown), **wiki**
(a `[[`-linked namespace of pages, whose slugs are globally unique but whose files can be
organised into purely-cosmetic folders), and **tasks** (a kanban board plus **goals**/milestones)
— each live as plain markdown files under `notes/`, `wiki/`, `tasks/`, with only rebuildable
config/cache under `.ido/`. A left **rail** switches sections; per-pane **tab strips** keep
several entries open (and a second editor pane can **split** off, side by side); a `Ctrl+K`
**command palette** searches all three sections at once. [README.md](README.md) is the human-facing
overview; this file is the working guide — the **Current state** section below tracks what's built and
what's next. (Project docs live in this codebase + an ido well; `docs/` holds the working design
docs — currently [mcp-server.md](docs/mcp-server.md), the plan for exposing the store over MCP
with semantic search.)

## The big picture: a 3-layer app

This app deliberately owns very little. It composes two shared upstream layers:

- **`vendor/latent-design`** (git submodule) — all styling: CSS tokens, component styles,
  self-hosted fonts, SVG assets, **and the ecosystem's engineering/design canon** under
  `vendor/latent-design/docs/`. Trunk needs real file paths, so this is a submodule, not a
  crate. **Run `git submodule update --init --recursive` after cloning** or the build can't
  find tokens/fonts and renders unstyled.
- **`latent-ui`** (Cargo git dep, pinned tag in [Cargo.toml](Cargo.toml)) — shared Rust/Leptos
  behavior: `ThemeToggle` and the theme machinery (`theme::initial_theme`,
  `theme::setup_theme_effect`). Ships no CSS.

**This repo owns only `src/` (frontend logic) and `style/app.css` (page layout).** Never
restate or fork the upstream layers here.

Two crates in one Cargo workspace:
- root crate **`ido-ui`** — the Leptos frontend; `index.html` is the Trunk entry point.
- **`src-tauri/`** — the Rust backend and the **local-first store**. Tauri commands are the only
  disk access, grouped by area (all `pub` in their module, listed in `generate_handler!`):
  - **wells:** `recent_wells` / `pick_folder` / `open_well` / `create_well` / `migrate_well` /
    `set_task_columns` (the settings column editor; names slugified + deduped, order kept) /
    `set_archive_days` (the auto-archive threshold; its write also runs the sweep immediately) /
    `saved_views` / `set_saved_views` (the toolbar's saved views, whole-list, `[[views]]` tables
    in `well.toml`; blank names dropped). `migrate_well` scaffolds the section folders +
    `.ido/well.toml`, moves a legacy flat well's root notes into `notes/`, upgrades legacy
    columns, and runs the **auto-archive sweep** (done-column tasks whose `completed:` — file
    mtime as the pre-stamp fallback — is older than `archive_done_after_days`) — **idempotent,
    run on every open**.
  - **notes** (scoped under `notes/`): `list_tree` / `read_note` / `note_meta` / `write_note` /
    `create_note` / `create_folder` / `rename_entry` / `delete_entry` / `move_entry`. A note's
    **name is its file name** (independent of body); `note_meta` returns created/modified epoch ms.
  - **wiki** (slugs under `wiki/`, optionally nested in organisational folders): `list_wiki`
    (returns a `TreeNode` tree, reusing `notes::build_tree`) / `read_page` / `write_page` /
    `create_page` (into a folder) / `ensure_page` / `rename_page` / `delete_page` /
    `create_wiki_folder` / `rename_wiki_folder` / `delete_wiki_folder` / `move_wiki_entry`.
    A page's **slug is globally unique** across the section (folders are purely cosmetic and never
    touch identity), so a page is resolved by slug **anywhere in the tree** (`find_page`) and
    moving it between folders keeps its slug — links never break. `backlinks` is **cross-section**
    (scans notes, wiki — recursing folders — *and* task/goal bodies for `[[links]]`); rename
    rewrites inbound `[[links]]` across **all of them**.
  - **tasks** (one md file per task under `tasks/`, frontmatter metadata): `task_columns` /
    `archive_days` / `list_tasks` / `create_task` / `move_task` / `reorder_column` / `set_task_field` /
    `update_task_body` / `rename_task` / `delete_task` / `restore_task`; **goals** (under
    `tasks/goals/`): `list_goals` / `create_goal` / `reorder_goals` / `set_goal_field` /
    `update_goal_body` / `rename_goal` / `delete_goal` / `restore_goal`. `delete_task` /
    `delete_goal` return the deleted file so a `restore_*` can undo it. A task/goal's **id is a
    slug but its display name is free text** (`title:` frontmatter, falling back to the id):
    `create_task` takes a title (+ an optional `due`), and `rename_task` / `rename_goal` set the
    title + re-slug the file (colliding slugs uniquify with `-N` — duplicate titles are fine,
    never an error). All three status-writing paths (`move_task` / `reorder_column` /
    `set_task_field("status")`) share one choke point (`apply_status`): a genuine transition
    **into** the done column (the last column) stamps `completed: YYYY-MM-DD` (local, via chrono),
    leaving clears it — and a task with a `repeat:` spec **spawns its next occurrence** (new file,
    first column, due advanced from the *printed* due; `repeat:` moves to the spawned file so the
    completed one can't double-fire; an unparseable spec spawns nothing and is kept).
  - **assets:** `save_asset` / `read_asset` — a shared `assets/` folder (a well-root sibling of
    the sectioned folders, invisible to the notes tree) for pasted/dropped images, referenced from
    any section's markdown as `![](assets/foo.png)`. `read_asset` returns a `data:` URI (the
    frontend can't reach the filesystem directly to load one as a plain file path).
  - **search:** `search` (one command; brute-force scan of notes + wiki + tasks → ranked `SearchHit`s).
  - **session:** `read_session` / `write_session` (open tabs, cached in `.ido/session.toml`).
  - **window:** `apply_window` / `show_window` / `win_minimize` / `win_toggle_maximize` / `win_close`.
  - **external:** `open_external` (open a URL in the OS browser).

  The folder picker uses `tauri-plugin-dialog` **from Rust** (no capability entry needed); recent
  wells live in `<app_data_dir>/wells.json`. Window chrome is custom (decorations off).
  `withGlobalTauri` is on.

Both crates are split into small, documented modules (module-level `//!` + item `///` docs):
- **Backend** (`src-tauri/src/`): `lib` wires modules + `run()`; `model` (shapes), `paths` (pure
  id/name/**slug** helpers + tests), `frontmatter` (minimal `--- key: value ---` parse/merge +
  tests), `registry` (recent wells), `wells` (open/create/**migrate** + scaffold), `notes` (tree +
  CRUD + tests), `wiki` (flat pages + **cross-section** backlinks + tests), `tasks` (kanban + goals
  over frontmatter + tests), `repeat` (pure recurrence-spec parse + `advance` date math on
  chrono's `NaiveDate` + tests), `assets` (the shared image-attachment store + tests), `search`
  (cross-section scan + tests), `session`, `window`, `external`.
- **Frontend** (`src/`): `model`, `ipc` (**the only place that calls `invoke`** — typed wrappers +
  arg structs), `state` (one `Copy` `State` of all signals via Leptos context; backend work lives in
  its action methods. Editing state lives on a `Pane` — `panes: [Pane; 2]` for split view — and the
  block/source editor runs on a `Buffer` (editing signals + a save `Callback`) built from a pane or,
  in the task drawer, locally), `blocks` (top-level block segmentation: `segment` → `Block`s with
  byte ranges, `splice` commits one block back), `markdown` (markdown → HTML via pulldown-cmark;
  `[[wikilinks]]` expanded **pre-parse**; LaTeX → MathML via latex2mathml; raw HTML sanitised to
  text; `Event`-transform seam for `#tags`/etc.), `dates` (shared **Monday-first** date math —
  `weekday` / `add_days` / `add_months` / `ymd` + tests; `today()` reads `js_sys` and must never
  run in host-side tests — pure helpers take "today" as a parameter), `icon`, `app` (root), and
  `components/`:
  - **shell:** `titlebar` (custom chrome + well name; per-OS controls), `launch` (launcher), `rail` (section switcher
    + 井戸 switch-well + **search** + settings), `workspace` (rail + section sidebar + editor pane(s);
    global tab/split/search keyboard shortcuts + the native-context-menu suppressor), `settings`,
    `search` (the `Ctrl+K` command palette), `toast` (undo / error toasts), `contextmenu` (the
    right-click menu).
  - **sections:** `notes` (tree sidebar), `wiki` (folder-tree sidebar), `tasks` (full-width board ⇄
    **calendar** view toggle + backlog + goals bar + detail drawers; `tasks/calendar.rs` is the
    month grid), `tree` (the recursive note tree), `datepicker` (a custom
    calendar popover replacing the un-themeable native `<input type=date>`; used by the task/goal drawers).
  - **shared editing:** `mainpane` (`MainPane(pane, idx)` — one pane's tabs + the active tab's
    document in source/live/reading modes; reused by notes & wiki, and mounted twice when split;
    its `attachments` submodule extracts pasted/dropped image files and splices a `![](assets/…)`
    reference in — shared by every markdown textarea, so it also covers the task/goal drawer) and
    `tabs` (`TabStrip(pane, idx)` — a pane's tab strip, with cross-pane drag).

  Components read `expect_context::<State>()` instead of prop-drilling. To add a feature: a command
  (backend module + `generate_handler!`), an `ipc` wrapper, a `State` method, and a component.

## Read the canon, don't restate it

Engineering and design rules for the whole ecosystem live in the submodule and ship into this
tree. Consult these before changing architecture, conventions, or visuals — they are the
source of truth:

- `vendor/latent-design/docs/ecosystem.md` — the 3-layer model, CSS cascade, anti-FOUC, pinning
- `vendor/latent-design/docs/conventions.md` — Rust/Leptos/Trunk/theme conventions
- `vendor/latent-design/docs/bootstrap-new-app.md` — how this app is wired
- `vendor/latent-design/README.md` — the brand canon (color, type, voice)

The `/latent-design` Claude skill surfaces these (symlinked at `.claude/skills/latent-design`).
`.claude/` is gitignored, so the symlink is machine-local — recreate it per
`bootstrap-new-app.md` step 3 if the skill is missing.

## Commands

```sh
just verify        # fmt-check + clippy (-D warnings) + backend tests — exactly what CI runs
just fmt           # cargo fmt + leptosfmt (the only correct way to format — see below)
just check         # cargo clippy --workspace -- -D warnings
just test          # cargo test -p ido — backend store logic tests
just dev           # cargo tauri dev: Trunk on :1420 + native window, hot reload
just dev-debug     # same, + WebView2 CDP remote debugging on :9222 (Windows only —
                   # see `.claude/skills/run` for the driver script: screenshots, DOM
                   # queries, real clicks/keystrokes against the running app)
just               # list all recipes

cargo tauri build                          # bundle a release binary for this platform
cargo tauri icon src-tauri/app-icon.svg    # regenerate the app-icon set from the source SVG
cargo check -p ido-ui                      # fast type-check of just the frontend (host target is fine)
```

- **Never run `cargo fmt` alone** — it cannot parse Leptos `view!` macros and corrupts them.
  Always `just fmt`, which runs `cargo fmt` then `leptosfmt src`. Editor format-on-save delegates
  to `leptosfmt --stdin --rustfmt` via `rust-analyzer.toml` (+ `.vscode/settings.json`).
- `.githooks/pre-commit` runs `just fmt-check`; activate once per clone with
  `git config core.hooksPath .githooks`. CI (`.github/workflows/ci.yml`) runs `just verify` on
  Ubuntu with Tauri's webkit deps installed.
- **Tests:** two suites; only the backend one is in `just verify`.
  - **Backend** (`cargo test -p ido` / `just test`): tempdir-based unit tests across `paths`,
    `frontmatter`, `notes`, `wiki` (incl. cross-section backlinks), `tasks` (+ goals), `search`,
    `wells` migration, `session`. Commands take plain args (no mock runtime). `just verify` includes these.
  - **Frontend** (`cargo test -p ido-ui`): pure-Rust unit tests in `src/blocks.rs` (segmentation /
    splice), `src/markdown.rs` (slugify + wikilink expansion), `src/dates.rs` (the shared date
    math), `components/tasks/logic.rs` (due/sort/overdue/tag helpers), `components/tasks/calendar.rs`
    (month-grid + chip-visibility invariants), `components/tasks/table.rs` (column comparators),
    and `components/datepicker.rs` (due-time split/combine/normalize) — run on the host target
    (no WASM). Run them explicitly; `just verify` does not include them yet.
  - No E2E tests yet — verify UI changes by running the app (`just dev`).

## Rules that are easy to violate

- **`style/app.css` is the only stylesheet this app owns.** Reference design tokens via
  `var(--…)`; never paste token values or component styles inline. To change a token or a shared
  component, edit the `vendor/latent-design` submodule — not this app.
- **CSS cascade order in `index.html` is mandatory:** `tokens.css` → `components.css` →
  `style/app.css`. Reordering renders unstyled.
- **Theme is one `RwSignal<bool>` from context**, provided at the `App` root and read via
  `use_context` — never passed as props. The anti-FOUC inline script in `index.html` sets
  `data-theme` before CSS loads; ido **defaults to light** (it's a writing surface). The script,
  `latent_ui::theme::initial_theme()`, and `setup_theme_effect` keep DOM + `localStorage`
  (`"latent-theme"`) in sync.
- **Icons are Lucide only** — and inlined as SVG (the `Icon` component in `src/icon.rs`), not via
  the CDN script (it can't re-bind icons across reactive re-renders). Use 1.6px stroke and
  `currentColor`. Never emoji, never hand-drawn.
- **Brand:** lowercase voice; the `latent.` mark is the umbrella identity **only** — ido's mark
  is the 井戸 kanji. The current app icon ([src-tauri/app-icon.svg](src-tauri/app-icon.svg)) uses
  the latent mark as a temporary placeholder.
- **Tauri maps camelCase JS arg keys → snake_case Rust params.** A frontend `invoke` arg struct
  with a multi-word field (e.g. `is_dir`) must carry `#[serde(rename_all = "camelCase")]`, or the
  command silently rejects and nothing happens. Single-word args (`well`, `id`, `name`…) are
  unaffected — which is why this only ever bites multi-word params.
- **Window chrome is custom** (`decorations: false`). The window starts hidden (`visible:
  false`) and **must** be revealed by `show_window` (called at the end of startup) — skip it and
  the app is invisible. Dragging uses `data-tauri-drag-region` on `.ido-titlebar`, which needs
  `core:window:allow-start-dragging` in `capabilities/default.json`; the min/maximize/close
  buttons call Rust commands, so they need no capability. The controls **follow the host OS**
  (`platform::is_mac`, UA-sniffed — the frontend is WASM, so `cfg!(target_os)` describes the wrong
  machine): three latent-palette dots on the left on macOS, a minimize/maximize/close row on the
  right elsewhere. The **third control routes per platform** (`window::toggle_zoom`): macOS gets
  **native fullscreen**, because only that gives the window its own Space — which is what makes
  `Ctrl+←/→` swipe between it and the desktop — whereas `maximize()` on a borderless window can
  only resize it to the work area. `decorations: false` is no obstacle; tao swaps in a
  `Titled | Resizable` mask around `toggleFullScreen:` and restores the borderless one on exit.
  Windows/Linux keep native maximize, which also drives snap and the taskbar's window state.
  macOS fullscreen is **never persisted** (reopening into a Space the user has left is
  disorienting), so `WinState.maximized` is a Windows/Linux-only concern.
  **In-webview HTML5 drag-and-drop (the
  note tree, kanban cards + backlog, the tab strip) requires `dragDropEnabled: false` on the
  window** — the OS file-drop handler otherwise swallows `dragstart`/`drop` before they reach the page.
  Because of this, an OS file drop (e.g. an image) reaches the page as a normal HTML5 drop instead —
  `workspace.rs` installs a global `dragover`/`drop` listener that always `prevent_default`s as a
  fallback, since an unhandled one would otherwise navigate the webview (see the rule below).
- **Rendered-markdown links must not navigate the webview.** Click handlers in
  `components/mainpane.rs` (both the reading view and rendered Live blocks) route `<a>` clicks
  through `State::open_link`: internal `[[wikilinks]]` (`ido:wiki/<slug>`) open/create the page in a
  tab; everything else goes to `open_external` (OS browser). A navigated webview white-screens the
  app. Raw HTML in notes is rendered as text, not executed (see `markdown.rs`).
- **Both crates are edition 2024**, matching the ecosystem convention in
  `vendor/latent-design/docs/conventions.md`. Two consequences bite in practice: `gen` is a
  **reserved keyword** (generation-counter locals are named `this_gen`, not `gen`), and **let-chains
  are stable** — clippy's `collapsible_if` now *requires* `if let Some(x) = a && cond` instead of
  nested `if`s, so `-D warnings` fails on the old form.

## Current state

Two top-level screens, gated on whether a well is open: the **launcher** (recent wells + open /
create) and the **workspace**. Startup reopens the most-recent well and its saved tabs, falling
back to the launcher. The window is borderless with a custom title bar (showing the well name):
fixed on the launcher, resizable in the workspace — the editor window's size, **position**, and
maximized state are **remembered** per user (`window::save_geometry` → `app_data/window.json`; a
debounced resize handler in `workspace` saves size, and a `CloseRequested` handler in `run()` saves
the final geometry — the reliable capture point for position, since the OS gives no move event).
`restore_window` re-applies on open: size **clamped to the monitor work area**, position restored
only if it's still on a connected monitor (else centred). On **first run** there is nothing to
restore, so the size is derived from the display instead of fixed in pixels — 86% of the work-area
height, width from a 1.6 aspect ratio, capped at 90% of the work-area width (`window::default_editor`)
— which lands sensibly on a 13" laptop and a 32" panel alike. Two edge-cases are handled so nothing is silently lost:
while maximized only the flag flips (the un-maximized restore size is kept), and a size that merely
echoes a monitor-**clamp** isn't written back — so a big window shrunk to fit a laptop is restored
full-size back on the large monitor.

**Workspace layout** — the left **rail** (`components/rail.rs`) switches sections and holds the
井戸 switch-well button + search + settings. Notes and wiki render a **section sidebar + the editor
pane(s)**; tasks renders a **full-width board** instead (its editing is a drawer, not the main pane).
Each editor pane has its own **tab strip** (`components/tabs.rs`) atop it; tabs are global across
sections, independent of the rail/sidebar, and the **primary** pane's tabs are restored per well
from `.ido/session.toml`. Shortcuts (all `Ctrl`-gated — or `Cmd` on macOS — so they don't clash
with the editor's bare keys), acting on the **focused** pane: `Ctrl+Tab` / `Ctrl+Shift+Tab` cycle, `Ctrl+1…9` jump,
`Ctrl+W` close, `Ctrl+\` toggle split, `Ctrl+K` open the search palette. One bare key is global:
**Escape closes the topmost surface, layered** — surfaces that self-handle Esc win (the palette,
context menu, calendar/date popovers, an active Live block — their handlers `stop_propagation`,
which matters because an input's own target-phase keydown always beats window listeners), and the
workspace-level fallback (which also checks the DOM for open popovers, order-independent) then
closes the task/goal drawer. So escaping an active block edit takes one Esc; closing the drawer
takes a second.

**Split view** — `State` holds `panes: [Pane; 2]` + `split` + `focused`; `Workspace::EditorPanes`
mounts `MainPane(panes[0], 0)` plus `MainPane(panes[1], 1)` when split. Open a split with the header
button or `Ctrl+\` (duplicates the active doc into the new pane); clicking a pane focuses it (sidebar
opens + shortcuts target the focused pane). Tabs **drag across panes**: reorder within a strip, drop
on the other pane's strip to move, or drop on the right-edge **split zone** to split — an insertion
line marks the spot (`move_tab` / `commit_tab_drop`, `TabDrop`). Emptying a pane collapses the split.
The split is a within-session view (only the primary pane persists). Capped at two panes.

**Search** — a `Ctrl+K` command palette (`components/search.rs`, mounted once in the workspace) over
the `search` command, **relevance-ranked** (title match, then occurrence count; archived tasks
excluded, task tags searched) with matches **highlighted**. The disk scan is **debounced**
(`search_gen`) so a fast typist triggers one scan, not one per keystroke. ↑/↓ select, Enter opens,
Esc/backdrop closes; a hit routes through `State::open_entry`. It also lists **commands** (`State::COMMANDS`,
kind `"cmd"` — new note/page/task, settings, toggle theme) handled by `State::run_command`.

**Toasts** — `components/toast.rs` (mounted in the workspace) over `State::toast`: deleting a note,
wiki page, task, or goal is a **soft-delete** with an undo toast (`UndoEntry` captures the raw file
by `EntryKind`; `undo_toast` rewrites + reopens — restoring a goal brings back the file but not the
`goal:` refs cleared off its tasks). Failed renames in **any** section (name collisions, surfaced via
`ipc::call_res` → `Result`) show an error toast. Auto-dismiss guarded by a `toast_gen` counter.

**Notes** — a recursive `Tree` / `TreeRow` pair (`Tree` returns `AnyView` to break the
recursive-`impl Trait` cycle, E0720). New note/folder into the selected `target` folder, inline
rename, delete (notes + empty folders only), drag-to-move (`move_entry`; drop on the empty list
area → well root).

**Wiki** — a slug namespace under `wiki/`, with a **folder tree** for organisation only (the
sidebar mirrors the notes `Tree`: create page/folder into a target folder, inline rename,
drag-to-move, delete — via `create_wiki_folder` / `rename_wiki_folder` / `delete_wiki_folder` /
`move_wiki_entry`, keyed on wiki-relative paths). A page's **slug stays globally unique** and is
its only identity: `[[slug]]` / `[[slug|label]]` links are expanded pre-parse in `markdown.rs` to
`ido:wiki/<slug>` anchors (folder-agnostic), clickable in Live *and* reading mode, creating the
page on click if it doesn't exist. Because identity is the slug, **moving a page between folders
changes nothing** — open tabs and `[[links]]` are untouched; only a *rename* (which changes the
slug) re-points tabs (`sync_page_id`) and rewrites inbound links. `State` carries a `wiki_tree`
(the sidebar) beside the flat `wiki` slug list (existence checks), kept in sync by `set_wiki`; the
wiki tree has its own `wiki_expanded` / `wiki_renaming` / `wiki_dragging` / `wiki_drag_over` /
`wiki_target` signals (parallel to the notes tree's). The backlinks panel (reading-view footer) is
**cross-section** — it lists the notes *and* wiki pages (in any folder) that link here (`backlinks`
→ `LinkRef`s, opened via `State::open_entry`); inbound links are rewritten across all sections on
rename. Notes can link *to* wiki pages but aren't `[[link]]` targets themselves (a deliberate
non-goal for now).

**Tasks** — a kanban board, one markdown file per task under `tasks/` with frontmatter (`status`
= column, plus `title` / `priority` / `tags` / `order` / `due` / `goal` / `repeat` / `completed`).
Cards, drawers, search,
and backlinks all display the free-text `title:`; the file stem stays its slug. Columns come from
`.ido/well.toml` (default `todo / planning / in-progress / done`) and are **editable in settings**
(`set_task_columns`; the last column counts as done, orphaned statuses drop to the backlog). Drag
cards between columns *and* to a precise within-column position (`reorder_column`); the drop spot
is computed by one column-level `dragover` measuring card midpoints (`drop_before_at`), showing an
accent insertion line (a reactive class, never a list re-render — so it can't cancel the native
drag). Each column footer is a **quick-add** capture input (`QuickAdd`: type a title, Enter adds
and keeps capturing — no drawer opens). Card due dates render day-first ("7 Jul", year elided when
current) and flag `overdue` (accent + clock) / `due-today` — muted in the done column (`due_state`
/ `format_due` in `tasks/logic.rs`, host-tested; goal chips format targets the same way). A due
may carry an **optional time** — `YYYY-MM-DD HH:MM`, 24 h — lexicographic order stays
chronological, **every date comparison uses the 10-char prefix** (`due_state` / `overdue_count` /
calendar bucketing), the datepicker grew a loose-normalized `HH:MM` field, and calendar
drag-to-reschedule preserves the time-of-day (`reschedule` in `logic.rs`). Cards
also carry a **checklist rollup** (`3/7`, from `checks_done`/`checks_total` counted backend-side
via pulldown-cmark's `TaskListMarker` events — so fenced-code `- [ ]` never counts) and are
**keyboard-navigable** (`tabindex`, Enter/Space opens, arrows walk rows/columns via DOM-roving
focus in `logic.rs` — no reactive state). **Tag chips filter on click** (`tag_filter` on `State`,
`tag_ok` in `logic.rs`, applied wherever `matches` + `goal_ok` run — board, backlog, calendar,
overdue rollup — with a dismissible toolbar chip; cleared on well switch); the drawer's tags
field **autocompletes** from the well's live tag vocabulary (`tagsinput.rs`: prefix-then-substring
ranking in `logic.rs`, canonical first-seen casing, `mousedown` + `prevent_default` insertion to
dodge the blur-commit race). A **backlog** of un-columned tasks below the board — **reorderable
by drag** with the board's insertion line (`drop_before_at` takes the row selector; orphan-status
rows display but are never renumbered or targeted, enforced by a `data-orphan` selector scope) — a
toolbar (search / sort / hide-done / **saved views**: a bookmark dropdown over `[[views]]` in
`well.toml` snapshotting view + filter + tag + goal + hide-done + sort, replace-on-same-name,
applied without validation), and task/goal detail drawers whose
**body is the shared three-mode editor** (`DocEditor(buffer, mode)` in `mainpane.rs` — source/live/
reading on a local `Buffer`, saving via `update_task_body` / `update_goal_body`). The drawers are
**keyed on the id, not the task snapshot** (`DrawerInner(id)` seeds uncontrolled inputs once per
mount from an untracked read; existence/`archived` are reactive; the body `Buffer` is created once
per id) — so board refreshes (which follow every field/body commit) never remount the drawer or
clobber mid-typing input; renames fetch the refreshed list *before* re-pointing the active id.
**Goals**
(milestones) are markdown files under `tasks/goals/` with a `target` date; tasks link via a `goal:`
field; a goals bar shows derived progress (done/total) and scopes the board. **Recurrence**: the
drawer's `repeat` select (plus hand-written `every N …` specs, shown as an orphan option) drives
the backend's spawn-on-done; a quiet `repeat` icon marks recurring tasks on cards, backlog rows,
and calendar chips. Completing into the done column stamps `completed:` (see the backend notes
above). The rail's tasks button carries an **overdue badge** (`overdue_count` in `tasks/logic.rs`,
host-tested: unfiltered, done-column and archived excluded, compares the 10-char date prefix so a
future timed-due format keeps working); tasks/goals/columns load eagerly on well open, so it's
live in every section.

**Calendar** — the tasks toolbar toggles board ⇄ calendar ⇄ table (`TaskView` in state;
session-local, like `cal_month`). A **Monday-first month grid** (`tasks/calendar.rs`; `grid_days` host-tested):
day cells show goal-target bands + task chips (priority dot, `overdue` / `due-today` accents, done
muted; the board's filter / goal scope / hide-done pipeline applies via `cal_visible`), capped
with a `+N more` overflow into a **day popover** (full list + a QuickAdd-style input that creates
into the first column with that due). Chips **drag to reschedule** (`set_task_field("due")`; an
always-mounted, class-toggled **unschedule tray** clears it — mounted always so appearing mid-drag
can't reflow drop targets). The header has `‹ › / today` navigation and an **`overdue N` chip**
whose popover lists overdue tasks oldest-first. Drawers overlay the calendar exactly as the board.
Due times shipped; ICS export and read-only external feeds are the remaining unbuilt phases.

**Table** — the third task view (`tasks/table.rs`): every non-archived task (backlog included) in
a sticky-header table scrolling its own container; columns title / status / priority / tags
(click-to-filter) / due / goal (resolved title) / **completed** (the stamp, surfaced as
`Task.completed` on both sides). Header click sorts — table-local `(TableCol, asc)` state,
`TaskSort` untouched — with empties last on due/completed in both directions and status ranked by
board-column position (backlog, then orphans, last); pure comparators host-tested. Rows open the
drawer (click / Enter / Space), right-click menus work, the whole filter pipeline applies.

**Tabs** — a sidebar single-click opens a reusable **preview** tab (italic name) that the next
preview-open replaces; the first edit or a tab double-click **pins** it permanent (`Tab.preview`).
The block-editor machinery lives on `Buffer` (`commit_block` / `split_block` / `merge_with_prev` /
`update_source`); `State::pane_buffer(pane)` wires a pane's signals + a `save_pane` callback, so the
same editor serves both panes and the task drawer.

**Main pane — three modes** (icon buttons in the header), shared by notes & wiki via
`MainPane(pane, idx)` in `components/mainpane.rs`. The header also carries the **split toggle**
(`columns-2`); the three editor components (`SourceEditor` / `BlockEditor` / reading view) take a
`buffer` prop:

- **Source** (`</>`): one uncontrolled raw-markdown `<textarea>`, seeded imperatively from
  `buffer.source_editor` (`NodeRef`) on open/mode-switch. Autosaves on every input (no debounce).
- **Live** (pencil): **block live-preview**, the primary surface. `src/blocks.rs` `segment` splits
  the source into top-level `Block`s (raw string + byte range); `BlockEditor` renders each block
  as HTML and swaps the active one for a `<textarea>`. Keyboard:
  - **Enter on a blank line** → `split_block` (content before stays, content after becomes the
    next block); the blank line is a separator, not content.
  - **↑ / ↓ at first/last line** → `commit_and_go` to the adjacent block.
  - **Backspace at column 0** → `merge_with_prev` (join with `\n\n`).
  - **Escape** deactivates without committing; **blur** commits (failsafe, guarded).
  - `splice` writes one block back at its byte range. pulldown-cmark's `End` ranges include the
    trailing newline, so block ranges use `trim_end` length to keep inter-block separators.
- **Reading** (eye): fully rendered, read-only, under a quiet `NoteMetaRow` (folder + creation
  date via `note_meta`; nothing for wiki pages). Link clicks routed through `State::open_link`
  (see the rule above).

**Checkboxes** — rendered task-list items (`- [ ]`) are **clickable** on every rendered surface
(reading views, Live's inactive blocks, the drawer editor). `markdown::render` numbers each marker
(`data-check-idx`, replacing the writer's disabled inputs in `transform`); a click resolves through
`mainpane::checkbox_click_index` and `markdown::toggle_checkbox`, which locates the marker via the
parser's **source offsets** (so `- [ ]` inside a code fence is never touched) and flips `[ ]`/`[x]`
in the source — committed via `update_source` (reading) or `commit_block` (a Live block).

**Math** — both `markdown.rs` and `blocks.rs` enable `Options::ENABLE_MATH`; `Event::InlineMath`
(`$…$`) / `Event::DisplayMath` (`$$…$$`) convert to MathML via `latex2mathml` in `transform`.
Display `<math>` must **not** get `display: block` in CSS — Chromium's UA maps
`math[display="block"]` → `display: block math`, and overriding it garbles MathML layout.

**Images** — pasting or dropping an image file into any markdown textarea (Source or Live mode;
the main pane and the task/goal drawer alike, since they share the same editor components) saves
its bytes into the well's shared `assets/` folder (`assets::save_asset`, a well-root sibling of
the sectioned folders) and splices a `![](assets/…)` reference in at the cursor
(`mainpane::attachments::insert_images`, via the same `execCommand('insertText', …)` reflection
trick as the context menu's paste). The frontend can't read the file back directly, so
`markdown::render` takes a `resolve_asset` callback that looks up (and, on a cache miss, kicks off
a fetch for) a `data:` URI through `assets::read_asset` — `State::resolve_asset` backs this with an
`assets` cache keyed by well-relative id, cleared on well switch. A first view of a not-yet-cached
image briefly shows a broken-image icon until the fetch resolves and the surrounding reactive scope
re-renders. `workspace.rs` also installs a global `dragover`/`drop` fallback that always
`prevent_default`s, so a file dropped anywhere other than an editor (or a non-image file dropped
anywhere) doesn't navigate the webview instead of being silently ignored.

**What's next** — the tasks P0/P1/P2 batches have **all landed** (a Notion import script was
considered and dropped), and the wiki grew **organisational folders** (slugs stay globally unique,
links stay folder-agnostic). The next big bet is the **MCP server + semantic search** over the
store, specced end-to-end in [docs/mcp-server.md](docs/mcp-server.md): extract the store into a
tauri-free `ido-store` crate, add an `rmcp` + stdio `ido-mcp` binary with seven read-only tools,
then a local-embedding brute-force vector index (**candle** — pure Rust, no ONNX Runtime to
bundle) fused with the existing lexical scan.
Also open: the calendar's remaining phases (ICS export, read-only external feeds), small rough
edges (N-way / persisted split, deeper goal-status surfacing), and the other bigger bets, none
started: **local-LLM / Gemma** in-app assistance (on-device, private) and **web sync**
(self-hosted + optional cloud, end-to-end encrypted). Notes-as-`[[link]]`-targets remain a
deliberate non-goal.
