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

`Ctrl+K` searches by **meaning as well as wording**. Enable it once in settings — ido downloads a
small embedding model (~130 MB, the only time it ever uses the network) and indexes the well — and
from then on "why did we avoid shipping a native library" finds the note that argued it, even
though it shares no words with it. Everything stays on your machine, and if you never turn it on,
search keeps working exactly as before.

Any MCP client can read the well too — see [Agent access](#agent-access-mcp) below. Bigger bets
still ahead: on-device AI and optional sync.

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
> Get-FileHash .\ido_<version>_x64-setup.exe -Algorithm SHA256
> ```
>
> If you would rather not trust an unsigned binary, build from source below — it is
> the same code. Code signing is planned, but not yet in place.

macOS and Linux builds aren't published yet; both build from source.

## Agent access (MCP)

Any [MCP](https://modelcontextprotocol.io) client — Claude Code first — can read a well:
notes, wiki, and tasks, searchable and addressable by id. The server is **read-only unless you
explicitly start it with `--allow-write`**, and needs no network access at all; it doesn't touch
anything ido itself couldn't also read.

It's a separate, on-demand process, not a background service — **ido doesn't need to be
running** for a client to use it, and you never start it by hand. Setup is install once,
register once:

1. **Install ido** (above). The `ido-mcp` server binary ships alongside it.
2. **Open ido → settings → "agent access — mcp"** and copy one of the two snippets shown
   there for your currently-open well:
   - a `.mcp.json` entry, to register the server for one project, or
   - a `claude mcp add …` command, to register it globally for every Claude Code session.
3. Done. From then on, Claude Code spawns the server itself whenever it needs a tool call
   and shuts it down when it's finished — the same way it manages any other stdio MCP server.

The server exposes seven read-only tools (`well_info`, `search`, `get_entry`, `list_entries`,
`backlinks`, `list_tasks`, `list_goals`). `search` is **hybrid** — the same meaning-plus-wording
retrieval `Ctrl+K` uses, sharing the same index — and degrades to plain keyword search, saying so,
whenever the model or index isn't there. Every entry is also addressable as a resource
(`ido://note/…`, `ido://wiki/…`, `ido://task/…`, `ido://goal/…`), and two prompts —
`daily_review` and `weekly_digest` — summarise the well for you.

Adding `--allow-write` to the server's arguments turns on four more tools: create a note or wiki
page, append to an existing one, create a task, and set one field of a task. **None of them can
destroy anything** — there is no delete, nothing is ever overwritten or truncated, and a name
that's already taken gets a suffix rather than clobbering what's there. Leave the flag off and
those tools don't exist at all.

Full design and roadmap: [`docs/mcp-server.md`](docs/mcp-server.md).

For development, `just mcp` runs the server directly against your most-recent well (stderr to
the terminal) and `just mcp-inspect` runs it under the
[MCP inspector](https://github.com/modelcontextprotocol/inspector) for protocol-level debugging.

## Develop

Prerequisites: the Rust toolchain with the `wasm32-unknown-unknown` target, plus
`just`, `trunk`, `leptosfmt`, and the Tauri CLI. See
[`vendor/latent-design/docs/bootstrap-new-app.md`](vendor/latent-design/docs/bootstrap-new-app.md)
for the full setup.

```sh
git submodule update --init --recursive   # populate vendor/latent-design (styling + canon)
cp .mcp.json.example .mcp.json             # optional: dev MCP servers — edit the well paths
just dev                                   # run the desktop app, hot reload
just verify                                # format check + lint — exactly what CI runs
```

## Learn more

- [CLAUDE.md](CLAUDE.md) — architecture, commands, conventions, current state, and what's next.
- [`vendor/latent-design/docs/`](vendor/latent-design/docs/) — the ecosystem canon
  (architecture, conventions, glossary).
- `vendor/latent-design/README.md` — the latent. brand canon.
