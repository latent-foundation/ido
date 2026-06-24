//! Opening things outside the app. For now just URLs in the OS default handler
//! (the browser, mail client, …); a home for "reveal in file manager" and
//! similar OS hand-offs later.

use tauri_plugin_opener::OpenerExt;

/// Open `url` with the operating system's default handler.
#[tauri::command]
pub fn open_external(app: tauri::AppHandle, url: String) -> Result<(), String> {
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| e.to_string())
}
