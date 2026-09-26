//! Message delivery model (plan §4.4, gate 7 P5): ids, `queued -> sent ->
//! confirmed | failed`, one idempotency layer (the `messages` table,
//! `crate::store::messages`), the busy level chosen by the caller, pushes
//! carry the full content. Paths that cannot confirm delivery stay `sent`
//! (unconfirmed), never `confirmed`.
//!
//! What each state means for codex (the driver in `crate::driver::codex`):
//! - `queued`: in the DB, no reply from the backend yet (also while the
//!   app-server is unreachable, and for a backend without a driver yet);
//!   not a failure.
//! - `sent`: the backend accepted it (`turn/start`, `turn/steer`,
//!   `thread/queue/add` answered).
//! - `confirmed`: the message is a user message in the agent's thread.
//! - `failed`: the backend refused it, or the instance is `failed`.
//!
//! Must NOT: fall back to typing into a PTY, or report a parked/queued message
//! as a failure.

/// The text the agent receives: `From:` and `Task:` headers (no `Task:`
/// without a task), a blank line, then the whole body, never cut.
pub fn render(from: &str, task_id: Option<&str>, body: &str) -> String {
    match task_id {
        Some(task) => format!("From: {from}\nTask: {task}\n\n{body}"),
        None => format!("From: {from}\n\n{body}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headers_then_the_whole_body() {
        let body = "line 1\n\nline 3 ".repeat(1000);
        let text = render("dev-1", Some("T-7"), &body);
        assert_eq!(text, format!("From: dev-1\nTask: T-7\n\n{body}"));
        assert_eq!(render("operator", None, "hi"), "From: operator\n\nhi");
    }
}
