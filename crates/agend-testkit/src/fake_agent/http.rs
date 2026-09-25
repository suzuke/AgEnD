//! Minimal HTTP/1.1 over loopback TCP for `fake-opencode-serve` and its
//! probes: one request per connection (`Connection: close`), bodies by
//! `Content-Length`, server-sent events as chunked `data:` frames.
//!
//! Must NOT: implement anything the fakes do not use (keep-alive, TLS,
//! compression).

use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

/// A parsed request.
#[derive(Debug)]
pub struct Request {
    pub method: String,
    /// Path without the query string.
    pub path: String,
    pub query: String,
    pub body: Vec<u8>,
}

pub(crate) fn read_request(stream: &TcpStream) -> io::Result<Request> {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let (Some(method), Some(target)) = (parts.next(), parts.next()) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "bad request line",
        ));
    };
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let (method, path, query) = (method.to_owned(), path.to_owned(), query.to_owned());
    let mut length = 0usize;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header)? == 0 || header.trim().is_empty() {
            break;
        }
        if let Some((name, value)) = header.split_once(':')
            && name.trim().eq_ignore_ascii_case("content-length")
        {
            length = value.trim().parse().unwrap_or(0);
        }
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    Ok(Request {
        method,
        path,
        query,
        body,
    })
}

pub(crate) fn respond(mut stream: &TcpStream, status: u16, body: Option<&str>) -> io::Result<()> {
    let reason = match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        404 => "Not Found",
        _ => "Error",
    };
    let body = body.unwrap_or("");
    let content_type = if body.is_empty() {
        String::new()
    } else {
        "Content-Type: application/json\r\n".to_owned()
    };
    write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\n{content_type}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    stream.flush()
}

pub(crate) fn start_event_stream(mut stream: &TcpStream) -> io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nTransfer-Encoding: chunked\r\n\r\n"
    )?;
    stream.flush()
}

/// Writes one `data: <json>` event as one chunk.
pub(crate) fn send_event(mut stream: &TcpStream, json: &str) -> io::Result<()> {
    let frame = format!("data: {json}\n\n");
    write!(stream, "{:x}\r\n{frame}\r\n", frame.len())?;
    stream.flush()
}

/// Sends one request to `127.0.0.1:<port>`; returns status and body.
pub fn call(port: u16, method: &str, path: &str, body: Option<&str>) -> io::Result<(u16, String)> {
    let mut stream = TcpStream::connect(("127.0.0.1", port))?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let body = body.unwrap_or("");
    write!(
        stream,
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    let status = response
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "bad status line"))?;
    let body = response
        .split_once("\r\n\r\n")
        .map(|(_, b)| b.to_owned())
        .unwrap_or_default();
    Ok((status, body))
}

/// Reads `GET /event` as a stream of event JSON strings.
pub struct EventStream {
    reader: BufReader<TcpStream>,
}

impl EventStream {
    pub fn open(port: u16) -> io::Result<EventStream> {
        let mut stream = TcpStream::connect(("127.0.0.1", port))?;
        stream.set_read_timeout(Some(Duration::from_secs(10)))?;
        write!(
            stream,
            "GET /event HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nAccept: text/event-stream\r\n\r\n"
        )?;
        let mut reader = BufReader::new(stream);
        loop {
            let mut header = String::new();
            if reader.read_line(&mut header)? == 0 {
                return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "no headers"));
            }
            if header.trim().is_empty() {
                break;
            }
        }
        Ok(EventStream { reader })
    }

    /// The next event's `data` payload (chunk framing is skipped).
    pub fn next_event(&mut self) -> io::Result<String> {
        loop {
            let mut line = String::new();
            if self.reader.read_line(&mut line)? == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "stream closed",
                ));
            }
            if let Some(data) = line.trim_end().strip_prefix("data: ") {
                return Ok(data.to_owned());
            }
        }
    }
}
