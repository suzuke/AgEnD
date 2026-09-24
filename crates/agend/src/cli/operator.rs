//! Operator commands: instances, teams, repos, workflows (`agend workflow
//! list/show/new --from/edit/apply/check/history/rollback/delete`,
//! `agend team set-workflow`, D19), export/import, telegram setup, uninstall,
//! daemon restart/shutdown.
//!
//! Must NOT: be callable by agent identities (the daemon rejects them).
