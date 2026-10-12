//! opencode driver: HTTP + SSE against `opencode serve` (held by the holder).
//! SSE has no replay: reconcile with REST after reconnect. Queue with
//! `prompt_async`, interrupt with `POST /session/:id/abort`; no steer.
//! Permission requests: poll `GET /permission` as the source of truth.
//!
//! Must NOT: treat SSE as the only source of permission requests.

/// Separate policies: changing endpoint admission must not widen permission replies.
pub const UNMANAGED_ENDPOINT_VERSION: &str = "1.18.34";
pub const PERMISSION_REPLY_VERSION: &str = "1.18.34";

pub mod api;
pub mod driver;
pub mod history;
pub mod http;
pub mod launch;
pub mod permission;
pub mod runtime;
pub mod worker;
pub use driver::OpenCodeDriver;

#[cfg(test)]
mod contract_tests;
