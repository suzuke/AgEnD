//! Classifies a git invocation: allow, redirect to the bound worktree, or
//! refuse. Refuses agent-created worktrees/branches (`git worktree`,
//! `checkout -b`, `switch -c`, `branch <new>`). Inputs: identity/home env,
//! bypass env, whether the parent is `gh`, canonical-repo detection, worktree
//! existence, the real git location (excluding the shim directory).
//!
//! Must NOT: guess a binding when the snapshot is missing.
