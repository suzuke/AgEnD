//! Bounded native service-manager adapter. Never use a shell or a global stop.

use std::io::Read;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use agend_core::setup::service::{Installation, LAUNCHD_LABEL, Manager, SYSTEMD_UNIT};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Absent,
    Owned { running: bool },
}

pub trait ServiceManager {
    fn inspect(&self, record: &Installation) -> Result<State, String>;
    fn start(&self, record: &Installation) -> Result<(), String>;
    fn stop(&self, record: &Installation) -> Result<(), String>;
    fn reload(&self, record: &Installation) -> Result<(), String>;
}

#[cfg(target_os = "macos")]
mod launchd;
mod systemd;

pub struct Native;

impl ServiceManager for Native {
    fn inspect(&self, record: &Installation) -> Result<State, String> {
        match record.spec.manager {
            Manager::Launchd => {
                let reply = run(Manager::Launchd, &["print", &target()])?;
                if reply.code == 113 && reply.stderr.contains("Could not find service") {
                    return Ok(State::Absent);
                }
                if reply.code != 0 {
                    return Err(format!(
                        "launchctl print failed (exit {}); no ownership assumed",
                        reply.code
                    ));
                }
                if !reply
                    .stdout
                    .lines()
                    .any(|line| line.trim() == format!("path = {}", record.service_path))
                {
                    return Err(
                        "launchd label belongs to another service path; preserving it".into(),
                    );
                }
                let pid = reply.stdout.lines().find_map(|line| {
                    line.trim()
                        .strip_prefix("pid = ")
                        .and_then(|v| v.parse::<u32>().ok())
                        .filter(|p| *p > 1)
                });
                if let Some(pid) = pid {
                    #[cfg(target_os = "macos")]
                    launchd::live_process(record, pid)?;
                    #[cfg(not(target_os = "macos"))]
                    {
                        let _ = pid;
                        return Err("launchd requires macOS".into());
                    }
                }
                Ok(State::Owned {
                    running: pid.is_some(),
                })
            }
            Manager::Systemd => {
                let reply = run(
                    Manager::Systemd,
                    &[
                        "--user",
                        "show",
                        SYSTEMD_UNIT,
                        "--property=LoadState",
                        "--property=FragmentPath",
                        "--property=ActiveState",
                        "--property=MainPID",
                    ],
                )?;
                let prop = |name: &str| reply.stdout.lines().find_map(|l| l.strip_prefix(name));
                if prop("LoadState=") == Some("not-found") && prop("FragmentPath=") == Some("") {
                    return Ok(State::Absent);
                }
                if reply.code != 0 {
                    return Err(format!(
                        "systemctl show failed (exit {}); no ownership assumed",
                        reply.code
                    ));
                }
                if prop("LoadState=") != Some("loaded")
                    || prop("FragmentPath=") != Some(record.service_path.as_str())
                {
                    return Err("systemd unit belongs to another service path or cannot be loaded; preserving it".into());
                }
                systemd::inspect(record)
            }
        }
    }

    fn start(&self, record: &Installation) -> Result<(), String> {
        match record.spec.manager {
            Manager::Launchd => match self.inspect(record)? {
                State::Absent => success(
                    run(
                        Manager::Launchd,
                        &["bootstrap", &domain(), &record.service_path],
                    )?,
                    "launchctl bootstrap",
                ),
                State::Owned { .. } => Ok(()),
            },
            Manager::Systemd => {
                self.reload(record)?;
                // Reload must resolve our exact file before enable/start by name.
                if !matches!(self.inspect(record)?, State::Owned { .. }) {
                    return Err("systemd did not load the owned unit; nothing started".into());
                }
                success(
                    run(
                        Manager::Systemd,
                        &["--user", "enable", "--no-reload", "--now", SYSTEMD_UNIT],
                    )?,
                    "systemctl enable",
                )
            }
        }
    }

