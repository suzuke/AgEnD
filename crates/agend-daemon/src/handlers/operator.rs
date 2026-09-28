//! Operator requests (`operator`, gate 9 P6, P7): only the operator (a
//! `hello` without `caller`); an agent gets `forbidden` with what to do.
//!
//! - `instance_add` / `instance_remove`: done by the supervisor (it owns the
//!   instances' state); the reply comes once the row is written (add, then
//!   the start begins) or gone (remove: the holder gets `Shutdown` and at
//!   most 5 s; the workspace stays).
//! - `daemon_restart` (D2): one at a time. A fresh copy of `agend.db`
//!   (`VACUUM INTO`) goes to a new `mkdtemp` home `/tmp/agend-pf-XXXXXX`;
//!   `<binary> daemon preflight <that home>` runs there (60 s at most) and
//!   must report its steps and exit 0. Any failure: `preflight_failed`, the
//!   temporary home is removed and nothing else changed. Success: the reply
//!   `restarting`, then the server hands the binary to the supervisor, which
//!   stops like on SIGINT and the daemon `exec`s it (`crate::daemon`).
//! - `task_cancel`: `not_supported` until gate 10.
//!
//! Must NOT: run the preflight on `agend.db` itself or in the real home.

use std::ffi::OsStr;
use std::fs;
use std::io::{self, BufRead, BufReader, Read};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use agend_core::protocol::client::{
    ClientCommandResultData, ClientResponse, CommandResult, OperatorCommand, OperatorData,
    RestartingData, error_code,
};
use tokio::sync::oneshot;

use super::{Context, Outcome, error};
use crate::log;
use crate::store::DB_FILE;
use crate::supervisor::{AddRequest, Event};

/// Longest a preflight may run.
pub const PREFLIGHT_WITHIN: Duration = Duration::from_secs(60);

/// What an agent gets for an operator request.
pub fn forbidden(command: &OperatorCommand) -> &'static str {
    match command {
        OperatorCommand::InstanceAdd { .. } => {
            "only the operator can add instances; ask the operator"
        }
        OperatorCommand::InstanceRemove { .. } => {
            "only the operator can remove instances; ask the operator"
        }
        OperatorCommand::DaemonRestart { .. } => {
            "only the operator can restart the daemon; ask the operator"
        }
        OperatorCommand::TaskCancel { .. } => {
            "only the operator can cancel tasks; ask the operator"
        }
        OperatorCommand::Unknown => {
            "only the operator can send operator requests; ask the operator"
        }
    }
}

fn result(request_id: String, result: CommandResult) -> ClientResponse {
    ClientResponse::CommandResult {
        data: ClientCommandResultData { request_id, result },
    }
}

/// Handles an operator request (the caller was checked).
pub async fn handle(ctx: &Context, data: OperatorData) -> Outcome {
    let request_id = data.request_id;
    let stopping = |id| {
        error(
            Some(id),
            error_code::NOT_SUPPORTED,
            "the daemon is stopping; nothing changed",
        )
    };
    let reply = match data.command {
        OperatorCommand::InstanceAdd {
            instance_id,
            backend,
            working_directory,
            program,
            args,
        } => {
            let (reply, answer) = oneshot::channel();
            let request = AddRequest {
                id: instance_id,
                backend,
                working_directory,
                program,
                args,
            };
            if ctx.supervisor.send(Event::Add { request, reply }).is_err() {
                return Outcome::Reply(stopping(request_id));
            }
            match answer.await {
                Ok(Ok(added)) => result(request_id, CommandResult::InstanceAdded { data: added }),
                Ok(Err((code, message))) => error(Some(request_id), code, message),
                Err(_) => stopping(request_id),
            }
        }
        OperatorCommand::InstanceRemove { instance_id } => {
            let (reply, answer) = oneshot::channel();
            let event = Event::Remove {
                id: instance_id,
                reply,
            };
            if ctx.supervisor.send(event).is_err() {
                return Outcome::Reply(stopping(request_id));
            }
            match answer.await {
                Ok(Ok(())) => result(request_id, CommandResult::Accepted),
                Ok(Err((code, message))) => error(Some(request_id), code, message),
                Err(_) => stopping(request_id),
            }
        }
        OperatorCommand::DaemonRestart { binary } => return restart(ctx, request_id, binary).await,
        OperatorCommand::TaskCancel { task_id } => error(
            Some(request_id),
            error_code::NOT_SUPPORTED,
            format!("agend task cancel arrives in gate 10; {task_id} is unchanged"),
        ),
        OperatorCommand::Unknown => error(
            Some(request_id),
            error_code::UNKNOWN_REQUEST,
            "unknown operator command",
        ),
    };
    Outcome::Reply(reply)
}

