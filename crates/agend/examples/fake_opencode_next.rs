//! Second version identity with the same native OpenCode fixture protocol.
#[path = "../../agend-testkit/src/bin/fake-opencode-cli.rs"]
mod fixture;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    fixture::run("1.18.35")
}
