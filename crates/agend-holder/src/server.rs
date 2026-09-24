//! Holder protocol server: versioned with negotiation, backward compatible,
//! accepts a new daemon connection after the old one goes away. Holders are
//! updated rarely.
//!
//! Must NOT: exit when the daemon disconnects.
