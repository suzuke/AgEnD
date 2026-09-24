//! Protected-ref check: refuses writes to main via `update-ref`, `push .`,
//! `branch -f` (v1 let a bound agent's `update-ref` through:
//! `classify.rs:752-759`). Required for the local forge.
//!
//! Must NOT: have exceptions for bound agents.
