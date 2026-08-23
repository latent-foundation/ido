//! The settings modal — theme, the open well's path, the task board's
//! columns, semantic search, MCP agent access, and the running version; a home
//! for preferences as they appear.

use std::time::Duration;

use latent_ui::Icon;
use latent_ui::ThemeToggle;
use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::ipc;
use crate::model::McpInfo;
use crate::state::State;

/// The running app's version, e.g. `1.2.0`.
///
/// A compile-time constant rather than a command round-trip, which is only
/// honest because every crate in the workspace inherits one
/// `[workspace.package] version` — so cargo guarantees this equals the version
/// Tauri stamps on the installer. If that inheritance is ever undone, this
/// starts quietly lying and a command is the right fix.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

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

/// Unix-epoch milliseconds as a local `YYYY-MM-DD` — the same shape the
/// reading view's metadata row uses, so dates read alike across the app.
fn fmt_date(millis: u64) -> String {
    let d = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(millis as f64));
    format!(
        "{:04}-{:02}-{:02}",
        d.get_full_year(),
        d.get_month() + 1,
        d.get_date(),
    )
}

/// Bytes as whole megabytes. The model is ~134 MB and the exact byte count is
/// noise at that size — a user deciding whether to download wants the order of
/// magnitude, not the digits.
fn megabytes(bytes: u64) -> String {
    format!("{} mb", bytes.div_ceil(1_000_000))
}

/// The running job's completion as a percentage, or `None` while the total is
/// still unknown — a download reports `0` for its total until the server sends
/// a length, and dividing by that would render `NaN%` in the bar.
fn percent(done: u64, total: u64) -> Option<f64> {
    (total > 0).then(|| (done as f64 / total as f64 * 100.0).clamp(0.0, 100.0))
}

