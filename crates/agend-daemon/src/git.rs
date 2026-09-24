//! Git adapter: worktree and branch creation/removal for the daemon, always
//! "record in DB first, then create, then mark done" so a crash can resume.
//! Only the daemon creates worktrees and branches. Calls the real git binary,
//! never the agent-facing shim.
//!
//! Must NOT: create anything outside `agend/<task-id>/...` / `worktrees/<task-id>/`.
