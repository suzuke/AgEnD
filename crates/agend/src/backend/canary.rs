//! Explicit operator canary: isolated daemon, three deliveries, bounded work,
//! and owned-process cleanup. A report is not an activation command.
use super::files;
use agend_core::protocol::client::*;
pub use agend_core::setup::backend::canary::CanaryReport as Report;
use std::fs;
use std::io::{Read, Write};
use std::path::Path;
use std::time::{Duration, Instant};
mod credentials;
mod process;
const ID: &str = "canary";

fn uuid() -> Result<String, String> {
    let mut bytes = [0; 16];
    fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .map_err(|e| e.to_string())?;
    Ok(uuid_v4(bytes))
}
fn publish(dir: &Path, report: &Report) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(report).map_err(|e| e.to_string())?;
    let path = dir.join("canary.json");
    let replace = files::regular(&path)?.is_some();
    files::publish(&path, 0o600, replace, |out| out.write_all(&bytes))
}

pub fn run(
    home: &Path,
    backend: &str,
    version: &str,
    seconds: u64,
    model: Option<&str>,
    auth_file: Option<&Path>,
) -> Result<Report, String> {
    let args = model_args(backend, model)?;
    let credentials = auth_file
        .map(|path| credentials::read(backend, path))
        .transpose()?;
    if !(10..=300).contains(&seconds) {
        return Err("canary timeout must be 10-300 seconds".into());
    }
    let _activity =
        agend_daemon::store::maintenance::Activity::acquire(home).map_err(|e| e.to_string())?;
    let _lock = files::lock(&home.join("backends"))?;
    let artifact = agend_daemon::backend_versions::inspect(home, backend, version)?;
    let dir = home.join("backends").join(backend).join(version);
    let program = dir
        .join("program")
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let agend = std::env::current_exe()
        .and_then(|p| p.canonicalize())
        .map_err(|e| e.to_string())?;
    let agend_sha256 = files::sha256(fs::File::open(&agend).map_err(|e| e.to_string())?)?;
    let began = Instant::now();
    let deadline = began + Duration::from_secs(seconds);
    let mut report = Report {
        format: 1,
        run_id: uuid()?,
        artifact,
        agend_sha256,
        os: std::env::consts::OS.into(),
        arch: std::env::consts::ARCH.into(),
        started_at_unix_ms: agend_daemon::log::now_unix_ms(),
        elapsed_ms: 0,
        observed_version: None,
        model: model.map(str::to_owned),
        receipts: Vec::new(),
        outcomes: Vec::new(),
        passed: false,
        cleanup_complete: false,
        error: Some("canary in progress; no admission".into()),
        retained_home: None,
    };
    // Invalidate any previous success before executing a candidate again.
    publish(&dir, &report)?;
    let mut lab = process::Lab::new(&report.run_id)?;
    let tested = (|| {
        let observed = lab.version(&program, deadline)?;
        let expected = match backend {
            "codex" => format!("codex-cli {version}"),
            "claude" => format!("{version} (Claude Code)"),
            "opencode" => version.to_owned(),
            _ => return Err("unsupported canary backend".into()),
        };
        if observed.trim() != expected {
            return Err("backend --version did not match the imported version".into());
        }
        report.observed_version = Some(observed.trim().into());
        if let Some(bytes) = &credentials {
            credentials::install(&lab.home, backend, bytes)?;
        }
        agend_daemon::backend_versions::canary_scope::create(
            &lab.home,
            home,
            &report.artifact,
            &args,
        )?;
        lab.start(&agend)?;
        while !lab.home.join(DAEMON_SOCKET).exists() {
            lab.check_alive()?;
            pause(deadline)?;
        }
        let response = rpc(
            &lab.home,
            ClientRequest::Operator {
                data: OperatorData {
                    request_id: uuid()?,
                    command: OperatorCommand::InstanceAdd {
                        instance_id: ID.into(),
                        backend: backend.into(),
                        working_directory: Some(lab.home.join("workspace").display().to_string()),
                        program: Some(program.display().to_string()),
                        args,
                    },
                },
            },
            deadline,
        )?;
        if !matches!(
            response,
            ClientResponse::CommandResult {
                data: ClientCommandResultData {
                    result: CommandResult::InstanceAdded { .. },
                    ..
                }
            }
        ) {
            return Err("canary instance was not accepted".into());
        }
        wait_idle(&lab.home, deadline).map_err(|e| format!("initial readiness: {e}"))?;
        for index in 1..=3 {
            let message_id = uuid()?;
            let instructions = if backend == "claude" {
                "First acknowledge this message using the required AgEnD receipt tool. No other tools or file changes."
            } else {
                "No tools or file changes."
            };
            let response = rpc(
                &lab.home,
                ClientRequest::Operator {
                    data: OperatorData {
                        request_id: uuid()?,
                        command: OperatorCommand::SendMessage {
                            to: ID.into(),
                            message: format!(
                                "AgEnD backend canary {index}/3. {instructions} Reply briefly: CANARY {} {index}",
                                report.run_id
                            ),
                            message_id: message_id.clone(),
                        },
                    },
                },
                deadline,
            )?;
            if !matches!(
                response,
                ClientResponse::CommandResult {
                    data: ClientCommandResultData {
                        result: CommandResult::Accepted,
                        ..
                    }
                }
            ) {
                return Err("canary send did not return accepted; not replayed".into());
            }
            loop {
                let reply = rpc(
                    &lab.home,
                    ClientRequest::Operator {
                        data: OperatorData {
                            request_id: uuid()?,
                            command: OperatorCommand::MessageDelivery {
                                message_id: message_id.clone(),
                            },
                        },
                    },
                    deadline,
                )?;
                let ClientResponse::CommandResult {
                    data:
                        ClientCommandResultData {
                            result:
                                CommandResult::MessageDelivery {
                                    data: Some(receipt),
                                },
                            ..
                        },
                } = reply
                else {
                    return Err("missing canary delivery receipt".into());
                };
                if receipt.message_id != message_id
                    || receipt.to_instance != ID
                    || receipt.from_instance != OPERATOR_MESSAGE_SENDER
                {
                    return Err("canary delivery identity mismatch".into());
                }
                match receipt.state {
                    MessageDeliveryState::Confirmed => {
                        report.receipts.push(receipt);
                        break;
                    }
                    MessageDeliveryState::Queued | MessageDeliveryState::Sent => pause(deadline)?,
                    _ => return Err("canary delivery failed or has an unknown state".into()),
                }
            }
            report.outcomes.push(wait_outcome(
                &lab.home,
                report.receipts.last().unwrap(),
                deadline,
            )?);
            wait_idle(&lab.home, deadline).map_err(|e| format!("message {index} idle: {e}"))?;
        }
        if agend_daemon::backend_versions::inspect(home, backend, version)? != report.artifact
            || files::sha256(fs::File::open(&agend).map_err(|e| e.to_string())?)?
                != report.agend_sha256
        {
            return Err("candidate or agend changed during canary".into());
        }
        if Instant::now() >= deadline {
            return Err("canary deadline expired during final verification".into());
        }
        Ok(())
    })();
    let cleaned = lab.cleanup();
    report.cleanup_complete = cleaned.is_ok();
    report.passed = tested.is_ok() && cleaned.is_ok();
    report.error = match (tested, cleaned) {
        (Ok(()), Ok(())) => None,
        (Err(e), Ok(())) => Some(e),
        (result, Err(e)) => Some(format!(
            "{}; cleanup: {e}",
            result.err().unwrap_or_else(|| "canary completed".into())
        )),
    };
    if !report.cleanup_complete {
        report.retained_home = Some(lab.home.display().to_string());
    }
    report.elapsed_ms = began.elapsed().as_millis().min(u64::MAX as u128) as u64;
    if report.passed
        && let Err(reason) = report.validate(
            &report.artifact,
            &report.agend_sha256,
            std::env::consts::OS,
            std::env::consts::ARCH,
        )
    {
        report.passed = false;
        report.error = Some(reason.into());
    }
    publish(&dir, &report)?;
    Ok(report)
}
fn model_args(backend: &str, model: Option<&str>) -> Result<Vec<String>, String> {
    let Some(model) = model else {
        return Ok(Vec::new());
    };
    if model.is_empty()
        || model.len() > 256
        || model.starts_with('-')
        || !model.bytes().all(|b| b.is_ascii_graphic())
    {
        return Err("canary model must be 1-256 printable ASCII characters without spaces or a leading dash".into());
    }
    if backend == "opencode"
        && !model
            .split_once('/')
            .is_some_and(|(p, m)| !p.is_empty() && !m.is_empty())
    {
        return Err("OpenCode canary model must be provider/model".into());
    }
    if backend == "codex" {
        // app-server has no TUI --model flag; use its global config override.
        return Ok(vec![
            "-c".into(),
            format!(
                "model={}",
                serde_json::to_string(model).map_err(|_| "invalid model")?
            ),
        ]);
    }
    Ok(vec!["--model".into(), model.into()])
}
fn pause(deadline: Instant) -> Result<(), String> {
    if Instant::now() >= deadline {
        return Err("canary deadline expired; no admission".into());
    }
    std::thread::sleep(
        Duration::from_millis(100).min(deadline.saturating_duration_since(Instant::now())),
    );
    Ok(())
}
fn rpc(home: &Path, request: ClientRequest, deadline: Instant) -> Result<ClientResponse, String> {
    agend_client::exchange_once(&home.join(DAEMON_SOCKET), None, V1_7, &request, deadline)
        .map_err(|e| e.to_string())
}
fn wait_idle(home: &Path, deadline: Instant) -> Result<(), String> {
    loop {
        let response = rpc(
            home,
            ClientRequest::Operator {
                data: OperatorData {
                    request_id: uuid()?,
                    command: OperatorCommand::DriverStatus {
                        instance_id: ID.into(),
                    },
                },
            },
            deadline,
        )?;
        let ClientResponse::CommandResult {
            data:
                ClientCommandResultData {
                    result: CommandResult::DriverStatus { data },
                    ..
                },
        } = response
        else {
            return Err("missing canary driver status".into());
        };
        if data.instance_id != ID {
            return Err("canary driver identity mismatch".into());
        }
        match data.state {
            AgentState::Idle => return Ok(()),
            AgentState::Failed | AgentState::Stuck => {
                return Err("canary driver failed or is stuck".into());
            }
            _ => pause(deadline)?,
        }
    }
}

