//! Bounded process-group command runner. Every exit stops remaining children.
use agend_core::traits::{CommandOutput, Runner};
use std::collections::BTreeMap;
use std::io;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;

pub const OUTPUT_LIMIT: usize = 5 * 1024 * 1024;

#[derive(Clone)]
pub struct ProcessRunner {
    pub env: BTreeMap<String, String>,
}

pub fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

struct Group(u32);
impl Drop for Group {
    fn drop(&mut self) {
        if self.0 > 1 {
            // Child owns its process group, never the daemon's or another fixture's.
            unsafe {
                libc::kill(-(self.0 as i32), libc::SIGKILL);
            }
        }
    }
}

async fn drain(mut pipe: impl AsyncRead + Unpin) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buf = [0; 8192];
    let mut truncated = false;
    loop {
        match pipe.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                let keep = n.min(OUTPUT_LIMIT.saturating_sub(out.len()));
                out.extend_from_slice(&buf[..keep]);
                truncated |= keep != n;
            }
        }
    }
    if truncated {
        out.extend_from_slice(b"\n[output truncated at 5 MiB]\n");
    }
    out
}

impl ProcessRunner {
    async fn run_with_stdout(
        &self,
        command: &str,
        working_directory: &str,
        timeout_ms: u64,
        destination: Stdio,
    ) -> io::Result<CommandOutput> {
        // Drop cannot run after SIGKILL. A child in this same process group
        // watches the owner; it kills only its own group when the owner dies.
        let watched = format!(
            "(while kill -0 {} 2>/dev/null; do sleep 1; done; kill -KILL -$$) >/dev/null 2>&1 &\n{command}",
            std::process::id()
        );
        let mut child = Command::new("/bin/sh")
            .arg("-c")
            .arg(watched)
            .current_dir(working_directory)
            .env_clear()
            .envs(&self.env)
            .stdin(Stdio::null())
            .stdout(destination)
            .stderr(Stdio::piped())
            .process_group(0)
            .kill_on_drop(true)
            .spawn()?;
        let group = Group(
            child
                .id()
                .ok_or_else(|| io::Error::other("child has no pid"))?,
        );
        let stdout = child.stdout.take().map(|pipe| tokio::spawn(drain(pipe)));
        let stderr = tokio::spawn(drain(child.stderr.take().expect("piped stderr")));
        let waited = tokio::time::timeout(Duration::from_millis(timeout_ms), child.wait()).await;
        let (exit_code, timed_out) = match waited {
            Ok(result) => (result?.code(), false),
            Err(_) => (None, true),
        };
        drop(group);
        if timed_out {
            let _ = child.wait().await;
        }
        // A setsid descendant on macOS can retain a pipe after group cleanup.
        async fn collect(mut task: tokio::task::JoinHandle<Vec<u8>>) -> Vec<u8> {
            match tokio::time::timeout(Duration::from_secs(1), &mut task).await {
                Ok(Ok(output)) => output,
                _ => {
                    task.abort();
                    b"\n[output pipe did not close]\n".to_vec()
                }
            }
        }
        let (stdout, stderr) = tokio::join!(
            async {
                match stdout {
                    Some(task) => collect(task).await,
                    None => Vec::new(),
                }
            },
            collect(stderr)
        );
        Ok(CommandOutput {
            exit_code,
            stdout,
            stderr,
            timed_out,
        })
    }
}

impl ProcessRunner {
    /// Stream complete artifact bytes to an owned file, retaining timeout and group cleanup.
    pub async fn run_to_file(
        &self,
        command: &str,
        working_directory: &str,
        timeout_ms: u64,
        file: std::fs::File,
    ) -> io::Result<CommandOutput> {
        self.run_with_stdout(command, working_directory, timeout_ms, Stdio::from(file))
            .await
    }
}

impl Runner for ProcessRunner {
    type Error = io::Error;
    async fn run(
        &self,
        command: &str,
        working_directory: &str,
        timeout_ms: u64,
    ) -> io::Result<CommandOutput> {
        self.run_with_stdout(command, working_directory, timeout_ms, Stdio::piped())
            .await
    }
}
