//! The shim symlinks (gate 6 P3): `$AGEND_HOME/bin/{git,kill,killall,pkill}`
//! point at the running `agend` binary, which acts as the shim when started
//! under those names (D5). Checked at every daemon boot; a missing link or
//! one pointing anywhere else is replaced, so an upgraded binary takes over
//! at the next daemon start.
//!
//! Must NOT: touch anything in `bin/` except these four names.

use std::fs;
use std::io;
use std::os::unix::fs::{DirBuilderExt, symlink};
use std::path::{Path, PathBuf};

/// The shim directory inside the AgEnD home; first on every agent's `PATH`.
pub const BIN_DIR: &str = "bin";
/// Names the shim answers to (`agend_shim::Tool`).
pub const NAMES: [&str; 4] = ["git", "kill", "killall", "pkill"];

/// Makes every shim link in `<home>/bin` point at `exe`; returns the names it
/// (re)created.
pub fn ensure(home: &Path, exe: &Path) -> io::Result<Vec<&'static str>> {
    let bin = home.join(BIN_DIR);
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&bin)?;
    let mut fixed = Vec::new();
    for name in NAMES {
        let link = bin.join(name);
        if fs::read_link(&link).is_ok_and(|target| target == exe) {
            continue;
        }
        // Built beside the link and renamed over it: never a moment without one.
        let tmp: PathBuf = bin.join(format!(".{name}.new"));
        match fs::remove_file(&tmp) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
        symlink(exe, &tmp)?;
        fs::rename(&tmp, &link)?;
        fixed.push(name);
    }
    Ok(fixed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use agend_testkit::tempdir::TempDir;

    #[test]
    fn missing_or_wrong_links_are_replaced_and_others_left_alone() {
        let dir = TempDir::new("g6-shims").unwrap();
        let home = dir.path();
        let exe = Path::new("/opt/agend/bin/agend");
        assert_eq!(ensure(home, exe).unwrap(), NAMES);
        for name in NAMES {
            assert_eq!(fs::read_link(home.join(BIN_DIR).join(name)).unwrap(), exe);
        }
        assert!(ensure(home, exe).unwrap().is_empty(), "already right");

        let bin = home.join(BIN_DIR);
        fs::remove_file(bin.join("kill")).unwrap();
        fs::remove_file(bin.join("git")).unwrap();
        symlink("/old/agend", bin.join("git")).unwrap();
        fs::write(bin.join("mine"), "keep").unwrap();
        assert_eq!(ensure(home, exe).unwrap(), ["git", "kill"]);
        assert_eq!(fs::read_link(bin.join("git")).unwrap(), exe);
        assert_eq!(fs::read_to_string(bin.join("mine")).unwrap(), "keep");
        let mut names: Vec<String> = fs::read_dir(&bin)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["git", "kill", "killall", "mine", "pkill"]);
    }
}
