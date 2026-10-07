//! Bounded GitHub CLI transport. Credentials stay in the daemon environment.
use crate::runner::{ProcessRunner, quote};
use agend_core::traits::Runner;
use serde_json::Value;
use std::collections::BTreeMap;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct Api<R = ProcessRunner> {
    pub executable: PathBuf,
    pub runner: R,
    pub directory: PathBuf,
}

#[derive(Debug)]
pub struct Response {
    pub status: u16,
    pub value: Value,
}

impl Api {
    pub fn discover(home: &Path, directory: &Path) -> Result<Self, String> {
        let daemon = std::env::vars().collect::<BTreeMap<_, _>>();
        let path = crate::runtime::env::launch_path(home, &daemon);
        let shim = home.join("bin").canonicalize().ok();
        let own = std::env::current_exe()
            .ok()
            .and_then(|p| p.metadata().ok())
            .map(|m| (m.dev(), m.ino()));
        let executable = path
            .split(':')
            .filter_map(|dir| {
                if !Path::new(dir).is_absolute() {
                    return None;
                }
                let candidate = Path::new(dir).join("gh");
                let actual = candidate.canonicalize().ok()?;
                let metadata = actual.metadata().ok()?;
                if candidate.parent().and_then(|p| p.canonicalize().ok()) == shim
                    || Some((metadata.dev(), metadata.ino())) == own
                    || metadata.mode() & 0o111 == 0
                {
                    return None;
                }
                actual.is_file().then_some(actual)
            })
            .next()
            .ok_or("real gh not found outside AGEND_HOME/bin")?;
        let mut env = BTreeMap::from([
            ("PATH".into(), path),
            ("LANG".into(), "C".into()),
            ("GH_PROMPT_DISABLED".into(), "1".into()),
            ("GH_NO_UPDATE_NOTIFIER".into(), "1".into()),
            ("NO_COLOR".into(), "1".into()),
        ]);
        for key in [
            "HOME",
            "XDG_CONFIG_HOME",
            "GH_CONFIG_DIR",
            "GH_TOKEN",
            "GITHUB_TOKEN",
        ] {
            if let Some(value) = daemon.get(key) {
                env.insert(key.into(), value.clone());
            }
        }
        Ok(Self {
            executable,
            runner: ProcessRunner { env },
            directory: directory.into(),
        })
    }
}

impl<R: Runner> Api<R>
where
    R::Error: std::fmt::Display,
{
    /// No automatic replay. Explicit github.com paths and literal arguments only.
    pub async fn request(
        &self,
        method: &str,
        endpoint: &str,
        fields: &[(&str, &str)],
    ) -> Result<Response, String> {
        if !matches!(method, "GET" | "POST" | "PUT" | "DELETE")
            || !endpoint.starts_with("repos/")
            || endpoint
                .split('/')
                .any(|s| s.is_empty() || s == "." || s == "..")
            || !endpoint
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"/-_.".contains(&b))
            || fields.iter().any(|(key, _)| {
                key.is_empty() || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            })
        {
            return Err("invalid GitHub API request".into());
        }
        let mut args = vec![
            self.executable.to_string_lossy().into_owned(),
            "api".into(),
            "--hostname".into(),
            "github.com".into(),
            "--include".into(),
            "--method".into(),
            method.into(),
            "-H".into(),
            "Accept: application/vnd.github+json".into(),
            "-H".into(),
            "X-GitHub-Api-Version: 2022-11-28".into(),
            endpoint.into(),
        ];
        for (key, value) in fields {
            args.extend(["--raw-field".into(), format!("{key}={value}")]);
        }
        let command = args.iter().map(|s| quote(s)).collect::<Vec<_>>().join(" ");
        let result = self
            .runner
            .run(&command, &self.directory.to_string_lossy(), 60_000)
            .await
            .map_err(|_| "GitHub transport could not execute gh".to_string())?;
        if result.timed_out {
            return Err("GitHub request timed out; reconcile before another mutation".into());
        }
        if result.stdout.len() > crate::runner::OUTPUT_LIMIT {
            return Err("GitHub response exceeds output limit".into());
        }
        let response = parse_response(&result.stdout)?;
        if result.exit_code != Some(0) && response.status < 400 {
            return Err("gh failed without a conclusive GitHub error response".into());
        }
        Ok(response)
    }
}

