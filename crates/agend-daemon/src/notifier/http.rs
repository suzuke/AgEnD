//! Fixed Telegram origin, TLS verification, bounded calls and no mutation retry.
use super::config::Token;
use serde_json::Value;
use std::time::Duration;

pub struct Api {
    agent: ureq::Agent,
    base: String,
    token: Token,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    InvalidRequest,
    Transport,
    Status(u16),
    InvalidResponse,
    Rejected { code: u64, retry_after: Option<u64> },
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never format a URL, raw response, transport error, or remote description.
        write!(f, "Telegram request failed: {self:?}")
    }
}
impl std::error::Error for Error {}

#[derive(Clone, Copy)]
pub enum Method {
    GetMe,
    SendMessage,
    GetUpdates,
    AnswerCallbackQuery,
    DeleteMessage,
}
impl Method {
    fn name(self) -> &'static str {
        match self {
            Self::GetMe => "getMe",
            Self::SendMessage => "sendMessage",
            Self::GetUpdates => "getUpdates",
            Self::AnswerCallbackQuery => "answerCallbackQuery",
            Self::DeleteMessage => "deleteMessage",
        }
    }
}
impl Api {
    pub fn new(token: Token) -> Self {
        Self::with_origin(token, "https://api.telegram.org".into())
    }
    pub(super) fn with_origin(token: Token, base: String) -> Self {
        let config = ureq::Agent::config_builder()
            .proxy(None)
            .max_redirects(0)
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(35)))
            .max_idle_connections(0)
            .build();
        Self {
            agent: config.into(),
            base,
            token,
        }
    }
    #[cfg(test)]
    pub(crate) fn local_test(token: Token, base: String) -> Self {
        assert!(base.starts_with("http://127.0.0.1:"));
        Self::with_origin(token, base)
    }
    /// The worker runs blocking I/O off the async engine. Callers own durable
    /// attempt/cursor semantics; transport never retries a failed mutation.
    pub fn call(&self, method: Method, request: &Value) -> Result<Value, Error> {
        if !request.is_object() {
            return Err(Error::InvalidRequest);
        }
        let bytes = serde_json::to_vec(request).map_err(|_| Error::InvalidRequest)?;
        if bytes.len() > 1024 * 1024 {
            return Err(Error::InvalidRequest);
        }
        let url = format!("{}/bot{}/{}", self.base, self.token.value(), method.name());
        let mut response = self
            .agent
            .post(&url)
            .header("Content-Type", "application/json")
            .send(bytes.as_slice())
            .map_err(|_| Error::Transport)?;
        let status = response.status().as_u16();
        // Redirects are refused, including same-origin, without exposing token URLs.
        if (300..400).contains(&status) {
            return Err(Error::Status(status));
        }
        let bytes = response
            .body_mut()
            .with_config()
            .limit(1024 * 1024)
            .read_to_vec()
            .map_err(|_| Error::InvalidResponse)?;
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| Error::InvalidResponse)?;
        if value["ok"].as_bool() == Some(false) {
            return Err(Error::Rejected {
                code: value["error_code"].as_u64().ok_or(Error::InvalidResponse)?,
                retry_after: value["parameters"]["retry_after"].as_u64(),
            });
        }
        if status != 200 {
            return Err(Error::Status(status));
        }
        if value["ok"].as_bool() != Some(true) {
            return Err(Error::InvalidResponse);
        }
        value
            .get("result")
            .filter(|r| !r.is_null())
            .cloned()
            .ok_or(Error::InvalidResponse)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader, Read, Write},
        net::TcpListener,
        thread,
    };
    fn run(status: &str, body: String, headers: &str) -> Result<Value, Error> {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let status = status.to_string();
        let headers = headers.to_string();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut reader = BufReader::new(stream);
            let mut length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse::<usize>().unwrap();
                }
            }
            let mut bytes = vec![0; length];
            reader.read_exact(&mut bytes).unwrap();
            assert_eq!(
                serde_json::from_slice::<Value>(&bytes).unwrap(),
                serde_json::json!({})
            );
            write!(reader.get_mut(), "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n{body}", body.len()).unwrap();
            drop(reader);
            listener.set_nonblocking(true).unwrap();
            thread::sleep(Duration::from_millis(100));
            assert!(
                listener.accept().is_err(),
                "request was retried or redirected"
            );
        });
        let api = Api::with_origin(
            Token::parse("123:abcdefghijklmnopqrstuvwxyz_123456789".into()).unwrap(),
            origin,
        );
        let result = api.call(Method::GetMe, &serde_json::json!({}));
        server.join().unwrap();
        result
    }
    #[test]
    fn native_http_accepts_captured_telegram_response_and_rejects_redirects_and_bad_envelopes() {
        let capture = include_str!("../../tests/fixtures/telegram/get-me.json");
        assert_eq!(run("200 OK", capture.into(), "").unwrap()["is_bot"], true);
        assert_eq!(
            run(
                "302 Found",
                capture.into(),
                "Location: https://example.invalid/leak\r\n"
            ),
            Err(Error::Status(302))
        );
        assert_eq!(
            run("200 OK", "{broken".into(), ""),
            Err(Error::InvalidResponse)
        );
        assert_eq!(
            run("500 Broken", capture.into(), ""),
            Err(Error::Status(500))
        );
        let mut rejected: Value = serde_json::from_str(capture).unwrap();
        rejected["ok"] = false.into();
        rejected["error_code"] = 429.into();
        rejected["parameters"] = serde_json::json!({"retry_after": 3});
        rejected["description"] = "123:DO_NOT_ECHO_THIS_SECRET".into();
        let error = run("429 Too Many Requests", rejected.to_string(), "").unwrap_err();
        assert_eq!(
            error,
            Error::Rejected {
                code: 429,
                retry_after: Some(3)
            }
        );
        assert!(!error.to_string().contains("DO_NOT_ECHO"));
    }
}
