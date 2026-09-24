//! Agent commands (D17): status, done, result, review approve/changes, send,
//! inbox, ask, block/unblock, task create, remind. Fewer than 15 commands,
//! common ones take <= 2 arguments, `--help` leads with examples, `--json`
//! everywhere. `agend status` shows the current step and the possible next steps.
//!
//! Must NOT: accept context the daemon can derive (task id, branch, repo, PR,
//! head, correlation id).
