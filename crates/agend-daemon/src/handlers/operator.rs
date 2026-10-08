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
//!   temporary home is removed (also when the daemon stops meanwhile: the
//!   preflight child is then killed and reaped) and nothing else changed;
//!   the binary must still be the same file when the preflight ends.
//!   Success: the reply
//!   `restarting`, then the server hands the binary to the supervisor, which
//!   stops like on SIGINT and the daemon `exec`s it (`crate::daemon`).
//! - `task_cancel`: `not_supported` until gate 10.
//!
//! Must NOT: run the preflight on `agend.db` itself or in the real home.

use std::ffi::OsStr;
use std::fs;
use std::io::{self, Read};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use agend_core::protocol::client::{
    ClientCommandResultData, ClientResponse, CommandResult, OperatorCommand, OperatorData,
    RestartingData, error_code,
};
use tokio::sync::oneshot;

use super::{Context, Outcome, error};
use crate::log;
use crate::preflight::HOLDER_ID;
use crate::runtime::{files, shutdown_holder_within};
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
        _ => "only the operator can send operator requests; ask the operator",
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
        OperatorCommand::SendMessage {
            to,
            message,
            message_id,
        } => match super::agent::send_operator(ctx, to, message, message_id).await {
            Ok(value) => result(request_id, value),
            Err((code, message)) => error(Some(request_id), code, message),
        },
        OperatorCommand::DriverStatus { instance_id } => {
            use agend_core::protocol::client::{AgentState, DriverStatusData};
            let instance = match ctx.store.instance(&instance_id).await {
                Ok(Some(instance)) => instance,
                _ => {
                    return Outcome::Reply(error(
                        Some(request_id),
                        error_code::UNKNOWN_INSTANCE,
                        "driver instance is unavailable",
                    ));
                }
            };
            let state = if instance.status == crate::store::InstanceStatus::Failed {
                AgentState::Failed
            } else if instance.backend == agend_core::model::Backend::Codex {
                match ctx.codex.connected_busy(&instance_id) {
                    Some(true) => AgentState::Working,
                    Some(false) => AgentState::Idle,
                    None => AgentState::Unknown,
                }
            } else {
                ctx.fleet
                    .view()
                    .instances
                    .into_iter()
                    .find(|i| i.instance_id == instance_id)
                    .map(|i| i.state)
                    .unwrap_or(AgentState::Unknown)
            };
            result(
                request_id,
                CommandResult::DriverStatus {
                    data: DriverStatusData { instance_id, state },
                },
            )
        }

        OperatorCommand::MessageOutcome { message_id } => {
            use agend_core::protocol::client::{MessageOutcomeData, MessageOutcomeState};
            if message_id.is_empty() || message_id.len() > 128 {
                return Outcome::Reply(error(
                    Some(request_id),
                    error_code::INVALID_REQUEST,
                    "message id must contain 1-128 bytes",
                ));
            }
            let row = match ctx.store.message(&message_id).await {
                Ok(Some(row)) => row,
                _ => {
                    return Outcome::Reply(error(
                        Some(request_id),
                        error_code::INVALID_REQUEST,
                        "message is unavailable",
                    ));
                }
            };
            let backend = ctx
                .store
                .instance(&row.to_instance)
                .await
                .ok()
                .flatten()
                .map(|i| i.backend);
            let mut execution_id = row.turn_id.clone();
            let state = if backend == Some(agend_core::model::Backend::Codex) {
                ctx.codex
                    .message_outcome(row.clone())
                    .await
                    .unwrap_or(MessageOutcomeState::Unknown)
            } else if backend == Some(agend_core::model::Backend::Opencode) {
                crate::driver::opencode::driver::OpenCodeDriver::new(ctx.store.clone())
                    .message_outcome(row.clone())
                    .await
                    .unwrap_or(MessageOutcomeState::Unknown)
            } else if backend == Some(agend_core::model::Backend::Claude) {
                execution_id = ctx
                    .store
                    .claude_message_outcome(&message_id)
                    .await
                    .ok()
                    .flatten();
                if execution_id.is_some() {
                    MessageOutcomeState::Completed
                } else {
                    MessageOutcomeState::Unknown
                }
            } else {
                MessageOutcomeState::Unsupported
            };
            result(
                request_id,
                CommandResult::MessageOutcome {
                    data: MessageOutcomeData {
                        message_id,
                        instance_id: row.to_instance,
                        turn_id: row.turn_id,
                        execution_id,
                        state,
                    },
                },
            )
        }
        OperatorCommand::MessageDelivery { message_id } => {
            use agend_core::model::DeliveryState;
            use agend_core::protocol::client::{MessageDeliveryData, MessageDeliveryState};
            if message_id.is_empty() || message_id.len() > 128 {
                return Outcome::Reply(error(
                    Some(request_id),
                    error_code::INVALID_REQUEST,
                    "message id must contain 1-128 bytes",
                ));
            }
            match ctx.store.message(&message_id).await {
                Ok(message) => result(
                    request_id,
                    CommandResult::MessageDelivery {
                        data: message.map(|m| MessageDeliveryData {
                            message_id: m.id,
                            from_instance: m.from_instance,
                            to_instance: m.to_instance,
                            state: match m.state {
                                DeliveryState::Queued => MessageDeliveryState::Queued,
                                DeliveryState::Sent => MessageDeliveryState::Sent,
                                DeliveryState::Confirmed => MessageDeliveryState::Confirmed,
                                DeliveryState::Failed => MessageDeliveryState::Failed,
                            },
                            turn_id: m.turn_id,
                            attempted_at_unix_ms: m.attempted_at_unix_ms,
                            updated_at_unix_ms: m.updated_at_unix_ms,
                        }),
                    },
                ),
                Err(_) => error(
                    Some(request_id),
                    error_code::INVALID_REQUEST,
                    "cannot read persisted delivery receipt",
                ),
            }
        }
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
        OperatorCommand::Unknown => error(
            Some(request_id),
            error_code::UNKNOWN_REQUEST,
            "unknown operator command",
        ),
        other => super::pipeline_reply(request_id, ctx.pipeline.operator(other).await),
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

