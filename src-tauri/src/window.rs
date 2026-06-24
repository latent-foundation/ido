//! Window control. The chrome is custom (`decorations: false` in
//! `tauri.conf.json`), so the frontend drives size, resizability, reveal, and
//! the minimize / maximize / close buttons through these commands.

/// Smallest the resizable editor window may be dragged — enough to keep the
/// sidebar and a usable editor visible, so it can't collapse into a pill.
const EDITOR_MIN: (f64, f64) = (640.0, 480.0);

/// Resize and (un)lock the window, then re-centre it. When locking it fixed,
/// any maximized state is dropped first so the size actually takes effect. A
/// minimum size is enforced: [`EDITOR_MIN`] when resizable, otherwise the fixed
/// size itself.
#[tauri::command]
pub fn apply_window(
    window: tauri::WebviewWindow,
    width: f64,
    height: f64,
    resizable: bool,
) -> Result<(), String> {
    if !resizable && window.is_maximized().unwrap_or(false) {
        let _ = window.unmaximize();
    }
    let (min_w, min_h) = if resizable {
        EDITOR_MIN
    } else {
        (width, height)
    };
    let _ = window.set_min_size(Some(tauri::LogicalSize::new(min_w, min_h)));
    window.set_resizable(resizable).map_err(|e| e.to_string())?;
    window
        .set_size(tauri::LogicalSize::new(width, height))
        .map_err(|e| e.to_string())?;
    let _ = window.center();
    Ok(())
}

/// Reveal the window (it starts hidden, see `tauri.conf.json`) and focus it.
#[tauri::command]
pub fn show_window(window: tauri::WebviewWindow) -> Result<(), String> {
    window.show().map_err(|e| e.to_string())?;
    let _ = window.set_focus();
    Ok(())
}

/// Minimize the window.
#[tauri::command]
pub fn win_minimize(window: tauri::WebviewWindow) -> Result<(), String> {
    window.minimize().map_err(|e| e.to_string())
}

/// Toggle between maximized and restored.
#[tauri::command]
pub fn win_toggle_maximize(window: tauri::WebviewWindow) -> Result<(), String> {
    if window.is_maximized().unwrap_or(false) {
        window.unmaximize().map_err(|e| e.to_string())
    } else {
        window.maximize().map_err(|e| e.to_string())
    }
}

/// Close the window (quits the app).
#[tauri::command]
pub fn win_close(window: tauri::WebviewWindow) -> Result<(), String> {
    window.close().map_err(|e| e.to_string())
}
