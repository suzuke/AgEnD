//! The sweep after a codex holder died (gate 7 P2): hanging up the PTY does
//! not always end the app-server and the TUI (one may ignore SIGHUP), and a
//! dead holder sends no SIGKILL. So, each time a holder is found dead (and
//! before any new holder starts while `agent_pid` is set), the daemon sends
//! one SIGKILL to the old agent's process group, only when all three hold:
//!
//! 1. `1 < pgid <= i32::MAX` (`killpg(1)` is `kill(-1)`; a larger `u32`
//!    would turn negative);
//! 2. the group still has processes;
//! 3. one of them has an argv element exactly equal to this instance's full
//!    socket path, `unix://` + that path, or the element after `resume`
//!    equal to its thread id. No substrings (`g7-1.codex.sock` is not
//!    `xg7-1.codex.sock`) and not `agend-codex` (the same for every
//!    instance).
//!
//! Known window (accepted, P2): between reading argv and `killpg`, every
//! process of the group could end and the pgid be reused.
//!
//! Must NOT: signal anything that fails a condition above, or signal a
//! single pid.

use std::fmt;

/// What identifies one instance's codex processes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Markers {
    /// `$AGEND_HOME/run/holders/<id>.codex.sock`, as given to the wrapper.
    pub socket: String,
    pub thread: Option<String>,
}

impl Markers {
    /// Whether `argv` is one of this instance's processes (condition 3).
    pub fn matches(&self, argv: &[String]) -> bool {
        let listen = format!("unix://{}", self.socket);
        argv.iter().any(|a| *a == self.socket || *a == listen)
            || self
                .thread
                .as_deref()
                .is_some_and(|thread| argv.windows(2).any(|w| w[0] == "resume" && w[1] == thread))
    }
}

/// What a sweep did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Swept {
    /// pgid ≤ 1 or > `i32::MAX`: nothing looked at, nothing sent.
    OutOfRange,
    /// The group has no processes.
    Gone,
    /// None of the group's `n` processes is this instance's.
    NoMatch(usize),
    /// SIGKILL sent to the group; its processes (pid, command) before.
    Killed(Vec<(u32, String)>),
}

impl fmt::Display for Swept {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Swept::OutOfRange => f.write_str("pgid out of range; nothing sent"),
            Swept::Gone => f.write_str("already gone"),
            Swept::NoMatch(n) => write!(f, "{n} process(es), none of this instance; nothing sent"),
            Swept::Killed(procs) => {
                let list: Vec<String> = procs.iter().map(|(p, c)| format!("{p} {c}")).collect();
                write!(
                    f,
                    "SIGKILL sent ({} left: {})",
                    procs.len(),
                    list.join("; ")
                )
            }
        }
    }
}

/// Condition 1.
pub fn in_range(pgid: u32) -> bool {
    pgid > 1 && pgid <= i32::MAX as u32
}

/// Sweeps process group `pgid` for `markers`' instance.
pub fn sweep(pgid: u32, markers: &Markers) -> Swept {
    if !in_range(pgid) {
        return Swept::OutOfRange;
    }
    let members = group_members(pgid);
    if members.is_empty() {
        return Swept::Gone;
    }
    let procs: Vec<(u32, Vec<String>)> = members
        .iter()
        .map(|&pid| (pid, argv(pid).unwrap_or_default()))
        .collect();
    if !procs.iter().any(|(_, a)| markers.matches(a)) {
        return Swept::NoMatch(procs.len());
    }
    // SAFETY: killpg on a positive pgid in (1, i32::MAX] checked above.
    unsafe { libc::killpg(pgid as libc::pid_t, libc::SIGKILL) };
    Swept::Killed(
        procs
            .into_iter()
            .map(|(pid, a)| (pid, a.join(" ")))
            .collect(),
    )
}

/// The pids in process group `pgid` (zombies included).
#[cfg(target_os = "macos")]
pub fn group_members(pgid: u32) -> Vec<u32> {
    const PROC_PGRP_ONLY: u32 = 2;
    let mut pids = vec![0 as libc::pid_t; 1024];
    let bytes = (pids.len() * std::mem::size_of::<libc::pid_t>()) as libc::c_int;
    // SAFETY: the buffer holds `bytes` bytes; proc_listpids writes at most that.
    let n = unsafe { libc::proc_listpids(PROC_PGRP_ONLY, pgid, pids.as_mut_ptr().cast(), bytes) };
    if n <= 0 {
        return Vec::new();
    }
    let count = n as usize / std::mem::size_of::<libc::pid_t>();
    pids[..count.min(pids.len())]
        .iter()
        .filter(|&&p| p > 0)
        .map(|&p| p as u32)
        .collect()
}

/// The pids in process group `pgid` (zombies included).
#[cfg(target_os = "linux")]
pub fn group_members(pgid: u32) -> Vec<u32> {
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|n| n.parse::<u32>().ok())
        else {
            continue;
        };
        let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else {
            continue;
        };
        // `pid (comm) state ppid pgrp …`; comm may hold spaces and ')'.
        let Some((_, rest)) = stat.rsplit_once(')') else {
            continue;
        };
        if rest
            .split_whitespace()
            .nth(2)
            .and_then(|g| g.parse::<u32>().ok())
            == Some(pgid)
        {
            out.push(pid);
        }
    }
    out.sort_unstable();
    out
}

