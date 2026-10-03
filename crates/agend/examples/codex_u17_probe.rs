//! Deterministic full U17 acceptance demo. Only the Codex brain is fake.
//! Uses the same scenario and producer as the integration test; no model calls.
#[path = "../tests/common/codex_u17_app.rs"]
mod app_path;
#[path = "../../agend-daemon/tests/common/codex_process.rs"]
mod codex;
#[path = "../tests/common/codex_u17_fixture.rs"]
mod fixture_support;
#[path = "../../agend-daemon/tests/common/daemon_process.rs"]
mod lab;
use agend_daemon::driver::codex::history;
use fixture_support::{fixture, probe, text};
use std::path::{Path, PathBuf};
const ID: &str = "g11-codex";

fn agend_bin() -> PathBuf {
    std::env::var_os("AGEND_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::current_exe()
                .unwrap()
                .parent()
                .and_then(Path::parent)
                .unwrap()
                .join("agend")
        })
}
fn main() {
    if std::env::var("AGEND_U17_TEST_DAEMON").as_deref() == Ok("1") {
        app_path::daemon();
        return;
    }
    println!("== U17: full App/client/daemon/holder with fake Codex ==");
    app_path::default_denied();
    app_path::full_path();
    println!("== approved CLI: normal daemon and durable attribution ==");
    app_path::approved_full_path();
    println!(
        "U17 fake full-path demo: all sections passed; real U17 evidence is recorded separately"
    );
}