/// The "semantic search" section: whether meaning-based search can run here,
/// the one-time model download, and this well's index.
///
/// Both long operations run on a backend thread and are watched by polling
/// (`State::watch_job`), so this component only ever renders a snapshot. It
/// re-reads [`ipc::semantic_info`] whenever the modal opens *and* whenever a
/// job finishes — the latter inside `State::poll_job`, so the panel updates
/// once at the end rather than on every tick.
#[component]
fn SemanticSection() -> impl IntoView {
    let state = expect_context::<State>();
    Effect::new(move |_| {
        if state.settings_open.get() {
            state.refresh_semantic();
            // A download or index build started earlier may still be running;
            // pick it up rather than showing a stale idle panel.
            state.watch_job();
        }
    });
    view! {
        <span class="ido-settings-label">"semantic search"</span>
        <span class="ido-settings-hint">
            "finds entries by meaning, not only by the words they contain — \"where do i keep credentials\" turns up the note about keychains. keyword search keeps working regardless."
        </span>
        {move || {
            state
                .semantic
                .get()
                .map(|info| {
                    if !info.supported {
                        return view! {
                            <span class="ido-settings-mcp-missing">
                                "this build of ido was compiled without semantic search."
                            </span>
                        }
                            .into_any();
                    }
                    let job = state.job.get();
                    let busy = job.running;
                    let has_model = info.model_present;
                    let index = info.index.clone();
                    let model = info.model.clone();
                    let size = megabytes(info.download_bytes);
                    let status = match (&has_model, &index) {
                        (false, _) => {
                            view! {
                                <span class="ido-settings-hint">
                                    "needs a one-time model download — " <code>{model}</code> ", "
                                    {size}
                                    ". it is shared by every well on this machine, and it is the only time ido uses the network."
                                </span>
                            }
                                .into_any()
                        }
                        (true, None) => {
                            view! {
                                <span class="ido-settings-hint">
                                    "model ready. this well has not been indexed yet — indexing reads every note once and takes a while on a large well."
                                </span>
                            }
                                .into_any()
                        }
                        (true, Some(ix)) => {
                            let line = format!(
                                "{} chunks across {} files · built {}",
                                ix.chunks,
                                ix.files,
                                fmt_date(ix.built_ms),
                            );
                            let stale = ix.stale;
                            view! {
                                <span class="ido-settings-semantic-status">
                                    {line}
                                    {stale
                                        .then(|| {
                                            view! {
                                                <span class="ido-settings-stale">"well changed since"</span>
                                            }
                                        })}
                                </span>
                            }
                                .into_any()
                        }
                    };
                    let action = if !has_model {
                        view! {
                            <button
                                class="ido-settings-action"
                                disabled=busy
                                on:click=move |_| state.start_model_download()
                            >
                                "download model"
                            </button>
                        }
                            .into_any()
                    } else {
                        let label = if index.is_some() { "rebuild index" } else { "build index" };
                        view! {
                            <button
                                class="ido-settings-action"
                                disabled=busy
                                on:click=move |_| state.start_index_build()
                            >
                                {label}
                            </button>
                        }
                            .into_any()
                    };
                    view! {
                        <div class="ido-settings-semantic">
                            {status} {action}
                            {busy
                                .then(|| {
                                    let unit = if job.kind == "download" { "mb" } else { "chunks" };
                                    let scale = |n: u64| {
                                        if job.kind == "download" {
                                            (n / 1_000_000).to_string()
                                        } else {
                                            n.to_string()
                                        }
                                    };
                                    let counts = if job.total > 0 {
                                        format!("{} / {} {unit}", scale(job.done), scale(job.total))
                                    } else {
                                        String::new()
                                    };
                                    let pct = percent(job.done, job.total);
                                    // Bytes while downloading, chunks while indexing —
                                    // the same two numbers mean different things.
                                    view! {
                                        <div class="ido-settings-progress">
                                            <div class="ido-settings-progress-track">
                                                <div
                                                    class="ido-settings-progress-fill"
                                                    class:ido-settings-progress-indeterminate=pct.is_none()
                                                    style=move || {
                                                        pct.map(|p| format!("width: {p:.1}%")).unwrap_or_default()
                                                    }
                                                />
                                            </div>
                                            <span class="ido-settings-hint">
                                                {job.phase.clone()} " " {counts}
                                            </span>
                                        </div>
                                    }
                                })}
                            {job
                                .error
                                .clone()
                                .map(|e| {
                                    view! { <span class="ido-settings-error">{e}</span> }
                                })}
                        </div>
                    }
                        .into_any()
                })
        }}
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
    // Which snippet variant to show. Deliberately **not** persisted and
    // deliberately re-defaulting to off every time the modal opens: this is a
    // snippet generator, not a permission the app holds, so the safe posture
    // is the one you get without deciding anything.
    let allow_write = RwSignal::new(false);
    Effect::new(move |_| {
        if state.settings_open.get()
            && let Some(well) = state.well.get()
        {
            let write = allow_write.get();
            spawn_local(async move {
                info.set(ipc::mcp_info(well.path, write).await);
            });
        }
    });
    // Re-arm the safe default whenever the modal closes, so a snippet left on
    // "writes" doesn't greet the next person who opens settings.
    Effect::new(move |_| {
        if !state.settings_open.get() {
            allow_write.set(false);
        }
    });
    // No wrapper div: the mount point already provides the
    // `.ido-settings-row.ido-settings-row-stack` column — `-row` carries the
    // `display: flex` that `-row-stack` builds on, so an inner `-row-stack`
    // alone would collapse to a block and run the label into the hint.
    view! {
        <span class="ido-settings-label">"agent access — mcp"</span>
        <span class="ido-settings-hint">
            "any mcp client can read this well — notes, wiki, tasks — even while ido is closed. by default the server is read-only."
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
                            <label class="ido-settings-check">
                                <input
                                    type="checkbox"
                                    prop:checked=move || allow_write.get()
                                    on:change=move |ev| {
                                        allow_write.set(event_target_checked(&ev));
                                    }
                                />
                                <span>"let agents add to this well"</span>
                            </label>
                            <span class="ido-settings-hint">
                                "adds " <code>"--allow-write"</code>
                                " to the snippets below, so a client can create notes, pages and tasks and append to them. it can never delete anything, or overwrite or shorten an existing body. changing this only rewrites the text below — copy it into your client to apply."
                            </span>
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
                                                    <SemanticSection />
                                                </div>
                                                <div class="ido-settings-row ido-settings-row-stack">
                                                    <McpSection />
                                                </div>
                                            }
                                        })
                                }}
                                // Outside the `well` gate: the version is worth
                                // reading (and quoting in a bug report) whether
                                // or not a well happens to be open.
                                <div class="ido-settings-row ido-settings-version">
                                    <span class="ido-settings-label">"version"</span>
                                    <span class="ido-settings-value">
                                        {format!("ido {VERSION}")}
                                    </span>
                                </div>
                            </div>
                        </div>
                    }
                })
        }}
    }
}
