//! The actual fake frontend setup shared by the U17 tests and acceptance demo.
use crate::{ID, codex, lab};
use agend_core::protocol::terminal::TerminalFrame;
use agend_daemon::{driver::codex::launch, store::Instance};
use agend_testkit::fake_agent::codex::Probe;
use serde_json::json;
use std::path::Path;
pub fn text(frame: &TerminalFrame) -> String {
    frame
        .cells
        .iter()
        .flat_map(|row| row.iter())
        .map(|cell| cell.text.as_str())
        .collect()
}
pub fn fixture(native: &lab::Lab, home: &Path) -> Instance {
    let fake = codex::fake_codex().unwrap();
    let script = native.root.join("manual-codex");
    let fake = fake.display().to_string().replace('\'', "'\\''");
    std::fs::write(
        &script,
        format!("#!/bin/sh\nexec '{fake}' -c agend_fake_manual_tui=true \"$@\"\n"),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    codex::add(home, ID, &script, 1500).unwrap()
}
pub fn probe(home: &Path, thread: &str) -> Probe {
    let mut probe = Probe::connect(&launch::socket_path(home, ID)).unwrap();
    probe
        .call("initialize", json!({"clientInfo":{"name":"u17-observer"}}))
        .unwrap();
    probe
        .call(
            "thread/resume",
            json!({"threadId":thread,"excludeTurns":true}),
        )
        .unwrap();
    probe
}
