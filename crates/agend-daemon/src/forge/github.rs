//! GitHub forge: merge the PR through the API, passing the approved head SHA.
//!
//! Must NOT: merge a head other than the approved one.

pub mod api;
pub mod client;
pub mod pull;
