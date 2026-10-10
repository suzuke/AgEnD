//! Native ACK/Stop producer for isolated installation canaries; no model.
fn main() -> std::process::ExitCode {
    agend_testkit::fake_agent::claude::receipt_cli::main(std::env::args().skip(1))
}
