//! Explicit opt-in U17 smoke through App/client/daemon/holder and real Codex.
//! Four model turns, owned temporary home, same holder across daemon restart.
//! Build outside record-sandbox.sh; run the built example inside it.
//! A successful run does not authorize production input or merge.
#[path = "../../agend-daemon/tests/common/daemon_process.rs"]
mod lab;
#[path = "../tests/common/codex_u17_live.rs"]
mod live;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

fn main() -> ExitCode {
    // Check before resolving paths, creating homes, or starting any backend.
    if std::env::var("AGEND_REAL_CODEX").as_deref() != Ok("1") {
        eprintln!("codex_u17_live spends four real model turns; set AGEND_REAL_CODEX=1 to opt in");
        return ExitCode::from(2);
    }
    let agend = std::env::var_os("AGEND_BIN")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::current_exe()
                .ok()?
                .parent()?
                .parent()
                .map(|p| p.join("agend"))
        });
    let Some(agend) = agend.filter(|p| p.is_absolute() && p.is_file()) else {
        eprintln!("build agend or set absolute AGEND_BIN");
        return ExitCode::from(2);
    };
    if std::env::args().nth(1).as_deref() == Some("daemon") {
        let Some(home) = std::env::var_os("AGEND_HOME") else {
            return ExitCode::from(2);
        };
        return agend_daemon::daemon::run_u17_probe(home.into(), agend, live::ID.into());
    }
    match live::run(Path::new(&agend)) {
        Ok(()) => {
            println!("U17 live: passed; version approval is still required");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("U17 live: FAILED: {error}");
            ExitCode::FAILURE
        }
    }
}
