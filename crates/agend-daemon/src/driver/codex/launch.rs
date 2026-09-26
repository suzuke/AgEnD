//! How a codex instance runs in its holder (gate 7 P2, P4): the holder's one
//! PTY child is a small fixed `sh` wrapper ([`WRAPPER`]). It starts
//! `codex app-server` in the background (same process group and session as
//! the TUI: a non-interactive `sh` has no job control), waits for the
//! handoff file `$GO` the daemon writes once the thread exists, then `exec`s
//! the TUI `codex resume <thread> --remote unix://<real socket>`. No holder
//! protocol change (no `SpawnSidecar`).
//!
//! - Paths and settings are positional parameters (never pasted into the
//!   script, never in the environment): `$1` codex, `$2` the socket
//!   `$AGEND_HOME/run/holders/<id>.codex.sock`, `$3` `$GO`
//!   (`…/<id>.codex-go`), `$4` the holder log, `$5` the trust `-c` value;
//!   the instance's own arguments follow and go to `app-server`. `$0` is
//!   [`WRAPPER_NAME`] (the same for every instance: only for `ps`).
//! - Settings are per-launch `-c` options before the subcommand; the daemon
//!   never writes `~/.codex` (P4). The TUI gets only trust and the update
//!   check (v1: `resume --remote` refused permission overrides).
//! - `$GO`: line 1 the thread id, line 2 the resolved socket; written to a
//!   temp file and renamed ([`write_go`]), so the wrapper never reads half.
//!   The wrapper waits for it at most [`GO_WITHIN`] (> 20 s ready + 30 s
//!   thread), then exits 1 (a death, gate 6 P6).
//! - Before every `Spawn` the old socket (and the codex socket it links to
//!   under `/private/tmp/codex-daemon-<uid>/`) and the old `$GO` are removed
//!   ([`prepare`]).
//! - Shims under a login zsh (P4 option A): `ZDOTDIR=$AGEND_HOME/zsh`, whose
//!   `.zprofile` ([`ZPROFILE`]) runs after `/etc/zprofile` (`path_helper`)
//!   and puts `$AGEND_HOME/bin` first again ([`ensure_zdotdir`]).
//!
//! Must NOT: put a path or setting into the script text, or write anything
//! under `~/.codex`.

use std::fs;
use std::io;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::runtime::files;
use crate::store::Instance;

/// The wrapper's program.
pub const SHELL: &str = "/bin/sh";
/// The wrapper's `$0`.
pub const WRAPPER_NAME: &str = "agend-codex";

/// The fixed wrapper script (gate 7 P2).
pub const WRAPPER: &str = r#"c=$1 s=$2 g=$3 l=$4 t=$5
shift 5
"$c" -c "$t" -c check_for_update_on_startup=false -c 'approval_policy="never"' -c 'sandbox_mode="danger-full-access"' app-server --listen "unix://$s" "$@" >>"$l" 2>&1 &
i=0
while [ ! -s "$g" ]; do
  i=$((i + 1))
  if [ "$i" -gt 600 ]; then echo "agend-codex: no thread id within 60 s" >&2; exit 1; fi
  sleep 0.1
done
{ IFS= read -r th; IFS= read -r rs; } <"$g"
exec "$c" -c "$t" -c check_for_update_on_startup=false resume "$th" --remote "unix://$rs"
"#;

/// How long the wrapper waits for `$GO` (600 × 0.1 s in [`WRAPPER`]).
pub const GO_WITHIN: Duration = Duration::from_secs(60);
/// How long the driver retries the app-server before it counts as a death
/// (v1 `ready_timeout_secs`).
pub const READY_WITHIN: Duration = Duration::from_secs(20);
/// `thread/start` and `thread/resume` each.
pub const THREAD_WITHIN: Duration = Duration::from_secs(30);