    fn stop(&self, record: &Installation) -> Result<(), String> {
        match self.inspect(record)? {
            State::Absent => return Ok(()),
            State::Owned { .. } => {}
        }
        let reply = match record.spec.manager {
            Manager::Launchd => run(Manager::Launchd, &["bootout", &target()])?,
            Manager::Systemd => run(
                Manager::Systemd,
                &["--user", "disable", "--no-reload", "--now", SYSTEMD_UNIT],
            )?,
        };
        success(reply, "service removal")?;
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            match self.inspect(record)? {
                State::Absent | State::Owned { running: false } => return Ok(()),
                State::Owned { running: true } if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(50))
                }
                _ => return Err("service is still running; files and data preserved".into()),
            }
        }
    }

    fn reload(&self, record: &Installation) -> Result<(), String> {
        match record.spec.manager {
            Manager::Launchd => Ok(()),
            Manager::Systemd => success(
                run(Manager::Systemd, &["--user", "daemon-reload"])?,
                "systemctl daemon-reload",
            ),
        }
    }
}

fn domain() -> String {
    // SAFETY: getuid returns this process's user id without mutation.
    format!("gui/{}", unsafe { libc::getuid() })
}
fn target() -> String {
    format!("{}/{LAUNCHD_LABEL}", domain())
}

struct Reply {
    code: i32,
    stdout: String,
    stderr: String,
}
fn success(reply: Reply, operation: &str) -> Result<(), String> {
    if reply.code == 0 {
        Ok(())
    } else {
        Err(format!(
            "{operation} failed (exit {}); retry to reconcile owned installation",
            reply.code
        ))
    }
}

fn run(manager: Manager, args: &[&str]) -> Result<Reply, String> {
    let program = match manager {
        Manager::Launchd => "/bin/launchctl",
        Manager::Systemd => "/usr/bin/systemctl",
    };
    run_program(program, args)
}

fn run_program(program: &str, args: &[&str]) -> Result<Reply, String> {
    let mut child = Command::new(program)
        .args(args)
        .env_remove("AGEND_INSTANCE")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
        .map_err(|e| format!("cannot run {program}: {e}"))?;
    let pid = child.id();
    let deadline = Instant::now() + Duration::from_secs(20);
    let (tx, rx) = mpsc::channel();
    let stdout = child.stdout.take().expect("piped");
    let stderr = child.stderr.take().expect("piped");
    for (index, pipe) in [
        (0, Box::new(stdout) as Box<dyn Read + Send>),
        (1, Box::new(stderr) as Box<dyn Read + Send>),
    ] {
        let tx = tx.clone();
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let result = pipe.take(131073).read_to_end(&mut bytes);
            let _ = tx.send((index, result.map(|_| bytes)));
        });
    }
    drop(tx);
    let mut reaped = false;
    let result = (|| {
        let status = loop {
            if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                reaped = true;
                break status;
            }
            if Instant::now() >= deadline {
                return Err("service manager timed out; retry to reconcile state".into());
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        let mut output = [String::new(), String::new()];
        for _ in 0..2 {
            let (index, bytes) = rx
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .map_err(|_| "service manager output timed out")?;
            let bytes = bytes.map_err(|e| e.to_string())?;
            if bytes.len() > 131072 {
                return Err("service manager output exceeded 128 KiB".into());
            }
            output[index] =
                String::from_utf8(bytes).map_err(|_| "service manager output is not UTF-8")?;
        }
        Ok(Reply {
            code: status.code().unwrap_or(-1),
            stdout: output[0].clone(),
            stderr: output[1].clone(),
        })
    })();
    if result.is_err() && !reaped {
        // SAFETY: this is the isolated process group just spawned by this call.
        // No service daemon is a child of launchctl/systemctl; only this command
        // and its helpers are stopped on timeout/output failure.
        unsafe {
            libc::kill(-(pid as libc::pid_t), libc::SIGKILL);
        }
        let _ = child.wait();
    }
    result
}
