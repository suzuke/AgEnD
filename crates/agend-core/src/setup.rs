//! Setup rules as data and pure functions (gate 13): tested backend version
//! ranges, how to recognise a logged-in backend from its output, the minimum
//! git version (merge-tree needs >= 2.38), and the text of generated
//! launchd/systemd units. Used by `agend doctor`, `agend init` and the
//! installer in the `agend` crate.
//!
//! Must NOT: run commands, read or write files, or register services (the
//! `agend` crate's `setup` module executes these rules).
