//! Minimal synchronous client for the holder protocol: connect, `hello`, send
//! requests, read responses with a timeout. Used by the `holder_probe` example
//! and by tests. The daemon has its own client (`agend_daemon::runtime`,
//! gate 6): it does not link this crate.
//!
//! Must NOT: retry, reconnect or interpret the screen; callers decide.

use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};

use agend_core::protocol::ProtocolVersion;
use agend_core::protocol::holder::{ExitedData, HolderRequest, HolderResponse};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;

pub struct HolderClient {
    writer: UnixStream,
    reader: BufReader<UnixStream>,
    partial: Vec<u8>,
}

/// What a holder sends right after `hello`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Greeting {
    pub version: ProtocolVersion,
    pub screen: String,
    /// Present when the agent had already ended before this connection.
    pub exited: Option<ExitedData>,
}

impl HolderClient {
    /// Connects and says `hello` with `request` (normally
    /// `HolderRequest::hello()`). Does not read the reply.
    pub fn connect_with(socket: &Path, hello: &HolderRequest) -> io::Result<Self> {
        let stream = UnixStream::connect(socket)?;
        let mut client = Self {
            writer: stream.try_clone()?,
            reader: BufReader::new(stream),
            partial: Vec::new(),
        };
        client.send(hello)?;
        Ok(client)
    }

    /// Connects, says `hello`, and reads the greeting. `Exited` is looked for
    /// until `exited_wait` passes, since it only follows when the agent ended.
    pub fn connect(socket: &Path, exited_wait: Duration) -> io::Result<(Self, Greeting)> {
        let mut client = Self::connect_with(socket, &HolderRequest::hello())?;
        let version = match client.recv_required()? {
            HolderResponse::Hello { data } => data.selected,
            other => return Err(unexpected(&other)),
        };
        let screen = match client.recv_required()? {
            HolderResponse::ScreenSnapshot { data } => data.screen,
            other => return Err(unexpected(&other)),
        };
        let exited = match client.recv(exited_wait)? {
            Some(HolderResponse::Exited { data }) => Some(data),
            Some(other) => {
                client.pushback(&other);
                None
            }
            None => None,
        };
        Ok((
            client,
            Greeting {
                version,
                screen,
                exited,
            },
        ))
    }

    pub fn send(&mut self, request: &HolderRequest) -> io::Result<()> {
        let mut line = serde_json::to_vec(request).map_err(io::Error::other)?;
        line.push(b'\n');
        self.writer.write_all(&line)
    }

    /// Next response, or `None` if nothing complete arrives within `timeout`.
    /// End of stream is `UnexpectedEof`.
    pub fn recv(&mut self, timeout: Duration) -> io::Result<Option<HolderResponse>> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(end) = self.partial.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = self.partial.drain(..=end).collect();
                return serde_json::from_slice(&line)
                    .map(Some)
                    .map_err(io::Error::other);
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(None);
            }
            self.reader.get_ref().set_read_timeout(Some(left))?;
            match self.reader.read_until(b'\n', &mut self.partial) {
                Ok(0) => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "holder closed the connection",
                    ));
                }
                Ok(_) => {}
                Err(e)
                    if matches!(
                        e.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                    ) =>
                {
                    return Ok(None);
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
    }

    fn recv_required(&mut self) -> io::Result<HolderResponse> {
        self.recv(Duration::from_secs(10))?
            .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "no reply from holder"))
    }

    fn pushback(&mut self, response: &HolderResponse) {
        let mut line = serde_json::to_vec(response).expect("response serializes");
        line.push(b'\n');
        line.extend_from_slice(&self.partial);
        self.partial = line;
    }

    /// Waits until the holder closes this connection.
    pub fn wait_closed(&mut self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            match self.recv(deadline.saturating_duration_since(Instant::now())) {
                Ok(_) => {}
                Err(_) => return true,
            }
        }
        false
    }
}

fn unexpected(response: &HolderResponse) -> io::Error {
    io::Error::other(format!("unexpected holder response: {response:?}"))
}

/// Decodes a `PtyBytes` payload.
pub fn decode_bytes(bytes_base64: &str) -> io::Result<Vec<u8>> {
    BASE64.decode(bytes_base64).map_err(io::Error::other)
}

pub fn encode_bytes(bytes: &[u8]) -> String {
    BASE64.encode(bytes)
}
