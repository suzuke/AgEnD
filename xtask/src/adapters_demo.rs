//! Native adapter scenarios share the integration tests' assertions.
use crate::accept::step;

pub fn prepare() -> Result<(), String> {
    step(&[
        "build",
        "--quiet",
        "-p",
        "agend",
        "-p",
        "agend-testkit",
        "--bins",
    ])?;
    step(&["build", "--quiet", "-p", "agend", "--example", "fake_codex"])
}

pub fn run() -> Result<(), String> {
    prepare()?;
    println!("== Native adapter demo: Claude, OpenCode, GitHub, Telegram ==");
    println!("No real model or external API calls. Live acceptance is recorded separately.");
    println!("== Claude ==");
    println!("== Driver, launch ownership, sweep ==");
    step(&[
        "test",
        "--quiet",
        "-p",
        "agend-daemon",
        "--lib",
        "driver::claude",
        "--",
        "--nocapture",
    ])?;
    println!("== Native supervisor, holders, retries, orphan protection ==");
    step(&[
        "test",
        "--quiet",
        "-p",
        "agend",
        "--test",
        "claude_process",
        "--",
        "--nocapture",
    ])?;
    println!("== Channel, Stop, explicit ACK, spool, control, Git pipeline, DRV-1..9 ==");
    println!("boot_child is a subprocess entry, not a separate passing scenario.");
    step(&[
        "test",
        "--quiet",
        "-p",
        "agend",
        "--test",
        "claude_bridge",
        "--",
        "--nocapture",
    ])?;
    println!("== OpenCode driver, durable delivery, permissions and native bridge ==");
    step(&[
        "test",
        "--quiet",
        "-p",
        "agend-daemon",
        "--lib",
        "driver::opencode",
        "--",
        "--nocapture",
    ])?;
    step(&[
        "test",
        "--quiet",
        "-p",
        "agend",
        "--test",
        "opencode_bridge",
        "--",
        "--nocapture",
    ])?;
    println!("== GitHub production forge, strict base policy and native daemon recovery ==");
    step(&[
        "test",
        "--quiet",
        "-p",
        "agend-daemon",
        "--test",
        "github_forge",
        "--test",
        "github_pipeline",
        "--",
        "--test-threads=1",
        "--nocapture",
    ])?;
    println!("== Telegram transport, routing, inbound guards and durable receipts ==");
    step(&[
        "test",
        "--quiet",
        "-p",
        "agend-daemon",
        "--lib",
        "notifier::",
        "--",
        "--nocapture",
    ])?;
    println!("== Telegram daemon shutdown, remaining parts and supervisor Retry ==");
    println!("The ignored child entry is executed by its parent lifecycle scenarios.");
    step(&[
        "test",
        "--quiet",
        "-p",
        "agend-daemon",
        "--lib",
        "daemon::telegram_tests::",
        "--",
        "--nocapture",
    ])?;
    println!("== Shared read state and operator disposition after restart ==");
    step(&[
        "test",
        "--quiet",
        "-p",
        "agend",
        "--test",
        "shared_read",
        "--test",
        "telegram_unknown",
        "--",
        "--nocapture",
    ])?;
    step(&[
        "test",
        "--quiet",
        "-p",
        "agend",
        "--bin",
        "agend",
        "doctor::telegram_tests",
        "--",
        "--nocapture",
    ])?;
    println!(
        "Native Claude, OpenCode, GitHub and Telegram scenarios passed; live acceptance is recorded separately."
    );
    Ok(())
}
