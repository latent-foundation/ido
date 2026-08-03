//! The right-click context menu — ido-specific actions per surface, styled to
//! the latent. system. Driven by `state.menu` (set by each surface's
//! `on:contextmenu`); mounted once in the workspace. Esc or a click away closes.

use leptos::prelude::*;
use leptos::task::spawn_local;
use wasm_bindgen::{JsCast, JsValue};

use crate::icon::Icon;
use crate::state::{MenuTarget, Mode, State, parent_of};

/// `(document, document.<name>)` — `execCommand` isn't bound in this web-sys, so
/// it's reached reflectively. `pub(crate)`: `mainpane::attachments` reuses this
/// for the same `insertText` trick when splicing in a pasted/dropped image.
pub(crate) fn document_fn(name: &str) -> Option<(web_sys::Document, js_sys::Function)> {
    let doc = web_sys::window().and_then(|w| w.document())?;
    let f = js_sys::Reflect::get(&doc, &JsValue::from_str(name))
        .ok()?
        .dyn_into::<js_sys::Function>()
        .ok()?;
    Some((doc, f))
}

/// Run a `document.execCommand(cmd)` (copy / cut / selectAll on the focused element).
fn exec(cmd: &str) {
    if let Some((doc, f)) = document_fn("execCommand") {
        let _ = f.call1(&doc, &JsValue::from_str(cmd));
    }
}

/// Paste the clipboard text at the cursor of the focused editable element. Uses
/// `insertText`, which keeps the cursor and fires `input` (so the editor saves).
fn paste_clipboard() {
    let Some(win) = web_sys::window() else {
        return;
    };
    let clip = win.navigator().clipboard();
    spawn_local(async move {
        if let Ok(v) = wasm_bindgen_futures::JsFuture::from(clip.read_text()).await
            && let (Some(text), Some((doc, f))) = (v.as_string(), document_fn("execCommand"))
        {
            let _ = f.call3(
                &doc,
                &JsValue::from_str("insertText"),
                &JsValue::FALSE,
                &JsValue::from_str(&text),
            );
        }
    });
}

/// One menu row — an icon + label that runs `action` (which should close the menu).
#[component]
fn MenuItem(
    icon: &'static str,
    label: &'static str,
    #[prop(into)] action: Callback<()>,
) -> impl IntoView {
    view! {
        <button class="ido-menu-item" on:click=move |_| action.run(())>
            <span class="ido-menu-icon">
                <Icon name=icon size=14 />
            </span>
            {label}
        </button>
    }
}

/// A separator between groups of items.
#[component]
fn MenuSep() -> impl IntoView {
    view! { <div class="ido-menu-sep"></div> }
}

/// The context menu overlay.
#[component]
pub fn ContextMenu() -> impl IntoView {
    let state = expect_context::<State>();

    // Esc closes the menu.
    let handle = window_event_listener(leptos::ev::keydown, move |ev| {
        if ev.key() == "Escape" && state.menu.get_untracked().is_some() {
            state.close_menu();
        }
    });
    on_cleanup(move || handle.remove());

    view! {
        {move || {
            let menu = state.menu.get()?;
            let style = format!(
                "left: min({}px, calc(100vw - 196px)); top: min({}px, calc(100vh - 244px));",
                menu.x,
                menu.y,
            );
            Some(
                // Clamp to the viewport (rough max menu size) so it never overflows.
                view! {
                    <div
                        class="ido-menu-backdrop"
                        on:mousedown=move |_| state.close_menu()
                        on:contextmenu=move |ev| {
                            ev.prevent_default();
                            state.close_menu();
                        }
                    >
                        <div
                            class="ido-menu"
                            style=style
                            on:mousedown=move |ev| {
                                ev.stop_propagation();
                                ev.prevent_default();
                            }
                            on:contextmenu=move |ev| ev.prevent_default()
                        >
                            {menu_items(state, menu.target)}
                        </div>
                    </div>
                },
            )
        }}
    }
}

