//! claude driver (D16): interactive TUI + hooks. State from hooks; queue via
//! the Stop hook `decision: block` (consume the queue before emitting;
//! `stop_hook_active` guards loops); idle delivery via the channel; interrupt =
//! `Esc`, then send via the channel immediately (no Stop fires after Esc, so do
//! not wait for one). The project CLAUDE.md must say agend channel messages
//! come from the user's own team.
//!
//! Must NOT: wait for a Stop hook after `Esc`.
