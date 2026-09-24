//! Local forge: merge with `git merge-tree --write-tree`, then a CAS
//! `update-ref` that checks main is still at the expected SHA. Never runs
//! `git merge` in the user's working directory; if the canonical checkout is on
//! main, its working directory going stale must be handled. Needs git >= 2.38.
//!
//! Must NOT: merge without the gate from `agend_core::policy::merge_gate`.
