//! Real pipeline fixture, with fake inbox workers. No real LLM is started.
#[path = "../tests/common/pipeline_process.rs"]
mod lab;
fn main() -> std::process::ExitCode {
    let result = match std::env::args().nth(1).as_deref() {
        Some("setup") => lab::home().and_then(|h| lab::setup(&h, &[])),
        Some("teardown") => lab::home().and_then(|h| lab::teardown(&h)),
        Some("demo") => lab::demo(),
        _ => Err("usage: pipeline_probe setup|teardown|demo".into()),
    };
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("pipeline_probe: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}
