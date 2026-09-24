//! Runs `command` stages (the checks): in a temporary detached worktree at the
//! head under test, never in the development worktree, with a timeout.
//!
//! Must NOT: run commands in an agent's working worktree.
