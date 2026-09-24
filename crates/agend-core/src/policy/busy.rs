//! Busy strategy (plan §4.4): the daemon picks one of three levels per message
//! by urgency, then maps it to what the target backend supports.

use crate::model::Backend;

/// How a message reaches an agent that is currently busy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusyLevel {
    /// Deliver after the current turn ends.
    Queue,
    /// Insert into the running turn without interrupting it.
    Steer,
    /// Interrupt the running turn, then deliver immediately.
    Interrupt,
}

/// The level actually used for `backend`. Only codex has a steer primitive
/// (`turn/steer`); claude and opencode fall back to interrupt (spike results,
/// docs/BACKEND-BEHAVIORS.md).
pub fn effective_level(backend: Backend, requested: BusyLevel) -> BusyLevel {
    match (backend, requested) {
        (Backend::Claude | Backend::Opencode, BusyLevel::Steer) => BusyLevel::Interrupt,
        (_, level) => level,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steer_falls_back_to_interrupt_without_native_support() {
        assert_eq!(
            effective_level(Backend::Codex, BusyLevel::Steer),
            BusyLevel::Steer
        );
        assert_eq!(
            effective_level(Backend::Claude, BusyLevel::Steer),
            BusyLevel::Interrupt
        );
        assert_eq!(
            effective_level(Backend::Opencode, BusyLevel::Steer),
            BusyLevel::Interrupt
        );
    }

    #[test]
    fn queue_and_interrupt_are_supported_everywhere() {
        for b in Backend::ALL {
            assert_eq!(effective_level(b, BusyLevel::Queue), BusyLevel::Queue);
            assert_eq!(
                effective_level(b, BusyLevel::Interrupt),
                BusyLevel::Interrupt
            );
        }
    }
}
