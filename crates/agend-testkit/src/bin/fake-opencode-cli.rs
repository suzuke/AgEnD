//! Native CLI launch fixture: version/serve/attach around the recorded REST
//! producer. It never runs a model or commands and touches only its own home.
use std::{
    io::{self, Read, Write},
    path::PathBuf,
    time::Duration,
};
#[allow(dead_code)] // Also included as a module by the second-version fixture.
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    run("1.18.34")
}

pub fn run(version: &'static str) -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args == ["--version"] {
        println!("{version}");
        return Ok(());
    }
    match args.first().map(String::as_str) {
        Some("serve") => {
            let port = args
                .windows(2)
                .find(|w| w[0] == "--port")
                .ok_or("missing port")?[1]
                .parse()?;
            let home = PathBuf::from(std::env::var("AGEND_HOME")?).canonicalize()?;
            let data = PathBuf::from(std::env::var("XDG_DATA_HOME")?).canonicalize()?;
            if !data.starts_with(home.join("opencode")) {
                return Err("fixture data must be inside its own OpenCode home".into());
            }
            let server = agend_testkit::fake_agent::opencode::Server::start_version(
                port,
                Duration::from_millis(500),
                Some(data.join("state.json")),
                version,
            )?;
            println!(
                "opencode server listening on http://127.0.0.1:{}",
                server.port()
            );
            io::stdout().flush()?;
            loop {
                std::thread::park();
            }
        }
        Some("attach") => {
            let session = &args
                .windows(2)
                .find(|w| w[0] == "--session")
                .ok_or("missing session")?[1];
            println!("fixture attached {session}");
            io::stdout().flush()?;
            let mut buf = [0; 1024];
            while io::stdin().read(&mut buf)? != 0 {}
            Ok(())
        }
        _ => Err("unsupported fixture CLI command".into()),
    }
}
