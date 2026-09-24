//! `agend init`: create the home and `config.toml`, register launchd/systemd,
//! detect backends, create the `general` team and one agent; inside a repo, ask
//! whether to register it; finish with `doctor`. Asks questions only on an
//! interactive terminal, otherwise everything comes from flags.
//!
//! Must NOT: modify the user's own git or PATH (shims go only on agent PATHs).
