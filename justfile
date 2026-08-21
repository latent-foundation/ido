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

# Type-check + lint the whole workspace, warnings as errors.
check:
    cargo clippy --workspace -- -D warnings

# Backend logic tests (the store: tree / create / rename / move / delete)
# plus the MCP server's tool rendering and its end-to-end stdio test.
test:
    cargo test -p ido-store
    cargo test -p ido
    cargo test -p ido-mcp

# Exactly what CI runs.
verify: fmt-check check test

# Build ido-mcp (release) and stage it as the Tauri sidecar for this host's
# target triple (docs/mcp-server.md P3): `bundle.externalBin` in
# tauri.conf.json expects `src-tauri/binaries/ido-mcp-<triple>[.exe]` to exist
# at dev/build time. `cargo tauri dev` needs it (wired below via `dev`/
# `dev-debug`); `cargo tauri build` needs it too but isn't a just recipe here,
# so run `just sidecar` before it by hand.
[windows]
sidecar:
    cargo build --release -p ido-mcp
    New-Item -ItemType Directory -Force -Path src-tauri/binaries | Out-Null
    $triple = ((rustc -vV | Select-String '^host:') -replace 'host:\s*', '').Trim(); Copy-Item -Force target/release/ido-mcp.exe "src-tauri/binaries/ido-mcp-$triple.exe"

[unix]
sidecar:
    cargo build --release -p ido-mcp
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

# Run ido-mcp under the MCP inspector for protocol-level debugging (needs Node).
mcp-inspect:
    npx @modelcontextprotocol/inspector cargo run -p ido-mcp
