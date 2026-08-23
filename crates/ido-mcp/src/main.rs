//! `ido-mcp` — a read-only [MCP](https://modelcontextprotocol.io) server over
//! one ido well, spoken over stdio.
//!
//! An MCP client (Claude Code first) spawns this binary; it reads the well's
//! markdown through [`ido_store`] and answers seven tools. The app does not
//! need to be running, and the two processes share nothing but the filesystem
//! (`docs/mcp-server.md` §3.3).
//!
//! Read-only is the default and the whole posture. `--allow-write` opts one
//! process into four additional, never-destructive tools (§8); without it they
//! are not advertised, so a client can't so much as attempt one.
//!
//! **stdout is the JSON-RPC channel.** Every diagnostic goes to stderr, and
//! stays sparse: a startup line and errors, never entry bodies (§9).
//!
//! This module owns argument parsing, well resolution, and the two offline
//! commands' entry points; [`server`] is the MCP layer, [`tools`] and [`write`]
//! hold the tool bodies, [`resources`] and [`prompts`] hold the P4 resources
//! and prompts surfaces (§5.3, §5.4), [`semantic`] owns the embedder and
//! index freshness, and [`maintenance`] implements `--reindex` / `--eval`.

mod maintenance;
mod prompts;
mod render;
mod resources;
mod semantic;
mod server;
mod tools;
mod write;

use std::path::{Path, PathBuf};

use rmcp::ServiceExt;
use rmcp::transport::stdio;

/// `--help` output. Written to stdout, which is safe because `--help` exits
/// before the JSON-RPC transport is ever opened.
const USAGE: &str = "\
ido-mcp — MCP server over one ido well (stdio transport), read-only by default

USAGE:
    ido-mcp [--well <path>] [--allow-write]
    ido-mcp [--well <path>] --reindex
    ido-mcp [--well <path>] --eval <eval.jsonl>

OPTIONS:
    --well <path>   The well folder to serve. Falls back to the IDO_WELL
                    environment variable, then to the most recently opened well
                    in ido's registry.
    --allow-write   Also serve the four write tools (create_entry,
                    append_to_entry, create_task, update_task_field). Off by
                    default; without it they are not advertised at all, so a
                    client cannot attempt one. Even with it, the server never
                    deletes an entry, never overwrites or truncates a body, and
                    edits a task one field at a time.
    --reindex       Rebuild the semantic index from scratch, print its status,
                    and exit without serving.
    --eval <path>   Score keyword / semantic / hybrid retrieval over a JSONL
                    query set, print the table, and exit without serving.
                    Exits 1 if hybrid fails the gate.
    -h, --help      Print this help and exit.
    -V, --version   Print the version and exit.

One well per process: register the server once per well in your MCP client.
Without --allow-write the server never writes or modifies well content. With
semantic search active it maintains a rebuildable search cache under
<well>/.ido/index.";

/// Exit code for a usage failure, an unresolvable well, or a command this
/// build/machine cannot run (no `semantic` feature, no downloaded model).
const EXIT_USAGE: i32 = 2;

/// What this invocation should do. Both offline commands exit before the
/// transport opens, which is what lets them print to stdout.
#[derive(Debug, PartialEq, Eq)]
enum Action {
    /// Speak MCP over stdio (the default).
    Serve,
    /// Force a full index rebuild, then exit.
    Reindex,
    /// Score retrieval over a query set, then exit.
    Eval(PathBuf),
}

/// The parsed command line.
#[derive(Debug, PartialEq, Eq)]
struct Cli {
    /// `--well`, if given (see [`resolve_well`] for the fallbacks).
    well: Option<PathBuf>,
    /// What to do once the well is resolved.
    action: Action,
    /// `--allow-write` (§8). Not an [`Action`]: it modifies serving rather than
    /// replacing it, and the offline commands ignore it.
    allow_write: bool,
}

/// Parse the flags by hand — six do not earn a CLI-parsing dependency.
/// `--help` and `--version` exit the process directly.
fn parse_args(args: impl Iterator<Item = String>) -> Result<Cli, String> {
    let mut well = None;
    let mut action = Action::Serve;
    let mut allow_write = false;
    let set_action = |next: Action, action: &mut Action| -> Result<(), String> {
        if *action != Action::Serve {
            return Err("--reindex and --eval can't be combined".to_string());
        }
        *action = next;
        Ok(())
    };
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
            "--allow-write" => allow_write = true,
            "--reindex" => set_action(Action::Reindex, &mut action)?,
            "--eval" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--eval needs a path to a .jsonl query set".to_string())?;
                set_action(Action::Eval(PathBuf::from(value)), &mut action)?;
            }
            other => {
                if let Some(value) = other.strip_prefix("--well=") {
                    well = Some(PathBuf::from(value));
                } else if let Some(value) = other.strip_prefix("--eval=") {
                    set_action(Action::Eval(PathBuf::from(value)), &mut action)?;
                } else {
                    return Err(format!("unknown argument `{other}`\n\n{USAGE}"));
                }
            }
        }
    }
    Ok(Cli {
        well,
        action,
        allow_write,
    })
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
fn warn_if_unscaffolded(well: &Path, allow_write: bool) {
    if !well.join(".ido").join("well.toml").exists() {
        eprintln!(
            "ido-mcp: warning: `{}` has no .ido/well.toml — serving it anyway, but it may not \
             be an ido well ({}).",
            well.display(),
            if allow_write {
                "this server never scaffolds one, so a write here would land in a folder ido \
                 hasn't claimed"
            } else {
                "this server never writes, so it will not scaffold one"
            }
        );
    }
}

