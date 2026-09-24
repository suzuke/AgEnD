//! Busy/idle transition debounce (P5). Busy takes effect immediately; idle
//! must remain stable for five seconds. Time is supplied by the caller.
//!
//! Must NOT: read a clock directly (see the `Clock` trait).

pub const IDLE_STABLE_MS: u64 = 5_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentActivity {
    Idle,
    Busy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DebounceState {
    pub effective: AgentActivity,
    idle_candidate_since_ms: Option<u64>,
}

impl DebounceState {
    pub const fn new(effective: AgentActivity) -> Self {
        Self {
            effective,
            idle_candidate_since_ms: None,
        }
    }

    /// Observe a structured status event at `now_ms`.
    pub fn observe(&mut self, observed: AgentActivity, now_ms: u64) -> bool {
        match observed {
            AgentActivity::Busy => {
                self.idle_candidate_since_ms = None;
                self.set_effective(AgentActivity::Busy)
            }
            AgentActivity::Idle if self.effective == AgentActivity::Idle => {
                self.idle_candidate_since_ms = None;
                false
            }
            AgentActivity::Idle => {
                self.idle_candidate_since_ms.get_or_insert(now_ms);
                self.advance(now_ms)
            }
        }
    }

    /// Advance a pending idle observation without requiring another backend
    /// event. Returns true only when the effective activity changed.
    pub fn advance(&mut self, now_ms: u64) -> bool {
        let Some(since_ms) = self.idle_candidate_since_ms else {
            return false;
        };
        if now_ms.saturating_sub(since_ms) < IDLE_STABLE_MS {
            return false;
        }
        self.idle_candidate_since_ms = None;
        self.set_effective(AgentActivity::Idle)
    }

    pub const fn pending_idle_since_ms(&self) -> Option<u64> {
        self.idle_candidate_since_ms
    }

    fn set_effective(&mut self, activity: AgentActivity) -> bool {
        let changed = self.effective != activity;
        self.effective = activity;
        changed
    }
}

impl Default for DebounceState {
    fn default() -> Self {
        Self::new(AgentActivity::Idle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn busy_is_effective_immediately() {
        let mut state = DebounceState::new(AgentActivity::Idle);
        assert!(state.observe(AgentActivity::Busy, 10));
        assert_eq!(state.effective, AgentActivity::Busy);
        assert_eq!(state.pending_idle_since_ms(), None);
    }

    #[test]
    fn idle_requires_five_seconds_of_stability() {
        let mut state = DebounceState::new(AgentActivity::Busy);
        assert!(!state.observe(AgentActivity::Idle, 100));
        assert!(!state.advance(5_099));
        assert_eq!(state.effective, AgentActivity::Busy);
        assert!(state.advance(5_100));
        assert_eq!(state.effective, AgentActivity::Idle);
    }

    #[test]
    fn a_busy_event_cancels_the_pending_idle_period() {
        let mut state = DebounceState::new(AgentActivity::Busy);
        state.observe(AgentActivity::Idle, 0);
        assert!(!state.observe(AgentActivity::Busy, 4_000));
        assert_eq!(state.pending_idle_since_ms(), None);
        assert!(!state.advance(10_000));
        assert_eq!(state.effective, AgentActivity::Busy);
    }

    #[test]
    fn repeated_idle_events_do_not_restart_the_timer() {
        let mut state = DebounceState::new(AgentActivity::Busy);
        state.observe(AgentActivity::Idle, 1_000);
        state.observe(AgentActivity::Idle, 4_000);
        assert!(state.advance(6_000));
        assert_eq!(state.effective, AgentActivity::Idle);
    }
}
