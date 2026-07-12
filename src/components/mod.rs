//! UI components — the two screens and the pieces they compose. Each reads
//! shared state via `expect_context::<crate::state::State>()` and calls its
//! action methods, so components stay declarative.

pub mod contextmenu;
pub mod datepicker;
pub mod launch;
pub mod mainpane;
pub mod notes;
pub mod rail;
pub mod search;
pub mod settings;
pub mod tabs;
pub mod tasks;
pub mod titlebar;
pub mod toast;
pub mod tree;
pub mod wiki;
pub mod workspace;