/// The argv of `pid`; `None` when it cannot be read (gone, a zombie).
#[cfg(target_os = "macos")]
pub fn argv(pid: u32) -> Option<Vec<String>> {
    let pid = libc::c_int::try_from(pid).ok()?;
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
    let mut size: libc::size_t = 0;
    // SAFETY: a size query (null buffer) for a valid mib.
    let rc = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            std::ptr::null_mut(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 || size < 4 {
        return None;
    }
    let mut buf = vec![0u8; size];
    // SAFETY: `buf` holds `size` bytes.
    let rc = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            buf.as_mut_ptr().cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 || size < 4 {
        return None;
    }
    buf.truncate(size);
    // argc, the exec path, NUL padding, then argc NUL-terminated strings.
    let argc = i32::from_ne_bytes(buf[..4].try_into().ok()?) as usize;
    let rest = &buf[4..];
    let path_end = rest.iter().position(|&b| b == 0)?;
    let mut at = path_end;
    while rest.get(at) == Some(&0) {
        at += 1;
    }
    let args: Vec<String> = rest[at..]
        .split(|&b| b == 0)
        .take(argc)
        .map(|a| String::from_utf8_lossy(a).into_owned())
        .collect();
    (args.len() == argc).then_some(args)
}

/// The argv of `pid`; `None` when it cannot be read (gone, a zombie).
#[cfg(target_os = "linux")]
pub fn argv(pid: u32) -> Option<Vec<String>> {
    let raw = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    if raw.is_empty() {
        return None;
    }
    let raw = raw.strip_suffix(&[0]).unwrap_or(&raw);
    Some(
        raw.split(|&b| b == 0)
            .map(|a| String::from_utf8_lossy(a).into_owned())
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::CommandExt;
    use std::process::{Child, Command};
    use std::time::{Duration, Instant};

    const SOCK: &str = "/tmp/g7-sweep/run/holders/g7-1.codex.sock";

    fn markers() -> Markers {
        Markers {
            socket: SOCK.into(),
            thread: Some("thread-1".into()),
        }
    }

    /// A process group of our own: `sh` (with `args` in its argv) and a
    /// `sleep` child.
    fn group(args: &[&str]) -> Child {
        Command::new("/bin/sh")
            .args(["-c", "sleep 60; :"])
            .args(args)
            .process_group(0)
            .spawn()
            .unwrap()
    }

    fn members_within(pgid: u32, want: usize) -> Vec<u32> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let m = group_members(pgid);
            if m.len() >= want || Instant::now() > deadline {
                return m;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn markers_match_whole_elements_only() {
        let m = markers();
        let s = |v: &[&str]| v.iter().map(|x| (*x).to_owned()).collect::<Vec<_>>();
        assert!(m.matches(&s(&["sh", "-c", "x", "agend-codex", "codex", SOCK])));
        assert!(m.matches(&s(&[
            "codex",
            "app-server",
            "--listen",
            &format!("unix://{SOCK}")
        ])));
        assert!(m.matches(&s(&[
            "codex",
            "-c",
            "t",
            "resume",
            "thread-1",
            "--remote",
            "unix:///x"
        ])));
        let other = SOCK.replace("g7-1", "xg7-1");
        assert!(!m.matches(&s(&["codex", "--listen", &format!("unix://{other}")])));
        assert!(!m.matches(&s(&["sh", "agend-codex", "codex"])));
        assert!(!m.matches(&s(&["codex", "resume", "thread-10"])));
        assert!(!m.matches(&s(&["echo", "thread-1", "resume"])));
    }

    #[test]
    fn out_of_range_pgids_are_never_signalled() {
        for pgid in [0, 1, i32::MAX as u32 + 1, u32::MAX] {
            assert_eq!(sweep(pgid, &markers()), Swept::OutOfRange, "{pgid}");
        }
    }

    #[test]
    fn argv_reads_every_element_of_our_own_child() {
        let mut child = group(&["agend-codex", "a b", SOCK]);
        let deadline = Instant::now() + Duration::from_secs(5);
        let args = loop {
            match argv(child.id()) {
                Some(a) if a.len() == 6 => break a,
                _ if Instant::now() > deadline => panic!("argv of {}", child.id()),
                _ => std::thread::sleep(Duration::from_millis(20)),
            }
        };
        child.kill().unwrap();
        child.wait().unwrap();
        assert_eq!(args[1..], ["-c", "sleep 60; :", "agend-codex", "a b", SOCK]);
    }

    /// The group of an unrelated program (no marker) is left alone; our own
    /// test group with the marker is killed, both its processes.
    #[test]
    fn only_a_group_with_the_marker_is_killed() {
        let mut unrelated = group(&["agend-codex", "codex", &SOCK.replace("g7-1", "xg7-1")]);
        let pgid = unrelated.id();
        assert_eq!(members_within(pgid, 2).len(), 2);
        assert_eq!(sweep(pgid, &markers()), Swept::NoMatch(2));
        std::thread::sleep(Duration::from_millis(200));
        assert!(
            unrelated.try_wait().unwrap().is_none(),
            "unrelated group signalled"
        );
        unrelated.kill().unwrap();
        unrelated.wait().unwrap();

        let mut ours = group(&["agend-codex", "codex", SOCK]);
        let pgid = ours.id();
        assert_eq!(members_within(pgid, 2).len(), 2);
        let Swept::Killed(procs) = sweep(pgid, &markers()) else {
            panic!("not killed")
        };
        assert_eq!(procs.len(), 2, "{procs:?}");
        let status = ours.wait().unwrap();
        assert_eq!(
            std::os::unix::process::ExitStatusExt::signal(&status),
            Some(libc::SIGKILL)
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while !group_members(pgid).is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(sweep(pgid, &markers()), Swept::Gone);
    }
}
