//! `ido-mcp` — a read-only [MCP](https://modelcontextprotocol.io) server over
//! one ido well, spoken over stdio.
//!
//! An MCP client (Claude Code first) spawns this binary; it reads the well's
//! markdown through [`ido_store`] and answers seven tools. The app does not
//! need to be running, and the two processes share nothing but the filesystem
//! (`docs/mcp-server.md` §3.3).
//!
//! **stdout is the JSON-RPC channel.** Every diagnostic goes to stderr, and
//! stays sparse: a startup line and errors, never entry bodies (§9).
//!
//! This module owns argument parsing and well resolution; [`server`] is the
//! MCP layer and [`tools`] holds the tool bodies.

mod render;
mod server;
mod tools;

use std::path::{Path, PathBuf};

use rmcp::ServiceExt;
use rmcp::transport::stdio;

/// `--help` output. Written to stdout, which is safe because `--help` exits
/// before the JSON-RPC transport is ever opened.
const USAGE: &str = "\
ido-mcp — read-only MCP server over one ido well (stdio transport)

USAGE:
    ido-mcp [--well <path>]

OPTIONS:
    --well <path>   The well folder to serve. Falls back to the IDO_WELL
                    environment variable, then to the most recently opened well
                    in ido's registry.
    -h, --help      Print this help and exit.
    -V, --version   Print the version and exit.

One well per process: register the server once per well in your MCP client.
The server never writes to the well.";

/// Exit code for a usage or well-resolution failure (the well is the one thing
/// this process cannot start without).
const EXIT_USAGE: i32 = 2;

/// Parse `--well` / `--help` / `--version` by hand — three flags do not earn a
/// CLI-parsing dependency. `--help` and `--version` exit the process directly.
fn parse_args(args: impl Iterator<Item = String>) -> Result<Option<PathBuf>, String> {
    let mut well = None;
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            "-V" | "--version" => {
                println!("ido-mcp {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "--well" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--well needs a path".to_string())?;
                well = Some(PathBuf::from(value));
            }
            other => match other.strip_prefix("--well=") {
                Some(value) => well = Some(PathBuf::from(value)),
                None => return Err(format!("unknown argument `{other}`\n\n{USAGE}")),
            },
        }
    }
    Ok(well)
}

/// The most-recently-opened well from ido's registry, if it still exists.
///
/// Replicates `src-tauri`'s `registry` module rather than depending on it (that
/// module needs a Tauri `AppHandle`): a JSON array of absolute paths,
/// most-recent first, at `<app_data_dir>/latent.ido/wells.json`. `dirs`'
/// `data_dir` matches Tauri's `app_data_dir` base on all three platforms —
/// `%APPDATA%`, `~/Library/Application Support`, `$XDG_DATA_HOME`.
fn most_recent_well() -> Option<PathBuf> {
    let path = dirs::data_dir()?.join("latent.ido").join("wells.json");
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str::<Vec<String>>(&raw)
        .ok()?
        .into_iter()
        .map(PathBuf::from)
        .find(|p| p.is_dir())
}

/// Which well to serve: the flag, then `IDO_WELL`, then the registry
/// (`docs/mcp-server.md` §3.4). The result is checked to be a real folder here
/// so the failure is a clear startup message rather than seven empty tools.
///
/// Returns the resolution *source* alongside the path. It costs one word in
/// the startup banner and answers the question that actually bites when more
/// than one server is registered — "which well is this one on, and did I pin
/// it or did the registry pick for me?". A client tags every stderr line as an
/// error (§9: stderr is the only channel — the spec's logging capability is
/// deprecated), so each line has to earn its place.
fn resolve_well(flag: Option<PathBuf>) -> Result<(PathBuf, &'static str), String> {
    let env_well = std::env::var("IDO_WELL")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    let (well, source) = match (flag, env_well) {
        (Some(w), _) => (w, "--well"),
        (None, Some(w)) => (w, "IDO_WELL"),
        (None, None) => (
            most_recent_well().ok_or_else(|| {
                "no well to serve. Pass --well <path>, set IDO_WELL, or open a well in ido \
                 once so it lands in the recent-wells registry."
                    .to_string()
            })?,
            "the recent-wells registry",
        ),
    };
    if !well.is_dir() {
        return Err(format!(
            "`{}` (from {source}) is not a folder",
            well.display()
        ));
    }
    Ok((well, source))
}

/// Warn — on stderr, without refusing to serve — when the folder has no
/// `.ido/well.toml`. Every store read tolerates a missing section, so an
/// almost-a-well still answers usefully; silently pretending it's fine would
/// not.
fn warn_if_unscaffolded(well: &Path) {
    if !well.join(".ido").join("well.toml").exists() {
        eprintln!(
            "ido-mcp: warning: `{}` has no .ido/well.toml — serving it anyway, but it may not \
             be an ido well (this server never writes, so it will not scaffold one).",
            well.display()
        );
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let flag = match parse_args(std::env::args().skip(1)) {
        Ok(flag) => flag,
        Err(message) => {
            eprintln!("ido-mcp: {message}");
            std::process::exit(EXIT_USAGE);
        }
    };
    let (well, source) = match resolve_well(flag) {
        Ok(resolved) => resolved,
        Err(message) => {
            eprintln!("ido-mcp: {message}");
            std::process::exit(EXIT_USAGE);
        }
    };
    warn_if_unscaffolded(&well);
    eprintln!(
        "ido-mcp {} — serving `{}` (from {source}) read-only over stdio (keyword search)",
        env!("CARGO_PKG_VERSION"),
        well.display()
    );

    server::Ido::open(&well)
        .serve(stdio())
        .await?
        .waiting()
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> impl Iterator<Item = String> {
        list.iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
            .into_iter()
    }

    #[test]
    fn well_flag_parses_in_both_spellings() {
        assert_eq!(parse_args(args(&[])).unwrap(), None);
        assert_eq!(
            parse_args(args(&["--well", "/tmp/w"])).unwrap(),
            Some(PathBuf::from("/tmp/w"))
        );
        assert_eq!(
            parse_args(args(&["--well=/tmp/w"])).unwrap(),
            Some(PathBuf::from("/tmp/w"))
        );
    }

    #[test]
    fn bad_arguments_are_errors_not_panics() {
        assert!(parse_args(args(&["--well"])).is_err(), "missing value");
        assert!(parse_args(args(&["--wel", "/tmp/w"])).is_err(), "typo");
        assert!(parse_args(args(&["/tmp/w"])).is_err(), "positional");
    }

    #[test]
    fn a_missing_folder_is_rejected() {
        let err = resolve_well(Some(PathBuf::from("./definitely-not-a-well-xyz"))).unwrap_err();
        assert!(err.contains("is not a folder"), "got: {err}");
        assert!(err.contains("--well"), "the message names the source");
    }
}
