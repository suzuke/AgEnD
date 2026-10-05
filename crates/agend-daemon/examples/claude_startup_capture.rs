//! Opt-in passive true CLI startup recording, with no prompts or keys.
//! Version labels are supplied by a separately authorized version query.
#[path = "../tests/common/claude_startup_capture.rs"]
mod capture;
use std::{
    path::{Path, PathBuf},
    process::ExitCode,
};
fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.as_slice() == ["--help"] {
        println!("{}", capture::USAGE);
        return ExitCode::SUCCESS;
    }
    if std::env::var("AGEND_REAL_CLAUDE_STARTUP").as_deref() != Ok("1") {
        eprintln!(
            "passive capture launches the selected program; set AGEND_REAL_CLAUDE_STARTUP=1 only after authorization"
        );
        return ExitCode::from(2);
    }
    let result = (|| {
        let options = capture::Options::parse(&args)?;
        let agend = std::env::var_os("AGEND_BIN")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::current_exe()
                    .ok()?
                    .parent()?
                    .parent()
                    .map(|p| p.join("agend"))
            })
            .ok_or("cannot locate agend")?;
        capture::run(&options, Path::new(&agend))
    })();
    match result {
        Ok(frames) => {
            println!(
                "passive startup capture: {frames} frames; no input sent; startup completion not assessed"
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("claude_startup_capture: {e}");
            ExitCode::FAILURE
        }
    }
}
