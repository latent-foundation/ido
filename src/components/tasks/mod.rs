//! The tasks section: a kanban board (one column per `status`) with a search /
//! sort / hide-done toolbar, a backlog of un-columned tasks below it, and a
//! slide-in detail drawer for editing a task. Cards drag between columns (and
//! to/from the backlog) to change `status`; the hovered drop target highlights.
//! A toolbar toggle swaps the board for a month [`calendar`] view of the same
//! tasks (due dates) and goals (targets), or a flat sortable [`table`] view
//! (every task at once, for review).
//!
//! Split by area: pure [`logic`] helpers (sort / drop math / tag-autocomplete
//! ranking), the [`board`] itself (columns, cards, backlog), the [`calendar`]
//! month view, the [`table`] review view, the [`goals`] bar, the toolbar's
//! saved-[`views`] dropdown, the detail [`drawers`] (whose tags field is the
//! standalone [`tagsinput`] popover), and the [`archive`] view. [`TaskBoard`]
//! is the section's entry point.

mod archive;
mod board;
mod calendar;
mod drawers;
mod goals;
mod logic;
mod table;
mod tagsinput;
mod views;

pub use board::TaskBoard;
pub(crate) use logic::overdue_count;
