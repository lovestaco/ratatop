//! ratatop — btop's CPU box, rebuilt on ratatui.
//!
//! Exposed as a library so the collectors and widgets can be tested without a
//! terminal; see `src/main.rs` for the binary.

pub mod app;
pub mod collect;
pub mod theme;
pub mod ui;
pub mod widgets;
