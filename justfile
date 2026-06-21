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

# Exactly what CI runs.
verify: fmt-check check

# Dev: Trunk dev server + native window, hot reload.
dev:
    cargo tauri dev
