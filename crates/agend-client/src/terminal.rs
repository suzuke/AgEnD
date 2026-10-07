//! Dedicated client 1.4 terminal connection. Writes never reconnect or replay;
//! read refusals retain their request ID. Callers run I/O off the UI thread.

use crate::{Client, ClientError, Sender, version};
use agend_core::protocol::client::{
    ClientRequest, ClientResponse, ClientTerminalControlAck, ClientTerminalControlData,
    ClientTerminalFrameData, ClientTerminalOperation, ErrorData, TerminalControlChangedData,
    TerminalSubscribeData, TerminalViewportData, V1_4, error_code,
};
use agend_core::protocol::holder::MAX_REQUEST_LINE;
use agend_core::protocol::terminal::{MAX_FRAME_LINE, TerminalFrame};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use std::io::{self, BufRead, Write};
use std::sync::TryLockError;
use std::time::{Duration, Instant};

/// Correlation and view identity remain available to the UI, including errors.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FullTerminalUpdate {
    Frame(ClientTerminalFrameData),
    ControlAck(ClientTerminalControlAck),
    ControlChanged(TerminalControlChangedData),
    Rejected(ErrorData),
}

pub(crate) fn is_terminal_request(request: &ClientRequest) -> bool {
    matches!(
        request,
        ClientRequest::SubscribeTerminalFrames { .. }
            | ClientRequest::SetTerminalViewport { .. }
            | ClientRequest::TerminalControl { .. }
    )
}

fn invalid(message: impl Into<String>) -> ClientError {
    ClientError::Daemon {
        code: error_code::INVALID_REQUEST.into(),
        message: message.into(),
    }
}

impl Sender {
    /// Sends exactly once on this connection. Accepted writes are not operation
    /// acknowledgements; the reader receives the correlated completed reply.
    /// The whole line is checked before any byte is sent, with a 5 s write
    /// deadline. A partial/failed write shuts down the connection, never replaying.
    pub fn send_terminal(&mut self, request: &ClientRequest) -> Result<(), ClientError> {
        version::check_at_least(self.selected, V1_4).map_err(ClientError::Version)?;
        if !is_terminal_request(request) {
            return Err(invalid("not a client 1.4 terminal request"));
        }
        if let ClientRequest::TerminalControl { data } = request {
            match &data.operation {
                ClientTerminalOperation::Acquire { size }
                | ClientTerminalOperation::Resize { size, .. }
                    if !size.is_valid() =>
                {
                    return Err(invalid("PTY dimensions must be between 1 and 1000"));
                }
                ClientTerminalOperation::Input { bytes_base64, .. } => {
                    // Reject a known oversized payload before decoding it.
                    if bytes_base64.len() > MAX_REQUEST_LINE {
                        return Err(invalid("terminal request exceeds 1 MiB; nothing was sent"));
                    }
                    BASE64
                        .decode(bytes_base64)
                        .map_err(|_| invalid("terminal input is not base64"))?;
                }
                _ => {}
            }
        }
        let mut encoded = BoundedLine(Vec::new());
        if serde_json::to_writer(&mut encoded, request).is_err()
            || encoded.write_all(b"\n").is_err()
        {
            return Err(invalid("terminal request exceeds 1 MiB; nothing was sent"));
        }
        let line = encoded.0;
        let deadline = Instant::now() + Duration::from_secs(5);
        let lock = self.terminal_write_lock.clone();
        let _guard = loop {
            match lock.try_lock() {
                Ok(guard) => break guard,
                Err(TryLockError::Poisoned(_)) => {
                    self.close();
                    return Err(ClientError::Disconnected(
                        "terminal writer lock is poisoned".into(),
                    ));
                }
                Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(TryLockError::WouldBlock) => {
                    self.close();
                    return Err(ClientError::Disconnected(
                        "terminal write timed out; the operation was not replayed".into(),
                    ));
                }
            }
        };
        let result = (|| {
            let mut left = line.as_slice();
            while !left.is_empty() {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "terminal write timed out",
                    ));
                }
                self.stream.set_write_timeout(Some(remaining))?;
                match self.stream.write(left) {
                    Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                    Ok(n) => left = &left[n..],
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    Err(e) => return Err(e),
                }
            }
            Ok(())
        })();
        if let Err(e) = result {
            self.close();
            return Err(ClientError::Disconnected(format!(
                "terminal write failed: {e}; input may have been partly sent and was not replayed"
            )));
        }
        Ok(())
    }

    pub fn subscribe_terminal_frames(
        &mut self,
        data: TerminalSubscribeData,
    ) -> Result<(), ClientError> {
        self.send_terminal(&ClientRequest::SubscribeTerminalFrames { data })
    }

    pub fn set_terminal_viewport(&mut self, data: TerminalViewportData) -> Result<(), ClientError> {
        self.send_terminal(&ClientRequest::SetTerminalViewport { data })
    }

    pub fn terminal_control(&mut self, data: ClientTerminalControlData) -> Result<(), ClientError> {
        self.send_terminal(&ClientRequest::TerminalControl { data })
    }
}

