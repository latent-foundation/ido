# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`ido` (井戸, "a well") is a local-first, markdown-based knowledge app in the **latent.**
ecosystem — notes now, growing toward notes + wiki + project management. It is a Tauri
desktop app with a Leptos (CSR → WASM) frontend. Notes live in a **well** — any folder the
user picks; notes are the markdown files inside it (subfolders form the tree). The UI is
intentionally bare (open/create a well, then list/open/edit/create notes) and grows from there.
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
- **`src-tauri/`** — the Rust backend and the **local-first store**. The only disk access is the
  Tauri commands: `recent_wells` / `pick_folder` / `open_well` / `create_well` (wells), and
  `list_tree` / `read_note` / `write_note` / `create_note` / `create_folder` / `rename_entry` /
  `delete_entry` / `move_entry` (notes & folders, scoped to a well path — a note's **name is its
  file name**, independent of body text). The folder picker uses `tauri-plugin-dialog` **from
  Rust**, so no capability entry is needed; recent wells are remembered in
  `<app_data_dir>/wells.json`. Window chrome is custom (native decorations off), driven from Rust:
  `apply_window` (size + resizable), `show_window`, and `win_minimize` / `win_toggle_maximize` /
  `win_close`. `withGlobalTauri` is on.

Both crates are split into small, documented modules (module-level `//!` + item `///` docs):
- **Backend** (`src-tauri/src/`): `lib.rs` wires modules + `run()`; `model` (shapes), `paths`
  (pure id/name helpers + tests), `registry` (recent wells), `wells`, `notes` (tree + CRUD +
  tests), `window`. Commands are `pub` in their module and listed in `generate_handler!`.
- **Frontend** (`src/`): `model`, `ipc` (**the only place that calls `invoke`** — typed wrappers +
  arg structs), `state` (a `State` struct of all signals, provided via Leptos context; backend
  work lives in its action methods), `icon`, `components/{titlebar,launch,editor,tree,settings}`,
  and `app` (root composition). Components read `expect_context::<State>()` instead of
  prop-drilling. To add a feature: add a command (backend module + `generate_handler!`), an `ipc`
  wrapper, a `State` method, and a component.

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
- **Tests:** the backend store logic has unit tests in [src-tauri/src/lib.rs](src-tauri/src/lib.rs)
  (`#[cfg(test)]`, tempdir-based — the note/folder commands take plain args, so they're called
  directly, no mock runtime). `just test` runs them; `just verify` includes them. No frontend /
  E2E tests yet — E2E via `tauri-driver` + WebdriverIO is the planned next layer. Verify UI
  changes by running the app (`just dev`).

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
  buttons call Rust commands, so they need no capability. **In-webview HTML5 drag-and-drop (the
  note tree) requires `dragDropEnabled: false` on the window** — the OS file-drop handler
  otherwise swallows `dragstart`/`drop` before they reach the page.
- Frontend crate is edition 2021; the ecosystem convention is edition 2024. Don't "fix" this
  silently.

## Current state / not built yet

Two screens, gated on whether a well is open: a **launch screen** (recent wells + open / create)
and the **editor** (a **folder tree** of notes + a textarea, autosaving on every keystroke — no
debounce yet). On startup it reopens the most-recent well, falling back to the launcher only if
there's none. The window is borderless with our own title bar: fixed / non-resizable on the
launcher, resizable in the editor. The sidebar tree is a recursive `Tree` / `TreeRow` component
pair — `Tree` returns `AnyView` to break the recursive-`impl Trait` cycle (E0720); new note /
folder are created into the selected `target` folder, rename is inline, delete removes notes (and
empty folders only), and rows are drag-and-droppable to move them between folders (`move_entry`;
drop on the empty list area moves to the well root). The editor is **uncontrolled** — content
pushed in imperatively via the `state.editor` `NodeRef` on open, to avoid cursor jumps from a
reactive `value` binding. Not built yet: markdown rendering (you edit raw markdown), search, and
the wiki / task / goal surfaces ido is ultimately aiming at.
