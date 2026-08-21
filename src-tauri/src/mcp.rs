//! App-side glue for the bundled `ido-mcp` sidecar (see `crates/ido-mcp` and
//! `docs/mcp-server.md` P3). This app never spawns the server itself — stdio
//! MCP servers are spawned by the client — so the only job here is resolving
//! where the sidecar landed and handing the settings pane copy-paste config
//! for wiring an MCP client (Claude Code first) at the current well.

use std::path::PathBuf;

use serde::Serialize;

/// Everything the settings pane needs to point an MCP client at a well.
#[derive(Serialize)]
pub struct McpInfo {
    /// Absolute path to the `ido-mcp` sidecar next to the running executable,
    /// if it's there. `None` in a dev build before `just sidecar` has run.
    pub bin: Option<String>,
    /// A ready-to-paste `.mcp.json` snippet (see `docs/mcp-server.md` Appendix
    /// A). Falls back to the bare command name (`ido-mcp`, relying on `PATH`)
    /// when `bin` is `None`, so the snippet is still useful.
    pub json: String,
    /// A ready-to-paste `claude mcp add` one-liner, same fallback as `json`.
    pub cli: String,
}

/// The sidecar's platform-specific file name next to the app executable.
/// `cfg!(windows)` (not `target_os`) is correct here — this runs natively in
/// the Tauri backend, not in the WASM frontend, so it describes the machine
/// actually running the code.
fn sidecar_name() -> &'static str {
    if cfg!(windows) {
        "ido-mcp.exe"
    } else {
        "ido-mcp"
    }
}

/// Resolve the `ido-mcp` sidecar next to the running executable. In dev,
/// `cargo tauri dev` copies the `externalBin` sidecar into `target/debug/`
/// beside `ido.exe`; in a bundle it sits beside the installed binary — so
/// this one lookup resolves in both.
fn find_sidecar() -> Option<PathBuf> {
    let dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let path = dir.join(sidecar_name());
    path.is_file().then_some(path)
}

/// Resolve the sidecar for `well` and build the settings pane's copy-paste
/// MCP client config.
#[tauri::command]
pub fn mcp_info(well: String) -> McpInfo {
    let bin = find_sidecar().map(|p| p.to_string_lossy().into_owned());
    let command = bin.clone().unwrap_or_else(|| "ido-mcp".to_string());
    let cli = format!(r#"claude mcp add --scope user ido -- "{command}" --well "{well}""#);
    let json = serde_json::to_string_pretty(&serde_json::json!({
        "mcpServers": {
            "ido": {
                "command": command,
                "args": ["--well", well],
            }
        }
    }))
    .unwrap_or_default();
    McpInfo { bin, json, cli }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `.mcp.json` snippet is valid JSON and carries the well path
    /// through to `args`, and the CLI one-liner mentions it too.
    #[test]
    fn json_snippet_round_trips_and_contains_well() {
        let info = mcp_info("/Users/me/well".to_string());
        let parsed: serde_json::Value = serde_json::from_str(&info.json).unwrap();
        assert_eq!(parsed["mcpServers"]["ido"]["args"][1], "/Users/me/well");
        assert!(info.cli.contains("/Users/me/well"));
    }

    /// A Windows-style path with backslashes must escape correctly inside the
    /// JSON string and decode back to the exact same path.
    #[test]
    fn windows_style_path_round_trips() {
        let info = mcp_info(r"C:\Users\me\well".to_string());
        let parsed: serde_json::Value = serde_json::from_str(&info.json).unwrap();
        assert_eq!(parsed["mcpServers"]["ido"]["args"][1], r"C:\Users\me\well");
        assert!(info.cli.contains(r"C:\Users\me\well"));
    }
}
