//! Reads the per-agent binding snapshot written by the daemon (D6): agent,
//! task_id, branch, worktree, source_repo, plus the binding kind (work or
//! review). No HMAC.
//!
//! Must NOT: write the snapshot or read the DB.