/// The `ZDOTDIR` of codex agents, inside the AgEnD home.
pub const ZDOTDIR: &str = "zsh";
/// `$ZDOTDIR/.zprofile`: after `/etc/zprofile`, the shims come first again.
pub const ZPROFILE: &str = "\
# Written by agend (gate 7 P4). codex runs every command with `zsh -lc`; on
# macOS /etc/zprofile (path_helper) moves the shims in $AGEND_HOME/bin
# behind /usr/bin, so they are put first again here, last.
export PATH=\"$AGEND_HOME/bin:$PATH\"
";

/// `run/holders/<id>.codex.sock`: the app-server's `--listen` path.
pub fn socket_path(home: &Path, id: &str) -> PathBuf {
    files::holders_dir(home).join(format!("{id}.codex.sock"))
}

/// `run/holders/<id>.codex-go`: the handoff file.
pub fn go_path(home: &Path, id: &str) -> PathBuf {
    files::holders_dir(home).join(format!("{id}.codex-go"))
}

/// A TOML basic string.
fn toml_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The trust `-c` value for a working directory (already resolved):
/// `projects={"<dir>"={trust_level="trusted"}}` (v1 #3402).
pub fn trust_value(real_workdir: &str) -> String {
    format!(
        "projects={{{}={{trust_level=\"trusted\"}}}}",
        toml_string(real_workdir)
    )
}

/// The resolved working directory of `instance`.
pub fn real_workdir(instance: &Instance) -> Result<String, String> {
    fs::canonicalize(&instance.working_directory)
        .map(|p| p.display().to_string())
        .map_err(|e| format!("working directory {}: {e}", instance.working_directory))
}

/// The wrapper's arguments (after [`SHELL`]) for `instance` under `home`.
pub fn wrapper_args(home: &Path, instance: &Instance) -> Result<Vec<String>, String> {
    let id = &instance.id;
    let mut args = vec![
        "-c".to_owned(),
        WRAPPER.to_owned(),
        WRAPPER_NAME.to_owned(),
        instance.program.clone(),
        socket_path(home, id).display().to_string(),
        go_path(home, id).display().to_string(),
        files::log_path(home, id).display().to_string(),
        trust_value(&real_workdir(instance)?),
    ];
    args.extend(instance.args.iter().cloned());
    Ok(args)
}

