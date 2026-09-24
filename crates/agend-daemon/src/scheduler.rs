//! Scheduler: stage timeouts and cron schedules. One-shot schedules are task
//! reminders.
//!
//! Must NOT: sleep on the runtime's worker threads.
