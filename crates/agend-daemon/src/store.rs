//! SQLite store, the only persistent state: instance, team, repo, workflow,
//! task, message, binding, review, decision, schedule, events. Owned by one
//! dedicated thread and reached through a channel. Every table has a retention
//! period; a daily `VACUUM INTO` snapshot is kept (N copies).
//!
//! Must NOT: be called from async tasks directly (go through the DB thread).