/// Build the menu rows for a target. Every action runs then closes the menu.
fn menu_items(state: State, target: MenuTarget) -> AnyView {
    match target {
        MenuTarget::Note { id, is_dir } if is_dir => {
            let (a, b, c, d) = (id.clone(), id.clone(), id.clone(), id);
            view! {
                <MenuItem
                    icon="file-plus"
                    label="new note"
                    action=move |_| {
                        state.add_note(a.clone());
                        state.close_menu();
                    }
                />
                <MenuItem
                    icon="folder-plus"
                    label="new folder"
                    action=move |_| {
                        state.add_folder(b.clone());
                        state.close_menu();
                    }
                />
                <MenuSep />
                <MenuItem
                    icon="pencil"
                    label="rename"
                    action=move |_| {
                        state.renaming.set(Some((c.clone(), true)));
                        state.close_menu();
                    }
                />
                <MenuSep />
                <MenuItem
                    icon="trash"
                    label="delete"
                    action=move |_| {
                        state.remove_entry(d.clone(), true);
                        state.close_menu();
                    }
                />
            }
            .into_any()
        }
        MenuTarget::Note { id, .. } => {
            let parent = parent_of(&id);
            let (open, ren, del) = (id.clone(), id.clone(), id);
            let (pn, pf) = (parent.clone(), parent);
            view! {
                <MenuItem
                    icon="file-text"
                    label="open"
                    action=move |_| {
                        state.preview_note(open.clone());
                        state.close_menu();
                    }
                />
                <MenuItem
                    icon="pencil"
                    label="rename"
                    action=move |_| {
                        state.renaming.set(Some((ren.clone(), false)));
                        state.close_menu();
                    }
                />
                <MenuSep />
                <MenuItem
                    icon="file-plus"
                    label="new note"
                    action=move |_| {
                        state.add_note(pn.clone());
                        state.close_menu();
                    }
                />
                <MenuItem
                    icon="folder-plus"
                    label="new folder"
                    action=move |_| {
                        state.add_folder(pf.clone());
                        state.close_menu();
                    }
                />
                <MenuSep />
                <MenuItem
                    icon="trash"
                    label="delete"
                    action=move |_| {
                        state.remove_entry(del.clone(), false);
                        state.close_menu();
                    }
                />
            }
            .into_any()
        }
        MenuTarget::Wiki { path, is_dir } if is_dir => {
            let (a, b, c, d) = (path.clone(), path.clone(), path.clone(), path);
            view! {
                <MenuItem
                    icon="file-plus"
                    label="new page"
                    action=move |_| {
                        state.add_page(a.clone());
                        state.close_menu();
                    }
                />
                <MenuItem
                    icon="folder-plus"
                    label="new folder"
                    action=move |_| {
                        state.add_wiki_folder(b.clone());
                        state.close_menu();
                    }
                />
                <MenuSep />
                <MenuItem
                    icon="pencil"
                    label="rename"
                    action=move |_| {
                        state.wiki_renaming.set(Some((c.clone(), true)));
                        state.close_menu();
                    }
                />
                <MenuSep />
                <MenuItem
                    icon="trash"
                    label="delete"
                    action=move |_| {
                        state.remove_wiki_entry(d.clone(), true);
                        state.close_menu();
                    }
                />
            }
            .into_any()
        }
        MenuTarget::Wiki { path, .. } => {
            let parent = parent_of(&path);
            let slug = path.rsplit('/').next().unwrap_or(&path).to_string();
            let (open, ren, del) = (slug, path.clone(), path);
            let (pn, pf) = (parent.clone(), parent);
            view! {
                <MenuItem
                    icon="book"
                    label="open"
                    action=move |_| {
                        state.preview_wiki(open.clone());
                        state.close_menu();
                    }
                />
                <MenuItem
                    icon="pencil"
                    label="rename"
                    action=move |_| {
                        state.wiki_renaming.set(Some((ren.clone(), false)));
                        state.close_menu();
                    }
                />
                <MenuSep />
                <MenuItem
                    icon="file-plus"
                    label="new page"
                    action=move |_| {
                        state.add_page(pn.clone());
                        state.close_menu();
                    }
                />
                <MenuItem
                    icon="folder-plus"
                    label="new folder"
                    action=move |_| {
                        state.add_wiki_folder(pf.clone());
                        state.close_menu();
                    }
                />
                <MenuSep />
                <MenuItem
                    icon="trash"
                    label="delete"
                    action=move |_| {
                        state.remove_wiki_entry(del.clone(), false);
                        state.close_menu();
                    }
                />
            }
            .into_any()
        }
        MenuTarget::Task { id } => {
            let (open, arch, del) = (id.clone(), id.clone(), id);
            view! {
                <MenuItem
                    icon="square-kanban"
                    label="open"
                    action=move |_| {
                        state.open_task(open.clone());
                        state.close_menu();
                    }
                />
                <MenuSep />
                <MenuItem
                    icon="archive"
                    label="archive"
                    action=move |_| {
                        state.archive_task(arch.clone(), true);
                        state.close_menu();
                    }
                />
                <MenuItem
                    icon="trash"
                    label="delete"
                    action=move |_| {
                        state.remove_task(del.clone());
                        state.close_menu();
                    }
                />
            }
            .into_any()
        }
        MenuTarget::Goal { id } => {
            let (edit, arch, del) = (id.clone(), id.clone(), id);
            view! {
                <MenuItem
                    icon="pencil"
                    label="edit"
                    action=move |_| {
                        state.open_goal(edit.clone());
                        state.close_menu();
                    }
                />
                <MenuSep />
                <MenuItem
                    icon="archive"
                    label="archive"
                    action=move |_| {
                        state.archive_goal(arch.clone(), true);
                        state.close_menu();
                    }
                />
                <MenuItem
                    icon="trash"
                    label="delete"
                    action=move |_| {
                        state.remove_goal(del.clone());
                        state.close_menu();
                    }
                />
            }
            .into_any()
        }
        MenuTarget::Tab { pane, idx } => view! {
            <MenuItem
                icon="x"
                label="close"
                action=move |_| {
                    state.close_tab(pane, idx);
                    state.close_menu();
                }
            />
            <MenuItem
                icon="x"
                label="close others"
                action=move |_| {
                    state.close_other_tabs(pane, idx);
                    state.close_menu();
                }
            />
            <MenuSep />
            <MenuItem
                icon="columns-2"
                label="split editor"
                action=move |_| {
                    state.open_split();
                    state.close_menu();
                }
            />
        }
        .into_any(),
        MenuTarget::Editor { pane } => {
            let p = state.panes[pane.min(1)];
            view! {
                <MenuItem
                    icon="scissors"
                    label="cut"
                    action=move |_| {
                        exec("cut");
                        state.close_menu();
                    }
                />
                <MenuItem
                    icon="copy"
                    label="copy"
                    action=move |_| {
                        exec("copy");
                        state.close_menu();
                    }
                />
                <MenuItem
                    icon="clipboard"
                    label="paste"
                    action=move |_| {
                        paste_clipboard();
                        state.close_menu();
                    }
                />
                <MenuItem
                    icon="square"
                    label="select all"
                    action=move |_| {
                        exec("selectAll");
                        state.close_menu();
                    }
                />
                <MenuSep />
                <MenuItem
                    icon="code"
                    label="source"
                    action=move |_| {
                        state.set_mode(p, Mode::Source);
                        state.close_menu();
                    }
                />
                <MenuItem
                    icon="pencil"
                    label="live"
                    action=move |_| {
                        state.set_mode(p, Mode::Live);
                        state.close_menu();
                    }
                />
                <MenuItem
                    icon="eye"
                    label="reading"
                    action=move |_| {
                        state.set_mode(p, Mode::Reading);
                        state.close_menu();
                    }
                />
                <MenuSep />
                <MenuItem
                    icon="columns-2"
                    label="split editor"
                    action=move |_| {
                        state.open_split();
                        state.close_menu();
                    }
                />
            }
            .into_any()
        }
    }
}
