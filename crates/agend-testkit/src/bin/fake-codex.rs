//! Fake `codex` CLI for the gate 7 wrapper; see
//! `agend_testkit::fake_agent::codex_cli`.

fn main() -> std::process::ExitCode {
    agend_testkit::fake_agent::codex_cli::main(std::env::args().skip(1))
}
