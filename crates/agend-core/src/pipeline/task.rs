//! Task-level relations and operations, which are not stages (plan §4.5.0).
//!
//! Relations: `parent`, `depends_on` (editable, may cross teams; cross-repo
//! work is expressed this way, D15), `superseded_by`.
//! Operations: reassign, reopen (a human reopens a done task), supersede (the
//! input changed; a new task takes over; not a failure).
//! A task pins the workflow version it was created with (D21).
//!
//! Must NOT: resolve identities or read storage.
