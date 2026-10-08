//! Explicit data deletion under the installation and maintenance locks.
//! Keep lock inodes and the receipt until completion so retries remain safe.

use std::fs::{self, File, OpenOptions};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;

use super::files;

pub fn preflight(home: &Path) -> Result<(), String> {
    if !files::existing_directory(home)? {
        return Err("installation home disappeared; refusing deletion".into());
    }
    let meta = fs::symlink_metadata(home).map_err(|e| e.to_string())?;
    let mount = mount_identity(&open_directory(home)?)?;
    inspect(home, meta.dev(), meta.uid(), &mount, 0)
}

fn inspect(path: &Path, device: u64, owner: u32, mount: &[u8], depth: usize) -> Result<(), String> {
    let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if depth > 128 || meta.dev() != device || meta.uid() != owner {
        return Err(format!(
            "{} crosses a deletion ownership, mount or depth boundary; preserving data",
            path.display()
        ));
    }
    if path.file_name().is_some_and(|name| name == ".git") {
        return Err(format!(
            "{} is a Git workspace; finish/cancel tasks and move retained repositories out of this home before deleting data",
            path.display()
        ));
    }
    if meta.is_dir() {
        let dir = open_directory(path)?;
        let opened = dir.metadata().map_err(|e| e.to_string())?;
        if opened.ino() != meta.ino()
            || opened.dev() != meta.dev()
            || mount_identity(&dir)? != mount
        {
            return Err(format!(
                "{} changed or crosses a mount boundary; preserving data",
                path.display()
            ));
        }
        if meta.mode() & 0o022 != 0 {
            return Err(format!(
                "{} is writable by others; preserving data",
                path.display()
            ));
        }
        for entry in fs::read_dir(path).map_err(|e| e.to_string())? {
            inspect(
                &entry.map_err(|e| e.to_string())?.path(),
                device,
                owner,
                mount,
                depth + 1,
            )?;
        }
    }
    // Symlinks are leaves: delete the link, never inspect/delete its target.
    Ok(())
}

pub fn delete(home: &Path) -> Result<(), String> {
    // Repeat after the daemon and holders have stopped, before deleting data.
    preflight(home)?;
    for entry in fs::read_dir(home).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        if entry.file_name() == ".agend-maintenance.lock" {
            continue;
        }
        if entry.file_name() == "service" {
            for item in fs::read_dir(entry.path()).map_err(|e| e.to_string())? {
                let item = item.map_err(|e| e.to_string())?;
                if item.file_name() != "install.lock" && item.file_name() != files::RECORD {
                    remove(&item.path())?;
                }
            }
        } else {
            remove(&entry.path())?;
        }
    }
    fs::File::open(home)
        .and_then(|file| file.sync_all())
        .map_err(|e| e.to_string())
}

fn remove(path: &Path) -> Result<(), String> {
    let meta = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    let result = if meta.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    };
    result.map_err(|e| {
        format!(
            "data deletion incomplete at {}: {e}; retry with the same confirmation",
            path.display()
        )
    })
}

fn open_directory(path: &Path) -> Result<File, String> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(path)
        .map_err(|e| e.to_string())
}

#[cfg(target_os = "linux")]
fn mount_identity(dir: &File) -> Result<Vec<u8>, String> {
    // /proc documents mnt_id as the mount identity of this open descriptor:
    // https://docs.kernel.org/filesystems/proc.html#proc-pid-fdinfo-fd-information-about-opened-file
    let info = fs::read_to_string(format!("/proc/self/fdinfo/{}", dir.as_raw_fd()))
        .map_err(|e| e.to_string())?;
    let value = info
        .lines()
        .find_map(|line| line.strip_prefix("mnt_id:"))
        .map(str::trim)
        .ok_or("cannot establish mount identity; refusing deletion")?;
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("invalid mount identity; refusing deletion".into());
    }
    Ok(value.as_bytes().to_vec())
}

#[cfg(target_os = "macos")]
fn mount_identity(dir: &File) -> Result<Vec<u8>, String> {
    let mut info = std::mem::MaybeUninit::<libc::statfs>::uninit();
    // SAFETY: dir owns a valid fd; the output has statfs size/alignment and is
    // read only after fstatfs reports success.
    if unsafe { libc::fstatfs(dir.as_raw_fd(), info.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    // SAFETY: successful fstatfs initialized the structure above.
    let info = unsafe { info.assume_init() };
    Ok(info
        .f_mntonname
        .iter()
        .take_while(|&&byte| byte != 0)
        .map(|&byte| byte as u8)
        .collect())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn mount_identity(_: &File) -> Result<Vec<u8>, String> {
    Err("data deletion requires macOS or Linux mount identity checks".into())
}
