//! Merge gate and approval retention (plan §4.5, D14).
//!
//! Merge is allowed only when checks passed and the approved head equals the
//! current head. After main advances the daemon rebases and reruns checks; the
//! approval is kept only if the rebase had no conflict and the `git patch-id`
//! of the branch's own diff is unchanged, otherwise the task returns to work.
//!
//! Must NOT: compute patch-ids or run git (inputs are passed in).
