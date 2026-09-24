//! Command handlers, separate from transport (D7). Caller identity -> DB
//! binding gives task, branch, repo, PR, expected head; agents pass intent
//! only. Permissions depend on caller identity (agent vs operator commands,
//! D17); a rejection includes the correct command to run.
//!
//! Must NOT: read the caller's cwd to infer context, or know which transport
//! (socket, future MCP adapter) carried the call.