impl Client {
    /// Reads the dedicated full-terminal stream, bounded to 8 MiB INCLUDING
    /// newline. Malformed/oversized/partial-EOF lines close all cloned handles,
    /// so the daemon observes EOF and can release the connection's control.
    /// This does not change the legacy plaintext reader's size contract.
    pub fn next_full_terminal(&mut self) -> Result<FullTerminalUpdate, ClientError> {
        version::check_at_least(self.selected(), V1_4).map_err(ClientError::Version)?;
        if self.terminal_failed {
            return Err(ClientError::Disconnected(
                "terminal connection is closed; reconnect read-only".into(),
            ));
        }
        let _ = self.reader.get_ref().set_read_timeout(None);
        let result = (|| loop {
            let Some(response) = read_bounded(&mut self.reader)? else {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "the daemon closed the terminal connection",
                ));
            };
            match response {
                ClientResponse::TerminalFrame { data } => {
                    validate_frame(&data.frame)?;
                    return Ok(FullTerminalUpdate::Frame(data));
                }
                ClientResponse::TerminalControlAck { data } => {
                    if let Some(frame) = &data.frame {
                        validate_frame(frame)?;
                        if frame.generation != data.generation {
                            return Err(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "terminal acknowledgement generation does not match its frame",
                            ));
                        }
                    }
                    return Ok(FullTerminalUpdate::ControlAck(data));
                }
                ClientResponse::TerminalControlChanged { data } => {
                    return Ok(FullTerminalUpdate::ControlChanged(data));
                }
                ClientResponse::Error { data } => return Ok(FullTerminalUpdate::Rejected(data)),
                _ => {}
            }
        })();
        result.map_err(|e| {
            self.terminal_failed = true;
            crate::connection::shutdown(self.reader.get_ref());
            ClientError::Disconnected(e.to_string())
        })
    }
}

/// A paste remains one operation, even when it contains local exit key bytes.
pub fn input_operation(attach_id: &str, bytes: &[u8]) -> ClientTerminalOperation {
    ClientTerminalOperation::Input {
        attach_id: attach_id.into(),
        bytes_base64: BASE64.encode(bytes),
    }
}

fn validate_frame(frame: &TerminalFrame) -> io::Result<()> {
    if !frame.size.is_valid()
        || frame.generation.is_empty()
        || frame.cells.is_empty()
        || frame.cells.len() > usize::from(frame.size.rows)
        || frame
            .cells
            .iter()
            .any(|row| row.len() != usize::from(frame.size.columns))
        || (frame.cursor.visible
            && (usize::from(frame.cursor.row) >= frame.cells.len()
                || frame.cursor.column >= frame.size.columns))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid terminal frame dimensions, generation or cursor",
        ));
    }
    Ok(())
}

fn read_bounded(reader: &mut impl BufRead) -> io::Result<Option<ClientResponse>> {
    let mut line = Vec::new();
    loop {
        let chunk = reader.fill_buf()?;
        if chunk.is_empty() {
            return if line.is_empty() {
                Ok(None)
            } else {
                Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "incomplete terminal protocol line",
                ))
            };
        }
        let newline = chunk.iter().position(|&byte| byte == b'\n');
        let count = newline.map_or(chunk.len(), |n| n + 1);
        if count > MAX_FRAME_LINE - line.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "terminal frame line exceeds 8 MiB; nothing was truncated",
            ));
        }
        line.extend_from_slice(&chunk[..count]);
        reader.consume(count);
        if newline.is_some() {
            if line.iter().all(u8::is_ascii_whitespace) {
                line.clear();
                continue;
            }
            return decode_response(&line).map(Some).map_err(|e| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("invalid terminal protocol line: {e}"),
                )
            });
        }
    }
}

struct BoundedLine(Vec<u8>);
impl Write for BoundedLine {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > MAX_REQUEST_LINE - self.0.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "terminal request exceeds 1 MiB",
            ));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn decode_response(line: &[u8]) -> serde_json::Result<ClientResponse> {
    serde_json::from_slice(line)
}

#[cfg(test)]
mod decode_contract {
    use super::decode_response;
    #[test]
    fn producer_golden_accepts_both_orders_and_rejects_ambiguous_frames() {
        // This golden is generated and checked by the real holder parser test.
        let golden: serde_json::Value = serde_json::from_str(include_str!(
            "../../agend-holder/tests/golden/terminal-frame-1.4.json"
        ))
        .unwrap();
        let native = serde_json::to_string(&golden["client"]).unwrap();
        let expected = decode_response(native.as_bytes()).unwrap();
        let data = serde_json::to_string(&golden["client"]["data"]).unwrap();
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
            "../../agend-holder/tests/golden/terminal-frame-1.4.json"
        ))
        .unwrap();
        let native = serde_json::to_string(&golden["client"]).unwrap();
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
