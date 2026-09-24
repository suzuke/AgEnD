//! AgEnD TUI: the attention-first dashboard. Hierarchy Fleet -> Team -> Task or
//! Agent; everything is grouped by team, and the repo appears only in Task
//! Detail. `<-`/`->` always mean one level up/down.
//!
//! Must NOT: talk to the daemon except through `agend-client`, or hold state
//! the daemon does not also have (it is a client of protocol v1 like any other).

pub mod agent_detail;
pub mod attention;
pub mod home;
pub mod i18n;
pub mod task_detail;
pub mod team;
pub mod terminal;
