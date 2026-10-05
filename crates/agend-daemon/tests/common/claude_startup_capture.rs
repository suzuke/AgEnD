//! Startup evidence through the actual daemon and holder. Passive by default;
//! explicit bounded probes can select workspace trust and local channels. No model
//! prompt or message delivery is issued; readiness is never inferred.
#[path = "daemon_process.rs"]
mod lab;
#[path = "claude_trust_control.rs"]
mod trust;
use agend_core::{
    model::Backend,
    protocol::{client::*, terminal::*},
};
use agend_daemon::store::{Instance, InstanceStatus, SqliteStore};
use agend_testkit::{
    block_on,
    fake_daemon::ProbeClient,
    recorder::{Entry, Side, redact},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const ID: &str = "g12-startup-capture";
pub const USAGE: &str = "claude_startup_capture --program <absolute-path> --sha256 <hex> --version-label <label> --columns <20..200> --rows <5..100> --seconds <1..60> --out <new-directory> [--workspace-trust-control accept [--development-channel-control accept]]";

pub struct Options {
    pub program: PathBuf,
    pub sha256: String,
    pub version_label: String,
    pub size: TerminalSize,
    pub seconds: u64,
    pub out: PathBuf,
    pub accept_workspace_trust: bool,
    pub accept_development_channels: bool,
}
impl Options {
    pub fn parse(args: &[String]) -> Result<Self, String> {
        let mut values = std::collections::BTreeMap::new();
        for pair in args.chunks(2) {
            if pair.len() != 2
                || ![
                    "--program",
                    "--sha256",
                    "--version-label",
                    "--columns",
                    "--rows",
                    "--seconds",
                    "--out",
                    "--workspace-trust-control",
                    "--development-channel-control",
                ]
                .contains(&pair[0].as_str())
            {
                return Err(USAGE.into());
            }
            if values.insert(pair[0].as_str(), pair[1].as_str()).is_some() {
                return Err("duplicate capture option".into());
            }
        }
        let get = |key| values.get(key).copied().ok_or_else(|| USAGE.to_owned());
        let options = Self {
            program: get("--program")?.into(),
            sha256: get("--sha256")?.into(),
            version_label: get("--version-label")?.into(),
            out: get("--out")?.into(),
            accept_workspace_trust: match values.get("--workspace-trust-control").copied() {
                None => false,
                Some("accept") => true,
                _ => return Err("workspace trust control must be accept".into()),
            },
            accept_development_channels: match values.get("--development-channel-control").copied()
            {
                None => false,
                Some("accept") => true,
                _ => return Err("development channel control must be accept".into()),
            },
            size: TerminalSize {
                rows: get("--rows")?.parse().map_err(|_| USAGE.to_owned())?,
                columns: get("--columns")?.parse().map_err(|_| USAGE.to_owned())?,
            },
            seconds: get("--seconds")?.parse().map_err(|_| USAGE.to_owned())?,
        };
        options.validate()?;
        Ok(options)
    }
    fn validate(&self) -> Result<(), String> {
        if self.accept_development_channels && !self.accept_workspace_trust {
            return Err(
                "development channel control requires workspace trust control; no process started"
                    .into(),
            );
        }
        if !self.program.is_absolute()
            || !self.out.is_absolute()
            || self.sha256.len() != 64
            || !self
                .sha256
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
            || self.version_label.is_empty()
            || self.version_label.len() > 80
            || self.version_label.chars().any(char::is_control)
            || !(20..=200).contains(&self.size.columns)
            || !(5..=100).contains(&self.size.rows)
            || !(1..=60).contains(&self.seconds)
        {
            return Err("invalid capture options; no process started".into());
        }
        if self.accept_workspace_trust
            && (self.version_label != "2.1.284"
                || self.size.columns != 100 && self.size.columns != 140
                || self.size.rows != 24)
        {
            return Err(
                "trust control requires recorded 2.1.284 and 100x24 or 140x24; no process started"
                    .into(),
            );
        }
        Ok(())
    }
    pub fn input_limit(&self) -> usize {
        if self.accept_development_channels {
            3
        } else if self.accept_workspace_trust {
            2
        } else {
            0
        }
    }
}

pub fn digest(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err("capture program must be a file".into());
    }
    let mut hash = Sha256::new();
    let mut bytes = [0; 65536];
    loop {
        let n = file.read(&mut bytes).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hash.update(&bytes[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn write_line(file: &mut File, value: &Value) -> Result<(), String> {
    serde_json::to_writer(&mut *file, value).map_err(|e| e.to_string())?;
    file.write_all(b"\n")
        .and_then(|_| file.flush())
        .map_err(|e| e.to_string())
}
fn record(
    file: &mut File,
    redactor: &redact::Redactor,
    frame: &TerminalFrame,
) -> Result<(), String> {
    let mut text = String::new();
    let mut soft_wraps = Vec::new();
    for row in &frame.cells {
        let wrapped = row.iter().any(|cell| cell.wrap);
        soft_wraps.push(wrapped);
        let line = row
            .iter()
            .filter(|cell| cell.width != 0 && !cell.leading_spacer)
            .map(|cell| cell.text.as_str())
            .collect::<String>();
        // Soft wrapping splits one logical identifier across physical rows.
        // Preserve its complete bytes for redaction, including real spaces;
        // only a hard line boundary inserts a newline.
        text.push_str(if wrapped { &line } else { line.trim_end() });
        if !wrapped {
            text.push('\n');
        }
    }
    // Redact whole rendered text, not individual cells: identifiers can span cells.
    let entries = redactor.entries(&[Entry { from: Side::Backend, via: "screen".into(), msg: json!({"text":text, "soft_wraps":soft_wraps, "size":frame.size, "cursor":frame.cursor, "revision":frame.revision, "generation":frame.generation, "alternate_screen":frame.alternate_screen}) }]);
    if !redact::scan(&json!({}), &entries).is_empty() {
        return Err("startup screen failed the secret scan; not written".into());
    }
    write_line(
        file,
        &json!({"from":"backend", "via":"screen", "msg":entries[0].msg}),
    )
}
fn next(client: &mut ProbeClient, deadline: Instant) -> Result<ClientResponse, String> {
    client
        .recv_within(deadline.saturating_duration_since(Instant::now()))
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "capture connection closed".into())
}
fn capture(
    home: &Path,
    options: &Options,
    file: &mut File,
    redactor: &redact::Redactor,
    progress: &mut trust::Progress,
) -> Result<usize, String> {
    let (mut client, version) =
        ProbeClient::hello(&home.join(DAEMON_SOCKET), None).map_err(|e| e.to_string())?;
    if version < V1_4 {
        return Err("capture needs the full terminal capability".into());
    }
    client
        .send(&ClientRequest::SubscribeTerminalFrames {
            data: TerminalSubscribeData {
                request_id: "capture-subscribe".into(),
                instance_id: ID.into(),
                viewport: TerminalViewport {
                    top: None,
                    rows: options.size.rows,
                },
            },
        })
        .map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(15);
    let first = match next(&mut client, deadline)? {
        ClientResponse::TerminalFrame { data } => data,
        _ => return Err("capture subscription refused".into()),
    };
    if first.instance_id != ID || first.view_id.is_empty() || first.frame.generation.is_empty() {
        return Err("capture subscription identity changed".into());
    }
    // This view belongs to the probe's freshly created instance. Acquire is
    // for real PTY dimensions. Later Input requires its explicit bounded mode;
    // this diagnostic path never sends a daemon key.
    client
        .send(&ClientRequest::TerminalControl {
            data: ClientTerminalControlData {
                request_id: "capture-size".into(),
                instance_id: ID.into(),
                view_id: first.view_id.clone(),
                generation: first.frame.generation.clone(),
                operation: ClientTerminalOperation::Acquire { size: options.size },
            },
        })
        .map_err(|e| e.to_string())?;
    let mut count = 0;
    let acquired = loop {
        match next(&mut client, deadline)? {
            ClientResponse::TerminalControlAck { data } if data.request_id == "capture-size" => {
                break data;
            }
            ClientResponse::TerminalFrame { data } => {
                // Resizing can publish either the old or requested dimensions
                // before its ACK. All frames must still belong to this view.
                if data.instance_id != ID
                    || data.view_id != first.view_id
                    || data.frame.generation != first.frame.generation
                    || data.frame.size != first.frame.size && data.frame.size != options.size
                {
                    return Err("capture resize frame identity or size changed".into());
                }
            }
            ClientResponse::Error { .. } => return Err("capture resize refused".into()),
            _ => {
                if Instant::now() >= deadline {
                    return Err("capture resize deadline".into());
                }
            }
        }
    };
    if acquired.instance_id != ID
        || acquired.view_id != first.view_id
        || acquired.generation != first.frame.generation
    {
        return Err("capture resize identity changed".into());
    }
    let attach_id = match acquired.control {
        TerminalControlState::Controlled { attach_id } if !attach_id.is_empty() => attach_id,
        _ => return Err("capture did not acquire its terminal".into()),
    };
    let acquired = acquired.frame.ok_or("capture resize has no frame")?;
    if acquired.size != options.size || acquired.generation != first.frame.generation {
        return Err("capture size mismatch".into());
    }
    record(file, redactor, &acquired)?;
    count += 1;
    let generation = acquired.generation.clone();
    let mut revision = acquired.revision;
    let deadline = Instant::now() + Duration::from_secs(options.seconds);
    let workspace = home
        .join("workspace")
        .join(ID)
        .canonicalize()
        .map_err(|e| e.to_string())?;
    let mut observed = Some(acquired);
    while Instant::now() < deadline {
        match client.recv_within(
            deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(250)),
        ) {
            Ok(Some(ClientResponse::TerminalFrame { data })) => {
                if data.instance_id != ID
                    || data.view_id != first.view_id
                    || data.frame.generation != generation
                    || data.frame.size != options.size
                {
                    return Err("capture terminal identity or size changed".into());
                }
                if data.frame.revision != revision {
                    if count >= 512 {
                        return Err("capture frame limit reached".into());
                    }
                    record(file, redactor, &data.frame)?;
                    count += 1;
                    revision = data.frame.revision;
                    observed = Some(data.frame);
                }
            }
            Ok(Some(
                ClientResponse::Error { .. } | ClientResponse::TerminalControlChanged { .. },
            )) => return Err("capture lost its terminal or control".into()),
            Ok(None) => return Err("capture connection closed".into()),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(e) => return Err(e.to_string()),
            _ => {}
        }
        if Instant::now() >= deadline {
            break;
        }
        if options.accept_workspace_trust
            && let Some(frame) = observed.as_ref()
            && let Some((key, bytes)) =
                progress.decide(frame, &workspace, options.accept_development_channels)?
        {
            write_line(
                file,
                &json!({"from":"probe", "via":"operator-control", "msg":{"key":key, "phase":"intent", "generation":frame.generation, "revision":frame.revision}}),
            )?;
            file.sync_data().map_err(|e| e.to_string())?;
            let frames = trust::Grant {
                instance: ID,
                view: &first.view_id,
                attach: &attach_id,
            }
            .send(&mut client, frame, key, bytes, deadline)?;
            observed = frames.last().cloned();
            progress.completed += 1;
            write_line(
                file,
                &json!({"from":"probe", "via":"operator-control", "msg":{"key":key, "phase":"completed"}}),
            )?;
            for frame in frames {
                if count >= 512 {
                    return Err("capture frame limit reached".into());
                }
                record(file, redactor, &frame)?;
                count += 1;
                revision = frame.revision;
            }
            continue;
        }
    }
    if options.accept_workspace_trust && progress.completed != options.input_limit() {
        return Err("startup control incomplete; no input replay".into());
    }
    Ok(count)
}

pub fn run(options: &Options, agend: &Path) -> Result<usize, String> {
    options.validate()?;
    let program = options.program.canonicalize().map_err(|e| e.to_string())?;
    if digest(&program)? != options.sha256 {
        return Err("capture program hash mismatch; no process started".into());
    }
    // Refuse an existing destination instead of overwriting prior evidence.
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&options.out)
        .map_err(|e| e.to_string())?;
    let native = lab::Lab::with_prefix(agend, "g12-startup-capture");
    let home = native.home(1);
    let workspace = home.join("workspace").join(ID);
    fs::create_dir_all(&workspace).map_err(|e| e.to_string())?;
    let instance = Instance {
        id: ID.into(),
        backend: Backend::Claude,
        program: program.display().to_string(),
        args: vec![
            "--model".into(),
            "haiku".into(),
            "--effort".into(),
            "low".into(),
        ],
        working_directory: workspace.display().to_string(),
        session_id: Some(
            agend_daemon::store::instances::new_session_id().map_err(|e| e.to_string())?,
        ),
        status: InstanceStatus::New,
        session_started: false,
        agent_pid: None,
        legacy_no_thread: false,
        delivery: "push".into(),
    };
    {
        let store = SqliteStore::open(&home, 0).map_err(|e| e.to_string())?;
        block_on(store.add_instance(&instance)).map_err(|e| e.to_string())?;
    }
    let launch = agend_daemon::supervisor::launch(&home, &instance, false)?;
    let redactor = redact::Redactor::new(&native.root.canonicalize().map_err(|e| e.to_string())?);
    let mut header = redactor.value(&json!({"format":"startup-capture-v1", "backend":"claude", "version_label":options.version_label, "version_was_queried":false, "program":program.display().to_string(), "launch_args":launch.args, "requested_size":options.size, "duration_seconds":options.seconds, "terminal_input_operations":0, "message_delivery_operations":0, "startup":"not_assessed"}));
    if !redact::scan(&header, &[]).is_empty() {
        return Err("capture header failed the secret scan".into());
    }
    // The only unredacted identifier is the validated executable fingerprint.
    // It comes from the pinned file, not backend output or account metadata.
    header["program_sha256"] = options.sha256.clone().into();
    if options.accept_workspace_trust {
        header["capture_mode"] = if options.accept_development_channels {
            "development-channel-control"
        } else {
            "workspace-trust-control"
        }
        .into();
        header["terminal_input_operations"] = Value::Null;
        header["terminal_input_operation_limit"] = options.input_limit().into();
        header["production_daemon_key_path_tested"] = false.into();
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(options.out.join("screens.jsonl"))
        .map_err(|e| e.to_string())?;
    write_line(&mut file, &header)?;
    // Private lifecycle identifiers are kept separately from the redacted
    // transcript so cleanup does not race an external SQLite observer.
    let mut lifecycle = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(options.out.join("cleanup-identity.json"))
        .map_err(|e| e.to_string())?;
    write_line(
        &mut lifecycle,
        &json!({
            "format": "startup-cleanup-v1", "instance_id": ID,
            "session_id": instance.session_id,
            "home": home.canonicalize().map_err(|e| e.to_string())?,
            "workspace": workspace.canonicalize().map_err(|e| e.to_string())?
        }),
    )?;
    lifecycle.sync_all().map_err(|e| e.to_string())?;
    let mut progress = trust::Progress::default();
    let mut daemon = lab::Daemon::start(&native, &home, &[])?;
    let result = daemon
        .ready()
        .and_then(|_| capture(&home, options, &mut file, &redactor, &mut progress));
    // Remove while the daemon is live: production removal stops the holder,
    // sweeps this instance, and cannot schedule a replacement holder.
    let cleanup = (|| {
        let (mut operator, _) =
            ProbeClient::hello(&home.join(DAEMON_SOCKET), None).map_err(|e| e.to_string())?;
        match operator
            .request(&ClientRequest::Operator {
                data: OperatorData {
                    request_id: "capture-cleanup".into(),
                    command: OperatorCommand::InstanceRemove {
                        instance_id: ID.into(),
                    },
                },
            })
            .map_err(|e| e.to_string())?
        {
            ClientResponse::CommandResult { data } if data.result == CommandResult::Accepted => {
                Ok(())
            }
            _ => Err("capture instance cleanup refused".to_owned()),
        }
    })();
    let stopped = daemon.interrupt().map(|_| ());
    let unchanged = digest(&program).and_then(|hash| {
        if hash == options.sha256 {
            Ok(())
        } else {
            Err("capture program changed during capture".into())
        }
    });
    let final_result = result.and_then(|count| {
        cleanup?;
        stopped?;
        unchanged?;
        Ok(count)
    });
    let status = json!({"ok":final_result.is_ok(), "frames":final_result.as_ref().ok(), "startup":"not_assessed", "input_sent":if progress.started == 0 { Some(false) } else if progress.completed == progress.started { Some(true) } else { None }, "terminal_input_operations_started":progress.started, "terminal_input_operations_completed":progress.completed, "production_daemon_key_path_tested":false, "version_was_queried":false});
    let mut status_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(options.out.join("result.json"))
        .map_err(|e| e.to_string())?;
    write_line(&mut status_file, &status)?;
    final_result
}
