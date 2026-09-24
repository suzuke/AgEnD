//! Screen classifier over a holder's rendered screen. Recognises only hard
//! gates: usage limit, permission/approval prompt, rate limit, auth error,
//! context full, startup/update menus. Hard gates are not overridden by
//! structured events.
//!
//! Known prompts are data (per-backend rule files: pattern -> single key), each
//! rule backed by a real screen fixture under `tests/fixtures/screens/`.
//!
//! Must NOT: decide busy/idle (that comes from structured events), or send
//! keys itself.
