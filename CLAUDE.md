# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`ido` (井戸, "a well") is a local-first, markdown-based knowledge app in the **latent.**
ecosystem — notes now, growing toward notes + wiki + project management. It is a Tauri
desktop app with a Leptos (CSR → WASM) frontend. Notes are local markdown files on disk;
the UI is intentionally bare (list, open, edit, create) and grows from there.
[README.md](README.md) is the human-facing overview; this file is the working guide.

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
- **`src-tauri/`** — the Rust backend and the **local-first store**. `src-tauri/src/lib.rs`
  exposes the only disk access as Tauri commands — `list_notes`, `read_note`, `write_note`,
  `create_note` — over markdown files in `<app_data_dir>/notes`. The frontend calls them via
  the `invoke` binding in `src/app.rs`. `withGlobalTauri` is on.

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
just verify        # fmt-check + clippy (-D warnings) — exactly what CI runs; run before pushing
just fmt           # cargo fmt + leptosfmt (the only correct way to format — see below)
just check         # cargo clippy --workspace -- -D warnings
just dev           # cargo tauri dev: Trunk on :1420 + native window, hot reload
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
- **Tests: none yet.** Verify changes by running the app (`just dev`), not by adding test
  scaffolding unless asked.

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
- **Icons are Lucide only** — and inlined as SVG, not via the CDN script (it can't re-bind
  icons across reactive re-renders). Use 1.6px stroke and `currentColor`. Never emoji, never
  hand-drawn. (The current bare UI has no icons yet.)
- **Brand:** lowercase voice; the `latent.` mark is the umbrella identity **only** — ido's mark
  is the 井戸 kanji. The current app icon ([src-tauri/app-icon.svg](src-tauri/app-icon.svg)) uses
  the latent mark as a temporary placeholder.
- Frontend crate is edition 2021; the ecosystem convention is edition 2024. Don't "fix" this
  silently.

## Current state / not built yet

The app is a deliberately bare local-first markdown editor: a sidebar list of notes + a
textarea, autosaving to disk on every keystroke (no debounce yet). The editor is **uncontrolled**
— content is pushed in imperatively via a `NodeRef` on open, to avoid cursor jumps from a
reactive `value` binding. Not built yet: markdown rendering (you edit raw markdown), delete /
rename, search, and the wiki / task / goal surfaces ido is ultimately aiming at.
