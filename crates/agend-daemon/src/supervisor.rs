//! Supervisor: detects stuck agents, usage limits and unknown prompts;
//! reassigns work to another allowed backend; escalates to a human only for
//! exceptions (requests, blocked gates, stuck agents).
//!
//! Must NOT: kill or respawn holders on daemon shutdown.