async fn restart(ctx: &Context, request_id: String, binary: Option<String>) -> Outcome {
    if ctx.restarting.swap(true, Ordering::SeqCst) {
        return Outcome::Reply(error(
            Some(request_id),
            error_code::INVALID_REQUEST,
            "a restart is already in progress",
        ));
    }
    let binary = binary.map_or_else(|| ctx.exe.clone(), PathBuf::from);
    log::line(&format!(
        "restart requested: preflight of {}",
        binary.display()
    ));
    match preflight(ctx, &binary).await {
        Ok(lines) => {
            for line in &lines {
                log::line(&format!("preflight: {line}"));
            }
            log::line(&format!(
                "preflight passed; restarting with {}",
                binary.display()
            ));
            Outcome::Restart {
                reply: result(
                    request_id,
                    CommandResult::Restarting {
                        data: RestartingData { preflight: lines },
                    },
                ),
                binary,
            }
        }
        Err(reason) => {
            log::line(&format!("preflight failed: {reason}; not restarting"));
            ctx.restarting.store(false, Ordering::SeqCst);
            Outcome::Reply(error(
                Some(request_id),
                error_code::PREFLIGHT_FAILED,
                format!(
                    "{reason}; the daemon keeps running agend {}",
                    env!("CARGO_PKG_VERSION")
                ),
            ))
        }
    }
}

/// A new private directory `/tmp/agend-pf-XXXXXX` (0700): short, so the
/// holder socket under it stays within 100 bytes (gate 6 H13).
fn mkdtemp() -> io::Result<PathBuf> {
    let mut template = *b"/tmp/agend-pf-XXXXXX\0";
    // SAFETY: a NUL-terminated buffer we own; mkdtemp only rewrites the Xs.
    let made = unsafe { libc::mkdtemp(template.as_mut_ptr().cast()) };
    if made.is_null() {
        return Err(io::Error::last_os_error());
    }
    let path = &template[..template.len() - 1];
    Ok(PathBuf::from(OsStr::from_bytes(path)))
}

async fn preflight(ctx: &Context, binary: &Path) -> Result<Vec<String>, String> {
    let dir = mkdtemp().map_err(|e| format!("cannot make the preflight home: {e}"))?;
    log::line(&format!("preflight home {}", dir.display()));
    let copied = ctx
        .store
        .copy_to(&dir.join(DB_FILE))
        .await
        .map_err(|e| format!("cannot copy agend.db for the preflight: {e}"));
    let outcome = match copied {
        Ok(()) => {
            let (binary, home) = (binary.to_path_buf(), dir.clone());
            tokio::task::spawn_blocking(move || run_preflight(&binary, &home))
                .await
                .unwrap_or_else(|e| Err(format!("the preflight panicked: {e}")))
        }
        Err(e) => Err(e),
    };
    if let Err(e) = fs::remove_dir_all(&dir) {
        log::line(&format!("cannot remove {}: {e}", dir.display()));
    }
    outcome
}

fn describe(status: ExitStatus) -> String {
    use std::os::unix::process::ExitStatusExt;
    match (status.code(), status.signal()) {
        (Some(code), _) => format!("status {code}"),
        (None, Some(signal)) => format!("signal {signal}"),
        _ => "an unknown status".into(),
    }
}

/// Runs `<binary> daemon preflight <home>` and returns its stdout lines.
fn run_preflight(binary: &Path, home: &Path) -> Result<Vec<String>, String> {
    let shown = binary.display();
    let mut child = Command::new(binary)
        .args(["daemon", "preflight"])
        .arg(home)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run {shown}: {e}"))?;
    let stdout = child.stdout.take().expect("piped");
    let mut stderr = child.stderr.take().expect("piped");
    let out = std::thread::spawn(move || {
        BufReader::new(stdout)
            .lines()
            .map_while(Result::ok)
            .collect::<Vec<_>>()
    });
    let err = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text);
        text
    });
    let deadline = Instant::now() + PREFLIGHT_WITHIN;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
            Ok(None) => {
                // Our own child, not yet reaped.
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "{shown} daemon preflight did not finish within {} s",
                    PREFLIGHT_WITHIN.as_secs()
                ));
            }
            Err(e) => return Err(format!("{shown} daemon preflight: wait: {e}")),
        }
    };
    let lines = out.join().unwrap_or_default();
    let said = err.join().unwrap_or_default();
    let said = said
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .map(str::trim);
    if !status.success() {
        return Err(match said {
            Some(said) => format!(
                "{shown} daemon preflight exited with {}: {said}",
                describe(status)
            ),
            None => format!("{shown} daemon preflight exited with {}", describe(status)),
        });
    }
    let reported = lines.first().is_some_and(|l| l.starts_with("agend "))
        && lines.iter().any(|l| l.starts_with("db copy: "))
        && lines
            .iter()
            .any(|l| l == "holder: hello ok, spawn ok, shutdown ok");
    if !reported {
        return Err(format!(
            "{shown} daemon preflight exited 0 without reporting its steps (is it agend?)"
        ));
    }
    Ok(lines)
}
