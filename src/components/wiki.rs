//! The wiki section sidebar: a flat list of pages by slug, with create / inline
//! rename / delete. Pages open into the shared [`crate::components::mainpane`];
//! their `[[wikilinks]]` and backlinks are handled there and in `markdown`.

use leptos::prelude::*;

use crate::icon::Icon;
use crate::state::{MenuTarget, State};

/// The wiki sidebar (`<aside>`), shown when the wiki section is active.
#[component]
pub fn WikiSidebar() -> impl IntoView {
    let state = expect_context::<State>();
    view! {
        <aside class="ido-sidebar">
            <div class="ido-section-head">
                <Icon name="book" size=14 />
                <span class="ido-section-label">"wiki"</span>
            </div>

            <div class="ido-toolbar">
                <button class="ido-tool" title="New page" on:click=move |_| state.add_page()>
                    <Icon name="file-plus" size=15 />
                </button>
            </div>

            <nav class="ido-list">
                {move || {
                    let pages = state.wiki.get();
                    if pages.is_empty() {
                        view! { <div class="ido-wiki-empty">"no pages yet"</div> }.into_any()
                    } else {
                        pages
                            .into_iter()
                            .map(|slug| view! { <PageRow slug=slug /> })
                            .collect_view()
                            .into_any()
                    }
                }}
            </nav>
        </aside>
    }
}

/// A single wiki page row: click to open; hover reveals rename + delete; the
/// rename pencil swaps the label for an inline input.
#[component]
fn PageRow(slug: String) -> impl IntoView {
    let state = expect_context::<State>();
    let s_click = slug.clone();
    let s_active = slug.clone();
    let s_match = slug.clone();
    let s_menu = slug.clone();
    let label = slug;

    view! {
        <div
            class="ido-wiki-row"
            class:active=move || state.active_page().as_deref() == Some(s_active.as_str())
            on:click=move |_| state.preview_wiki(s_click.clone())
            on:contextmenu=move |ev| {
                ev.prevent_default();
                ev.stop_propagation();
                state
                    .open_menu(
                        ev.client_x(),
                        ev.client_y(),
                        MenuTarget::Wiki {
                            slug: s_menu.clone(),
                        },
                    );
            }
        >
            {move || {
                if state.page_renaming.get().as_deref() == Some(s_match.as_str()) {
                    let s_key = s_match.clone();
                    let s_blur = s_match.clone();
                    view! {
                        <input
                            class="ido-rename"
                            prop:value=label.clone()
                            autofocus
                            on:click=move |ev| ev.stop_propagation()
                            on:keydown=move |ev| {
                                match ev.key().as_str() {
                                    "Enter" => {
                                        state
                                            .commit_page_rename(s_key.clone(), event_target_value(&ev))
                                    }
                                    "Escape" => state.page_renaming.set(None),
                                    _ => {}
                                }
                            }
                            on:blur=move |ev| {
                                state.commit_page_rename(s_blur.clone(), event_target_value(&ev))
                            }
                        />
                    }
                        .into_any()
                } else {
                    let s_ren = s_match.clone();
                    let s_del = s_match.clone();
                    view! {
                        <span class="ido-wiki-name">{label.clone()}</span>
                        <span class="ido-tree-actions">
                            <button
                                class="ido-tree-action"
                                title="Rename"
                                on:click=move |ev| {
                                    ev.stop_propagation();
                                    state.page_renaming.set(Some(s_ren.clone()));
                                }
                            >
                                <Icon name="pencil" size=13 />
                            </button>
                            <button
                                class="ido-tree-action"
                                title="Delete"
                                on:click=move |ev| {
                                    ev.stop_propagation();
                                    state.remove_page(s_del.clone());
                                }
                            >
                                <Icon name="trash" size=13 />
                            </button>
                        </span>
                    }
                        .into_any()
                }
            }}
        </div>
    }
}
