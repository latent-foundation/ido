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
  tests), `window`, `external` (open URLs in the OS). Commands are `pub` in their module and
  listed in `generate_handler!`.
- **Frontend** (`src/`): `model`, `ipc` (**the only place that calls `invoke`** — typed wrappers +
  arg structs), `state` (a `State` struct of all signals, provided via Leptos context; backend
  work lives in its action methods), `blocks` (top-level block segmentation for the live editor —
  `segment` splits source into `Block`s with byte ranges; `splice` commits an edited block back
  without touching the rest of the document), `markdown` (markdown → HTML via pulldown-cmark;
  LaTeX math → MathML via latex2mathml; raw HTML sanitised to text; `Event`-transform seam for
  future wikilinks/tags), `icon`, `components/{titlebar,launch,editor,tree,settings}`,
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
- **Tests:** two suites, neither included in `just verify` yet for the frontend:
  - **Backend** (`cargo test -p ido` / `just test`): tempdir-based unit tests in
    [src-tauri/src/lib.rs](src-tauri/src/lib.rs); note/folder commands take plain args, no mock
    runtime needed. `just verify` includes these.
  - **Frontend** (`cargo test -p ido-ui`): pure-Rust unit tests in `src/blocks.rs` covering
    block segmentation and splice round-trips (11 tests). These run on the host target (no WASM
    needed) and catch range/separator bugs early. Run them explicitly — `just verify` does not
    include them yet.
  - No E2E tests yet — `tauri-driver` + WebdriverIO is the planned next layer. Verify UI changes
    by running the app (`just dev`).

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
- **Rendered-markdown links must not navigate the webview.** The reading view intercepts `<a>`
  clicks and routes them to `open_external` (OS browser); a link that navigated the webview would
  white-screen the app. That handler (in `components/editor.rs`) is where internal `[[wikilinks]]`
  will branch later. Raw HTML in notes is rendered as text, not executed (see `markdown.rs`).
- Frontend crate is edition 2021; the ecosystem convention is edition 2024. Don't "fix" this
  silently.

## Current state / not built yet

Two screens, gated on whether a well is open: a **launch screen** (recent wells + open / create)
and the **editor** (sidebar folder tree + main pane). On startup it reopens the most-recent well,
falling back to the launcher only if there's none. The window is borderless with its own title
bar: fixed / non-resizable on the launcher, resizable in the editor.

**Sidebar tree** — a recursive `Tree` / `TreeRow` component pair (`Tree` returns `AnyView` to
break the recursive-`impl Trait` cycle, E0720). New note / folder are created into the selected
`target` folder, rename is inline, delete removes notes (and empty folders only), rows are
drag-and-droppable to move them between folders (`move_entry`; drop on the empty list area moves
to the well root).

**Main pane — three modes** (icon buttons in the editor header):

- **Source** (`</>`): a single uncontrolled raw-markdown `<textarea>`, seeded imperatively from
  `state.editor` (`NodeRef`) on open/mode-switch to avoid cursor jumps. Autosaves on every input
  event (no debounce yet).

- **Live** (pencil ✏): **block live-preview editor**, the primary editing surface. `src/blocks.rs`
  (`segment`) splits the document source into top-level `Block`s, each with a raw-source string
  and a byte range. `BlockEditor` renders the list: inactive blocks show rendered HTML (via
  `markdown::render`), the active block shows a `<textarea>`. Keyboard model:
  - **Enter on a blank line** — splits or appends a block. When the cursor sits on a blank line
    anywhere in the textarea (detected by `cursor_on_blank_line`), pressing Enter fires
    `State::split_block`: content before the blank line stays as the current block, content after
    becomes the next block (or an empty new block is appended). The blank line acts as a block
    separator, not literal content.
  - **↑ / ↓ at first/last line** — `commit_and_go` saves the current block and moves focus to
    the adjacent one.
  - **Backspace at column 0** — `merge_with_prev` concatenates the current block onto the end of
    the previous one (joined by `\n\n`) and focuses the merge point.
  - **Escape** — deactivates the current block without committing (reverts to last-saved).
  - **Blur** — commits as a failsafe (guarded against double-commit when the guard signal is
    already cleared).
  - `splice` writes the edited block back into the full document source at its byte range without
    touching any other block. pulldown-cmark's `End`-event ranges include the trailing newline;
    block ranges are stored using `trim_end` length to avoid eating inter-block separators on
    splice.

- **Reading** (eye 👁): fully rendered, read-only. `<a>` clicks are intercepted and routed to
  `open_external` (OS browser) — navigating the webview directly would white-screen the app. This
  handler is where internal `[[wikilinks]]` will branch later.

**Math** — both `markdown.rs` and `blocks.rs` enable `Options::ENABLE_MATH`. `pulldown-cmark`
emits `Event::InlineMath` (`$…$`) and `Event::DisplayMath` (`$$…$$`); the `transform` function
in `markdown.rs` converts them to MathML via `latex2mathml` before the HTML-sanitisation arm
runs. Display `<math>` elements must **not** have `display: block` set in CSS — Chromium's UA
stylesheet maps `math[display="block"]` → `display: block math`, and overriding it with
`display: block` removes the `math` inner display type and garbles MathML layout.

**Not built yet:** search, wikilinks, and the wiki / task / goal surfaces ido is ultimately
aiming at.
