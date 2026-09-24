//! Records the agent's exit status (code or signal) and reports it to the
//! daemon, including across a daemon reconnect.
//!
//! Must NOT: restart the agent on its own; the daemon decides.
