//! SQLite store, the only persistent state. Gate 5 builds three tables:
//! `tasks`, `workflows`, `task_events`; every later gate that needs
//! persistence adds its own tables in its own migration. Owned by one
//! dedicated thread and reached through a channel. Every table has a
//! retention period; a daily `VACUUM INTO` snapshot is kept (7 copies).
//!
//! Must NOT: be called from async tasks directly (go through the DB thread).
