//! One JSON-RPC connection to `codex app-server`: WebSocket text frames over
//! the unix socket, blocking I/O (gate 7 P1). Messages have no `jsonrpc`
//! field (`{id, method, params}`, `{method, params}`, `{id, result}`,
//! `{id, error}`), as the recordings show.
//!
//! [`Conn::open`] resolves the `--listen` path first (codex binds elsewhere
//! and leaves a symlink, pitfall 1). Reads time out after [`POLL`], so one
//! thread can both read and act on requests from others.
//!
//! Must NOT: retry or reconnect (the link decides).

use std::io;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};
use tungstenite::{Message, WebSocket};

use super::socket_connect_path;

/// How long one read waits before the caller gets control back.
pub const POLL: Duration = Duration::from_millis(20);
/// JSON-RPC "invalid request": codex's answer when a turn is not in the
/// state the request expects (spike S3, pitfall 2).
pub const INVALID_REQUEST: i64 = -32600;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RpcError {
    /// The server answered with an error.
    Rpc { code: i64, message: String },
    /// The connection failed or ended.
    Transport(String),
    /// No answer in time.
    Timeout(String),
}

impl RpcError {
    pub fn is_invalid_request(&self) -> bool {
        matches!(self, RpcError::Rpc { code, .. } if *code == INVALID_REQUEST)
    }
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RpcError::Rpc { code, message } => write!(f, "codex error {code}: {message}"),
            RpcError::Transport(e) => write!(f, "app-server connection: {e}"),
            RpcError::Timeout(what) => write!(f, "no answer from the app-server: {what}"),
        }
    }
}

/// One message from the server.
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    Notification {
        method: String,
        params: Value,
    },
    /// A server→client request (an approval).
    Request {
        id: Value,
        method: String,
        params: Value,
    },
    Response {
        id: i64,
        result: Result<Value, RpcError>,
    },
}

impl Incoming {
    pub fn parse(message: Value) -> Option<Incoming> {
        let method = message["method"].as_str().map(str::to_owned);
        match (message.get("id"), method) {
            (Some(id), Some(method)) => Some(Incoming::Request {
                id: id.clone(),
                method,
                params: message["params"].clone(),
            }),
            (None, Some(method)) => Some(Incoming::Notification {
                method,
                params: message["params"].clone(),
            }),
            (Some(id), None) => {
                let id = id.as_i64()?;
                let result = match message.get("error") {
                    Some(error) => Err(RpcError::Rpc {
                        code: error["code"].as_i64().unwrap_or(0),
                        message: error["message"].as_str().unwrap_or_default().to_owned(),
                    }),
                    None => Ok(message["result"].clone()),
                };
                Some(Incoming::Response { id, result })
            }
            (None, None) => None,
        }
    }
}

pub struct Conn {
    ws: WebSocket<UnixStream>,
    next_id: i64,
}

fn transport(e: impl std::fmt::Display) -> RpcError {
    RpcError::Transport(e.to_string())
}

impl Conn {
    /// Connects to the app-server listening on `listen` (resolved first).
    pub fn open(listen: &Path) -> io::Result<Conn> {
        let real = socket_connect_path(listen)?;
        let stream = UnixStream::connect(&real)?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        let (ws, _) = tungstenite::client::client("ws://localhost/", stream)
            .map_err(|e| io::Error::other(format!("websocket handshake: {e}")))?;
        ws.get_ref().set_read_timeout(Some(POLL))?;
        Ok(Conn { ws, next_id: 1 })
    }

    pub fn send(&mut self, message: &Value) -> Result<(), RpcError> {
        self.ws
            .send(Message::text(message.to_string()))
            .map_err(transport)
    }

    /// Sends a request; returns its id.
    pub fn request(&mut self, method: &str, params: Value) -> Result<i64, RpcError> {
        let id = self.next_id;
        self.next_id += 1;
        self.send(&json!({"id": id, "method": method, "params": params}))?;
        Ok(id)
    }

    /// The next message, or `None` after [`POLL`] without one.
    pub fn read_message(&mut self) -> Result<Option<Incoming>, RpcError> {
        loop {
            match self.ws.read() {
                Ok(Message::Text(text)) => {
                    let Ok(value) = serde_json::from_str::<Value>(text.as_str()) else {
                        continue;
                    };
                    if let Some(incoming) = Incoming::parse(value) {
                        return Ok(Some(incoming));
                    }
                }
                Ok(Message::Close(_)) => return Err(transport("the app-server closed it")),
                Ok(_) => {}
                Err(tungstenite::Error::Io(e))
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    return Ok(None);
                }
                Err(e) => return Err(transport(e)),
            }
        }
    }
}

impl Drop for Conn {
    fn drop(&mut self) {
        let _ = self.ws.close(None);
        let _ = self.ws.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_are_told_apart_by_id_and_method() {
        let n = Incoming::parse(json!({"method": "turn/started", "params": {"a": 1}}));
        assert!(
            matches!(n, Some(Incoming::Notification { method, .. }) if method == "turn/started")
        );
        let r = Incoming::parse(
            json!({"id": 7, "method": "item/commandExecution/requestApproval", "params": {}}),
        );
        assert!(
            matches!(r, Some(Incoming::Request { method, .. }) if method.ends_with("requestApproval"))
        );
        let ok = Incoming::parse(json!({"id": 3, "result": {"turnId": "t"}}));
        assert_eq!(
            ok,
            Some(Incoming::Response {
                id: 3,
                result: Ok(json!({"turnId": "t"}))
            })
        );
        let err = Incoming::parse(json!({"id": 4, "error": {"code": -32600, "message": "no"}}));
        let Some(Incoming::Response { result: Err(e), .. }) = err else {
            panic!("{err:?}")
        };
        assert!(e.is_invalid_request());
    }
}
