//! Fake `codex app-server`; see `agend_testkit::fake_agent::codex`.

fn main() -> std::process::ExitCode {
    agend_testkit::fake_agent::codex::main(std::env::args().skip(1))
}
