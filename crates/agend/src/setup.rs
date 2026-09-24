//! Executes the setup rules from `agend_core::setup` (gate 13): runs version
//! and login probes, writes launchd/systemd units and registers the service,
//! installs and removes the agent-PATH shims (`agend uninstall` asks before
//! deleting data). `agend telegram setup` asks the daemon to pair; the
//! daemon's notifier does the pairing, this crate has no Telegram client.
//!
//! Must NOT: hold setup rules itself (they are pure data in core), or touch
//! the user's own git or PATH.
