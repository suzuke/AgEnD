//! Sending one message to a codex thread with the caller's busy level
//! (gate 7 P6). The driver's own view of busy/idle (from
//! `thread/status/changed`, never debounced) picks the method:
//!
//! | Thread | Level | Method |
//! |---|---|---|
//! | idle | any | `turn/start` |
//! | busy | `Queue` | `thread/queue/add`; if the thread is already idle when the reply comes, `thread/queue/start` once (an `invalid request` answer means codex started it itself: fine; owner-approved exception, P6) |
//! | busy | `Steer` | `turn/steer {expectedTurnId}`; `invalid request` (the turn just ended) → `turn/start` once |
//! | busy | `Interrupt` | `turn/interrupt`, wait at most [`INTERRUPT_WAIT`] for that turn to end, then `turn/start` (a busy `turn/start` joins the running turn: at worst a steer, spike S3) |
//!
//! Every send carries `clientUserMessageId` = the message id (U2: whether
//! `turn/start` and `turn/steer` keep it is not verified).
//!
//! Must NOT: call `thread/queue/start` except in the one case above, or
//! retry in a loop.

use std::time::Duration;

use agend_core::policy::busy::BusyLevel;
use serde_json::{Value, json};

use super::rpc::RpcError;

/// Longest wait for an interrupted turn to end before the `turn/start`.
pub const INTERRUPT_WAIT: Duration = Duration::from_secs(5);

/// What a send goes through: a connection plus the driver's view of it.
pub trait Rpc {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, RpcError>;
    /// Busy as the driver sees it now.
    fn busy(&self) -> bool;
    /// The running turn, when known.
    fn active_turn(&self) -> Option<String>;
    /// Reads until `turn` is no longer the running turn or `within`
    /// passed; true when it ended.
    fn wait_turn_end(&mut self, turn: &str, within: Duration) -> bool;
}

/// What a send did: the turn it went into (none for a queued message) and
/// the methods called, for the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub turn_id: Option<String>,
    pub via: Vec<String>,
}

fn input(text: &str) -> Value {
    json!([{"type": "text", "text": text, "text_elements": []}])
}

fn turn_start(
    rpc: &mut dyn Rpc,
    thread: &str,
    text: &str,
    id: &str,
    mut via: Vec<String>,
) -> Result<Outcome, RpcError> {
    let result = rpc.call(
        "turn/start",
        json!({"threadId": thread, "input": input(text), "clientUserMessageId": id}),
    )?;
    via.push("turn/start".into());
    Ok(Outcome {
        turn_id: result["turn"]["id"].as_str().map(str::to_owned),
        via,
    })
}

