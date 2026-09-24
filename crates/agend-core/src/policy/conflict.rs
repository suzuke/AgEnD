//! Conflict prevention: given the files each in-flight task touches, warn or
//! order new assignments that overlap (plan §1, §4.5).
//!
//! Must NOT: run git (the daemon collects the file lists).
