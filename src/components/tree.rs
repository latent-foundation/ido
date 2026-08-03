//! The recursive note-tree sidebar. [`Tree`] renders one level; `TreeRow`
//! renders a row (folder or note) with inline rename, delete, and drag-and-drop.

use leptos::prelude::*;

use crate::icon::Icon;
use crate::model::TreeNode;
use crate::state::{MenuTarget, State, parent_of};

/// Render a level of the tree, recursing into expanded folders.
///
/// Returns `AnyView` so the recursion through `<Tree/>` doesn't form an
/// unnameable, infinitely-recursive `impl Trait` type (E0720).
#[component]
pub fn Tree(nodes: Vec<TreeNode>, depth: usize) -> AnyView {
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
                    <TreeRow name=name path=path is_dir=is_dir indent=indent />
                    {move || {
                        (is_dir && state.expanded.get().contains(&p_children))
                            .then(|| {
                                let children = children.clone();
                                view! { <Tree nodes=children depth=depth + 1 /> }
                            })
                    }}
                </div>
            }
        })
        .collect_view()
        .into_any()
}

/// A single tree row. Click opens a note / toggles a folder (and targets it for
/// new items); hover reveals rename + delete; the row is draggable and folders
/// are drop targets.
#[component]
fn TreeRow(name: String, path: String, is_dir: bool, indent: i32) -> impl IntoView {
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
                !is_dir && state.active_note().as_deref() == Some(p_active.as_str())
            }
            class:target=move || is_dir && state.target.get() == p_target
            class:drop=move || {
                is_dir && state.drag_over.get().as_deref() == Some(p_dropclass.as_str())
            }
            style=format!("padding-left:{indent}px")
            on:click=move |_| {
                if is_dir {
                    state.toggle_expand(p_click.clone());
                    state.target.set(p_click.clone());
                } else {
                    state.preview_note(p_click.clone());
                    state.target.set(parent_of(&p_click));
                }
            }
            on:contextmenu=move |ev| {
                ev.prevent_default();
                ev.stop_propagation();
                state
                    .open_menu(
                        ev.client_x(),
                        ev.client_y(),
                        MenuTarget::Note {
                            id: p_menu.clone(),
                            is_dir,
                        },
                    );
            }
            on:dragstart=move |_| state.dragging.set(Some((p_drag.clone(), is_dir)))
            on:dragend=move |_| {
                state.dragging.set(None);
                state.drag_over.set(None);
            }
            on:dragover=move |ev| {
                if is_dir {
                    ev.prevent_default();
                    ev.stop_propagation();
                    state.drag_over.set(Some(p_over.clone()));
                }
            }
            on:drop=move |ev| {
                if is_dir {
                    ev.prevent_default();
                    ev.stop_propagation();
                    state.drag_over.set(None);
                    state.move_into(p_drop.clone());
                }
            }
        >
            {move || {
                if state.renaming.get() == Some((p_match.clone(), is_dir)) {
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
                                            .commit_rename(
                                                p_key.clone(),
                                                is_dir,
                                                event_target_value(&ev),
                                            )
                                    }
                                    "Escape" => state.renaming.set(None),
                                    _ => {}
                                }
                            }
                            on:blur=move |ev| {
                                state.commit_rename(p_blur.clone(), is_dir, event_target_value(&ev))
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
                                            class:open=move || state.expanded.get().contains(&p_chev)
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
                                    state.renaming.set(Some((p_ren.clone(), is_dir)));
                                }
                            >
                                <Icon name="pencil" size=13 />
                            </button>
                            <button
                                class="ido-tree-action"
                                title="Delete"
                                on:click=move |ev| {
                                    ev.stop_propagation();
                                    state.remove_entry(p_del.clone(), is_dir);
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
