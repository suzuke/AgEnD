//! The holder owns a fixed shell wrapper, its loopback server and attached TUI.
//! The wrapper remains alive as a process-group identity marker. Settings and
//! credentials live only in the instance's private AgEnD directory.
use crate::store::{Instance, instances::validate_id};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

pub const WRAPPER_NAME: &str = "agend-opencode";

/// Model selection belongs in the session API, never in `serve` or `attach`.
/// Other options need an explicit mapping before they can be accepted.
pub fn model(args: &[String]) -> Result<Option<(String, String)>, String> {
    let value = match args {
        [] => return Ok(None),
        [flag, value] if flag == "--model" || flag == "-m" => value.as_str(),
        [arg] if arg.starts_with("--model=") => &arg[8..],
        _ => {
            return Err(
                "OpenCode push supports only --model provider/model in instance args".into(),
            );
        }
    };
    let (provider, model) = value
        .split_once('/')
        .filter(|(p, m)| !p.is_empty() && !m.is_empty())
        .ok_or("OpenCode model must be provider/model")?;
    Ok(Some((provider.into(), model.into())))
}
pub const WRAPPER: &str = r#"set -eu
c=$1 d=$2
IFS= read -r OPENCODE_SERVER_PASSWORD <"$d/password"
IFS= read -r port <"$d/port"
export OPENCODE_SERVER_PASSWORD OPENCODE_SERVER_USERNAME=agend
export XDG_DATA_HOME="$d/data" XDG_CONFIG_HOME="$d/config"
export XDG_CACHE_HOME="$d/cache" XDG_STATE_HOME="$d/state"
export OPENCODE_CONFIG_CONTENT='{"autoupdate":false,"plugin":[]}'
umask 077
v=$("$c" --version)
printf '%s\n%s\n' "$PPID" "$v" >"$d/version.tmp"
mv "$d/version.tmp" "$d/version"
exec 3<&0
"$c" serve --pure --hostname 127.0.0.1 --port "$port" >"$d/server.log" 2>&1 &
s=$! t=
cleanup() {
  if [ -n "$t" ]; then kill -TERM "$t" 2>/dev/null || :; fi
  kill -TERM "$s" 2>/dev/null || :
  wait "$s" 2>/dev/null || :
}
trap cleanup 0
trap 'exit 1' HUP INT TERM
i=0
while [ ! -s "$d/go" ]; do
  kill -0 "$s" 2>/dev/null || exit 1
  i=$((i+1))
  if [ "$i" -gt 600 ]; then echo 'agend-opencode: session handoff timed out' >&2; exit 1; fi
  sleep 0.1
done
{ IFS= read -r session; IFS= read -r url; } <"$d/go"
"$c" attach "$url" --pure --session "$session" --dir "$PWD" <&3 &
t=$!
wait "$t"
"#;

#[derive(Clone)]
pub struct Layout {
    root: PathBuf,
}

fn private_dir(path: &Path) -> io::Result<()> {
    match fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            let metadata = fs::symlink_metadata(path)?;
            // SAFETY: getuid has no preconditions.
            if !metadata.is_dir()
                || metadata.uid() != unsafe { libc::getuid() }
                || metadata.mode() & 0o077 != 0
            {
                return Err(io::Error::other(
                    "OpenCode directory is not private and owned",
                ));
            }
            Ok(())
        }
        Err(e) => Err(e),
    }
}

fn read_private(path: &Path, limit: u64) -> io::Result<Vec<u8>> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    let metadata = file.metadata()?;
    // SAFETY: getuid has no preconditions.
    if !metadata.is_file()
        || metadata.uid() != unsafe { libc::getuid() }
        || metadata.mode() & 0o077 != 0
        || metadata.len() > limit
    {
        return Err(io::Error::other(
            "OpenCode file is not private, owned and bounded",
        ));
    }
    let mut bytes = Vec::new();
    (&mut file).take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(io::Error::other("OpenCode file exceeds limit"));
    }
    Ok(bytes)
}

