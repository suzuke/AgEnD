//! Receives claude hook events and other structured events. Hook events
//! written to the on-disk queue while the daemon was down are replayed on
//! startup, then the state is confirmed once with the screen classifier
//! (v1 dropped them: `src/main.rs:1243-1245`).
//!
//! Must NOT: decide pipeline transitions itself.
