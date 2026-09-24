//! Workflow definitions (TOML; the DB is the source of truth, each save is a
//! new version) and the save-time checks of D19:
//!
//! - stage ids unique, kinds valid;
//! - a `work` producing a branch precedes `submit`;
//! - `submit`/`merge` require `requires = ["repo"]`;
//! - `merge` is preceded by an `approval` bound to head, unless
//!   `allow_unreviewed = true` is written explicitly;
//! - `on_fail` only points to an earlier stage;
//! - every role exists in the team the workflow is applied to.
//!
//! Built-in (read-only) workflows: `code` (work -> submit -> command ->
//! approval -> merge), `research` (work(result) -> approval), `epic`
//! (work(plan) -> fanout -> approval).
//!
//! Must NOT: parse from disk or touch the DB.
