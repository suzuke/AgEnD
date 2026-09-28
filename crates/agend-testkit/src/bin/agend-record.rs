//! `agend-record`: records real backend CLIs into transcripts, and is also
//! the hook and channel relay for claude. See `agend_testkit::recorder`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use agend_testkit::recorder::{self, Scenario, claude};

const USAGE: &str = "\
usage: agend-record <backend> [scenario...] --out <dir>
       agend-record list
       agend-record startup-check claude   (start, answer dialogs, /exit; no prompt, no tokens)
       agend-record redact <transcript.jsonl>...   (re-apply redaction rules in place)
       agend-record hook-relay <socket>      (claude hook command)
       agend-record channel-relay <socket>   (claude MCP channel server)

Runs the REAL CLI. Run it only under a write sandbox (e.g. via
`cargo xtask record`); every scenario runs in mktemp -d /private/tmp/agend-rec-<backend>-XXXX.
Scenarios: one_turn interrupt approval busy resume; codex also turns_list queue_idle
resume_empty (default: all the backend supports).
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("hook-relay") if args.len() == 2 => claude::hook_relay(Path::new(&args[1])),
        Some("channel-relay") if args.len() == 2 => claude::channel_relay(Path::new(&args[1])),
        Some("redact") if args.len() > 1 => {
            for path in &args[1..] {
                if let Err(e) = recorder::redact_file(Path::new(path)) {
                    eprintln!("agend-record: {e}");
                    return ExitCode::FAILURE;
                }
            }
            ExitCode::SUCCESS
        }
        Some("startup-check") if args.get(1).map(String::as_str) == Some("claude") => {
            let relay = std::env::current_exe().unwrap_or_default();
            match claude::startup_check(&relay) {
                Ok(entries) => {
                    for e in entries {
                        let what = e.str("hook_event_name").or(e.str("method")).unwrap_or("-");
                        println!("{} {} {what}", e.from.name(), e.via);
                    }
                    println!("startup-check: ok");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("startup-check: {e}");
                    ExitCode::FAILURE
                }
            }
        }
        Some("list") => {
            for b in recorder::BACKENDS {
                let names: Vec<&str> = b.scenarios().iter().map(|s| s.name()).collect();
                println!("{} ({}): {}", b.name(), b.program(), names.join(" "));
            }
            ExitCode::SUCCESS
        }
        Some(name) if !name.starts_with('-') => match record(name, &args[1..]) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("agend-record: {e}");
                ExitCode::FAILURE
            }
        },
        _ => {
            eprint!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn record(name: &str, rest: &[String]) -> Result<(), String> {
    let backend = recorder::backend(name).ok_or_else(|| format!("unknown backend {name}"))?;
    let mut out = None;
    let mut scenarios = Vec::new();
    let mut rest = rest.iter();
    while let Some(arg) = rest.next() {
        if arg == "--out" {
            out = rest.next().map(PathBuf::from);
        } else {
            let s = Scenario::parse(arg).ok_or_else(|| format!("unknown scenario {arg}"))?;
            if !backend.scenarios().contains(&s) {
                return Err(format!("{name} does not support {arg}"));
            }
            scenarios.push(s);
        }
    }
    let out = out.ok_or("--out <dir> is required")?;
    if scenarios.is_empty() {
        scenarios = backend.scenarios().to_vec();
    }
    let relay = std::env::current_exe().map_err(|e| e.to_string())?;
    recorder::record_real(backend, &scenarios, &out, &relay)
}
