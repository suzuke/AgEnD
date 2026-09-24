//! Message delivery model (plan §4.4): ids, `queued -> sent -> confirmed |
//! failed`, one idempotency layer, busy level chosen by urgency, pushes carry
//! the full content. Paths that cannot confirm delivery are marked unconfirmed.
//!
//! Must NOT: fall back to typing into a PTY, or report a parked/queued message
//! as a failure.
