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

use agend_core::protocol::ProtocolVersion;
use agend_core::protocol::holder::{HolderRequest, HolderResponse, V1};
use agend_core::protocol::terminal::MAX_FRAME_LINE;

mod reader;
use reader::SocketReader;

/// Longest wait for each greeting line.
const GREETING_TIMEOUT: Duration = Duration::from_secs(10);

pub struct Conn {
    writer: UnixStream,
    reader: BufReader<SocketReader>,
    partial: Vec<u8>,
    pub version: ProtocolVersion,
}

impl Conn {
    /// Connects, says `hello`, and reads the reply and the screen snapshot.
    /// Returns the connection and the screen text.
    pub fn connect(socket: &Path) -> io::Result<(Self, String)> {
        let stream = UnixStream::connect(socket)?;
        let mut conn = Self {
            writer: stream.try_clone()?,
            reader: BufReader::new(SocketReader::new(stream)),
            partial: Vec::new(),
            version: V1,
        };
        conn.send(&HolderRequest::hello())?;
        match conn.recv_within(GREETING_TIMEOUT)? {
            HolderResponse::Hello { data }
                if data.selected.major == 1
                    && data.selected <= agend_core::protocol::holder::V1_2 =>
            {
                conn.version = data.selected;
            }
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
        self.reader.get_mut().deadline = deadline;
        loop {
            if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                return Ok(None);
            }
            match self.reader.fill_buf() {
                Ok([]) => {
                    return Err(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "holder closed the connection",
                    ));
                }
                Ok(chunk) => {
                    // Search only the new chunk. Rescanning the accumulated
                    // frame for each chunk makes an 8 MiB line quadratic.
                    let end = chunk.iter().position(|&byte| byte == b'\n');
                    let take = end.map_or(chunk.len(), |end| end + 1);
                    if self.partial.len() + take > MAX_FRAME_LINE {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "holder response exceeds 8 MiB",
                        ));
                    }
                    self.partial.extend_from_slice(&chunk[..take]);
                    self.reader.consume(take);
                    if end.is_some() {
                        let line = std::mem::take(&mut self.partial);
                        return decode_response(&line).map(Some).map_err(io::Error::other);
                    }
                }
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

fn decode_response(line: &[u8]) -> serde_json::Result<HolderResponse> {
    serde_json::from_slice(line)
}

#[cfg(test)]
mod tests {
    use super::*;
    use agend_core::protocol::terminal::TerminalOperationError;

    fn response_line(len: usize) -> Vec<u8> {
        let response = HolderResponse::TerminalOperationError {
            data: TerminalOperationError {
                request_id: "boundary".into(),
                code: "probe".into(),
                message: String::new(),
            },
        };
        let overhead = serde_json::to_vec(&response).unwrap().len() + 1;
        let HolderResponse::TerminalOperationError { mut data } = response else {
            unreachable!()
        };
        data.message = "x".repeat(len - overhead);
        let mut line =
            serde_json::to_vec(&HolderResponse::TerminalOperationError { data }).unwrap();
        line.push(b'\n');
        assert_eq!(line.len(), len);
        line
    }

    fn receive(line: Vec<u8>) -> io::Result<Option<HolderResponse>> {
        let (reader, mut peer) = UnixStream::pair().unwrap();
        let mut conn = Conn {
            writer: reader.try_clone().unwrap(),
            reader: BufReader::new(SocketReader::new(reader)),
            partial: Vec::new(),
            version: V1,
        };
        let writer = std::thread::spawn(move || {
            let _ = peer.write_all(&line);
        });
        let result = conn.recv(Some(Duration::from_secs(10)));
        // A rejected overlong producer may still be writing; close the owned
        // test connection before joining its producer.
        drop(conn);
        writer.join().unwrap();
        result
    }

