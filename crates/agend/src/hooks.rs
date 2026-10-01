//! Internal daemon-to-shim hook lifecycle command (not public CLI help).
use std::path::Path;
use std::process::{Command, ExitCode};
pub fn run(args: &[std::ffi::OsString]) -> ExitCode {
    if args.len() != 2 {
        eprintln!("agend hooks: expected install|uninstall <worktree>");
        return ExitCode::from(2);
    }
    let Some(home) = std::env::var_os("AGEND_HOME") else {
        eprintln!("AGEND_HOME is not set");
        return ExitCode::from(2);
    };
    let home = Path::new(&home);
    let result = (|| {
        let git = agend_daemon::git::Git::discover(home)?;
        let make = || {
            let mut c = Command::new(&git.executable);
            c.env_clear().envs(&git.runner.env).args([
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "core.fsmonitor=false",
            ]);
            c
        };
        let worktree = Path::new(&args[1]);
        if args[0] == "install" {
            agend_shim::install_hooks(
                &make,
                &home.join("hooks"),
                &std::env::current_exe().map_err(|e| e.to_string())?,
                worktree,
            )
        } else if args[0] == "uninstall" {
            agend_shim::uninstall_hooks(&make, worktree)
        } else {
            Err("unknown hooks operation".into())
        }
    })();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("agend hooks: {e}");
            ExitCode::from(1)
        }
    }
}
