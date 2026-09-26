//! The daemon's side of one holder connection: JSON Lines over the holder's
//! unix socket (gate 4 P4). `connect` says `hello` and reads the greeting
//! (selected version, then the screen snapshot); everything after that is
//! read with [`Conn::recv`].
//!
//! Must NOT: retry or reconnect (the link decides), or hold the stream in a
//! way that outlives [`Conn`] (the link shuts it down to stop).

use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::{Duration, Instant};

use agend_core::protocol::holder::{HolderRequest, HolderResponse};

/// Longest wait for each greeting line.
const GREETING_TIMEOUT: Duration = Duration::from_secs(10);

pub struct Conn {
    writer: UnixStream,
    reader: BufReader<UnixStream>,
    partial: Vec<u8>,
}

impl Conn {
    /// Connects, says `hello`, and reads the reply and the screen snapshot.
    /// Returns the connection and the screen text.
    pub fn connect(socket: &Path) -> io::Result<(Self, String)> {
        let stream = UnixStream::connect(socket)?;
        let mut conn = Self {
            writer: stream.try_clone()?,
            reader: BufReader::new(stream),
            partial: Vec::new(),
        };
        conn.send(&HolderRequest::hello())?;
        match conn.recv_within(GREETING_TIMEOUT)? {
            HolderResponse::Hello { .. } => {}
            other => return Err(unexpected(&other)),
        }
        match conn.recv_within(GREETING_TIMEOUT)? {
            HolderResponse::ScreenSnapshot { data } => Ok((conn, data.screen)),
            other => Err(unexpected(&other)),
        }
    }

    /// A handle to the same socket, for shutting it down from another thread.
    pub fn stream(&self) -> io::Result<UnixStream> {
        self.writer.try_clone()
    }

    pub fn send(&mut self, request: &HolderRequest) -> io::Result<()> {
        let mut line = serde_json::to_vec(request).map_err(io::Error::other)?;
        line.push(b'\n');
        self.writer.write_all(&line)
    }

    /// The next response, waiting at most `timeout` (`None`: wait forever).
    /// `Ok(None)` on timeout; end of stream is `UnexpectedEof`.
    pub fn recv(&mut self, timeout: Option<Duration>) -> io::Result<Option<HolderResponse>> {
        let deadline = timeout.map(|t| Instant::now() + t);
        loop {
            if let Some(end) = self.partial.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = self.partial.drain(..=end).collect();
                return serde_json::from_slice(&line)
                    .map(Some)
                    .map_err(io::Error::other);
            }
            let left = match deadline {
                Some(deadline) => {
                    let left = deadline.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        return Ok(None);
                    }
                    Some(left)
                }
                None => None,
            };
            self.reader.get_ref().set_read_timeout(left)?;
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

    /// The next response within `timeout`, or `TimedOut`.
    pub fn recv_within(&mut self, timeout: Duration) -> io::Result<HolderResponse> {
        self.recv(Some(timeout))?
            .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "no reply from holder"))
    }
}

fn unexpected(response: &HolderResponse) -> io::Error {
    io::Error::other(format!("unexpected holder response: {response:?}"))
}
