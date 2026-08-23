# latent. — common dev tasks. See vendor/latent-design/docs/conventions.md.
# On Windows, run recipes through PowerShell so `sh` isn't required on PATH.
set windows-shell := ["powershell.exe", "-NoLogo", "-Command"]

# List recipes.
default:
    @just --list

# Format Rust + Leptos `view!` macros. `cargo fmt` alone corrupts `view!`.
fmt:
    cargo fmt
    leptosfmt src

# Format check — runs in the pre-commit hook and CI.
fmt-check:
    cargo fmt --check
    leptosfmt --check src

# Type-check + lint the whole workspace, warnings as errors — twice over,
# because `--workspace` only ever sees default features and the semantic index
# (docs/mcp-server.md P2) is behind a cargo feature: without the second and
# third lines the entire candle path would go unlinted. The candle tree is a
# ~2-minute cold compile, then cached (CI keeps it in `Swatinem/rust-cache`).
# Depends on
# `sidecar` because compiling the `ido` (src-tauri) crate runs tauri-build,
# which validates `bundle.externalBin` and hard-fails when the staged binary is
# missing — as it is on any fresh clone, since src-tauri/binaries/ is gitignored.
check: sidecar
    cargo clippy --workspace -- -D warnings
    cargo clippy -p ido-store --features semantic --all-targets -- -D warnings
    cargo clippy -p ido-mcp --features semantic --all-targets -- -D warnings

# Backend logic tests (the store: tree / create / rename / move / delete)
# plus the MCP server's tool rendering and its end-to-end stdio test.
# The last two lines cover the semantic index. Tests that need the embedding
# model on disk are `#[ignore]`d (they would download ~133 MB), so these stay
# offline and fast — run them by hand with `--ignored` after `just eval`.
# Needs `sidecar` for the same reason `check` does (`cargo test -p ido`).
test: sidecar
    cargo test -p ido-store
    cargo test -p ido
    cargo test -p ido-mcp
    cargo test -p ido-store --features semantic
    cargo test -p ido-mcp --features semantic

# Exactly what CI runs. `just` runs each dependency at most once per
# invocation, so the sidecar `check` and `test` both pull in is built once.
verify: fmt-check check test

# Build ido-mcp (release) and stage it as the Tauri sidecar for this host's
# target triple (docs/mcp-server.md P3): `bundle.externalBin` in
# tauri.conf.json expects `src-tauri/binaries/ido-mcp-<triple>[.exe]` to exist
# at dev/build time — so *anything* that compiles the src-tauri crate needs it,
# not just a bundle: `check`, `test`, `dev` and `dev-debug` all depend on this
# recipe. `cargo tauri build` needs it too but isn't a just recipe here, so run
# `just sidecar` before it by hand.
#
# Built **with `--features semantic`** since P3: the app's settings pane can now
# fetch the embedding model into the per-machine cache at
# `<data_dir>/latent.ido/models/`, and the sidecar reads that same cache — so a
# semantic sidecar can actually obtain weights rather than silently degrading to
# keyword. Costs a one-time ~2-minute candle release build, cached thereafter.
[windows]
sidecar:
    cargo build --release -p ido-mcp --features semantic
    New-Item -ItemType Directory -Force -Path src-tauri/binaries | Out-Null
    $triple = ((rustc -vV | Select-String '^host:') -replace 'host:\s*', '').Trim(); Copy-Item -Force target/release/ido-mcp.exe "src-tauri/binaries/ido-mcp-$triple.exe"

[unix]
sidecar:
    cargo build --release -p ido-mcp --features semantic
    mkdir -p src-tauri/binaries
    triple=$(rustc -vV | grep '^host:' | awk '{print $2}') && cp target/release/ido-mcp "src-tauri/binaries/ido-mcp-$triple"

# Dev: Trunk dev server + native window, hot reload.
dev: sidecar
    cargo tauri dev

# Dev with WebView2 remote debugging on :9222 (Windows only — WebView2 is
# Chromium-based, so this exposes the Chrome DevTools Protocol: DOM queries,
# scripted clicks, screenshots. Used for AI-driven / automated UI verification;
# see `.claude/skills/run` for the driver script). Kill the window to stop it.
dev-debug: sidecar
    $env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = '--remote-debugging-port=9222'; cargo tauri dev

# Run ido-mcp against the most recent well, stderr to the terminal. stdout is
# the JSON-RPC channel, so this only looks alive once a client talks to it —
# for hands-on poking use `just mcp-inspect`. Add `-- --well <path>` to pin one.
mcp:
    cargo run -p ido-mcp

# Same, with the four write tools live (docs/mcp-server.md §8). Pointed at the
# committed dev-well fixture on purpose: writes land in scratch data you can
# throw away, not in whatever well you last had open. Add `-- --well <path>` to
# aim it somewhere else, deliberately.
mcp-write:
    cargo run -p ido-mcp -- --well dev-well --allow-write

# Score retrieval (keyword / semantic / hybrid) over the committed eval well and
# apply docs/mcp-server.md §6.7's gate: hybrid must not do worse than keyword on
# recall@5, overall or on the exact-identifier queries. Exits non-zero when the
# gate fails. Release, because a debug candle build makes embedding glacial; the
# model must already be downloaded (`--reindex` or the app's settings pane).
eval:
    cargo run --release -p ido-mcp --features semantic -- --well crates/ido-store/tests/fixtures/eval-well --eval crates/ido-store/tests/fixtures/eval-well/eval.jsonl

# Rebuild the semantic index of the most recent well from scratch, then exit.
# Add `-- --well <path>` to pin one. Downloads the embedding model on first run.
reindex:
    cargo run --release -p ido-mcp --features semantic -- --reindex

# Run ido-mcp under the MCP inspector for protocol-level debugging (needs Node).
mcp-inspect:
    npx @modelcontextprotocol/inspector cargo run -p ido-mcp
