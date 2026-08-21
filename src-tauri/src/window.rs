//! Window control. The chrome is custom (`decorations: false` in
//! `tauri.conf.json`), so the frontend drives size, resizability, reveal, and
//! the minimize / zoom / close buttons through these commands. The editor
//! window's size, position, and maximized state persist to
//! `app_data/window.json`.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::Manager;

/// Smallest the resizable editor window may be dragged — enough to keep the
/// sidebar and a usable editor visible, so it can't collapse into a pill.
const EDITOR_MIN: (f64, f64) = (640.0, 480.0);

/// First-run editor size, as a fraction of the monitor work area's **height**,
/// with the width derived from [`DEFAULT_ASPECT`]. Sizing off the display rather
/// than fixing pixels keeps the window a sensible share of a 13" laptop and of a
/// 32" desktop panel alike — one fixed size is cramped on the former and lost on
/// the latter. Overridden once the user resizes (their size is remembered).
const DEFAULT_HEIGHT_FRACTION: f64 = 0.86;

/// Width:height for the first-run window.
const DEFAULT_ASPECT: f64 = 1.6;

/// Cap on the first-run width, as a fraction of the work area — so the derived
/// width can't span an ultrawide end to end.
const DEFAULT_MAX_WIDTH_FRACTION: f64 = 0.9;

/// First-run size used only when the monitor can't be read.
const FALLBACK_EDITOR: (f64, f64) = (1100.0, 720.0);

/// Resize and (un)lock the window, then re-centre it. When locking it fixed, any
/// zoomed state is dropped first so the size actually takes effect. A minimum
/// size is enforced: [`EDITOR_MIN`] when resizable, otherwise the fixed size
/// itself.
#[tauri::command]
pub fn apply_window(
    window: tauri::WebviewWindow,
    width: f64,
    height: f64,
    resizable: bool,
) -> Result<(), String> {
    if !resizable {
        let _ = unzoom(&window);
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

/// The window's current monitor **work area** in logical px — the screen minus
/// the menu bar, taskbar, and dock.
fn work_area(window: &tauri::WebviewWindow) -> Option<(f64, f64)> {
    let mon = window.current_monitor().ok().flatten()?;
    let sf = mon.scale_factor();
    let size = mon.work_area().size;
    Some((size.width as f64 / sf, size.height as f64 / sf))
}

/// The first-run editor size for this window's monitor — see
/// [`DEFAULT_HEIGHT_FRACTION`].
fn default_editor(window: &tauri::WebviewWindow) -> (f64, f64) {
    let Some((area_w, area_h)) = work_area(window) else {
        return FALLBACK_EDITOR;
    };
    let h = (area_h * DEFAULT_HEIGHT_FRACTION).max(EDITOR_MIN.1);
    let w = (h * DEFAULT_ASPECT)
        .min(area_w * DEFAULT_MAX_WIDTH_FRACTION)
        .max(EDITOR_MIN.0);
    (w, h)
}

/// `(w, h)` (logical) clamped to the window's monitor work area, so a size saved
/// on a big display can't open off-screen on a small one.
fn clamp_to_monitor(window: &tauri::WebviewWindow, w: f64, h: f64) -> (f64, f64) {
    match work_area(window) {
        Some((area_w, area_h)) => (w.min(area_w), h.min(area_h)),
        None => (w, h),
    }
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

// --- zoom -----------------------------------------------------------------
//
// The third window control means different things per platform, so it routes to
// different APIs.
//
// On **macOS** it is native fullscreen. The platform's green button fullscreens
// rather than maximizes, and only real fullscreen gives the window its own Space
// — which is what makes Ctrl+←/→ swipe between it and the desktop. Merely
// resizing to the work area, which is all `maximize()` can do here, leaves the
// window a big rectangle on the current Space. `decorations: false` is no
// obstacle: tao temporarily swaps in a `Titled | Resizable` mask around
// `toggleFullScreen:` (AppKit ignores the call without it) and restores the
// borderless mask on exit.
//
// On **Windows and Linux** it is a plain maximize, which also drives snap and
// the taskbar's window state.

/// Toggle the window's zoomed state — fullscreen on macOS, maximized elsewhere.
fn toggle_zoom(window: &tauri::WebviewWindow) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let on = window.is_fullscreen().map_err(|e| e.to_string())?;
        window.set_fullscreen(!on).map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        if window.is_maximized().unwrap_or(false) {
            window.unmaximize()
        } else {
            window.maximize()
        }
        .map_err(|e| e.to_string())
    }
}

/// Drop any zoomed state, so an explicit [`apply_window`] size takes effect.
fn unzoom(window: &tauri::WebviewWindow) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        if window.is_fullscreen().unwrap_or(false) {
            return window.set_fullscreen(false).map_err(|e| e.to_string());
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        if window.is_maximized().unwrap_or(false) {
            return window.unmaximize().map_err(|e| e.to_string());
        }
    }
    Ok(())
}

