//! `fake_codex`: the `codex` CLI stand-in (`agend_testkit::fake_agent::
//! codex_cli`) built with `cargo test -p agend`, so the gate 7 tests with the
//! real `agend` binary find it next to their own executable
//! (`target/<profile>/examples/fake_codex`). Test-only; never installed.

fn main() -> std::process::ExitCode {
    agend_testkit::fake_agent::codex_cli::main(std::env::args().skip(1))
}