    #[test]
    fn holder_response_limit_includes_the_newline_and_rejects_the_whole_line() {
        let response = receive(response_line(MAX_FRAME_LINE)).unwrap().unwrap();
        assert!(
            matches!(response, HolderResponse::TerminalOperationError { data } if data.request_id == "boundary")
        );
        let error = receive(response_line(MAX_FRAME_LINE + 1)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("exceeds 8 MiB"));
    }
    #[test]
    fn a_timed_out_partial_line_is_preserved_and_eof_drains_its_remaining_bytes() {
        let line = response_line(20_000);
        let (reader, mut peer) = UnixStream::pair().unwrap();
        let mut conn = Conn {
            writer: reader.try_clone().unwrap(),
            reader: BufReader::new(SocketReader::new(reader)),
            partial: Vec::new(),
            version: V1,
        };
        let (release, wait) = std::sync::mpsc::channel();
        let writer = std::thread::spawn(move || {
            peer.write_all(&line[..13_000]).unwrap();
            wait.recv_timeout(Duration::from_secs(10)).unwrap();
            peer.write_all(&line[13_000..]).unwrap();
        });
        let deadline = Instant::now() + Duration::from_secs(10);
        while conn.partial.len() < 13_000 {
            assert!(
                conn.recv(Some(Duration::from_millis(50)))
                    .unwrap()
                    .is_none()
            );
            assert!(Instant::now() < deadline, "partial producer stalled");
        }
        assert_eq!(conn.partial.len(), 13_000);
        release.send(()).unwrap();
        let response = conn.recv(Some(Duration::from_secs(10))).unwrap().unwrap();
        assert!(
            matches!(response, HolderResponse::TerminalOperationError { data } if data.request_id == "boundary")
        );
        writer.join().unwrap();
        assert_eq!(
            conn.recv(Some(Duration::from_secs(1))).unwrap_err().kind(),
            io::ErrorKind::UnexpectedEof
        );
    }
}

#[cfg(test)]
mod decode_contract {
    use super::decode_response;
    #[test]
    fn producer_golden_accepts_both_orders_and_rejects_ambiguous_frames() {
        // This golden is generated and checked by the real holder parser test.
        let golden: serde_json::Value = serde_json::from_str(include_str!(
            "../../../agend-holder/tests/golden/terminal-frame-1.4.json"
        ))
        .unwrap();
        let native = serde_json::to_string(&golden["holder"]).unwrap();
        let expected = decode_response(native.as_bytes()).unwrap();
        let data = serde_json::to_string(&golden["holder"]["data"]).unwrap();
        let first = format!(r#"{{"type":"terminal_frame","data":{data}}}"#);
        assert_eq!(decode_response(first.as_bytes()).unwrap(), expected);
        let cases = [
            format!(r#"{{"type":"terminal_frame","type":"terminal_frame","data":{data}}}"#),
            format!(r#"{{"type":"terminal_frame","data":{data},"data":{data}}}"#),
            format!("{first} {{}}"),
            first.replacen(
                r#""request_id":"#,
                r#""request_id":"duplicate","request_id":"#,
                1,
            ),
            first[..first.len() - 1].to_string(),
        ];
        for (index, line) in cases.iter().enumerate() {
            assert!(
                decode_response(line.as_bytes()).is_err(),
                "accepted adversary {index}"
            );
        }
    }
    #[test]
    fn unknown_values_keep_the_original_string_number_and_depth_validation() {
        let golden: serde_json::Value = serde_json::from_str(include_str!(
            "../../../agend-holder/tests/golden/terminal-frame-1.4.json"
        ))
        .unwrap();
        let native = serde_json::to_string(&golden["holder"]).unwrap();
        for value in [
            r#""\ud800""#.to_owned(),
            "1e999".to_owned(),
            format!("{}null{}", "[".repeat(200), "]".repeat(200)),
        ] {
            for place in ["envelope", "data", "frame"] {
                let line = if place == "envelope" {
                    format!(
                        "{},\"fresh_unknown\":{value}}}",
                        &native[..native.len() - 1]
                    )
                } else {
                    let marker = format!("\"{place}\":{{");
                    native.replacen(&marker, &format!("{marker}\"fresh_unknown\":{value},"), 1)
                };
                assert!(
                    decode_response(line.as_bytes()).is_err(),
                    "accepted invalid unknown at {place}: {value}"
                );
            }
        }
        let extension = format!(
            "{},\"fresh_unknown\":\"valid extension\"}}",
            &native[..native.len() - 1]
        );
        assert_eq!(
            decode_response(extension.as_bytes()).unwrap(),
            decode_response(native.as_bytes()).unwrap()
        );
    }
}
