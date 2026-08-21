# ido

> 井戸 — a quiet well for thought.

`ido` is a local-first, markdown-based knowledge system. A **well** is any folder you
pick; inside it live three sections — **notes**, a **wiki**, and **tasks** (a kanban
board with goals) — all as plain `.md` files on your own machine. Durable, portable,
yours: no lock-in, no latency.

Part of the **latent.** ecosystem. A desktop app built with Tauri + Leptos (Rust).

## What's there

Open or create a **well**, then work in three sections, switched from the left rail:

- **notes** — freeform markdown in folders. A tree sidebar (create / rename / delete /
  drag-to-move) and three editor modes:
  - **source** — raw markdown, autosaves on every keystroke.
  - **live** — block live-preview: each top-level block renders as HTML; click to edit it.
    Enter on a blank line splits or appends a block, Backspace at the start merges up, ↑/↓
    navigate.
  - **reading** — fully rendered, read-only.

  Inline (`$…$`) and display (`$$…$$`) math render to MathML.
- **wiki** — a flat space of linked pages. `[[wikilinks]]` (created on click when they don't
  exist yet) and a "linked from" panel that spans **every section** — notes, wiki pages, even
  task and goal bodies that reference a page show up there; renaming a page updates inbound
  links across all of them.
- **tasks** — a kanban board (one markdown file per task) with quick capture (type a title in a
  column's footer, Enter, repeat), free-text task titles, due dates that flag overdue / due-today,
  a backlog of unstarted work, drag between columns *and* to a precise spot within one,
  search / sort / hide-done, columns editable in settings, and a detail drawer that edits the
  task body with the same block editor. **Goals** (milestones) group tasks and show progress.

  Task-list checkboxes (`- [ ]`) in any rendered view are clickable — a click flips the marker
  in the markdown itself.

Tabs keep several entries open at once: single-click to preview, edit to keep, restored when you
reopen a well. **Split** the editor into two panes side by side (drag a tab across, or to the edge),
and **search** across all three sections with `Ctrl+K`. Right-click any entry for its actions.
Bigger bets still ahead: an MCP server over the store, on-device AI, and optional sync.

## Install

**Windows** — download `ido_<version>_x64-setup.exe` from the
[latest release](https://github.com/latent-foundation/ido/releases/latest) and run it.
Releases are built from the tag by
[GitHub Actions](.github/workflows/release.yml).

> [!WARNING]
> **The installer is not code signed yet.** Windows SmartScreen will show
> "Windows protected your PC" the first time you run it — click **More info**, then
> **Run anyway**.
>
> Every release publishes a `SHA256SUMS` file. Verify your download before running it:
>
> ```powershell
> Get-FileHash .\ido_1.0.0_x64-setup.exe -Algorithm SHA256
> ```
>
> If you would rather not trust an unsigned binary, build from source below — it is
> the same code. Code signing is planned, but not yet in place.

macOS and Linux builds aren't published yet; both build from source.

## Develop

Prerequisites: the Rust toolchain with the `wasm32-unknown-unknown` target, plus
`just`, `trunk`, `leptosfmt`, and the Tauri CLI. See
[`vendor/latent-design/docs/bootstrap-new-app.md`](vendor/latent-design/docs/bootstrap-new-app.md)
for the full setup.

```sh
git submodule update --init --recursive   # populate vendor/latent-design (styling + canon)
just dev                                   # run the desktop app, hot reload
just verify                                # format check + lint — exactly what CI runs
```

## Learn more

- [CLAUDE.md](CLAUDE.md) — architecture, commands, conventions, current state, and what's next.
- [`vendor/latent-design/docs/`](vendor/latent-design/docs/) — the ecosystem canon
  (architecture, conventions, glossary).
- `vendor/latent-design/README.md` — the latent. brand canon.
