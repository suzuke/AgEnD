//! opencode driver: HTTP + SSE against `opencode serve` (held by the holder).
//! SSE has no replay: reconcile with REST after reconnect. Queue with
//! `prompt_async`, interrupt with `POST /session/:id/abort`; no steer.
//! Permission requests: poll `GET /permission` as the source of truth.
//!
//! Must NOT: treat SSE as the only source of permission requests.