fn parse_response(bytes: &[u8]) -> Result<Response, String> {
    let (at, delimiter) = bytes
        .windows(4)
        .position(|s| s == b"\r\n\r\n")
        .map(|p| (p, 4))
        .or_else(|| bytes.windows(2).position(|s| s == b"\n\n").map(|p| (p, 2)))
        .ok_or("GitHub response has no header boundary")?;
    let headers =
        std::str::from_utf8(&bytes[..at]).map_err(|_| "invalid GitHub response headers")?;
    let mut lines = headers.lines();
    let first = lines.next().ok_or("missing GitHub HTTP status")?;
    let mut words = first.split_whitespace();
    if !matches!(
        words.next(),
        Some("HTTP/1.0" | "HTTP/1.1" | "HTTP/2.0" | "HTTP/2" | "HTTP/3.0" | "HTTP/3")
    ) {
        return Err("invalid GitHub HTTP status line".into());
    }
    let status = words
        .next()
        .filter(|s| s.len() == 3)
        .and_then(|s| s.parse::<u16>().ok())
        .filter(|s| (200..600).contains(s))
        .ok_or("invalid GitHub HTTP status")?;
    let body = &bytes[at + delimiter..];
    let value = if status == 204 && body.iter().all(u8::is_ascii_whitespace) {
        Value::Null
    } else {
        serde_json::from_slice(body).map_err(|_| "invalid or incomplete GitHub JSON response")?
    };
    Ok(Response { status, value })
}

