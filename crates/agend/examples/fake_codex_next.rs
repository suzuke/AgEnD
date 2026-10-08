//! Second native version for production backend-switch integration tests.
fn main() -> std::process::ExitCode {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args == ["--version"] {
        println!("codex-cli 0.159.0");
        return std::process::ExitCode::SUCCESS;
    }
    agend_testkit::fake_agent::codex_cli::main(args)
}
