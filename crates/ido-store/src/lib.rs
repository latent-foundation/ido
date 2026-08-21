//! ido-store — the tauri-free local-first store: notes, wiki, tasks, wells,
//! search, and session, all as plain markdown/TOML on disk.
//!
//! A *well* is a folder the user chooses; notes are the markdown files within
//! it, and subfolders form the tree. A note's name is its file name
//! (independent of the body text). Every function here takes plain arguments
//! (`String`, not Tauri types) and returns plain values, so each is directly
//! unit-tested with tempdirs and no mock runtime — and, the reason this crate
//! is split out from `src-tauri`, directly linkable from a plain binary (a
//! future MCP server) without dragging in the Tauri runtime.
//!
//! `src-tauri` wraps every function below in a one-line `#[tauri::command]`
//! (see its `commands` module). The three exceptions that need an
//! `AppHandle` — the native folder picker, and the recent-wells registry
//! write on open/create — stay in `src-tauri`; [`wells::open_well`] and
//! [`wells::create_well`] here are their pure halves.
//!
//! Layout:
//! - [`model`] — shapes shared with the frontend (`WellRef`, `TreeNode`)
//! - [`paths`] — pure id/name/slug helpers (unit-tested)
//! - [`wells`] — opening / creating / migrating wells + well-level settings
//! - [`notes`] — the tree and note/folder CRUD (unit-tested)
//! - [`wiki`] — the flat wiki page namespace + backlinks (unit-tested)
//! - [`tasks`] — the kanban task store over markdown frontmatter (unit-tested)
//! - [`repeat`] — pure recurrence math for a task's `repeat:` rule (unit-tested)
//! - [`assets`] — the shared `assets/` image-attachment store (unit-tested)
//! - [`search`] — cross-section full-text scan (unit-tested)
//! - [`frontmatter`] — the minimal `--- key: value ---` parser (unit-tested)
//! - [`session`] — the per-well open-tabs session (`.ido/session.toml`)

pub mod assets;
pub mod frontmatter;
pub mod model;
pub mod notes;
pub mod paths;
pub mod repeat;
pub mod search;
pub mod session;
pub mod tasks;
pub mod wells;
pub mod wiki;
