//! Fake interactive `claude` with hooks and an MCP channel; see
//! `agend_testkit::fake_agent::claude`.

fn main() -> std::process::ExitCode {
    agend_testkit::fake_agent::claude::main(std::env::args().skip(1))
}