/// Sends `text` (message `id`) to `thread` at `level`.
pub fn send(
    rpc: &mut dyn Rpc,
    thread: &str,
    text: &str,
    id: &str,
    level: BusyLevel,
) -> Result<Outcome, RpcError> {
    let active = rpc.active_turn();
    let (true, Some(turn)) = (rpc.busy(), active) else {
        return turn_start(rpc, thread, text, id, Vec::new());
    };
    match level {
        BusyLevel::Queue => {
            rpc.call(
                "thread/queue/add",
                json!({"threadId": thread, "clientUserMessageId": id, "input": input(text)}),
            )?;
            let mut via = vec!["thread/queue/add".to_owned()];
            if !rpc.busy() {
                match rpc.call("thread/queue/start", json!({"threadId": thread})) {
                    Ok(_) => via.push("thread/queue/start".into()),
                    Err(e) if e.is_invalid_request() => {
                        via.push("thread/queue/start (already started)".into())
                    }
                    Err(e @ RpcError::Rpc { .. }) => {
                        via.push(format!("thread/queue/start failed: {e}"))
                    }
                    Err(e) => return Err(e),
                }
            }
            Ok(Outcome { turn_id: None, via })
        }
        BusyLevel::Steer => {
            let steered = rpc.call(
                "turn/steer",
                json!({"threadId": thread, "expectedTurnId": turn, "input": input(text),
                       "clientUserMessageId": id}),
            );
            match steered {
                Ok(result) => Ok(Outcome {
                    turn_id: result["turnId"].as_str().map(str::to_owned).or(Some(turn)),
                    via: vec!["turn/steer".into()],
                }),
                Err(e) if e.is_invalid_request() => turn_start(
                    rpc,
                    thread,
                    text,
                    id,
                    vec!["turn/steer (turn ended)".into()],
                ),
                Err(e) => Err(e),
            }
        }
        BusyLevel::Interrupt => {
            let mut via = vec!["turn/interrupt".to_owned()];
            match rpc.call(
                "turn/interrupt",
                json!({"threadId": thread, "turnId": turn}),
            ) {
                Ok(_) => {}
                Err(e) if e.is_invalid_request() => via[0].push_str(" (turn ended)"),
                Err(e) => return Err(e),
            }
            if !rpc.wait_turn_end(&turn, INTERRUPT_WAIT) {
                via.push("(no turn/completed within 5 s)".into());
            }
            turn_start(rpc, thread, text, id, via)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    /// A scripted connection for the races (they cannot be made to happen
    /// on cue against the fake app-server): answers in order, and flips its
    /// busy view when told to after a call.
    struct Script {
        busy: bool,
        active: Option<String>,
        answers: VecDeque<(Result<Value, RpcError>, Option<bool>)>,
        calls: Vec<String>,
    }

    impl Rpc for Script {
        fn call(&mut self, method: &str, _: Value) -> Result<Value, RpcError> {
            self.calls.push(method.into());
            let (answer, busy_after) = self.answers.pop_front().expect("scripted answer");
            if let Some(busy) = busy_after {
                self.busy = busy;
            }
            answer
        }
        fn busy(&self) -> bool {
            self.busy
        }
        fn active_turn(&self) -> Option<String> {
            self.active.clone()
        }
        fn wait_turn_end(&mut self, _: &str, _: Duration) -> bool {
            true
        }
    }

    fn invalid() -> RpcError {
        RpcError::Rpc {
            code: -32600,
            message: "thread already has an active or pending turn".into(),
        }
    }

    fn busy(answers: Vec<(Result<Value, RpcError>, Option<bool>)>) -> Script {
        Script {
            busy: true,
            active: Some("t-A".into()),
            answers: answers.into(),
            calls: Vec::new(),
        }
    }

    /// Race 1 (P6): the turn ended while `thread/queue/add` was on its way;
    /// the reply finds the thread idle, so `thread/queue/start` once, and
    /// "already has an active or pending turn" counts as success.
    #[test]
    fn queue_reply_on_an_idle_thread_starts_the_queue_once() {
        let mut rpc = busy(vec![
            (Ok(json!({"queuedSubmission": {"id": "q"}})), Some(false)),
            (Err(invalid()), None),
        ]);
        let out = send(&mut rpc, "th", "x", "m-q", BusyLevel::Queue).unwrap();
        assert_eq!(rpc.calls, ["thread/queue/add", "thread/queue/start"]);
        assert_eq!(out.turn_id, None);
        let mut rpc = busy(vec![(Ok(json!({"queuedSubmission": {"id": "q"}})), None)]);
        send(&mut rpc, "th", "x", "m-q", BusyLevel::Queue).unwrap();
        assert_eq!(
            rpc.calls,
            ["thread/queue/add"],
            "still busy: never queue/start"
        );
    }

    /// Race 2 (P6): the turn to steer just ended (`invalid request`): one
    /// `turn/start` instead.
    #[test]
    fn steer_into_an_ended_turn_becomes_one_turn_start() {
        let mut rpc = busy(vec![
            (Err(invalid()), None),
            (Ok(json!({"turn": {"id": "t-B"}})), None),
        ]);
        let out = send(&mut rpc, "th", "x", "m-s", BusyLevel::Steer).unwrap();
        assert_eq!(rpc.calls, ["turn/steer", "turn/start"]);
        assert_eq!(out.turn_id.as_deref(), Some("t-B"));
        let mut rpc = busy(vec![(
            Err(RpcError::Rpc {
                code: -32000,
                message: "other".into(),
            }),
            None,
        )]);
        assert!(send(&mut rpc, "th", "x", "m-s", BusyLevel::Steer).is_err());
        assert_eq!(rpc.calls, ["turn/steer"], "other errors are not retried");
    }

    #[test]
    fn idle_is_always_turn_start() {
        for level in [BusyLevel::Queue, BusyLevel::Steer, BusyLevel::Interrupt] {
            let mut rpc = busy(vec![(Ok(json!({"turn": {"id": "t-1"}})), None)]);
            rpc.busy = false;
            send(&mut rpc, "th", "x", "m", level).unwrap();
            assert_eq!(rpc.calls, ["turn/start"], "{level:?}");
        }
    }
}
