//! The single reconciliation pass (boot and daily): compares DB records with
//! git reality in both directions inside the daemon's namespace
//! (`agend/<task-id>/...`, `worktrees/<task-id>/`). Replaces v1's ~9 cleanup
//! mechanisms. WIP of finished tasks is archived as a patch before deletion.
//!
//! Must NOT: touch branches or directories outside the namespace.