/// The preflight's temporary home and child process. Dropping it — done,
/// failed, timed out, or the connection task aborted because the daemon is
/// stopping — kills and reaps the child (our own), sends `Shutdown` to a
/// `pf-check` holder still running in the home, and removes the home with
/// its copy of `agend.db` (verifier F1).
struct Preflight {
    home: PathBuf,
    child: Option<Child>,
}

impl Drop for Preflight {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            if let Ok(None) = child.try_wait() {
                log::line(&format!("preflight pid {}: stopped", child.id()));
                let _ = child.kill();
            }
            let _ = child.wait();
        }
        if let Ok(Some(_)) = files::running(&self.home, HOLDER_ID) {
            let stopped = shutdown_holder_within(&self.home, HOLDER_ID, Duration::from_secs(5));
            if let Err(e) = stopped {
                log::line(&format!("preflight holder: {e}"));
            }
        }
        if let Err(e) = fs::remove_dir_all(&self.home) {
            log::line(&format!("cannot remove {}: {e}", self.home.display()));
        }
    }
}

/// What a reader thread collected from one of the child's pipes (at most
/// [`COLLECT_BYTES`]); it may still be reading when the child has exited.
#[derive(Clone, Default)]
struct Collected {
    text: Arc<Mutex<Vec<u8>>>,
    done: Arc<AtomicBool>,
}

/// The most a preflight's stdout or stderr is kept.
const COLLECT_BYTES: usize = 64 * 1024;

impl Collected {
    fn start(mut pipe: impl Read + Send + 'static) -> Collected {
        let collected = Collected::default();
        let into = collected.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = pipe.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let mut text = into.text.lock().unwrap_or_else(|e| e.into_inner());
                let room = COLLECT_BYTES.saturating_sub(text.len());
                text.extend_from_slice(&buf[..n.min(room)]);
            }
            into.done.store(true, Ordering::SeqCst);
        });
        collected
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.text.lock().unwrap_or_else(|e| e.into_inner())).into_owned()
    }
}

/// The file a path names now: device, inode, size, modification time, and
/// change time (a writer can put the modification time back, but not the
/// change time: verifier r2).
type Identity = (u64, u64, u64, i64, i64, i64, i64);

fn identity(path: &Path) -> Option<Identity> {
    use std::os::unix::fs::MetadataExt;
    let m = fs::metadata(path).ok()?;
    Some((
        m.dev(),
        m.ino(),
        m.size(),
        m.mtime(),
        m.mtime_nsec(),
        m.ctime(),
        m.ctime_nsec(),
    ))
}

async fn preflight(ctx: &Context, binary: &Path) -> Result<Vec<String>, String> {
    let shown = binary.display();
    let home = mkdtemp().map_err(|e| format!("cannot make the preflight home: {e}"))?;
    log::line(&format!("preflight home {}", home.display()));
    let mut run = Preflight { home, child: None };
    ctx.store
        .copy_to(&run.home.join(DB_FILE))
        .await
        .map_err(|e| format!("cannot copy agend.db for the preflight: {e}"))?;
    let before = identity(binary);
    let mut child = Command::new(binary)
        .args(["daemon", "preflight"])
        .arg(&run.home)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot run {shown}: {e}"))?;
    log::line(&format!("preflight pid {}", child.id()));
    let out = Collected::start(child.stdout.take().expect("piped"));
    let err = Collected::start(child.stderr.take().expect("piped"));
    let child = run.child.insert(child);
    // The deadline holds whatever the child's pipes do (a process it left
    // behind may keep them open, verifier F2).
    let deadline = Instant::now() + PREFLIGHT_WITHIN;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Ok(None) => {
                return Err(format!(
                    "{shown} daemon preflight did not finish within {} s",
                    PREFLIGHT_WITHIN.as_secs()
                ));
            }
            Err(e) => return Err(format!("{shown} daemon preflight: wait: {e}")),
        }
    };
    // What the child wrote is in its pipes: a moment for the readers.
    let settle = Instant::now() + Duration::from_millis(500);
    while !(out.done.load(Ordering::SeqCst) && err.done.load(Ordering::SeqCst))
        && Instant::now() < settle
    {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let lines: Vec<String> = out.text().lines().map(str::to_owned).collect();
    let said = err.text();
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
    // `exec` finds the binary by its path again: it must still be the file
    // that passed (verifier F4; the moment until `exec` remains, see the
    // gate page's known risks).
    if identity(binary) != before {
        return Err(format!(
            "{shown} changed while its preflight ran; run agend daemon restart again"
        ));
    }
    Ok(lines)
}

fn describe(status: ExitStatus) -> String {
    use std::os::unix::process::ExitStatusExt;
    match (status.code(), status.signal()) {
        (Some(code), _) => format!("status {code}"),
        (None, Some(signal)) => format!("signal {signal}"),
        _ => "an unknown status".into(),
    }
}
