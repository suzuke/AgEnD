//! Native, locked release packaging. No tags, uploads, services or shared installs.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
};

fn output(command: &mut Command) -> Result<String, String> {
    let result = command.output().map_err(|e| e.to_string())?;
    if !result.status.success() {
        return Err(format!(
            "command failed ({:?}): {}",
            command.get_program(),
            String::from_utf8_lossy(&result.stderr)
        ));
    }
    String::from_utf8(result.stdout).map_err(|_| "command emitted non-UTF-8 output".into())
}
fn source(root: &Path) -> Result<String, String> {
    let status = output(Command::new("git").current_dir(root).args([
        "status",
        "--porcelain",
        "--untracked-files=normal",
    ]))?;
    if !status.is_empty() {
        return Err("release requires a clean committed checkout".into());
    }
    let head = output(
        Command::new("git")
            .current_dir(root)
            .args(["rev-parse", "HEAD"]),
    )?;
    let head = head.trim();
    if head.len() != 40 || !head.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err("invalid source commit".into());
    }
    Ok(head.into())
}
fn digest(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
fn native_target() -> Result<String, String> {
    let compiler = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let version = output(Command::new(compiler).arg("-vV"))?;
    let host = version
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .ok_or("rustc omitted its host")?;
    if !matches!(
        host,
        "aarch64-apple-darwin"
            | "x86_64-apple-darwin"
            | "x86_64-unknown-linux-gnu"
            | "aarch64-unknown-linux-gnu"
    ) {
        return Err(format!("release target is not supported: {host}"));
    }
    Ok(host.into())
}
fn version(root: &Path) -> Result<String, String> {
    let metadata: Value = serde_json::from_str(&output(
        Command::new(crate::cargo()).current_dir(root).args([
            "metadata",
            "--locked",
            "--no-deps",
            "--format-version",
            "1",
        ]),
    )?)
    .map_err(|e| e.to_string())?;
    let version = metadata["packages"]
        .as_array()
        .ok_or("missing packages")?
        .iter()
        .find(|p| p["name"] == "agend")
        .and_then(|p| p["version"].as_str())
        .ok_or("missing agend version")?;
    if version.is_empty()
        || !version
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b".-+".contains(&c))
    {
        return Err("invalid release version".into());
    }
    Ok(version.into())
}
fn build(root: &Path, target: &str) -> Result<PathBuf, String> {
    eprintln!("release: building native {target} with Cargo.lock");
    let messages = output(Command::new(crate::cargo()).current_dir(root).args([
        "build",
        "--locked",
        "--release",
        "-p",
        "agend",
        "--bin",
        "agend",
        "--target",
        target,
        "--message-format=json",
    ]))?;
    let mut executable = None;
    for line in messages.lines() {
        let Ok(message) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if message["reason"] == "compiler-artifact"
            && message["target"]["name"] == "agend"
            && let Some(path) = message["executable"].as_str()
        {
            executable = Some(PathBuf::from(path));
        }
    }
    executable.ok_or("cargo did not identify the release executable".into())
}
fn package(
    root: &Path,
    binary: &Path,
    out: &Path,
    version: &str,
    target: &str,
    commit: &str,
) -> Result<(), String> {
    // create_dir, not create_dir_all: pre-existing destinations are never reused.
    fs::create_dir(out).map_err(|e| format!("cannot reserve output directory: {e}"))?;
    fs::set_permissions(out, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    let name = format!("agend-{version}-{target}");
    let stage = out.join(&name);
    fs::create_dir(&stage).map_err(|e| e.to_string())?;
    let result = (|| {
        fs::copy(binary, stage.join("agend")).map_err(|e| e.to_string())?;
        fs::set_permissions(stage.join("agend"), fs::Permissions::from_mode(0o755))
            .map_err(|e| e.to_string())?;
        for file in ["LICENSE", "README.md"] {
            fs::copy(root.join(file), stage.join(file)).map_err(|e| e.to_string())?;
        }
        let binary_sha256 = digest(&stage.join("agend"))?;
        if binary_sha256 != digest(binary)? {
            return Err("release binary changed while copying".into());
        }
        let archive = format!("{name}.tar.gz");
        output(
            Command::new("tar")
                .args(["-czf"])
                .arg(out.join(&archive))
                .arg("-C")
                .arg(out)
                .arg(&name),
        )?;
        let sha256 = digest(&out.join(&archive))?;
        let manifest = json!({"format":1,"version":version,"target":target,"git_commit":commit,"archive":archive,"archive_sha256":sha256,"binary_sha256":binary_sha256});
        fs::write(
            out.join("manifest.json"),
            serde_json::to_vec_pretty(&manifest).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        fs::write(out.join("SHA256SUMS"), format!("{sha256}  {archive}\n"))
            .map_err(|e| e.to_string())?;
        Ok(())
    })();
    // This is our exclusively created staging tree, never an existing user directory.
    fs::remove_dir_all(&stage).map_err(|e| format!("cannot clean package staging: {e}"))?;
    result
}
pub fn run(args: &[String]) -> Result<(), String> {
    if args.len() != 2 || args[0] != "--out" {
        return Err("usage: cargo xtask release --out <absolute-new-directory>".into());
    }
    let out = PathBuf::from(&args[1]);
    if !out.is_absolute() || out.file_name().is_none() || out.symlink_metadata().is_ok() {
        return Err("release output must be a new absolute directory".into());
    }
    let root = crate::workspace_root();
    let parent = out
        .parent()
        .ok_or("output has no parent")?
        .canonicalize()
        .map_err(|e| e.to_string())?;
    if parent.starts_with(root.canonicalize().map_err(|e| e.to_string())?) {
        return Err("release output must be outside the source checkout".into());
    }
    let out = parent.join(out.file_name().unwrap());
    let commit = source(&root)?;
    let target = native_target()?;
    let version = version(&root)?;
    let binary = build(&root, &target)?;
    let actual = output(Command::new(&binary).env_clear().arg("--version"))?;
    if actual.trim() != format!("agend {version}") {
        return Err("release binary version does not match Cargo metadata".into());
    }
    if source(&root)? != commit {
        return Err("source changed during release build".into());
    }
    package(&root, &binary, &out, &version, &target, &commit)?;
    if source(&root)? != commit {
        return Err("source changed during packaging; artifacts are not publishable".into());
    }
    println!(
        "release: packaged {version} ({target}) at {}; nothing published",
        out.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Dir(PathBuf);
    impl Dir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "agend-release-test-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
    #[test]
    fn native_tar_round_trip_matches_manifest_and_retains_no_stage() {
        let dir = Dir::new();
        let out = dir.0.join("artifacts");
        let binary = std::env::current_exe().unwrap();
        let root = crate::workspace_root();
        package(
            &root,
            &binary,
            &out,
            "0.0.0",
            "test-native",
            &"a".repeat(40),
        )
        .unwrap();
        let manifest: Value =
            serde_json::from_slice(&fs::read(out.join("manifest.json")).unwrap()).unwrap();
        let archive = out.join(manifest["archive"].as_str().unwrap());
        assert_eq!(manifest["archive_sha256"], digest(&archive).unwrap());
        let extracted = dir.0.join("extracted");
        fs::create_dir(&extracted).unwrap();
        output(
            Command::new("tar")
                .arg("-xzf")
                .arg(&archive)
                .arg("-C")
                .arg(&extracted),
        )
        .unwrap();
        let payload = extracted.join("agend-0.0.0-test-native");
        assert_eq!(
            manifest["binary_sha256"],
            digest(&payload.join("agend")).unwrap()
        );
        assert_eq!(
            digest(&binary).unwrap(),
            digest(&payload.join("agend")).unwrap()
        );
        assert_eq!(
            fs::read(payload.join("LICENSE")).unwrap(),
            fs::read(root.join("LICENSE")).unwrap()
        );
        assert_eq!(
            fs::metadata(payload.join("agend"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o755
        );
        assert!(!out.join("agend-0.0.0-test-native").exists());
        assert_eq!(fs::read_dir(&out).unwrap().count(), 3);
        assert!(
            package(
                &root,
                &binary,
                &out,
                "0.0.0",
                "test-native",
                &"a".repeat(40)
            )
            .is_err()
        );
        assert_eq!(digest(&archive).unwrap(), manifest["archive_sha256"]);
    }
    #[test]
    fn committed_source_refuses_untracked_and_modified_inputs() {
        let dir = Dir::new();
        output(Command::new("git").arg("init").arg(&dir.0)).unwrap();
        fs::write(dir.0.join("source"), "original").unwrap();
        output(
            Command::new("git")
                .current_dir(&dir.0)
                .args(["add", "source"]),
        )
        .unwrap();
        output(Command::new("git").current_dir(&dir.0).args([
            "-c",
            "user.name=Release fixture",
            "-c",
            "user.email=release@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "fixture",
        ]))
        .unwrap();
        assert_eq!(source(&dir.0).unwrap().len(), 40);
        fs::write(dir.0.join("extra"), "untracked").unwrap();
        assert!(source(&dir.0).is_err());
        fs::remove_file(dir.0.join("extra")).unwrap();
        fs::write(dir.0.join("source"), "modified").unwrap();
        assert!(source(&dir.0).is_err());
    }
    #[test]
    fn output_arguments_refuse_existing_and_relative_destinations_before_build() {
        let dir = Dir::new();
        assert!(run(&["--out".into(), dir.0.display().to_string()]).is_err());
        assert!(run(&["--out".into(), "relative".into()]).is_err());
        assert!(run(&[]).is_err());
    }
}
