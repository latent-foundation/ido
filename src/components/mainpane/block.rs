//! Live / block mode: the block editor that renders each top-level block and
//! swaps the active one for a textarea, plus the textarea-cursor helpers that
//! drive its keyboard navigation ([`BlockEditor`]).

use leptos::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::HtmlTextAreaElement;

use super::attachments::{on_dragover, on_drop, on_paste};
use crate::markdown;
use crate::state::{Buffer, State};

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
/// textarea when `active_block == blocks.len()`. Operates on the given
/// [`Buffer`], so it serves both the main pane and the task drawer.
#[component]
pub(crate) fn BlockEditor(buffer: Buffer) -> impl IntoView {
    let state = expect_context::<State>();
    view! {
        <div class="ido-block-editor">
            {move || {
                let blocks = buffer.blocks.get();
                let active = buffer.active_block.get();
                if blocks.is_empty() {
                    // Document is open but empty — single click-to-edit area.
                    view! { <EmptyBlock buffer=buffer /> }
                        .into_any()
                } else {
                    let n = blocks.len();
                    let mut views: Vec<_> = blocks
                        .into_iter()
                        .enumerate()
                        .map(|(i, block)| {
                            let rendered = markdown::render(
                                &block.src,
                                |id| state.resolve_asset(id),
                            );
                            let raw = block.src.clone();
                            view! { <BlockView buffer=buffer idx=i raw=raw rendered=rendered /> }
                                .into_any()
                        })
                        .collect();
                    if active == Some(n) {
                        views
                            .push(
                                // When active_block == len, show the new-block textarea at the end.
                                view! { <NewBlockInput buffer=buffer idx=n /> }
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
fn BlockView(buffer: Buffer, idx: usize, raw: String, rendered: String) -> impl IntoView {
    // `state` only for link routing; all editing goes through `buffer`.
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
            if buffer.active_block.get() == Some(idx) {
                view! {
                    <textarea
                        class="ido-block-input"
                        node_ref=input_ref
                        on:blur=move |ev| {
                            if buffer.active_block.get_untracked() == Some(idx) {
                                buffer.commit_block(idx, event_target_value(&ev));
                            }
                        }
                        on:paste=on_paste(state)
                        on:dragover=on_dragover
                        on:drop=on_drop(state)
                        on:keydown=move |ev| {
                            match ev.key().as_str() {
                                "Escape" => {
                                    ev.stop_propagation();
                                    buffer.active_block.set(None);
                                }
                                "Enter" => {
                                    let (pos, val) = ta_cursor(&ev);
                                    if pos != u32::MAX && cursor_on_blank_line(pos, &val) {
                                        let (before, after) = split_at_blank_line(pos, &val);
                                        if !before.is_empty() || !after.is_empty() {
                                            ev.prevent_default();
                                            buffer.split_block(idx, before, after);
                                        }
                                    }
                                }
                                "ArrowUp" if idx > 0 => {
                                    let (pos, val) = ta_cursor(&ev);
                                    if pos != u32::MAX && cursor_on_first_line(pos, &val) {
                                        ev.prevent_default();
                                        buffer.commit_and_go(idx, val, idx - 1);
                                    }
                                }
                                "ArrowDown" => {
                                    let (pos, val) = ta_cursor(&ev);
                                    if pos != u32::MAX && cursor_on_last_line(pos, &val) {
                                        ev.prevent_default();
                                        buffer.commit_and_go(idx, val, idx + 1);
                                    }
                                }
                                "Backspace" if idx > 0 => {
                                    let (pos, val) = ta_cursor(&ev);
                                    if pos == 0 {
                                        ev.prevent_default();
                                        buffer.merge_with_prev(idx, val);
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
                let raw_click = raw.clone();
                // Stop here — otherwise this bubbles to the
                // workspace's global Escape and closes a
                // task/goal drawer in the same keystroke as
                // deactivating the block. One Esc leaves the
                // block; a second (with nothing left active)
                // closes the drawer.
                view! {
                    <div
                        class="ido-block"
                        on:click=move |ev| {
                            if let Some(i) = super::checkbox_click_index(&ev) {
                                ev.prevent_default();
                                if let Some(new_text) = markdown::toggle_checkbox(&raw_click, i) {
                                    buffer.commit_block(idx, new_text);
                                }
                                return;
                            }
                            let anchor = ev
                                .target()
                                .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
                                .and_then(|el| el.closest("a").ok().flatten());
                            if let Some(href) = anchor.and_then(|a| a.get_attribute("href")) {
                                ev.prevent_default();
                                state.open_link(href);
                            } else if buffer.open.get_untracked() {
                                buffer.active_block.set(Some(idx));
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
fn NewBlockInput(buffer: Buffer, idx: usize) -> impl IntoView {
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
            on:paste=on_paste(state)
            on:dragover=on_dragover
            on:drop=on_drop(state)
            on:blur=move |ev| {
                if buffer.active_block.get_untracked() == Some(idx) {
                    buffer.commit_block(idx, event_target_value(&ev));
                }
            }
            on:keydown=move |ev| {
                match ev.key().as_str() {
                    "Escape" => {
                        ev.stop_propagation();
                        buffer.active_block.set(None);
                    }
                    "Enter" => {
                        let (pos, val) = ta_cursor(&ev);
                        if pos != u32::MAX && cursor_on_blank_line(pos, &val) {
                            let (before, after) = split_at_blank_line(pos, &val);
                            if !before.is_empty() || !after.is_empty() {
                                ev.prevent_default();
                                buffer.split_block(idx, before, after);
                            }
                        }
                    }
                    "ArrowUp" if idx > 0 => {
                        let (pos, val) = ta_cursor(&ev);
                        if pos != u32::MAX && cursor_on_first_line(pos, &val) {
                            ev.prevent_default();
                            buffer.commit_and_go(idx, val, idx - 1);
                        }
                    }
                    _ => {}
                }
            }
        />
    }
}

/// Shown when a document is open but empty. A click-to-edit placeholder; clicking
/// shows a full-height textarea. Empty docs also auto-activate on open
/// (`load_active` sets `active_block = Some(0)`).
#[component]
fn EmptyBlock(buffer: Buffer) -> impl IntoView {
    let state = expect_context::<State>();
    let input_ref: NodeRef<leptos::html::Textarea> = NodeRef::new();

    Effect::new(move |_| {
        if let Some(ta) = input_ref.get() {
            let _ = ta.focus();
        }
    });

    view! {
        {move || {
            let doc_open = buffer.open.get();
            if doc_open && buffer.active_block.get() == Some(0) {
                view! {
                    <textarea
                        class="ido-block-input ido-block-input-full"
                        node_ref=input_ref
                        placeholder="Start writing…"
                        on:paste=on_paste(state)
                        on:dragover=on_dragover
                        on:drop=on_drop(state)
                        on:blur=move |ev| {
                            if buffer.active_block.get_untracked() == Some(0) {
                                buffer.commit_block(0, event_target_value(&ev));
                            }
                        }
                        on:keydown=move |ev| {
                            match ev.key().as_str() {
                                "Escape" => {
                                    ev.stop_propagation();
                                    buffer.active_block.set(None);
                                }
                                "Enter" => {
                                    let (pos, val) = ta_cursor(&ev);
                                    if pos != u32::MAX && cursor_on_blank_line(pos, &val) {
                                        let (before, after) = split_at_blank_line(pos, &val);
                                        if !before.is_empty() || !after.is_empty() {
                                            ev.prevent_default();
                                            buffer.split_block(0, before, after);
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    />
                }
                    .into_any()
            } else if doc_open {
                view! {
                    <div
                        class="ido-block-placeholder"
                        on:click=move |_| buffer.active_block.set(Some(0))
                    >
                        "Start writing…"
                    </div>
                }
                    .into_any()
            } else {
                view! {
                    <div class="ido-empty">
                        <span class="ido-empty-mark">"井戸"</span>
                        <span class="ido-empty-text">"open a note or page to begin"</span>
                    </div>
                }
                    .into_any()
            }
        }}
    }
}