/// Only explicit github.com origins select this adapter. No ambient gh default repo.
pub fn repository_from_origin(origin: &str) -> Result<String, String> {
    let path = origin
        .strip_prefix("https://github.com/")
        .or_else(|| origin.strip_prefix("git@github.com:"))
        .or_else(|| origin.strip_prefix("ssh://git@github.com/"))
        .ok_or("github forge requires a github.com origin remote")?;
    let path = path.strip_suffix(".git").unwrap_or(path);
    let parts = path.split('/').collect::<Vec<_>>();
    if parts.len() != 2
        || parts.iter().any(|s| {
            s.is_empty()
                || *s == "."
                || *s == ".."
                || !s
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        })
    {
        return Err("invalid GitHub origin owner/repository".into());
    }
    Ok(path.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use agend_core::traits::CommandOutput;
    use std::sync::Mutex;

    struct RecordedGh {
        calls: Mutex<Vec<String>>,
        response: Vec<u8>,
        exit: i32,
    }
    impl Runner for RecordedGh {
        type Error = String;
        async fn run(&self, command: &str, _: &str, timeout: u64) -> Result<CommandOutput, String> {
            assert_eq!(timeout, 60_000);
            self.calls.lock().unwrap().push(command.into());
            Ok(CommandOutput {
                exit_code: Some(self.exit),
                stdout: self.response.clone(),
                stderr: vec![],
                timed_out: false,
            })
        }
    }
    fn api(response: &[u8], exit: i32) -> Api<RecordedGh> {
        Api {
            executable: "/real/gh".into(),
            directory: "/repo".into(),
            runner: RecordedGh {
                calls: Mutex::new(vec![]),
                response: response.into(),
                exit,
            },
        }
    }
    const PULL: &[u8] = include_bytes!("../../../tests/fixtures/github/pull.http");
    const MISSING: &[u8] = include_bytes!("../../../tests/fixtures/github/not-found.http");

    #[tokio::test]
    async fn native_process_receives_literal_fields_without_shell_execution_or_token_arguments() {
        use std::os::unix::fs::PermissionsExt;
        let dir = agend_testkit::tempdir::TempDir::new("github-argv").unwrap();
        let executable = dir.path().join("gh");
        let captured = dir.path().join("arguments");
        let response = dir.path().join("response");
        let sentinel = dir.path().join("must-not-exist");
        std::fs::write(&response, PULL).unwrap();
        std::fs::write(&executable, format!(
            "#!/bin/sh\n[ \"$GH_TOKEN\" = fixture-token ] || exit 9\nprintf '%s\\0' \"$@\" > {}\nexec /bin/cat {}\n",
            quote(&captured.to_string_lossy()), quote(&response.to_string_lossy())
        )).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let api = Api {
            executable,
            directory: dir.path().into(),
            runner: ProcessRunner {
                env: BTreeMap::from([
                    ("PATH".into(), "/usr/bin:/bin".into()),
                    ("GH_TOKEN".into(), "fixture-token".into()),
                ]),
            },
        };
        let field = format!(
            "quote ' newline\n$(touch {}) `touch {}`",
            sentinel.display(),
            sentinel.display()
        );
        assert_eq!(
            api.request("POST", "repos/a/b/pulls", &[("title", &field)])
                .await
                .unwrap()
                .status,
            200
        );
        let arguments = std::fs::read_to_string(captured).unwrap();
        let arguments = arguments.split('\0').collect::<Vec<_>>();
        assert!(arguments.contains(&format!("title={field}").as_str()));
        assert!(!arguments.iter().any(|a| a.contains("fixture-token")));
        assert!(!sentinel.exists());
    }

    #[test]
    fn actual_gh_headers_and_success_or_failure_are_decoded() {
        let success = api(PULL, 0);
        let reply =
            agend_testkit::block_on(success.request("GET", "repos/suzuke/AgEnD/pulls/155", &[]))
                .unwrap();
        assert_eq!(reply.status, 200);
        assert_eq!(reply.value["number"], 155);
        let failure = api(MISSING, 1);
        let reply = agend_testkit::block_on(failure.request(
            "GET",
            "repos/suzuke/AgEnD/pulls/999999999",
            &[],
        ))
        .unwrap();
        assert_eq!(reply.status, 404);
        assert_eq!(failure.runner.calls.lock().unwrap().len(), 1);
    }

    #[test]
    fn malformed_or_truncated_http_never_becomes_a_success() {
        for bytes in [
            b"{}".as_slice(),
            b"HTTP/2.0 200 OK\n\n{",
            b"HTTP/2.0 100 Continue\n\n{}",
            b"HTTP/2.0 200 OK\n\n{} garbage",
        ] {
            assert!(parse_response(bytes).is_err());
        }
        let failed = api(PULL, 1);
        assert!(
            agend_testkit::block_on(failed.request("PUT", "repos/a/b/pulls/1/merge", &[])).is_err()
        );
        assert_eq!(failed.runner.calls.lock().unwrap().len(), 1);
    }

    #[test]
    fn origin_must_name_exactly_one_explicit_github_repository() {
        for origin in [
            "https://github.com/owner/repo.git",
            "git@github.com:owner/repo.git",
            "ssh://git@github.com/owner/repo",
        ] {
            assert_eq!(repository_from_origin(origin).unwrap(), "owner/repo");
        }
        for origin in [
            "https://github.com.evil/owner/repo",
            "https://token@github.com/owner/repo",
            "../owner/repo",
            "https://github.com/owner/../repo",
            "https://github.com/owner/repo?x=y",
            "git@github.com:/owner/repo",
            "https://github.com/./repo",
        ] {
            assert!(repository_from_origin(origin).is_err(), "{origin}");
        }
    }

    #[test]
    fn request_fields_are_literal_shell_arguments_and_mutations_are_not_retried() {
        let api = api(MISSING, 1);
        let title = "quoted ' title\n$(touch /tmp/never) `echo never`";
        agend_testkit::block_on(api.request("POST", "repos/a/b/pulls", &[("title", title)]))
            .unwrap();
        let calls = api.runner.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert!(calls[0].contains(&quote(&format!("title={title}"))));
        drop(calls);
        for endpoint in [
            "https://other/pulls",
            "repos/a/b/../pulls",
            "repos/a/b/%2e%2e/pulls",
            "repos/a/b/pulls?x=y",
        ] {
            assert!(agend_testkit::block_on(api.request("POST", endpoint, &[])).is_err());
        }
        assert_eq!(api.runner.calls.lock().unwrap().len(), 1);
    }
}
