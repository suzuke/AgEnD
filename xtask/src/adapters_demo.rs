//! Native Claude adapter scenarios share the integration tests' assertions.
use crate::accept::step;

pub fn run() -> Result<(), String> {
    println!("== Claude native adapter demo ==");
    println!("No real Claude CLI or models. Startup/version acceptance remains pending.");
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
    println!(
        "Claude native adapter demo passed; Gate 12A true CLI/startup acceptance remains pending."
    );
    Ok(())
}
