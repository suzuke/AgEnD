//! Fake `opencode serve`; see `agend_testkit::fake_agent::opencode`.

fn main() -> std::process::ExitCode {
    agend_testkit::fake_agent::opencode::main(std::env::args().skip(1))
}