impl Layout {
    pub fn new(home: &Path, id: &str) -> Result<Self, String> {
        validate_id(id)?;
        Ok(Self {
            root: home.join("opencode").join(id),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Only after the previous holder is gone and its process group is swept.
    /// Persistent backend data survives. Credentials rotate for a new holder:
    /// an old in-flight request must not authenticate to a reused TCP port.
    pub fn prepare(&self) -> io::Result<()> {
        private_dir(self.root.parent().expect("instance parent"))?;
        private_dir(&self.root)?;
        for name in ["data", "config", "cache", "state"] {
            private_dir(&self.root.join(name))?;
        }
        let password = self.root.join("password");
        if password.try_exists()? {
            self.password()?;
        }
        let mut random = [0u8; 32];
        File::open("/dev/urandom")?.read_exact(&mut random)?;
        let value: String = random.iter().map(|b| format!("{b:02x}")).collect();
        let temp = self.root.join(format!(
            ".password-{}",
            crate::store::instances::new_session_id()?
        ));
        let result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temp)?;
            writeln!(file, "{value}")?;
            file.sync_all()?;
            // A dangling symlink must also be preserved, never replaced.
            match fs::symlink_metadata(&password) {
                Ok(meta) if meta.is_file() => {
                    self.password()?;
                    fs::rename(&temp, &password)?;
                }
                Ok(_) => return Err(io::Error::other("unexpected OpenCode credential entry")),
                Err(e) if e.kind() == io::ErrorKind::NotFound => fs::hard_link(&temp, &password)?,
                Err(e) => return Err(e),
            }
            File::open(&self.root)?.sync_all()
        })();
        let _ = fs::remove_file(temp);
        result?;
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
        let port = listener.local_addr()?.port();
        drop(listener);
        let port_path = self.root.join("port");
        if port_path.try_exists()? {
            read_private(&port_path, 16)?;
            fs::remove_file(&port_path)?;
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(port_path)?;
        writeln!(file, "{port}")?;
        file.sync_all()?;
        for name in ["go", "version", "version.tmp", "server.log"] {
            let path = self.root.join(name);
            match fs::symlink_metadata(&path) {
                Ok(meta) if meta.is_file() => {
                    fs::remove_file(path)?;
                }
                Ok(_) => {
                    return Err(io::Error::other(
                        "unexpected OpenCode runtime entry; preserved",
                    ));
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => (),
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    pub fn password(&self) -> io::Result<String> {
        let bytes = read_private(&self.root.join("password"), 65)?;
        let text = String::from_utf8(bytes).map_err(io::Error::other)?;
        let password = text
            .strip_suffix('\n')
            .ok_or_else(|| io::Error::other("invalid OpenCode credential"))?;
        if password.len() != 64 || !password.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(io::Error::other("invalid OpenCode credential"));
        }
        Ok(password.into())
    }

    /// The attached session must match the durable instance, not merely share
    /// the same REST server. A missing handoff is not a ready session.
    pub fn session_endpoint(&self, holder_pid: u32, session: &str) -> io::Result<(u16, String)> {
        let endpoint = self.endpoint(holder_pid)?;
        let go = String::from_utf8(read_private(&self.root.join("go"), 1024)?)
            .map_err(io::Error::other)?;
        let expected = format!("{session}\nhttp://127.0.0.1:{}\n", endpoint.0);
        if go != expected {
            return Err(io::Error::other("OpenCode attached session mismatch"));
        }
        Ok(endpoint)
    }

    /// A startup record must belong to the currently locked holder, not an old
    /// version of the executable found on PATH after a daemon restart.
    pub fn endpoint(&self, holder_pid: u32) -> io::Result<(u16, String)> {
        let record = String::from_utf8(read_private(&self.root.join("version"), 1024)?)
            .map_err(io::Error::other)?;
        let mut lines = record.lines();
        if lines.next().and_then(|s| s.parse::<u32>().ok()) != Some(holder_pid) {
            return Err(io::Error::other("OpenCode holder generation mismatch"));
        }
        let version = lines
            .next()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| io::Error::other("missing OpenCode version"))?;
        if lines.next().is_some() {
            return Err(io::Error::other("invalid OpenCode version record"));
        }
        if self.root.join("go").try_exists()? {
            let go = String::from_utf8(read_private(&self.root.join("go"), 1024)?)
                .map_err(io::Error::other)?;
            let mut lines = go.lines();
            let session = lines.next().unwrap_or_default();
            let port = lines
                .next()
                .and_then(|s| s.strip_prefix("http://127.0.0.1:"))
                .and_then(|s| s.parse::<u16>().ok())
                .filter(|p| *p != 0);
            if !super::history::valid_id(session, "ses") || lines.next().is_some() || port.is_none()
            {
                return Err(io::Error::other("invalid OpenCode published endpoint"));
            }
            return Ok((port.expect("checked"), version.into()));
        }
        let log = String::from_utf8(read_private(&self.root.join("server.log"), 1024 * 1024)?)
            .map_err(io::Error::other)?;
        let ports: Vec<_> = log
            .lines()
            .filter_map(|line| {
                line.strip_prefix("opencode server listening on http://127.0.0.1:")
                    .and_then(|port| port.parse::<u16>().ok())
                    .filter(|p| *p != 0)
            })
            .collect();
        if ports.len() != 1 {
            return Err(io::Error::other("OpenCode endpoint not ready or ambiguous"));
        }
        Ok((ports[0], version.into()))
    }

    pub fn handoff(&self, session: &str, port: u16) -> io::Result<()> {
        if !super::history::valid_id(session, "ses") || port == 0 {
            return Err(io::Error::other("invalid OpenCode handoff"));
        }
        let bytes = format!("{session}\nhttp://127.0.0.1:{port}\n");
        let go = self.root.join("go");
        if go.try_exists()? {
            if read_private(&go, 1024)? == bytes.as_bytes() {
                return Ok(());
            }
            return Err(io::Error::other(
                "OpenCode handoff already names another session",
            ));
        }
        let temp = self.root.join(format!(
            ".go-{}",
            crate::store::instances::new_session_id()?
        ));
        let result = (|| {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temp)?;
            file.write_all(bytes.as_bytes())?;
            file.sync_all()?;
            fs::hard_link(&temp, &go)?;
            File::open(&self.root)?.sync_all()
        })();
        let _ = fs::remove_file(temp);
        result
    }

    pub fn args(&self, instance: &Instance) -> Vec<String> {
        vec![
            "-c".into(),
            WRAPPER.into(),
            WRAPPER_NAME.into(),
            instance.program.clone(),
            self.root.display().to_string(),
        ]
    }

    /// Called only after holder death is established by the supervisor.
    pub fn sweep(&self, pgid: u32, program: &str) -> crate::driver::codex::sweep::Swept {
        let port = read_private(&self.root.join("port"), 16)
            .ok()
            .and_then(|b| String::from_utf8(b).ok())
            .and_then(|s| s.trim().parse::<u16>().ok())
            .filter(|p| *p != 0);
        let attached = read_private(&self.root.join("go"), 1024)
            .ok()
            .and_then(|b| String::from_utf8(b).ok())
            .and_then(|text| {
                let mut lines = text.lines();
                let session = lines.next()?.to_owned();
                let url = lines.next()?.to_owned();
                if !super::history::valid_id(&session, "ses")
                    || lines.next().is_some()
                    || port.is_none_or(|p| url != format!("http://127.0.0.1:{p}"))
                {
                    return None;
                }
                Some((session, url))
            });
        let root = self.root.display().to_string();
        crate::driver::codex::sweep::sweep_matching(pgid, |argv| {
            argv.windows(4)
                .any(|w| w[0] == WRAPPER && w[1] == WRAPPER_NAME && w[2] == program && w[3] == root)
                || (argv.first().is_some_and(|a| a == program)
                    && attached.as_ref().is_some_and(|(session, url)| {
                        argv.windows(2).any(|w| w[0] == "attach" && w[1] == *url)
                            && argv
                                .windows(2)
                                .any(|w| w[0] == "--session" && w[1] == *session)
                    }))
                || (argv.first().is_some_and(|a| a == program)
                    && argv.iter().any(|a| a == "serve")
                    && argv
                        .windows(2)
                        .any(|w| w[0] == "--hostname" && w[1] == "127.0.0.1")
                    && port.is_some_and(|port| {
                        argv.windows(2)
                            .any(|w| w[0] == "--port" && w[1] == port.to_string())
                    }))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agend_testkit::tempdir::TempDir;
    use std::os::unix::fs::{PermissionsExt, symlink};
    use std::process::Command;
    use std::time::{Duration, Instant};

    fn write(path: &Path, value: &str) {
        fs::write(path, value).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).unwrap();
    }

    #[test]
    fn preparation_rotates_credentials_preserves_data_and_rejects_foreign_links() {
        let dir = TempDir::new("opencode-launch").unwrap();
        let layout = Layout::new(dir.path(), "open-1").unwrap();
        layout.prepare().unwrap();
        let password = layout.password().unwrap();
        write(&layout.root.join("data/session.db"), "persistent session");
        layout.handoff("ses_native", 12345).unwrap();
        layout.handoff("ses_native", 12345).unwrap();
        assert!(layout.handoff("ses_other", 12345).is_err());
        layout.prepare().unwrap();
        assert_ne!(layout.password().unwrap(), password);
        assert_eq!(
            fs::read_to_string(layout.root.join("data/session.db")).unwrap(),
            "persistent session"
        );
        assert!(!layout.root.join("go").exists());
        fs::remove_file(layout.root.join("password")).unwrap();
        let foreign = dir.path().join("foreign");
        write(&foreign, "leave me alone");
        symlink(&foreign, layout.root.join("password")).unwrap();
        assert!(layout.prepare().is_err());
        assert_eq!(fs::read_to_string(foreign).unwrap(), "leave me alone");
        assert!(Layout::new(dir.path(), "../escape").is_err());
    }

    #[test]
    fn endpoint_is_holder_bound_and_survives_log_growth_after_handoff() {
        let dir = TempDir::new("opencode-endpoint").unwrap();
        let layout = Layout::new(dir.path(), "open-1").unwrap();
        layout.prepare().unwrap();
        write(&layout.root.join("version"), "12345\n1.18.34\n");
        write(
            &layout.root.join("server.log"),
            "opencode server listening on http://127.0.0.1:4096\n",
        );
        assert_eq!(layout.endpoint(12345).unwrap(), (4096, "1.18.34".into()));
        assert!(layout.endpoint(54321).is_err());
        layout.handoff("ses_native", 4096).unwrap();
        write(
            &layout.root.join("server.log"),
            &"x".repeat(2 * 1024 * 1024),
        );
        assert_eq!(layout.endpoint(12345).unwrap().0, 4096);
    }

    #[test]
    fn real_shell_waits_for_handoff_and_keeps_secret_out_of_attach_arguments() {
        let dir = TempDir::new("opencode-wrapper").unwrap();
        let layout = Layout::new(dir.path(), "open-1").unwrap();
        layout.prepare().unwrap();
        let fake = dir.path().join("backend with spaces");
        fs::write(
            &fake,
            r#"#!/bin/sh
case "$1" in
--version) echo 1.18.34;;
serve) echo 'opencode server listening on http://127.0.0.1:45678'; exec sleep 60;;
attach) printf '%s\n' "$@" >"$XDG_STATE_HOME/attach-args"; test -n "$OPENCODE_SERVER_PASSWORD";;
*) exit 2;;
esac
"#,
        )
        .unwrap();
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o700)).unwrap();
        let mut child = Command::new("/bin/sh")
            .args(["-c", WRAPPER, WRAPPER_NAME])
            .arg(fake)
            .arg(layout.root())
            .current_dir(dir.path())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while layout.endpoint(std::process::id()).is_err() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let ready = layout.endpoint(std::process::id());
        // Always unblock the wrapper so a failed readiness assertion cannot
        // leave the test's backend running for the full startup deadline.
        layout.handoff("ses_native", 45678).unwrap();
        let status = child.wait().unwrap();
        assert!(ready.is_ok(), "{ready:?}");
        assert!(status.success());
        let args = fs::read_to_string(layout.root.join("state/attach-args")).unwrap();
        assert!(args.contains("http://127.0.0.1:45678\n--pure\n--session\nses_native\n--dir\n"));
        assert!(!args.contains(&layout.password().unwrap()));
    }
}
