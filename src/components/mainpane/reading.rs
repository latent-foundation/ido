//! Reading-view chrome: the note metadata row (folder + creation date) and the
//! cross-section backlinks panel.

use leptos::prelude::*;
use wasm_bindgen::JsValue;

use crate::icon::Icon;
use crate::state::{Pane, State};

// ═══════════════════════════════════════════════════ Reading metadata ═════════

/// Format Unix-epoch milliseconds as a local `YYYY-MM-DD` date.
fn fmt_date(millis: u64) -> String {
    let d = js_sys::Date::new(&JsValue::from_f64(millis as f64));
    format!(
        "{:04}-{:02}-{:02}",
        d.get_full_year(),
        d.get_month() + 1,
        d.get_date(),
    )
}

/// Reading-view header: a note's immediate folder and creation date, in muted
/// mono. Renders nothing for wiki pages (flat, no folder; no timestamps loaded)
/// or until something is open.
#[component]
pub(crate) fn NoteMetaRow(pane: Pane) -> impl IntoView {
    let active_id = move || {
        let i = pane.active_tab.get()?;
        pane.tabs
            .get()
            .get(i)
            .map(|t| t.target.entry_id().to_string())
    };
    view! {
        {move || {
            let id = active_id()?;
            let parent = crate::state::parent_of(&id);
            let folder = parent.rsplit('/').next().unwrap_or(&parent).to_string();
            let created = pane.meta.get().and_then(|m| m.created).map(fmt_date);
            (!folder.is_empty() || created.is_some())
                .then(|| {
                    view! {
                        <div class="ido-note-meta">
                            {(!folder.is_empty())
                                .then(|| {
                                    view! {
                                        <span class="ido-note-meta-item">
                                            <Icon name="folder" size=12 />
                                            {folder}
                                        </span>
                                    }
                                })}
                            {created
                                .map(|d| {
                                    view! {
                                        <span class="ido-note-meta-item">
                                            {format!("created {d}")}
                                        </span>
                                    }
                                })}
                        </div>
                    }
                })
        }}
    }
}

// ═══════════════════════════════════════════════════════ Backlinks ════════════

/// Reading-view footer for a wiki page: the notes, wiki pages, tasks, and goals
/// that link to it (the cross-section backlink graph). Renders nothing when
/// there are none. Clicking a backlink opens it (a tab, or a task/goal drawer).
#[component]
pub(crate) fn BacklinksPanel(pane: Pane) -> impl IntoView {
    let state = expect_context::<State>();
    view! {
        {move || {
            let links = pane.backlinks.get();
            (!links.is_empty())
                .then(|| {
                    view! {
                        <div class="ido-backlinks">
                            <div class="ido-backlinks-head">"linked from"</div>
                            {links
                                .into_iter()
                                .map(|link| {
                                    let icon = match link.kind.as_str() {
                                        "wiki" => "book",
                                        "task" => "square-kanban",
                                        "goal" => "target",
                                        _ => "file-text",
                                    };
                                    let kind = link.kind.clone();
                                    let id = link.id.clone();
                                    view! {
                                        <button
                                            class="ido-backlink"
                                            on:click=move |_| {
                                                state.open_entry(kind.clone(), id.clone())
                                            }
                                        >
                                            <Icon name=icon size=13 />
                                            {link.title}
                                        </button>
                                    }
                                })
                                .collect_view()}
                        </div>
                    }
                })
        }}
    }
}
