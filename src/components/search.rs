//! The cross-section search palette: a command-palette overlay (Ctrl+K, or the
//! rail's search button) that searches notes, wiki, and tasks at once. ↑/↓ move
//! the selection, Enter opens it, Esc (or a backdrop click) dismisses.

use leptos::prelude::*;

use crate::icon::Icon;
use crate::model::SearchHit;
use crate::state::State;

/// The Lucide icon for a hit's section.
fn kind_icon(kind: &str) -> &'static str {
    match kind {
        "wiki" => "book",
        "task" => "square-kanban",
        _ => "file-text",
    }
}

/// The icon for a hit — a command's own icon, else its section icon.
fn hit_icon(kind: &str, id: &str) -> &'static str {
    if kind == "cmd" {
        crate::state::COMMANDS
            .iter()
            .find(|(cid, _, _)| *cid == id)
            .map(|(_, _, icon)| *icon)
            .unwrap_or("circle-help")
    } else {
        kind_icon(kind)
    }
}

/// Render `text` with case-insensitive occurrences of `q` (already lowercase)
/// wrapped in a highlight `<mark>`. Falls back to plain text when lowercasing
/// shifts byte offsets (rare) so a slice never lands mid-character.
fn highlight(text: &str, q: &str) -> AnyView {
    let lower = text.to_lowercase();
    if q.is_empty() || lower.len() != text.len() || !lower.contains(q) {
        return text.to_string().into_any();
    }
    let mut parts: Vec<AnyView> = Vec::new();
    let mut i = 0;
    while let Some(rel) = lower[i..].find(q) {
        let start = i + rel;
        let end = start + q.len();
        let (Some(pre), Some(mid)) = (text.get(i..start), text.get(start..end)) else {
            break;
        };
        if !pre.is_empty() {
            parts.push(pre.to_string().into_any());
        }
        parts.push(view! { <mark class="ido-search-hl">{mid.to_string()}</mark> }.into_any());
        i = end;
    }
    if let Some(rest) = text.get(i..) {
        if !rest.is_empty() {
            parts.push(rest.to_string().into_any());
        }
    }
    parts.into_iter().collect_view().into_any()
}

/// The search overlay, rendered while `search_open` is set. Mounted once in the
/// workspace; the input is focused whenever the palette opens.
#[component]
pub fn SearchPalette() -> impl IntoView {
    let state = expect_context::<State>();
    let input_ref: NodeRef<leptos::html::Input> = NodeRef::new();

    // Focus the input when the palette opens (the ref turns `Some` on mount).
    Effect::new(move |_| {
        if state.search_open.get() {
            if let Some(el) = input_ref.get() {
                let _ = el.focus();
            }
        }
    });

    view! {
        {move || {
            if !state.search_open.get() {
                return ().into_any();
            }
            view! {
                <div class="ido-search-backdrop" on:click=move |_| state.close_search()>
                    <div class="ido-search-panel" on:click=move |ev| ev.stop_propagation()>
                        <div class="ido-search-bar">
                            <Icon name="search" size=16 />
                            <input
                                class="ido-search-input"
                                node_ref=input_ref
                                placeholder="search notes, wiki, tasks…"
                                spellcheck="false"
                                autocomplete="off"
                                on:input=move |ev| state.run_search(event_target_value(&ev))
                                on:keydown=move |ev| {
                                    match ev.key().as_str() {
                                        "Escape" => {
                                            ev.stop_propagation();
                                            state.close_search();
                                        }
                                        "ArrowDown" => {
                                            ev.prevent_default();
                                            state.search_move(1);
                                        }
                                        "ArrowUp" => {
                                            ev.prevent_default();
                                            state.search_move(-1);
                                        }
                                        "Enter" => {
                                            ev.prevent_default();
                                            state.open_selected_search();
                                        }
                                        _ => {}
                                    }
                                }
                            />
                        </div>
                        <div class="ido-search-results">
                            {move || {
                                let results = state.search_results.get();
                                if results.is_empty() {
                                    let msg = if state.search_query.get().trim().is_empty() {
                                        "type to search across sections"
                                    } else {
                                        "no matches"
                                    };
                                    return view! { <div class="ido-search-empty">{msg}</div> }
                                        .into_any();
                                }
                                let q = state.search_query.get().trim().to_lowercase();
                                results
                                    .into_iter()
                                    .enumerate()
                                    .map(|(i, hit)| {
                                        view! { <ResultRow idx=i hit=hit query=q.clone() /> }
                                    })
                                    .collect_view()
                                    .into_any()
                            }}
                        </div>
                    </div>
                </div>
            }
                .into_any()
        }}
    }
}

/// One result row — its section icon, title + kind tag, and a context snippet.
/// Hover or arrow-keys highlight it; click opens it.
#[component]
fn ResultRow(idx: usize, hit: SearchHit, query: String) -> impl IntoView {
    let state = expect_context::<State>();
    let SearchHit {
        kind,
        id,
        title,
        snippet,
    } = hit;
    let icon = hit_icon(&kind, &id);
    let kind_label = if kind == "cmd" {
        "command".to_string()
    } else {
        kind.clone()
    };
    let has_snippet = !snippet.is_empty();
    view! {
        <button
            class="ido-search-result"
            class:sel=move || state.search_sel.get() == idx
            on:mouseenter=move |_| state.search_sel.set(idx)
            on:click=move |_| state.open_search_hit(kind.clone(), id.clone())
        >
            <Icon name=icon size=15 />
            <div class="ido-search-text">
                <div class="ido-search-line">
                    <span class="ido-search-name">{highlight(&title, &query)}</span>
                    <span class="ido-search-kind">{kind_label}</span>
                </div>
                {has_snippet
                    .then(|| {
                        view! {
                            <div class="ido-search-snippet">{highlight(&snippet, &query)}</div>
                        }
                    })}
            </div>
        </button>
    }
}
