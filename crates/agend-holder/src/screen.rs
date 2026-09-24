//! Maintains the terminal screen with alacritty_terminal and serves snapshots.
//! A reconnecting daemon gets the current screen, never a byte replay (a
//! replay can start in the middle of an escape sequence).
//!
//! Must NOT: classify the screen (that is `agend_core::screen`).
