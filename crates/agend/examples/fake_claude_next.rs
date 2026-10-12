//! Second version identity with the same native Claude receipt fixture.
fn main() -> std::process::ExitCode {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args == ["--version"] {
        println!("2.1.285 (Claude Code)");
        return std::process::ExitCode::SUCCESS;
    }
    while std::path::Path::new(".g13-hold-start").exists() {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    agend_testkit::fake_agent::claude::receipt_cli::main(args)
}
