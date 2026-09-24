//! Notifier (Telegram): one "needs you" topic plus one topic per team;
//! per-instance topics are optional, not default (D13). Humans are notified
//! only for exceptions.
//!
//! Must NOT: silently drop inbound messages when the allowlist is empty (report
//! it via `agend doctor` instead).