/// Whether the window's current frame is a transient one that must never be
/// recorded as its normal geometry — fullscreen on macOS, maximized elsewhere.
fn zoomed(window: &tauri::WebviewWindow) -> bool {
    #[cfg(target_os = "macos")]
    {
        window.is_fullscreen().unwrap_or(false)
    }
    #[cfg(not(target_os = "macos"))]
    {
        window.is_maximized().unwrap_or(false)
    }
}

/// Apply a normal (un-zoomed) geometry: size clamped to the current monitor,
/// position re-applied only if it's still on a connected one (else centred).
fn apply_geometry(window: &tauri::WebviewWindow, saved: Option<WinState>) -> Result<(), String> {
    let (width, height) = saved
        .map(|s| (s.width, s.height))
        .unwrap_or_else(|| default_editor(window));
    let (width, height) = clamp_to_monitor(window, width, height);
    window
        .set_size(tauri::LogicalSize::new(width, height))
        .map_err(|e| e.to_string())?;
    let placed = saved.and_then(|s| s.x.zip(s.y)).is_some_and(|(x, y)| {
        position_on_monitor(window, x, y) && {
            let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
            true
        }
    });
    if !placed {
        let _ = window.center();
    }
    Ok(())
}

/// Persist the editor window's geometry (logical size, physical position,
/// maximized). Called on resize (debounced) and on close ([`crate::run`]'s
/// `CloseRequested` handler). Two subtleties:
///
/// - **While zoomed**, the window's frame is not its normal geometry. On
///   Windows/Linux the *restore* size is kept and only the maximized flag flips,
///   so relaunching maximized doesn't lose the un-maximized size. macOS
///   fullscreen is deliberately not persisted at all — reopening into a Space
///   the user has since left is disorienting — so nothing is written.
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
    if zoomed(&window) {
        #[cfg(not(target_os = "macos"))]
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

/// Restore the editor window to the remembered geometry (or, on first run, a
/// size proportional to the display — see [`default_editor`]): clamped to the
/// current monitor, position re-applied if it's still on a connected monitor
/// (else centred), and maximized if it was. macOS never reopens into fullscreen
/// (see [`save_geometry`]).
#[tauri::command]
pub fn restore_window(app: tauri::AppHandle, window: tauri::WebviewWindow) -> Result<(), String> {
    let saved = read_win(&app);
    let _ = window.set_min_size(Some(tauri::LogicalSize::new(EDITOR_MIN.0, EDITOR_MIN.1)));
    window.set_resizable(true).map_err(|e| e.to_string())?;
    apply_geometry(&window, saved)?;
    #[cfg(not(target_os = "macos"))]
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

/// Toggle the third window control — fullscreen on macOS, maximize elsewhere.
/// See the zoom notes above for why the platforms differ.
#[tauri::command]
pub fn win_toggle_maximize(window: tauri::WebviewWindow) -> Result<(), String> {
    toggle_zoom(&window)
}

/// Close the window (quits the app).
#[tauri::command]
pub fn win_close(window: tauri::WebviewWindow) -> Result<(), String> {
    window.close().map_err(|e| e.to_string())
}
