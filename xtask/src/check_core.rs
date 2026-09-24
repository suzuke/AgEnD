//! Structural checks that keep agend-core free of I/O (part of `check-deps`).
//!
//! Threat model: these stop *accidental* I/O in core (a stray `std` import,
//! a new dependency, a build script). They are not a defence against someone
//! deliberately evading them; code review is.
//!
//! | guard | stops |
//! |---|---|
//! | building for [`NO_STD_TARGET`] with `--all-features` | std reaching the compiled crate: `extern crate std` variants, `[lib] path` redirects, `include!`, std-using dependencies (verified cases listed in xtask/TESTING.md) |
//! | `-F unsafe-code` passed by that build | FFI such as `unsafe extern "C"` into libc, even if the source attribute is removed |
//! | `cargo metadata` rules below | a build script, any `[features]`, and any dependency not in [`CORE_DEP_ALLOWLIST`] |
//!
//! Known gap (accepted, deliberate evasion only): code gated on a cfg that is
//! false for the no-std target, e.g. `#[cfg(not(target_os = "none"))]`, is not
//! compiled there and so is not caught.

use crate::{cargo, workspace_root};
use serde_json::Value;
use std::process::Command;

pub const CORE: &str = "agend-core";

/// A tier-2 target without std. If core compiles for it, core does not use std.
pub const NO_STD_TARGET: &str = "thumbv7em-none-eabihf";

/// Dependencies (of any kind) agend-core may have. Empty on purpose; adding one
/// is a reviewed change.
pub const CORE_DEP_ALLOWLIST: &[&str] = &[];

/// Runs all core checks. Returns (problems, whether the no-std build was
/// skipped); `Err` only if a check could not run.
pub fn run(allow_skip: bool) -> Result<(Vec<String>, bool), String> {
    let mut skipped = false;
    let mut problems = metadata_problems(&workspace_metadata()?);
    match build_for_no_std_target()? {
        NoStdBuild::Ok => {}
        NoStdBuild::Failed(stderr) => problems.push(format!(
            "{CORE} does not compile for the no-std target {NO_STD_TARGET} (std use, unsafe code, or another compile error):\n{stderr}"
        )),
        NoStdBuild::TargetMissing => {
            let msg = format!(
                "SKIPPED: {CORE} no-std build: target {NO_STD_TARGET} is not installed for this cargo. \
                 Fix: `rustup target add {NO_STD_TARGET}` and run through rustup \
                 (`~/.cargo/bin/cargo xtask check-deps`); a Homebrew cargo has no extra targets."
            );
            if allow_skip {
                skipped = true;
                eprintln!("check-deps: {msg} (--allow-skip given, not failing)");
            } else {
                problems.push(msg);
            }
        }
    }
    Ok((problems, skipped))
}

/// Problems with agend-core's package metadata (`cargo metadata --no-deps`).
pub fn metadata_problems(metadata: &Value) -> Vec<String> {
    let Some(pkg) = metadata["packages"]
        .as_array()
        .and_then(|ps| ps.iter().find(|p| p["name"] == CORE))
    else {
        return vec![format!("{CORE} not found in cargo metadata")];
    };
    let mut problems = Vec::new();
    if pkg["features"].as_object().is_some_and(|f| !f.is_empty()) {
        problems.push(format!(
            "{CORE} must not declare [features] (a `std` feature could re-enable I/O)"
        ));
    }
    for target in pkg["targets"].as_array().into_iter().flatten() {
        let kinds = target["kind"].as_array().into_iter().flatten();
        if kinds.clone().any(|k| k == "custom-build") {
            problems.push(format!("{CORE} must not have a build script (build.rs)"));
        }
    }
    for dep in pkg["dependencies"].as_array().into_iter().flatten() {
        let name = dep["name"].as_str().unwrap_or("?");
        if !CORE_DEP_ALLOWLIST.contains(&name) {
            let kind = dep["kind"].as_str().unwrap_or("normal");
            problems.push(format!(
                "{CORE} has a {kind} dependency on {name}; core dependencies must be in CORE_DEP_ALLOWLIST (xtask/src/check_core.rs)"
            ));
        }
    }
    problems
}

