# ido

> 井戸 — a quiet well for thought.

`ido` is a local-first, markdown-based knowledge system: notes now, growing toward
notes, wiki, and project management. The archive is plain `.md` files on your own
machine — durable, portable, yours. No lock-in, no latency, no surveillance of thought.

Part of the **latent.** ecosystem. A desktop app built with
Tauri + Leptos (Rust).

## Status

Early, but functional. Today ido has two screens — a **launch screen** (open / create
a well) and an **editor** with three modes:

- **Source** — raw markdown textarea, autosaves on every keystroke.
- **Live** — block live-preview. Each top-level markdown block renders as HTML; click
  to open it in a textarea. Press Enter on a blank line to split or append a block,
  Backspace at column 0 to merge with the block above, and ↑/↓ to navigate.
- **Reading** — fully rendered, read-only. Links open in the OS browser.

Inline math (`$…$`) and display math (`$$…$$`) render to MathML. The note tree
supports create, rename, delete, and drag-to-move. Not yet built: search, wikilinks,
and the wiki / task / goal surfaces ido is ultimately aiming at.

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

- [CLAUDE.md](CLAUDE.md) — architecture, commands, and conventions for this repo.
- [`vendor/latent-design/docs/`](vendor/latent-design/docs/) — the ecosystem canon
  (architecture, conventions, glossary).
- `vendor/latent-design/README.md` — the latent. brand canon.
