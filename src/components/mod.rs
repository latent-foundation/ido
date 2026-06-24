//! UI components — the two screens and the pieces they compose. Each reads
//! shared state via `expect_context::<crate::state::State>()` and calls its
//! action methods, so components stay declarative.

pub mod editor;
pub mod launch;
pub mod settings;
pub mod titlebar;
pub mod tree;
