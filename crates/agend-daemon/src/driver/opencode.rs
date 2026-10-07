//! opencode driver: HTTP + SSE against `opencode serve` (held by the holder).
//! SSE has no replay: reconcile with REST after reconnect. Queue with
//! `prompt_async`, interrupt with `POST /session/:id/abort`; no steer.
//! Permission requests: poll `GET /permission` as the source of truth.
//!
//! Must NOT: treat SSE as the only source of permission requests.

pub mod api;
pub mod driver;
pub mod history;
pub mod http;
pub mod launch;
pub mod permission;
pub mod runtime;
pub mod worker;
pub use driver::OpenCodeDriver;