fn workspace_metadata() -> Result<Value, String> {
    let out = Command::new(cargo())
        .current_dir(workspace_root())
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .output()
        .map_err(|e| format!("cannot run cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("bad cargo metadata: {e}"))
}

pub enum NoStdBuild {
    Ok,
    Failed(String),
    TargetMissing,
}

fn build_for_no_std_target() -> Result<NoStdBuild, String> {
    let root = workspace_root();
    let mut cmd = Command::new(cargo());
    // Use the rustc that belongs to this cargo, not whichever is first on PATH
    // (a Homebrew rustc there would lack the rustup-installed target).
    if std::env::var_os("RUSTC").is_none() {
        let sibling = std::path::Path::new(&cargo()).with_file_name("rustc");
        if sibling.is_file() {
            cmd.env("RUSTC", sibling);
        }
    }
    let out = cmd
        .current_dir(&root)
        .args([
            "rustc",
            "--quiet",
            "-p",
            CORE,
            "--lib",
            "--all-features",
            "--target",
            NO_STD_TARGET,
        ])
        // Separate target dir: this may run while an outer cargo holds target/.
        .arg("--target-dir")
        .arg(root.join("target").join("xtask-no-std"))
        // Enforce no unsafe code from here, independent of the source attribute.
        .args(["--", "-F", "unsafe-code"])
        .output()
        .map_err(|e| format!("cannot run cargo rustc: {e}"))?;
    if out.status.success() {
        return Ok(NoStdBuild::Ok);
    }
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    Ok(if target_missing(&stderr) {
        NoStdBuild::TargetMissing
    } else {
        NoStdBuild::Failed(stderr)
    })
}

/// Whether a failed build failed only because the target's `core` is absent.
fn target_missing(stderr: &str) -> bool {
    stderr.contains("target may not be installed") || stderr.contains("can't find crate for `core`")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_metadata_of_core_passes() {
        let meta = workspace_metadata().unwrap();
        assert_eq!(metadata_problems(&meta), Vec::<String>::new());
    }

    fn core_mut(meta: &mut Value) -> &mut Value {
        meta["packages"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|p| p["name"] == CORE)
            .unwrap()
    }

    #[test]
    fn build_script_is_rejected() {
        // Start from the real producer's output, then add what cargo would emit.
        let mut meta = workspace_metadata().unwrap();
        core_mut(&mut meta)["targets"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({"name": "build-script-build", "kind": ["custom-build"]}));
        let problems = metadata_problems(&meta);
        assert!(
            problems.iter().any(|p| p.contains("build script")),
            "{problems:?}"
        );
    }

    #[test]
    fn any_dependency_kind_is_rejected() {
        for kind in [Value::Null, "build".into(), "dev".into()] {
            let mut meta = workspace_metadata().unwrap();
            core_mut(&mut meta)["dependencies"]
                .as_array_mut()
                .unwrap()
                .push(serde_json::json!({"name": "leaky", "kind": kind}));
            let problems = metadata_problems(&meta);
            assert!(problems.iter().any(|p| p.contains("leaky")), "{problems:?}");
        }
    }

    #[test]
    fn features_are_rejected() {
        let mut meta = workspace_metadata().unwrap();
        core_mut(&mut meta)["features"] = serde_json::json!({"std": []});
        let problems = metadata_problems(&meta);
        assert!(
            problems.iter().any(|p| p.contains("[features]")),
            "{problems:?}"
        );
    }

    #[test]
    fn missing_target_is_recognised() {
        assert!(target_missing(
            "error[E0463]: can't find crate for `core`\n  = note: the `thumbv7em-none-eabihf` target may not be installed"
        ));
        assert!(!target_missing(
            "error[E0433]: cannot find module or crate `std`"
        ));
    }
}
