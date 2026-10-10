//! Second version identity with the same native OpenCode fixture protocol.
#[path = "../../agend-testkit/src/bin/fake-opencode-cli.rs"]
mod fixture;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().skip(1).collect::<Vec<_>>() != ["--version"] {
        while std::path::Path::new(".g13-hold-start").exists() {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
    fixture::run("1.18.35")
}