/// Print `message` under the binary's name and exit — the one way this process
/// refuses to start.
fn die(message: &str, code: i32) -> ! {
    eprintln!("ido-mcp: {message}");
    std::process::exit(code)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = parse_args(std::env::args().skip(1)).unwrap_or_else(|m| die(&m, EXIT_USAGE));
    let allow_write = cli.allow_write;
    let (well, source) = resolve_well(cli.well).unwrap_or_else(|m| die(&m, EXIT_USAGE));
    warn_if_unscaffolded(&well, allow_write);

    // The offline commands run on the blocking side of the runtime — they are
    // long, CPU-bound, and nothing else is happening — and exit when done.
    match cli.action {
        Action::Reindex | Action::Eval(_) => {
            let result = tokio::task::spawn_blocking(move || match cli.action {
                Action::Reindex => maintenance::reindex(&well),
                Action::Eval(path) => maintenance::run_eval(&well, &path),
                Action::Serve => unreachable!("guarded by the outer match"),
            })
            .await?;
            match result {
                Ok(()) => std::process::exit(0),
                Err(e) => die(&e.message, e.code),
            }
        }
        Action::Serve => {}
    }

    let ido = server::Ido::open(&well, allow_write);
    // Line one answers "why did an agent change my notes?" without reading any
    // further: the access mode is the first thing after the version.
    eprintln!(
        "ido-mcp {} — {} — serving `{}` (from {source}) over stdio\n{}",
        env!("CARGO_PKG_VERSION"),
        if allow_write {
            "WRITE-ENABLED (--allow-write): agents can create notes, pages and tasks, append to \
             bodies, and set task fields in this well. It never deletes, never overwrites a body"
        } else {
            "READ-ONLY: no tool can create, edit or delete anything in this well (pass \
             --allow-write to enable the four write tools)"
        },
        well.display(),
        match semantic::Semantic::new(&well.to_string_lossy()).unavailable() {
            Some(reason) => format!("ido-mcp: keyword search only — {reason}"),
            None => "ido-mcp: hybrid search available (semantic index maintained in the \
                     background under .ido/index)"
                .to_string(),
        }
    );
    // §6.6: one sweep at startup, detached — the server must be answering
    // `tools/list` in milliseconds, not after a cold index build.
    ido.start_index_maintenance();

    ido.serve(stdio()).await?.waiting().await?;
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

    fn well_of(list: &[&str]) -> Option<PathBuf> {
        parse_args(args(list)).unwrap().well
    }

    #[test]
    fn well_flag_parses_in_both_spellings() {
        assert_eq!(well_of(&[]), None);
        assert_eq!(
            well_of(&["--well", "/tmp/w"]),
            Some(PathBuf::from("/tmp/w"))
        );
        assert_eq!(well_of(&["--well=/tmp/w"]), Some(PathBuf::from("/tmp/w")));
    }

    #[test]
    fn serving_is_the_default_action() {
        assert_eq!(parse_args(args(&[])).unwrap().action, Action::Serve);
        assert_eq!(
            parse_args(args(&["--well", "/tmp/w"])).unwrap().action,
            Action::Serve
        );
    }

    #[test]
    fn the_offline_commands_parse_with_a_well() {
        let cli = parse_args(args(&["--well", "/tmp/w", "--reindex"])).unwrap();
        assert_eq!(cli.well, Some(PathBuf::from("/tmp/w")));
        assert_eq!(cli.action, Action::Reindex);

        for spelling in [vec!["--eval", "q.jsonl"], vec!["--eval=q.jsonl"]] {
            let cli = parse_args(args(&spelling)).unwrap();
            assert_eq!(cli.action, Action::Eval(PathBuf::from("q.jsonl")));
        }
    }

    /// §8's gate is opt-in at the process level, and combines with serving
    /// rather than replacing it (unlike the two offline commands).
    #[test]
    fn writes_are_off_unless_asked_for() {
        assert!(!parse_args(args(&[])).unwrap().allow_write);
        assert!(
            !parse_args(args(&["--well", "/tmp/w"])).unwrap().allow_write,
            "opening a well is not consent to write to it"
        );
        let cli = parse_args(args(&["--well", "/tmp/w", "--allow-write"])).unwrap();
        assert!(cli.allow_write);
        assert_eq!(
            cli.action,
            Action::Serve,
            "it modifies serving, not replaces"
        );
        assert_eq!(cli.well, Some(PathBuf::from("/tmp/w")));
        assert!(
            parse_args(args(&["--allow-write", "--well", "/tmp/w"]))
                .unwrap()
                .allow_write,
            "order-independent"
        );
    }

    #[test]
    fn bad_arguments_are_errors_not_panics() {
        assert!(parse_args(args(&["--well"])).is_err(), "missing value");
        assert!(parse_args(args(&["--wel", "/tmp/w"])).is_err(), "typo");
        assert!(parse_args(args(&["/tmp/w"])).is_err(), "positional");
        assert!(parse_args(args(&["--eval"])).is_err(), "missing query set");
        assert!(
            parse_args(args(&["--allow-writes"])).is_err(),
            "a near-miss spelling must not be silently ignored — it would read as writes-on"
        );
        assert!(
            parse_args(args(&["--reindex", "--eval", "q.jsonl"])).is_err(),
            "two offline commands in one run is a mistake, not a sequence"
        );
    }

    #[test]
    fn a_missing_folder_is_rejected() {
        let err = resolve_well(Some(PathBuf::from("./definitely-not-a-well-xyz"))).unwrap_err();
        assert!(err.contains("is not a folder"), "got: {err}");
        assert!(err.contains("--well"), "the message names the source");
    }
}
