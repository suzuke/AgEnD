//! Second native version for production backend-switch integration tests.
fn main() -> std::process::ExitCode {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args == ["--version"] {
        println!("codex-cli 0.159.0");
        return std::process::ExitCode::SUCCESS;
    }
    // The crash harness can hold only its fleet launch before the app-server
    // becomes ready; isolated canary workspaces never contain this marker.
    let hold = std::path::Path::new(".g13-hold-start");
    while hold.exists() {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    if args.iter().any(|arg| arg == "resume") && std::path::Path::new(".g13-exit-start").exists() {
        return std::process::ExitCode::FAILURE;
    }
    if args.iter().any(|arg| arg == "app-server")
        && std::path::Path::new(".g13-exit-server").exists()
    {
        return std::process::ExitCode::FAILURE;
    }
    agend_testkit::fake_agent::codex_cli::main(args)
}
