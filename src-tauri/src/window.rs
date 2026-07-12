//! Window control. The chrome is custom (`decorations: false` in
//! `tauri.conf.json`), so the frontend drives size, resizability, reveal, and
//! the minimize / maximize / close buttons through these commands. The editor
//! window's size, position, and maximized state persist to
//! `app_data/window.json`.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::Manager;

/// Smallest the resizable editor window may be dragged — enough to keep the
/// sidebar and a usable editor visible, so it can't collapse into a pill.
const EDITOR_MIN: (f64, f64) = (640.0, 480.0);

/// First-run editor size — roomy enough for the full task board (rail + four
/// columns). Clamped to the monitor on small screens; overridden once the user
/// resizes (their size is remembered).
const DEFAULT_EDITOR: (f64, f64) = (1280.0, 840.0);

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

/// The remembered editor-window geometry: logical size, physical position
/// (absolute across monitors), and maximized state. `x`/`y` default so an older
/// `window.json` without them still loads.
#[derive(Serialize, Deserialize, Clone, Copy)]
struct WinState {
    width: f64,
    height: f64,
    maximized: bool,
    #[serde(default)]
    x: Option<i32>,
    #[serde(default)]
    y: Option<i32>,
}

/// Path to `window.json` in the app data dir.
fn win_path(app: &tauri::AppHandle) -> Option<PathBuf> {
    let dir = app.path().app_data_dir().ok()?;
    let _ = fs::create_dir_all(&dir);
    Some(dir.join("window.json"))
}

fn read_win(app: &tauri::AppHandle) -> Option<WinState> {
    let s = fs::read_to_string(win_path(app)?).ok()?;
    serde_json::from_str(&s).ok()
}

fn write_win(app: &tauri::AppHandle, st: WinState) {
    if let Some(p) = win_path(app) {
        let _ = fs::write(p, serde_json::to_string_pretty(&st).unwrap_or_default());
    }
}

/// `(w, h)` clamped to the window's current monitor, leaving room for the
/// taskbar. Same margins the restore uses, so [`save_geometry`] can recognise a
/// size that merely echoes a clamp.
fn clamp_to_monitor(window: &tauri::WebviewWindow, w: f64, h: f64) -> (f64, f64) {
    if let Ok(Some(mon)) = window.current_monitor() {
        let sf = mon.scale_factor();
        let size = mon.size();
        return (
            w.min(size.width as f64 / sf - 20.0),
            h.min(size.height as f64 / sf - 60.0),
        );
    }
    (w, h)
}

/// Whether the physical point `(x, y)` falls inside any connected monitor — used
/// to reject a saved position on a monitor that's since been unplugged (which
/// would open the window off-screen).
fn position_on_monitor(window: &tauri::WebviewWindow, x: i32, y: i32) -> bool {
    window
        .available_monitors()
        .map(|monitors| {
            monitors.iter().any(|m| {
                let p = m.position();
                let s = m.size();
                x >= p.x && x < p.x + s.width as i32 && y >= p.y && y < p.y + s.height as i32
            })
        })
        .unwrap_or(false)
}

/// Persist the editor window's geometry (logical size, physical position,
/// maximized). Called on resize (debounced) and on close ([`crate::run`]'s
/// `CloseRequested` handler). Two subtleties:
///
/// - **While maximized**, the *restore* size is kept and only the flag flips, so
///   relaunching maximized doesn't lose the un-maximized size.
/// - **A clamped restore isn't written back.** When a big saved size is opened on
///   a smaller monitor, [`restore_window`] shrinks it to fit; the resize that
///   fires would otherwise overwrite the (larger) saved size with the clamp. If
///   the current size just echoes the clamp of the saved size, only the position
///   is updated — so moving back to the big monitor restores the big size.
pub(crate) fn save_geometry(app: &tauri::AppHandle) {
    let Some(window) = app.webview_windows().into_values().next() else {
        return;
    };
    let saved = read_win(app);
    if window.is_maximized().unwrap_or(false) {
        if let Some(mut s) = saved {
            s.maximized = true;
            write_win(app, s);
        }
        return;
    }
    let sf = window.scale_factor().unwrap_or(1.0);
    let Some((width, height)) = window
        .inner_size()
        .ok()
        .map(|p| (p.width as f64 / sf, p.height as f64 / sf))
    else {
        return;
    };
    let (x, y) = match window.outer_position() {
        Ok(p) => (Some(p.x), Some(p.y)),
        Err(_) => (None, None),
    };
    // Don't overwrite a larger saved size with its own monitor-clamp.
    if let Some(s) = saved.filter(|s| !s.maximized) {
        let (cw, ch) = clamp_to_monitor(&window, s.width, s.height);
        let was_clamped = s.width > cw + 1.0 || s.height > ch + 1.0;
        if was_clamped && (width - cw).abs() < 2.0 && (height - ch).abs() < 2.0 {
            write_win(app, WinState { x, y, ..s });
            return;
        }
    }
    write_win(
        app,
        WinState {
            width,
            height,
            maximized: false,
            x,
            y,
        },
    );
}

/// Remember the editor window's current geometry. Thin command wrapper over
/// [`save_geometry`] (the frontend's debounced resize listener calls it).
#[tauri::command]
pub fn remember_window(app: tauri::AppHandle) {
    save_geometry(&app);
}

/// Restore the editor window to the remembered geometry (or [`DEFAULT_EDITOR`]):
/// size clamped to the current monitor, position re-applied if it's still on a
/// connected monitor (else centred), and maximized if it was.
#[tauri::command]
pub fn restore_window(app: tauri::AppHandle, window: tauri::WebviewWindow) -> Result<(), String> {
    let saved = read_win(&app);
    let (width, height) = saved.map(|s| (s.width, s.height)).unwrap_or(DEFAULT_EDITOR);
    let (width, height) = clamp_to_monitor(&window, width, height);
    let _ = window.set_min_size(Some(tauri::LogicalSize::new(EDITOR_MIN.0, EDITOR_MIN.1)));
    window.set_resizable(true).map_err(|e| e.to_string())?;
    window
        .set_size(tauri::LogicalSize::new(width, height))
        .map_err(|e| e.to_string())?;
    // Reopen where it was left, unless that monitor is gone — then centre. Set
    // position before maximizing so it maximizes on the intended monitor.
    let placed = saved.and_then(|s| s.x.zip(s.y)).is_some_and(|(x, y)| {
        position_on_monitor(&window, x, y) && {
            let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
            true
        }
    });
    if !placed {
        let _ = window.center();
    }
    if saved.map(|s| s.maximized).unwrap_or(false) {
        let _ = window.maximize();
    }
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
