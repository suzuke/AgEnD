//! Debounce for agent state: an idle<->active change takes effect only after it
//! has been stable for N seconds (v1 saw ~750k transitions in about two days).
//! N is not decided yet.
//!
//! Must NOT: read clocks directly (time is passed in, see the `Clock` trait).
