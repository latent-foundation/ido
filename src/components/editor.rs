//! The editor screen: sidebar (tree, toolbar, brand) plus the main editing pane.
//!
//! ## Three-mode system
//!
//! Three icon buttons in the header switch between modes (`state.mode`):
//!
//! - **Source** (`</>`): single raw-markdown `<textarea>`, uncontrolled — seeded
//!   imperatively on open/switch so the cursor never jumps. Saves on every input.
//! - **Live** (pencil): block live-preview. Each top-level block is rendered as
//!   HTML; clicking it swaps it for a `<textarea>` that auto-grows with content.
//!   **Double-Enter** (blank line) commits the current block and opens the next.
//!   **↑/↓** navigate between blocks when the cursor is at the first/last line.
//!   **Backspace at column 0** merges the block with the one above.
//!   Escape cancels without committing; blur commits (failsafe).
//! - **Reading** (eye): fully rendered, read-only. Link clicks → OS browser.
//!
//! ## Not yet built (future increments)
//! - Inline live-preview inside the active block (Increment 4 stretch goal).

use leptos::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::HtmlTextAreaElement;

use crate::components::tree::Tree;
use crate::icon::Icon;
use crate::markdown;
use crate::state::{Mode, State};

/// Sidebar + main editing pane. Two flex siblings under `.ido-app`.
#[component]
pub fn Editor() -> impl IntoView {
    let state = expect_context::<State>();
    view! {
        <aside class="ido-sidebar">
            <div class="ido-brand">
                <button
                    class="ido-well-switch"
                    title="Switch well"
                    on:click=move |_| state.leave_well()
                >
                    <span class="ido-wordmark">"ido"</span>
                </button>
                <span class="ido-well-name">
                    {move || state.well.get().map(|w| w.name).unwrap_or_default()}
                </span>
            </div>

            <div class="ido-toolbar">
                <button
                    class="ido-tool"
                    title="New note"
                    on:click=move |_| state.add_note(state.target.get_untracked())
                >
                    <Icon name="file-plus" size=15 />
                </button>
                <button
                    class="ido-tool"
                    title="New folder"
                    on:click=move |_| state.add_folder(state.target.get_untracked())
                >
                    <Icon name="folder-plus" size=15 />
                </button>
                <button
                    class="ido-tool-target"
                    title="New items go here — click for the well root"
                    on:click=move |_| state.target.set(String::new())
                >
                    {move || {
                        let t = state.target.get();
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
                    state.drag_over.set(None);
                }
                on:drop=move |ev| {
                    ev.prevent_default();
                    state.drag_over.set(None);
                    state.move_into(String::new());
                }
            >
                {move || view! { <Tree nodes=state.tree.get() depth=0 /> }}
            </nav>

            <div class="ido-status">
                <button class="ido-settings-btn" on:click=move |_| state.settings_open.set(true)>
                    <Icon name="settings" size=15 />
                    "settings"
                </button>
            </div>
        </aside>

        <main class="ido-main">
            <div class="ido-editor-head">
                <span class="ido-editor-title">
                    {move || {
                        state
                            .active
                            .get()
                            .map(|id| id.rsplit('/').next().unwrap_or(&id).to_string())
                            .unwrap_or_default()
                    }}
                </span>

                // Three-way mode toggle + help hint.
                <div class="ido-head-controls">
                    <div class="ido-mode-buttons">
                        <button
                            class="ido-mode-btn"
                            class:active=move || state.mode.get() == Mode::Source
                            title="Source"
                            on:click=move |_| state.set_mode(Mode::Source)
                        >
                            <Icon name="code" size=14 />
                        </button>
                        <button
                            class="ido-mode-btn"
                            class:active=move || state.mode.get() == Mode::Live
                            title="Live"
                            on:click=move |_| state.set_mode(Mode::Live)
                        >
                            <Icon name="pencil" size=14 />
                        </button>
                        <button
                            class="ido-mode-btn"
                            class:active=move || state.mode.get() == Mode::Reading
                            title="Reading"
                            on:click=move |_| state.set_mode(Mode::Reading)
                        >
                            <Icon name="eye" size=14 />
                        </button>
                    </div>
                    <HelpTip />
                </div>
            </div>

            <div class="ido-editor-body">
                {move || match state.mode.get() {
                    Mode::Source => view! { <SourceEditor /> }.into_any(),
                    Mode::Live => view! { <BlockEditor /> }.into_any(),
                    Mode::Reading => {
                        view! {
                            <div
                                class="markdown-body"
                                on:click=move |ev| {
                                    let anchor = ev
                                        .target()
                                        .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
                                        .and_then(|el| el.closest("a").ok().flatten());
                                    if let Some(a) = anchor {
                                        if let Some(href) = a.get_attribute("href") {
                                            ev.prevent_default();
                                            state.open_link(href);
                                        }
                                    }
                                }
                                inner_html=move || markdown::render(&state.content.get())
                            />
                        }
                            .into_any()
                    }
                }}
            </div>
        </main>
    }
}

// ══════════════════════════════════════════════════════ Help tooltip ═════════

/// A `?` button that reveals a CSS-only shortcut cheatsheet on hover/focus.
#[component]
fn HelpTip() -> impl IntoView {
    view! {
        <div class="ido-help-wrap">
            <button class="ido-help-btn" aria-label="Editor help">
                <Icon name="circle-help" size=14 />
            </button>
            <div class="ido-help-tip" role="tooltip">
                <div class="ido-help-head">"live mode"</div>
                <dl class="ido-shortcuts">
                    <div>
                        <dt>"click"</dt>
                        <dd>"edit block"</dd>
                    </div>
                    <div>
                        <dt>"↑ / ↓"</dt>
                        <dd>"navigate"</dd>
                    </div>
                    <div>
                        <dt>"↵ on blank line"</dt>
                        <dd>"new block / split"</dd>
                    </div>
                    <div>
                        <dt>"⌫ at start"</dt>
                        <dd>"merge up"</dd>
                    </div>
                    <div>
                        <dt>"esc"</dt>
                        <dd>"cancel"</dd>
                    </div>
                </dl>
            </div>
        </div>
    }
}

// ══════════════════════════════════════════════════════ Source mode ══════════

/// Full raw-markdown `<textarea>`, uncontrolled. Seeded via the NodeRef stored in
/// `state.source_editor` when the element mounts or when a note is opened.
#[component]
fn SourceEditor() -> impl IntoView {
    let state = expect_context::<State>();

    // Populate the textarea the moment it appears in the DOM (mode switch or
    // first open in Source mode). `state.source_editor.get()` is tracked, so
    // the effect fires when the element mounts. `content.get_untracked()` avoids
    // re-running (and jumping the cursor) on every subsequent content change.
    Effect::new(move |_| {
        if let Some(ta) = state.source_editor.get() {
            ta.set_value(&state.content.get_untracked());
            let _ = ta.focus();
        }
    });

    view! {
        <textarea
            class="ido-editor"
            node_ref=state.source_editor
            on:input=move |ev| state.update_source(event_target_value(&ev))
            prop:disabled=move || state.active.get().is_none()
            placeholder="Select a note, or create one to begin writing…"
        ></textarea>
    }
}

// ══════════════════════════════════════════════════════ Live / block mode ════

// ── Cursor helpers ───────────────────────────────────────────────────────────

/// Extract the cursor position (`selectionStart`) and current value from the
/// textarea that fired `ev`. Returns `(u32::MAX, "")` on failure.
fn ta_cursor(ev: &web_sys::KeyboardEvent) -> (u32, String) {
    let Some(target) = ev.target() else {
        return (u32::MAX, String::new());
    };
    let Ok(ta) = target.dyn_into::<HtmlTextAreaElement>() else {
        return (u32::MAX, String::new());
    };
    let pos = ta.selection_start().ok().flatten().unwrap_or(u32::MAX);
    (pos, ta.value())
}

/// `true` when `pos` falls on (or before) the first line of `value`.
fn cursor_on_first_line(pos: u32, value: &str) -> bool {
    match value.find('\n') {
        Some(nl) => pos <= nl as u32,
        None => true,
    }
}

/// `true` when `pos` falls on the last line of `value`.
fn cursor_on_last_line(pos: u32, value: &str) -> bool {
    match value.rfind('\n') {
        Some(nl) => pos > nl as u32,
        None => true,
    }
}

/// `true` when the entire line the cursor sits on is blank (whitespace-only).
///
/// Used to detect the "double-Enter → new block" gesture: after the first Enter
/// creates a blank line, pressing Enter a second time on that blank line splits
/// or appends. Works for blank lines anywhere in the textarea, not just at the
/// trailing end.
fn cursor_on_blank_line(pos: u32, value: &str) -> bool {
    let pos = pos as usize;
    if pos > value.len() {
        return false;
    }
    let line_start = value[..pos].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let line_end = value[pos..]
        .find('\n')
        .map(|i| pos + i)
        .unwrap_or(value.len());
    value[line_start..line_end].trim().is_empty()
}

/// Return `(text_before, text_after)` the blank line that `pos` sits on.
///
/// `text_before` is the content up to (but not including) the blank line's
/// leading `\n`.  `text_after` is the content after the blank line's trailing
/// `\n`, with leading blank lines stripped.  Both are trimmed of trailing
/// whitespace so the caller can pass them directly to `State::split_block`.
fn split_at_blank_line(pos: u32, value: &str) -> (String, String) {
    let pos = pos as usize;
    let line_start = value[..pos].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let line_end = value[pos..]
        .find('\n')
        .map(|i| pos + i)
        .unwrap_or(value.len());
    // Everything before the blank line's opening newline.
    let before = value[..line_start.saturating_sub(1)].trim_end().to_string();
    // Everything after the blank line's closing newline.
    let after = value[line_end..]
        .trim_start_matches('\n')
        .trim_end()
        .to_string();
    (before, after)
}

// ── Components ───────────────────────────────────────────────────────────────

/// Container that renders each top-level block, plus a "new block at end"
/// textarea when `active_block == blocks.len()`.
#[component]
fn BlockEditor() -> impl IntoView {
    let state = expect_context::<State>();
    view! {
        <div class="ido-block-editor">
            {move || {
                let blocks = state.blocks.get();
                let active = state.active_block.get();
                if blocks.is_empty() {
                    // Note is open but empty — single click-to-edit area.
                    view! { <EmptyBlock /> }
                        .into_any()
                } else {
                    let n = blocks.len();
                    let mut views: Vec<_> = blocks
                        .into_iter()
                        .enumerate()
                        .map(|(i, block)| {
                            let rendered = markdown::render(&block.src);
                            let raw = block.src.clone();
                            view! { <BlockView idx=i raw=raw rendered=rendered /> }.into_any()
                        })
                        .collect();
                    if active == Some(n) {
                        views
                            .push(
                                // When active_block == len, show the new-block textarea at the end.
                                view! { <NewBlockInput idx=n /> }
                                    .into_any(),
                            );
                    }
                    views.into_iter().collect_view().into_any()
                }
            }}
        </div>
    }
}

/// A single rendered block. Clicking it activates inline editing.
///
/// **Keyboard shortcuts inside the active textarea:**
/// - Double-Enter (blank line) — commit + open next block
/// - ↑ on first line — commit + focus previous block
/// - ↓ on last line — commit + focus next block
/// - Backspace at column 0 — merge with the block above
/// - Escape — cancel (revert to rendered view)
///
/// Blur commits (failsafe), but is skipped when keyboard navigation already
/// committed the block (guard: `active_block` is no longer `Some(idx)`).
#[component]
fn BlockView(idx: usize, raw: String, rendered: String) -> impl IntoView {
    let state = expect_context::<State>();
    let input_ref: NodeRef<leptos::html::Textarea> = NodeRef::new();

    // Focus + seed the textarea the moment it mounts (when this block becomes active).
    // `field-sizing: content` in CSS handles height automatically.
    let raw_for_effect = raw.clone();
    Effect::new(move |_| {
        if let Some(ta) = input_ref.get() {
            ta.set_value(&raw_for_effect);
            let _ = ta.focus();
        }
    });

    view! {
        {move || {
            if state.active_block.get() == Some(idx) {
                view! {
                    <textarea
                        class="ido-block-input"
                        node_ref=input_ref
                        on:blur=move |ev| {
                            if state.active_block.get_untracked() == Some(idx) {
                                state.commit_block(idx, event_target_value(&ev));
                            }
                        }
                        on:keydown=move |ev| {
                            match ev.key().as_str() {
                                "Escape" => state.active_block.set(None),
                                "Enter" => {
                                    let (pos, val) = ta_cursor(&ev);
                                    if pos != u32::MAX && cursor_on_blank_line(pos, &val) {
                                        let (before, after) = split_at_blank_line(pos, &val);
                                        if !before.is_empty() || !after.is_empty() {
                                            ev.prevent_default();
                                            state.split_block(idx, before, after);
                                        }
                                    }
                                }
                                "ArrowUp" if idx > 0 => {
                                    let (pos, val) = ta_cursor(&ev);
                                    if pos != u32::MAX && cursor_on_first_line(pos, &val) {
                                        ev.prevent_default();
                                        state.commit_and_go(idx, val, idx - 1);
                                    }
                                }
                                "ArrowDown" => {
                                    let (pos, val) = ta_cursor(&ev);
                                    if pos != u32::MAX && cursor_on_last_line(pos, &val) {
                                        ev.prevent_default();
                                        state.commit_and_go(idx, val, idx + 1);
                                    }
                                }
                                "Backspace" if idx > 0 => {
                                    let (pos, val) = ta_cursor(&ev);
                                    if pos == 0 {
                                        ev.prevent_default();
                                        state.merge_with_prev(idx, val);
                                    }
                                }
                                _ => {}
                            }
                        }
                    />
                }
                    .into_any()
            } else {
                let r = rendered.clone();
                // Split only when cursor is on a trailing blank line:
                // cursor is at end AND last char is already '\n'.
                view! {
                    <div
                        class="ido-block"
                        on:click=move |_| {
                            if state.active.get_untracked().is_some() {
                                state.active_block.set(Some(idx));
                            }
                        }
                        inner_html=r
                    />
                }
                    .into_any()
            }
        }}
    }
}

/// A fresh textarea appended after the last block — appears when double-Enter is
/// pressed on the final block, or when `active_block` is set to `blocks.len()`.
/// Blurring with content appends a new block; blurring empty dismisses.
#[component]
fn NewBlockInput(idx: usize) -> impl IntoView {
    let state = expect_context::<State>();
    let input_ref: NodeRef<leptos::html::Textarea> = NodeRef::new();

    Effect::new(move |_| {
        if let Some(ta) = input_ref.get() {
            let _ = ta.focus();
        }
    });

    view! {
        <textarea
            class="ido-block-input"
            node_ref=input_ref
            placeholder="…"
            on:blur=move |ev| {
                if state.active_block.get_untracked() == Some(idx) {
                    state.commit_block(idx, event_target_value(&ev));
                }
            }
            on:keydown=move |ev| {
                match ev.key().as_str() {
                    "Escape" => state.active_block.set(None),
                    "Enter" => {
                        let (pos, val) = ta_cursor(&ev);
                        if pos != u32::MAX && cursor_on_blank_line(pos, &val) {
                            let (before, after) = split_at_blank_line(pos, &val);
                            if !before.is_empty() || !after.is_empty() {
                                ev.prevent_default();
                                state.split_block(idx, before, after);
                            }
                        }
                    }
                    "ArrowUp" if idx > 0 => {
                        let (pos, val) = ta_cursor(&ev);
                        if pos != u32::MAX && cursor_on_first_line(pos, &val) {
                            ev.prevent_default();
                            state.commit_and_go(idx, val, idx - 1);
                        }
                    }
                    _ => {}
                }
            }
        />
    }
}

/// Shown when the note is open but has no blocks yet (empty file).
/// A click-to-edit placeholder; clicking shows a textarea for the whole note.
/// Empty notes also auto-activate on open (`open_note` sets `active_block = Some(0)`).
#[component]
fn EmptyBlock() -> impl IntoView {
    let state = expect_context::<State>();
    let input_ref: NodeRef<leptos::html::Textarea> = NodeRef::new();

    Effect::new(move |_| {
        if let Some(ta) = input_ref.get() {
            let _ = ta.focus();
        }
    });

    view! {
        {move || {
            let note_open = state.active.get().is_some();
            if note_open && state.active_block.get() == Some(0) {
                view! {
                    <textarea
                        class="ido-block-input ido-block-input-full"
                        node_ref=input_ref
                        placeholder="Start writing…"
                        on:blur=move |ev| {
                            if state.active_block.get_untracked() == Some(0) {
                                state.commit_block(0, event_target_value(&ev));
                            }
                        }
                        on:keydown=move |ev| {
                            match ev.key().as_str() {
                                "Escape" => state.active_block.set(None),
                                "Enter" => {
                                    let (pos, val) = ta_cursor(&ev);
                                    if pos != u32::MAX && cursor_on_blank_line(pos, &val) {
                                        let (before, after) = split_at_blank_line(pos, &val);
                                        if !before.is_empty() || !after.is_empty() {
                                            ev.prevent_default();
                                            state.split_block(0, before, after);
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    />
                }
                    .into_any()
            } else if note_open {
                view! {
                    <div
                        class="ido-block-placeholder"
                        on:click=move |_| state.active_block.set(Some(0))
                    >
                        "Start writing…"
                    </div>
                }
                    .into_any()
            } else {
                view! {
                    <div class="ido-block-placeholder ido-block-placeholder-idle">
                        "Select a note, or create one to begin writing…"
                    </div>
                }
                    .into_any()
            }
        }}
    }
}
