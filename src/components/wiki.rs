//! The wiki section sidebar: a header, the new-page/folder toolbar, and the
//! recursive wiki tree. Pages open into the shared [`crate::components::mainpane`]
//! by their slug; folders are purely organisational. Mirrors the notes sidebar /
//! [`crate::components::tree`], but page identity is the slug (the tree path's
//! last segment), so a folder move never disturbs an open tab or a `[[link]]`.

use leptos::prelude::*;

use crate::icon::Icon;
use crate::model::TreeNode;
use crate::state::{MenuTarget, State, parent_of};

/// The slug of a page node — the last segment of its wiki-relative path.
fn slug_of(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}

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
                <button
                    class="ido-tool"
                    title="New page"
                    on:click=move |_| state.add_page(state.wiki_target.get_untracked())
                >
                    <Icon name="file-plus" size=15 />
                </button>
                <button
                    class="ido-tool"
                    title="New folder"
                    on:click=move |_| state.add_wiki_folder(state.wiki_target.get_untracked())
                >
                    <Icon name="folder-plus" size=15 />
                </button>
                <button
                    class="ido-tool-target"
                    title="New pages go here — click for the wiki root"
                    on:click=move |_| state.wiki_target.set(String::new())
                >
                    {move || {
                        let t = state.wiki_target.get();
                        if t.is_empty() {
                            "root".to_string()
                        } else {
                            t.rsplit('/').next().unwrap_or(&t).to_string()
                        }
                    }}
                </button>
            </div>

            <nav
                class="ido-list"
                on:dragover=move |ev| {
                    ev.prevent_default();
                    state.wiki_drag_over.set(None);
                }
                on:drop=move |ev| {
                    ev.prevent_default();
                    state.wiki_drag_over.set(None);
                    state.move_wiki_into(String::new());
                }
            >
                {move || {
                    let nodes = state.wiki_tree.get();
                    if nodes.is_empty() {
                        view! { <div class="ido-wiki-empty">"no pages yet"</div> }.into_any()
                    } else {
                        view! { <WikiTree nodes=nodes depth=0 /> }.into_any()
                    }
                }}
            </nav>
        </aside>
    }
}

/// Render a level of the wiki tree, recursing into expanded folders.
///
/// Returns `AnyView` so the recursion through `<WikiTree/>` doesn't form an
/// unnameable, infinitely-recursive `impl Trait` type (E0720).
#[component]
fn WikiTree(nodes: Vec<TreeNode>, depth: usize) -> AnyView {
    let state = expect_context::<State>();
    nodes
        .into_iter()
        .map(|node| {
            let TreeNode {
                name,
                path,
                is_dir,
                children,
            } = node;
            let indent = 8 + depth as i32 * 13;
            let p_children = path.clone();
            view! {
                <div class="ido-tree-item">
                    <WikiTreeRow name=name path=path is_dir=is_dir indent=indent />
                    {move || {
                        (is_dir && state.wiki_expanded.get().contains(&p_children))
                            .then(|| {
                                let children = children.clone();
                                view! { <WikiTree nodes=children depth=depth + 1 /> }
                            })
                    }}
                </div>
            }
        })
        .collect_view()
        .into_any()
}

/// A single wiki-tree row. Click opens a page / toggles a folder (and targets it
/// for new items); hover reveals rename + delete; the row is draggable and
/// folders are drop targets.
#[component]
fn WikiTreeRow(name: String, path: String, is_dir: bool, indent: i32) -> impl IntoView {
    let state = expect_context::<State>();
    // One clone per closure that needs to own the path.
    let p_click = path.clone();
    let p_active = path.clone();
    let p_target = path.clone();
    let p_match = path.clone();
    let p_chevron = path.clone();
    let p_drag = path.clone();
    let p_over = path.clone();
    let p_dropclass = path.clone();
    let p_menu = path.clone();
    let p_drop = path;
    let label = name;

    view! {
        <div
            class="ido-tree-row"
            draggable="true"
            class:active=move || {
                !is_dir && state.active_page().as_deref() == Some(slug_of(&p_active).as_str())
            }
            class:target=move || is_dir && state.wiki_target.get() == p_target
            class:drop=move || {
                is_dir && state.wiki_drag_over.get().as_deref() == Some(p_dropclass.as_str())
            }
            style=format!("padding-left:{indent}px")
            on:click=move |_| {
                if is_dir {
                    state.toggle_wiki_expand(p_click.clone());
                    state.wiki_target.set(p_click.clone());
                } else {
                    state.preview_wiki(slug_of(&p_click));
                    state.wiki_target.set(parent_of(&p_click));
                }
            }
            on:contextmenu=move |ev| {
                ev.prevent_default();
                ev.stop_propagation();
                state
                    .open_menu(
                        ev.client_x(),
                        ev.client_y(),
                        MenuTarget::Wiki {
                            path: p_menu.clone(),
                            is_dir,
                        },
                    );
            }
            on:dragstart=move |_| state.wiki_dragging.set(Some((p_drag.clone(), is_dir)))
            on:dragend=move |_| {
                state.wiki_dragging.set(None);
                state.wiki_drag_over.set(None);
            }
            on:dragover=move |ev| {
                if is_dir {
                    ev.prevent_default();
                    ev.stop_propagation();
                    state.wiki_drag_over.set(Some(p_over.clone()));
                }
            }
            on:drop=move |ev| {
                if is_dir {
                    ev.prevent_default();
                    ev.stop_propagation();
                    state.wiki_drag_over.set(None);
                    state.move_wiki_into(p_drop.clone());
                }
            }
        >
            {move || {
                if state.wiki_renaming.get() == Some((p_match.clone(), is_dir)) {
                    let p_key = p_match.clone();
                    let p_blur = p_match.clone();
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
                                            .commit_wiki_rename(
                                                p_key.clone(),
                                                is_dir,
                                                event_target_value(&ev),
                                            )
                                    }
                                    "Escape" => state.wiki_renaming.set(None),
                                    _ => {}
                                }
                            }
                            on:blur=move |ev| {
                                state
                                    .commit_wiki_rename(
                                        p_blur.clone(),
                                        is_dir,
                                        event_target_value(&ev),
                                    )
                            }
                        />
                    }
                        .into_any()
                } else {
                    let p_ren = p_match.clone();
                    let p_del = p_match.clone();
                    let p_chev = p_chevron.clone();
                    view! {
                        <span class="ido-tree-label">
                            {is_dir
                                .then(|| {
                                    view! {
                                        <span
                                            class="ido-chevron"
                                            class:open=move || {
                                                state.wiki_expanded.get().contains(&p_chev)
                                            }
                                        >
                                            <Icon name="chevron-right" size=13 />
                                        </span>
                                    }
                                })} <span class="ido-tree-name">{label.clone()}</span>
                        </span>
                        <span class="ido-tree-actions">
                            <button
                                class="ido-tree-action"
                                title="Rename"
                                on:click=move |ev| {
                                    ev.stop_propagation();
                                    state.wiki_renaming.set(Some((p_ren.clone(), is_dir)));
                                }
                            >
                                <Icon name="pencil" size=13 />
                            </button>
                            <button
                                class="ido-tree-action"
                                title="Delete"
                                on:click=move |ev| {
                                    ev.stop_propagation();
                                    state.remove_wiki_entry(p_del.clone(), is_dir);
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