fn wait_outcome(
    home: &Path,
    receipt: &MessageDeliveryData,
    deadline: Instant,
) -> Result<MessageOutcomeData, String> {
    loop {
        let response = rpc(
            home,
            ClientRequest::Operator {
                data: OperatorData {
                    request_id: uuid()?,
                    command: OperatorCommand::MessageOutcome {
                        message_id: receipt.message_id.clone(),
                    },
                },
            },
            deadline,
        )?;
        let ClientResponse::CommandResult {
            data:
                ClientCommandResultData {
                    result: CommandResult::MessageOutcome { data },
                    ..
                },
        } = response
        else {
            return Err("missing canary execution outcome".into());
        };
        if data.message_id != receipt.message_id
            || data.instance_id != ID
            || data.turn_id != receipt.turn_id
        {
            return Err("canary execution outcome identity mismatch".into());
        }
        match data.state {
            MessageOutcomeState::Completed => {
                if data.execution_id.as_ref().is_none_or(|s| s.is_empty()) {
                    return Err("completed canary execution has no identity".into());
                }
                return Ok(data);
            }
            MessageOutcomeState::Running | MessageOutcomeState::Unknown => pause(deadline)?,
            _ => {
                return Err(
                    "canary execution failed or backend completion evidence is unsupported".into(),
                );
            }
        }
    }
}
