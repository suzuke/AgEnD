//! Notifier (Telegram): one "needs you" topic plus one topic per team;
//! per-instance topics are optional, not default (D13). Humans are notified
//! only for exceptions.
//!
//! Must NOT: silently drop inbound messages when the allowlist is empty (report
//! it via `agend doctor` instead).

pub mod config;
pub mod delivery;
pub mod http;
pub mod inbound;
pub mod pairing;
pub mod pairing_service;
pub mod poll;
pub mod worker;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod poll_tests;
