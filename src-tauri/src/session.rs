//! The per-well editor session (open tabs) persisted in `.ido/session.toml`.
//!
//! Pure cache: it records which note tabs were open so reopening a well can
//! restore the workspace. A missing or unparseable file yields an empty
//! session, so a deleted or stale file is never an error.

use std::fs;
use std::path::{Path, PathBuf};

use crate::model::Session;

/// Path to a well's `.ido/session.toml`.
fn session_path(well: &str) -> PathBuf {
    Path::new(well).join(".ido").join("session.toml")
}

/// A well's saved session (open tabs); empty when absent or unreadable.
#[tauri::command]
pub fn read_session(well: String) -> Session {
    fs::read_to_string(session_path(&well))
        .ok()
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_default()
}

/// Persist a well's open tabs + active index. Best-effort.
#[tauri::command]
pub fn write_session(well: String, tabs: Vec<String>, active: Option<usize>) -> Result<(), String> {
    let session = Session { tabs, active };
    let path = session_path(&well);
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    fs::write(path, toml::to_string(&session).unwrap_or_default()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn session_round_trips_and_defaults_when_absent() {
        let dir = tempdir().unwrap();
        let well = dir.path().to_string_lossy().into_owned();
        // No file yet → empty session.
        let empty = read_session(well.clone());
        assert!(empty.tabs.is_empty());
        assert!(empty.active.is_none());

        write_session(
            well.clone(),
            vec!["welcome".into(), "folder/note".into()],
            Some(1),
        )
        .unwrap();
        let got = read_session(well);
        assert_eq!(got.tabs, vec!["welcome".to_string(), "folder/note".into()]);
        assert_eq!(got.active, Some(1));
    }
}
