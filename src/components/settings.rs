//! The settings modal — theme, the open well's path, the task board's
//! columns, and MCP agent access; a home for preferences as they appear.

use std::time::Duration;

use latent_ui::Icon;
use latent_ui::ThemeToggle;
use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::ipc;
use crate::model::McpInfo;
use crate::state::State;

/// Copy `text` to the OS clipboard — the write-side counterpart of the paste
/// path in `contextmenu.rs` (`navigator.clipboard`, awaited via `JsFuture`).
fn copy_to_clipboard(text: String) {
    let Some(win) = web_sys::window() else {
        return;
    };
    let clip = win.navigator().clipboard();
    spawn_local(async move {
        let _ = wasm_bindgen_futures::JsFuture::from(clip.write_text(&text)).await;
    });
}

/// A small button that copies `text` to the clipboard, briefly confirming.
#[component]
fn CopyButton(text: String) -> impl IntoView {
    let copied = RwSignal::new(false);
    view! {
        <button
            class="ido-settings-copy-btn"
            on:click=move |_| {
                copy_to_clipboard(text.clone());
                copied.set(true);
                set_timeout(move || copied.set(false), Duration::from_secs(2));
            }
        >
            <Icon name="copy" size=13 />
            <span>{move || if copied.get() { "copied" } else { "copy" }}</span>
        </button>
    }
}

/// The "agent access — mcp" section: how to point an MCP client (Claude Code
/// first) at the current well. `mcp_info` is fetched fresh whenever the
/// settings modal opens — cheap (a file-existence check + string formatting),
/// and it keeps the well path current if the well changed since last open.
#[component]
fn McpSection() -> impl IntoView {
    let state = expect_context::<State>();
    let info = RwSignal::new(None::<McpInfo>);
    Effect::new(move |_| {
        if state.settings_open.get()
            && let Some(well) = state.well.get()
        {
            spawn_local(async move {
                info.set(ipc::mcp_info(well.path).await);
            });
        }
    });
    // No wrapper div: the mount point already provides the
    // `.ido-settings-row.ido-settings-row-stack` column — `-row` carries the
    // `display: flex` that `-row-stack` builds on, so an inner `-row-stack`
    // alone would collapse to a block and run the label into the hint.
    view! {
        <span class="ido-settings-label">"agent access — mcp"</span>
        <span class="ido-settings-hint">
            "any mcp client can read this well — notes, wiki, tasks — even while ido is closed. the server is read-only."
        </span>
        {move || {
            info.get()
                .map(|i| {
                    let status = match i.bin.clone() {
                        Some(bin) => {
                            view! { <code class="ido-settings-value">{bin}</code> }.into_any()
                        }
                        None => {
                            view! {
                                <span class="ido-settings-mcp-missing">
                                    "server binary not found — run " <code>"just sidecar"</code>
                                    " (dev) or reinstall (bundle)."
                                </span>
                            }
                                .into_any()
                        }
                    };
                    view! {
                        <div class="ido-settings-mcp">
                            <div class="ido-settings-mcp-status">{status}</div>
                            <div class="ido-settings-snippet">
                                <pre>
                                    <code>{i.json.clone()}</code>
                                </pre>
                                <CopyButton text=i.json />
                            </div>
                            <div class="ido-settings-snippet">
                                <pre>
                                    <code>{i.cli.clone()}</code>
                                </pre>
                                <CopyButton text=i.cli />
                            </div>
                        </div>
                    }
                })
        }}
    }
}

/// A centred modal over the app, shown while `settings_open` is set. Clicking
/// the backdrop (or close) dismisses it; clicking the panel does not.
#[component]
pub fn Settings() -> impl IntoView {
    let state = expect_context::<State>();
    view! {
        {move || {
            state
                .settings_open
                .get()
                .then(|| {
                    view! {
                        <div
                            class="ido-modal-backdrop"
                            on:click=move |_| state.settings_open.set(false)
                        >
                            <div class="ido-modal" on:click=move |ev| ev.stop_propagation()>
                                <div class="ido-modal-head">
                                    <span class="ido-modal-title">"settings"</span>
                                    <button
                                        class="ido-modal-close"
                                        on:click=move |_| state.settings_open.set(false)
                                    >
                                        <Icon name="x" size=15 />
                                    </button>
                                </div>
                                <div class="ido-settings-row">
                                    <span class="ido-settings-label">"appearance"</span>
                                    <ThemeToggle />
                                </div>
                                <div class="ido-settings-row">
                                    <span class="ido-settings-label">"well"</span>
                                    <span class="ido-settings-value">
                                        {move || {
                                            state.well.get().map(|w| w.path).unwrap_or_default()
                                        }}
                                    </span>
                                </div>
                                {move || {
                                    state
                                        .well
                                        .get()
                                        .map(|_| {
                                            view! {
                                                <div class="ido-settings-row ido-settings-row-stack">
                                                    <span class="ido-settings-label">"board columns"</span>
                                                    <input
                                                        class="ido-settings-input"
                                                        prop:value=move || state.columns.get().join(", ")
                                                        spellcheck="false"
                                                        autocomplete="off"
                                                        on:change=move |ev| {
                                                            state.set_columns(event_target_value(&ev))
                                                        }
                                                    />
                                                    <span class="ido-settings-hint">
                                                        "comma-separated, in board order — the last column counts as done. tasks from a removed column drop to the backlog."
                                                    </span>
                                                </div>
                                            }
                                        })
                                }}
                                {move || {
                                    state
                                        .well
                                        .get()
                                        .map(|_| {
                                            view! {
                                                <div class="ido-settings-row ido-settings-row-stack">
                                                    <span class="ido-settings-label">
                                                        "auto-archive done after"
                                                    </span>
                                                    <div class="ido-settings-inline">
                                                        <input
                                                            type="number"
                                                            min="1"
                                                            step="1"
                                                            class="ido-settings-input ido-settings-input-narrow"
                                                            placeholder="off"
                                                            prop:value=move || {
                                                                state
                                                                    .archive_days
                                                                    .get()
                                                                    .map(|d| d.to_string())
                                                                    .unwrap_or_default()
                                                            }
                                                            on:change=move |ev| {
                                                                let raw = event_target_value(&ev);
                                                                let days = raw
                                                                    .trim()
                                                                    .parse::<u32>()
                                                                    .ok()
                                                                    .filter(|d| *d > 0);
                                                                state.set_archive_days(days);
                                                            }
                                                        />
                                                        <span class="ido-settings-suffix">"days"</span>
                                                    </div>
                                                    <span class="ido-settings-hint">
                                                        "done tasks older than this are archived automatically (restorable from the archive view); empty = off."
                                                    </span>
                                                </div>
                                            }
                                        })
                                }}
                                {move || {
                                    state
                                        .well
                                        .get()
                                        .map(|_| {
                                            view! {
                                                <div class="ido-settings-row ido-settings-row-stack">
                                                    <McpSection />
                                                </div>
                                            }
                                        })
                                }}
                            </div>
                        </div>
                    }
                })
        }}
    }
}
