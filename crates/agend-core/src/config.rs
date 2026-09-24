//! Daemon-level `config.toml`: home path, Telegram connection settings, and
//! references (env var name or file path) to secrets.
//!
//! This is the only human-written config file. The daemon reads it and never
//! writes it back. Instances, teams, repos and workflows live in the DB, not
//! here (D8).
//!
//! Must NOT: read files or env vars itself (callers pass the text in), or hold
//! secret values inline.
