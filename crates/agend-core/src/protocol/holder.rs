//! Holder protocol: daemon <-> per-instance holder. Carries spawn, PTY
//! resize/signal, single control keys, screen snapshots, output stream and
//! exit status. Needs real version negotiation (v1's `framing.rs` had a single
//! version byte and no negotiation).
//!
//! Must NOT: carry message text for the agent. Message content goes through
//! the backend's structured API; the PTY only ever receives single control
//! keys such as `Esc`.

/// A single control key the daemon may ask a holder to write to an agent PTY.
///
/// This is deliberately the whole vocabulary: typing text into a PTY is not a
/// supported delivery path in v2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlKey {
    /// Interrupts the current turn (claude driver, D16).
    Esc,
}
