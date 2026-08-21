//! Image attachments: a shared `assets/` folder at the well root (a sibling of
//! `notes/`/`tasks/`/`wiki/`, outside every section so it never shows up in the
//! notes tree), referenced from any section's markdown by a well-relative path
//! (`![](assets/foo.png)`). The frontend can't reach the filesystem directly, so
//! reading one back for inline rendering also goes through a command, encoded as
//! a `data:` URI (see `read_asset`).

use std::fs;
use std::path::Path;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;

use crate::paths::unique_name;

/// The reserved folder (a well-root sibling of the sectioned folders) that
/// pasted/dropped images are saved into.
pub const ASSETS_DIR: &str = "assets";

/// Guess a `data:` URI mime type from a file extension — the image formats a
/// paste/drop actually produces. Falls back to a generic binary type for
/// anything else (the `<img>` just won't render, rather than erroring).
fn mime_for(ext: &str) -> &'static str {
    match ext.to_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "bmp" => "image/bmp",
        _ => "application/octet-stream",
    }
}

/// Save a pasted/dropped image's bytes into the well's `assets/` folder under a
/// uniquely-named file (the original name's stem + extension, defaulting to
/// `png` when the source gave none). Returns the well-relative id
/// (`assets/<file>`) to embed as `![](…)`.
#[tauri::command]
pub fn save_asset(well: String, name: String, bytes: Vec<u8>) -> Result<String, String> {
    let root = Path::new(&well).join(ASSETS_DIR);
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let p = Path::new(&name);
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("image");
    let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("png");
    let stem = unique_name(&root, stem, ext);
    let filename = format!("{stem}.{ext}");
    fs::write(root.join(&filename), bytes).map_err(|e| e.to_string())?;
    Ok(format!("{ASSETS_DIR}/{filename}"))
}

/// Read an asset (by well-relative id, e.g. `assets/foo.png`) back as a `data:`
/// URI, for inline rendering — the reading/live views resolve an image's
/// markdown path through this and cache the result (see the frontend's
/// `State::resolve_asset`).
#[tauri::command]
pub fn read_asset(well: String, id: String) -> Result<String, String> {
    let path = Path::new(&well).join(&id);
    let bytes = fs::read(&path).map_err(|e| e.to_string())?;
    let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
    Ok(format!(
        "data:{};base64,{}",
        mime_for(ext),
        STANDARD.encode(bytes)
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn save_and_read_round_trip() {
        let dir = tempdir().unwrap();
        let well = dir.path().to_string_lossy().into_owned();

        let id = save_asset(well.clone(), "screenshot.png".into(), vec![1, 2, 3, 4]).unwrap();
        assert_eq!(id, "assets/screenshot.png");
        assert!(dir.path().join("assets/screenshot.png").exists());

        let data_url = read_asset(well.clone(), id).unwrap();
        assert!(data_url.starts_with("data:image/png;base64,"));

        // A second save with the same name doesn't collide.
        let id2 = save_asset(well, "screenshot.png".into(), vec![5, 6]).unwrap();
        assert_eq!(id2, "assets/screenshot-2.png");
    }

    #[test]
    fn save_asset_defaults_extension_when_name_has_none() {
        let dir = tempdir().unwrap();
        let well = dir.path().to_string_lossy().into_owned();
        let id = save_asset(well, "pasted".into(), vec![0]).unwrap();
        assert_eq!(id, "assets/pasted.png");
    }
}
