//! Host-platform detection, for the few places the chrome must match the OS.
//!
//! The frontend compiles to `wasm32`, so `cfg!(target_os = …)` describes the
//! WASM target rather than the machine ido is running on. The webview's user
//! agent is the signal we use instead — and it has to be a *synchronous* one,
//! because the title bar's control layout differs per platform and an async
//! round trip to the backend would paint the wrong chrome first.

use std::sync::OnceLock;

/// Whether ido is running on macOS — WKWebView reports `Macintosh` in its user
/// agent. Probed once and cached; the answer can't change mid-session.
pub fn is_mac() -> bool {
    static MAC: OnceLock<bool> = OnceLock::new();
    *MAC.get_or_init(|| {
        web_sys::window()
            .and_then(|w| w.navigator().user_agent().ok())
            .is_some_and(|ua| ua.contains("Macintosh"))
    })
}
