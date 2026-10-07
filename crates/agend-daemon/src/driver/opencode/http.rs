//! Bounded loopback transport. Mutations are issued once, never replayed after
//! an ambiguous response. Redirects and ambient proxies are disabled.
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::Value;
use std::time::Duration;

pub const MAX_JSON_BYTES: u64 = 16 * 1024 * 1024;

/// Credentials deliberately do not implement Debug.
pub struct Http {
    agent: ureq::Agent,
    base: String,
    authorization: String,
    directory: String,
}

#[derive(Debug)]
pub enum Error {
    InvalidRequest,
    Transport(String),
    Status(u16),
    InvalidJson,
    TooLarge,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidRequest => f.write_str("invalid OpenCode request"),
            Self::Transport(error) => write!(f, "OpenCode transport: {error}"),
            Self::Status(status) => write!(f, "OpenCode HTTP status {status}"),
            Self::TooLarge => f.write_str("OpenCode response exceeds size limit"),
            Self::InvalidJson => f.write_str("invalid OpenCode JSON response"),
        }
    }
}
impl std::error::Error for Error {}

impl Http {
    pub fn new(port: u16, password: &str, directory: &str) -> Result<Self, Error> {
        if port == 0 || password.is_empty() || directory.is_empty() {
            return Err(Error::InvalidRequest);
        }
        let config = ureq::Agent::config_builder()
            .proxy(None)
            .max_redirects(0)
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(5)))
            .max_idle_connections(0)
            .build();
        Ok(Self {
            agent: config.into(),
            base: format!("http://127.0.0.1:{port}"),
            authorization: format!("Basic {}", STANDARD.encode(format!("agend:{password}"))),
            directory: directory.to_owned(),
        })
    }

    fn url(&self, path: &str) -> Result<String, Error> {
        // Only API paths assembled from validated backend ids enter here.
        if !path.starts_with('/')
            || path.starts_with("//")
            || path.contains("..")
            || !path
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"/_-".contains(&b))
        {
            return Err(Error::InvalidRequest);
        }
        Ok(format!("{}{path}", self.base))
    }

    pub fn get(&self, path: &str) -> Result<Value, Error> {
        let response = self
            .agent
            .get(self.url(path)?)
            .query("directory", &self.directory)
            .header("Authorization", &self.authorization)
            .call()
            .map_err(|error| Error::Transport(error.to_string()))?;
        decode(response)
    }

    /// Follow only the opaque cursor, never a server-provided Link URL.
    pub fn page(
        &self,
        path: &str,
        limit: usize,
        before: Option<&str>,
    ) -> Result<(Value, Option<String>), Error> {
        if !(1..=64).contains(&limit) || before.is_some_and(|c| !valid_cursor(c)) {
            return Err(Error::InvalidRequest);
        }
        let mut request = self
            .agent
            .get(self.url(path)?)
            .query("directory", &self.directory)
            .query("limit", limit.to_string())
            .header("Authorization", &self.authorization);
        if let Some(cursor) = before {
            request = request.query("before", cursor);
        }
        let response = request
            .call()
            .map_err(|e| Error::Transport(e.to_string()))?;
        let next = response
            .headers()
            .get("x-next-cursor")
            .map(|h| h.to_str().map(str::to_owned))
            .transpose()
            .map_err(|_| Error::InvalidJson)?;
        if next
            .as_deref()
            .is_some_and(|c| !valid_cursor(c) || Some(c) == before)
        {
            return Err(Error::InvalidJson);
        }
        Ok((decode(response)?, next))
    }

    pub fn post(&self, path: &str, body: &Value) -> Result<Value, Error> {
        let url = self.url(path)?;
        let encoded = serde_json::to_vec(body).map_err(|_| Error::InvalidRequest)?;
        if encoded.len() as u64 > MAX_JSON_BYTES {
            return Err(Error::InvalidRequest);
        }
        let response = self
            .agent
            .post(url)
            .query("directory", &self.directory)
            .header("Authorization", &self.authorization)
            .header("Content-Type", "application/json")
            .send(encoded)
            .map_err(|error| Error::Transport(error.to_string()))?;
        decode(response)
    }
}

fn valid_cursor(cursor: &str) -> bool {
    !cursor.is_empty()
        && cursor.len() <= 1024
        && cursor
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-=".contains(&b))
}

fn decode(mut response: ureq::http::Response<ureq::Body>) -> Result<Value, Error> {
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        // Backend error bodies may contain prompts or credentials.
        return Err(Error::Status(status));
    }
    if response
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .is_some_and(|n| n > MAX_JSON_BYTES)
    {
        return Err(Error::TooLarge);
    }
    let bytes = response
        .body_mut()
        .with_config()
        .limit(MAX_JSON_BYTES)
        .read_to_vec()
        .map_err(|error| match error {
            ureq::Error::BodyExceedsLimit(_) => Error::TooLarge,
            other => Error::Transport(other.to_string()),
        })?;
    if status == 204 && bytes.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_slice(&bytes).map_err(|_| Error::InvalidJson)
}

#[cfg(test)]
mod tests {
    use super::*;
    use agend_testkit::fake_agent::opencode::Server;
    use serde_json::json;

    #[test]
    fn chunked_oversized_json_reports_the_same_limit_as_content_length() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let producer = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            socket
                .set_write_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut headers = Vec::new();
            let mut byte = [0];
            while !headers.ends_with(b"\r\n\r\n") {
                socket.read_exact(&mut byte).unwrap();
                headers.push(byte[0]);
            }
            let body = serde_json::to_vec(&"x".repeat(MAX_JSON_BYTES as usize)).unwrap();
            write!(socket, "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n", body.len()).unwrap();
            // The bounded reader may close before the producer finishes.
            let _ = socket.write_all(&body);
            let _ = socket.write_all(b"\r\n0\r\n\r\n");
        });
        let result = Http::new(port, "fixture", "/fixture")
            .unwrap()
            .get("/global/health");
        producer.join().unwrap();
        assert!(matches!(result, Err(Error::TooLarge)), "{result:?}");
    }

    #[test]
    fn native_producer_supports_session_prompt_history_and_abort() {
        let server = Server::start(0, Duration::from_secs(60), None).unwrap();
        let http = Http::new(server.port(), "fixture", "/fixture").unwrap();
        assert_eq!(http.get("/global/health").unwrap()["healthy"], true);
        let session = http.post("/session", &json!({})).unwrap();
        let id = session["id"].as_str().unwrap();
        assert_eq!(
            http.post(
                &format!("/session/{id}/prompt_async"),
                &json!({"parts":[{"type":"text","text":"transport fixture"}]})
            )
            .unwrap(),
            Value::Null
        );
        let history = http.get(&format!("/session/{id}/message")).unwrap();
        assert_eq!(history[0]["info"]["role"], "user");
        assert_eq!(history[0]["parts"][0]["text"], "transport fixture");
        assert_eq!(
            http.post(&format!("/session/{id}/abort"), &json!({}))
                .unwrap(),
            true
        );
        assert_eq!(http.get("/permission").unwrap(), json!([]));
        assert!(matches!(
            http.get("/session/ses_missing"),
            Err(Error::Status(404))
        ));
    }
}
