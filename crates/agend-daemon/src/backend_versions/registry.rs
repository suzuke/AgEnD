//! Read-only release discovery. Fixed TLS origin; no executable or credentials.
use agend_core::{
    model::Backend,
    setup::backend::{PublishedBackend, npm_package, valid_version},
};
use std::time::Duration;

const ORIGIN: &str = "https://registry.npmjs.org";
const MAX_BYTES: u64 = 256 * 1024;

/// Blocking I/O; daemon workers must invoke this outside the async engine.
pub fn latest(backend: Backend) -> Result<PublishedBackend, String> {
    fetch(backend, ORIGIN, Duration::from_secs(5))
}

pub(super) fn fetch(
    backend: Backend,
    origin: &str,
    within: Duration,
) -> Result<PublishedBackend, String> {
    let package = npm_package(backend);
    let config = ureq::Agent::config_builder()
        .proxy(None)
        .max_redirects(0)
        .http_status_as_error(false)
        .timeout_global(Some(within))
        .max_idle_connections(0)
        .build();
    let agent: ureq::Agent = config.into();
    let url = format!("{}/{}/latest", origin, package.replace('/', "%2f"));
    let mut response = agent
        .get(&url)
        .header("Accept", "application/json")
        .call()
        .map_err(|_| "backend registry request failed")?;
    if response.status().as_u16() != 200 {
        return Err(format!(
            "backend registry returned HTTP {}",
            response.status().as_u16()
        ));
    }
    let bytes = response
        .body_mut()
        .with_config()
        .limit(MAX_BYTES)
        .read_to_vec()
        .map_err(|_| "backend registry response unreadable or exceeds 256 KiB")?;
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|_| "backend registry response is not valid JSON")?;
    if value["name"].as_str() != Some(package) {
        return Err("backend registry package identity mismatch".into());
    }
    let version = value["version"]
        .as_str()
        .filter(|version| valid_version(version))
        .ok_or("backend registry response has no valid version")?;
    Ok(PublishedBackend {
        backend: backend.as_str().into(),
        package: package.into(),
        version: version.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
        time::Instant,
    };

    fn captured(backend: Backend) -> &'static [u8] {
        match backend {
            Backend::Claude => include_bytes!("../../tests/fixtures/backend_registry/claude.json"),
            Backend::Codex => include_bytes!("../../tests/fixtures/backend_registry/codex.json"),
            Backend::Opencode => {
                include_bytes!("../../tests/fixtures/backend_registry/opencode.json")
            }
        }
    }

    // Replay actual registry bodies through native HTTP, not a mocked transport.
    fn serve(status: &str, body: Vec<u8>, delay: Duration) -> (String, thread::JoinHandle<String>) {
        serve_paused(status, body, delay, false)
    }

    fn serve_paused(
        status: &str,
        body: Vec<u8>,
        delay: Duration,
        during_body: bool,
    ) -> (String, thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let status = status.to_owned();
        let handle = thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = Vec::new();
            let mut byte = [0];
            while !request.ends_with(b"\r\n\r\n") {
                socket.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
                assert!(request.len() < 8192);
            }
            if !during_body {
                thread::sleep(delay);
            }
            let header = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Type: application/json\r\nLocation: http://127.0.0.1:1/forbidden\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = socket.write_all(header.as_bytes());
            let middle = body.len() / 2;
            let _ = socket.write_all(&body[..middle]);
            if during_body {
                thread::sleep(delay);
            }
            let _ = socket.write_all(&body[middle..]);
            String::from_utf8(request).unwrap()
        });
        (origin, handle)
    }

    #[test]
    fn native_registry_binds_packages_and_only_reads_latest_metadata() {
        for backend in Backend::ALL {
            let body = captured(backend).to_vec();
            let manifest: serde_json::Value = serde_json::from_slice(&body).unwrap();
            let (origin, server) = serve("200 OK", body, Duration::ZERO);
            let result = fetch(backend, &origin, Duration::from_secs(2)).unwrap();
            assert_eq!(Some(result.version.as_str()), manifest["version"].as_str());
            assert_eq!(result.backend, backend.as_str());
            let request = server.join().unwrap();
            assert!(request.starts_with(&format!(
                "GET /{}/latest HTTP/1.1\r\n",
                npm_package(backend).replace('/', "%2f")
            )));
            assert!(!request.to_ascii_lowercase().contains("authorization:"));
        }
    }

    #[test]
    fn redirect_error_malformed_wrong_package_and_oversize_are_refused() {
        let native: serde_json::Value = serde_json::from_slice(captured(Backend::Codex)).unwrap();
        let mut wrong_name = native.clone();
        wrong_name["name"] = "wrong".into();
        let mut wrong_version = native;
        wrong_version["version"] = "../unsafe".into();
        for (status, body, message) in [
            ("302 Found", b"{}".to_vec(), "HTTP 302"),
            ("503 Unavailable", b"do-not-echo".to_vec(), "HTTP 503"),
            ("200 OK", b"do-not-echo".to_vec(), "valid JSON"),
            (
                "200 OK",
                serde_json::to_vec(&wrong_name).unwrap(),
                "identity",
            ),
            (
                "200 OK",
                serde_json::to_vec(&wrong_version).unwrap(),
                "valid version",
            ),
            ("200 OK", vec![b' '; MAX_BYTES as usize + 1], "256 KiB"),
        ] {
            let (origin, server) = serve(status, body, Duration::ZERO);
            let error = fetch(Backend::Codex, &origin, Duration::from_secs(2)).unwrap_err();
            assert!(error.contains(message), "{error}");
            assert!(!error.contains("do-not-echo"));
            server.join().unwrap();
        }
    }

    #[test]
    fn native_registry_deadline_is_bounded() {
        let (origin, server) = serve(
            "200 OK",
            captured(Backend::Codex).to_vec(),
            Duration::from_millis(500),
        );
        let start = Instant::now();
        let error = fetch(Backend::Codex, &origin, Duration::from_millis(100)).unwrap_err();
        assert!(error.contains("request failed"), "{error}");
        assert!(start.elapsed() < Duration::from_secs(2));
        server.join().unwrap();
    }
    #[test]
    fn native_registry_body_stall_shares_the_deadline() {
        let (origin, server) = serve_paused(
            "200 OK",
            captured(Backend::Codex).to_vec(),
            Duration::from_millis(500),
            true,
        );
        let start = Instant::now();
        let error = fetch(Backend::Codex, &origin, Duration::from_millis(100)).unwrap_err();
        assert!(error.contains("response unreadable"), "{error}");
        assert!(start.elapsed() < Duration::from_secs(2));
        server.join().unwrap();
    }
}