fn remove(path: &Path, removed: &mut Vec<String>) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => {
            removed.push(path.display().to_string());
            Ok(())
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

/// Before a `Spawn`: removes the old socket link, the codex socket it points
/// at when that is under `/private/tmp/codex-daemon-<uid>/` (or
/// `/tmp/codex-daemon-<uid>/`), and the old `$GO`. Returns what it removed.
pub fn prepare(home: &Path, id: &str) -> io::Result<Vec<String>> {
    let mut removed = Vec::new();
    let socket = socket_path(home, id);
    if let Ok(target) = fs::read_link(&socket) {
        // SAFETY: getuid has no preconditions.
        let uid = unsafe { libc::getuid() };
        let owned = [
            format!("/private/tmp/codex-daemon-{uid}/"),
            format!("/tmp/codex-daemon-{uid}/"),
        ];
        let text = target.display().to_string();
        if owned.iter().any(|dir| text.starts_with(dir.as_str())) && !text.contains("/../") {
            remove(&target, &mut removed)?;
        }
    }
    remove(&socket, &mut removed)?;
    remove(&go_path(home, id), &mut removed)?;
    Ok(removed)
}

/// Writes `$GO` (thread id, resolved socket) unless it exists: a temp file
/// in the same directory, then `rename`. Returns false when it existed.
pub fn write_go(home: &Path, id: &str, thread: &str, real_socket: &Path) -> io::Result<bool> {
    let go = go_path(home, id);
    if go.exists() {
        return Ok(false);
    }
    let tmp = files::holders_dir(home).join(format!(".{id}.codex-go.tmp"));
    fs::write(&tmp, format!("{thread}\n{}\n", real_socket.display()))?;
    fs::rename(&tmp, &go)?;
    Ok(true)
}

/// `$AGEND_HOME/zsh`, the `ZDOTDIR` of codex agents.
pub fn zdotdir(home: &Path) -> PathBuf {
    home.join(ZDOTDIR)
}

/// Makes `$AGEND_HOME/zsh/.zprofile` hold [`ZPROFILE`]; true when it wrote.
pub fn ensure_zdotdir(home: &Path) -> io::Result<bool> {
    let dir = zdotdir(home);
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dir)?;
    let file = dir.join(".zprofile");
    if fs::read_to_string(&file).is_ok_and(|s| s == ZPROFILE) {
        return Ok(false);
    }
    let tmp = dir.join(".zprofile.new");
    fs::write(&tmp, ZPROFILE)?;
    fs::rename(&tmp, &file)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::InstanceStatus;
    use agend_core::model::Backend;
    use agend_testkit::tempdir::TempDir;
    use std::process::Command;

    fn instance(dir: &Path) -> Instance {
        Instance {
            id: "g7-1".into(),
            backend: Backend::Codex,
            program: "/opt/codex/bin/codex".into(),
            args: vec!["--turn-ms".into(), "5".into()],
            working_directory: dir.display().to_string(),
            session_id: None,
            status: InstanceStatus::New,
            session_started: false,
            agent_pid: None,
            legacy_no_thread: false,
        }
    }

    #[test]
    fn the_trust_value_is_toml_with_the_path_escaped() {
        assert_eq!(
            trust_value("/Users/me/ws/g7-1"),
            r#"projects={"/Users/me/ws/g7-1"={trust_level="trusted"}}"#
        );
        assert_eq!(
            trust_value("/a \"b\"\\c\n"),
            r#"projects={"/a \"b\"\\c\u000A"={trust_level="trusted"}}"#
        );
    }

    /// Positional parameters only: the script is the fixed text whatever
    /// the paths are; paths never become part of it.
    #[test]
    fn the_wrapper_gets_paths_and_settings_as_positional_parameters() {
        let dir = TempDir::new("g7-launch").unwrap();
        let home = Path::new("/h o\"me");
        let args = wrapper_args(home, &instance(dir.path())).unwrap();
        let real = fs::canonicalize(dir.path()).unwrap();
        assert_eq!(args[0], "-c");
        assert_eq!(args[1], WRAPPER);
        assert_eq!(
            args[2..],
            [
                "agend-codex".to_owned(),
                "/opt/codex/bin/codex".into(),
                "/h o\"me/run/holders/g7-1.codex.sock".into(),
                "/h o\"me/run/holders/g7-1.codex-go".into(),
                "/h o\"me/run/holders/g7-1.log".into(),
                trust_value(&real.display().to_string()),
                "--turn-ms".into(),
                "5".into(),
            ]
        );
        assert!(!WRAPPER.contains("/h o"));
    }

    #[test]
    fn a_missing_working_directory_is_an_error() {
        let mut i = instance(Path::new("/nonexistent-g7-dir"));
        i.working_directory = "/nonexistent-g7-dir".into();
        let e = wrapper_args(Path::new("/h"), &i).unwrap_err();
        assert!(
            e.starts_with("working directory /nonexistent-g7-dir"),
            "{e}"
        );
    }

    /// The wrapper's command lines as `sh` builds them, with `echo` as codex:
    /// the app-server line (settings before the subcommand, the instance's
    /// arguments after `--listen`), then the TUI line from `$GO`.
    #[test]
    fn the_wrapper_starts_the_app_server_then_execs_the_tui_from_go() {
        let dir = TempDir::new("g7-wrapper").unwrap();
        let fake = dir.path().join("codex");
        fs::write(
            &fake,
            "#!/bin/sh\nfor a in \"$@\"; do printf '[%s]' \"$a\"; done; echo\n",
        )
        .unwrap();
        fs::set_permissions(&fake, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        let go = dir.path().join("go");
        let log = dir.path().join("log");
        fs::write(&go, "thread-1\n/private/tmp/real.sock\n").unwrap();
        let out = Command::new(SHELL)
            .args(["-c", WRAPPER, WRAPPER_NAME])
            .arg(&fake)
            .arg("/h/run/holders/g7-1.codex.sock")
            .arg(&go)
            .arg(&log)
            .arg("projects={\"/w\"={trust_level=\"trusted\"}}")
            .args(["--turn-ms", "5"])
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        let tui = String::from_utf8(out.stdout).unwrap();
        assert_eq!(
            tui.trim(),
            "[-c][projects={\"/w\"={trust_level=\"trusted\"}}][-c][check_for_update_on_startup=false]\
             [resume][thread-1][--remote][unix:///private/tmp/real.sock]"
        );
        // The background app-server line goes to the log, one `printf` per
        // argument: wait for its final newline.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let server = loop {
            let text = fs::read_to_string(&log).unwrap_or_default();
            if text.ends_with('\n') || std::time::Instant::now() > deadline {
                break text;
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        assert_eq!(
            server.trim(),
            "[-c][projects={\"/w\"={trust_level=\"trusted\"}}][-c][check_for_update_on_startup=false]\
             [-c][approval_policy=\"never\"][-c][sandbox_mode=\"danger-full-access\"][app-server]\
             [--listen][unix:///h/run/holders/g7-1.codex.sock][--turn-ms][5]"
        );
    }

    #[test]
    fn go_is_written_once_by_rename_and_prepare_removes_the_old_files() {
        let dir = TempDir::new("g7-go").unwrap();
        let home = dir.path();
        fs::create_dir_all(files::holders_dir(home)).unwrap();
        let real = Path::new("/private/tmp/x.sock");
        assert!(write_go(home, "g7-1", "t-1", real).unwrap());
        let go = go_path(home, "g7-1");
        assert_eq!(
            fs::read_to_string(&go).unwrap(),
            "t-1\n/private/tmp/x.sock\n"
        );
        assert!(!write_go(home, "g7-1", "t-2", real).unwrap(), "exists");
        assert_eq!(
            fs::read_to_string(&go).unwrap(),
            "t-1\n/private/tmp/x.sock\n"
        );
        let names: Vec<String> = fs::read_dir(files::holders_dir(home))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["g7-1.codex-go"], "no temp file left");

        let socket = socket_path(home, "g7-1");
        let keep = dir.path().join("elsewhere.sock");
        fs::write(&keep, "").unwrap();
        std::os::unix::fs::symlink(&keep, &socket).unwrap();
        let removed = prepare(home, "g7-1").unwrap();
        assert_eq!(
            removed,
            [socket.display().to_string(), go.display().to_string()]
        );
        assert!(keep.exists(), "a target outside codex-daemon-<uid> is kept");
        assert_eq!(prepare(home, "g7-1").unwrap(), Vec::<String>::new());
    }

    #[test]
    fn the_zprofile_is_written_once() {
        let dir = TempDir::new("g7-zdot").unwrap();
        assert!(ensure_zdotdir(dir.path()).unwrap());
        assert!(!ensure_zdotdir(dir.path()).unwrap());
        let text = fs::read_to_string(dir.path().join("zsh/.zprofile")).unwrap();
        assert_eq!(text, ZPROFILE);
    }

    /// Option A for real: a login zsh with our `ZDOTDIR` puts the shim
    /// directory first even after `/etc/zprofile` (macOS `path_helper`).
    /// Skipped where there is no `/bin/zsh`.
    #[test]
    fn a_login_zsh_with_our_zdotdir_finds_the_shims_first() {
        if !Path::new("/bin/zsh").exists() {
            println!("SKIPPED: no /bin/zsh here");
            return;
        }
        let dir = TempDir::new("g7-zsh").unwrap();
        let home = dir.path();
        ensure_zdotdir(home).unwrap();
        let bin = home.join("bin");
        fs::create_dir_all(&bin).unwrap();
        let out = Command::new("/bin/zsh")
            .args(["-lc", "echo $PATH"])
            .env_clear()
            .env("HOME", home)
            .env("AGEND_HOME", home)
            .env("ZDOTDIR", zdotdir(home))
            .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
            .output()
            .unwrap();
        let path = String::from_utf8(out.stdout).unwrap();
        assert_eq!(
            path.trim().split(':').next(),
            Some(bin.display().to_string().as_str()),
            "{path}"
        );
    }
}
