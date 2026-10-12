//! One local preference file, atomically replaced only on explicit confirmation.
use super::Theme;
use std::{
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
static SERIAL: AtomicU64 = AtomicU64::new(0);

pub fn path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("AGEND_TUI_THEME_FILE").filter(|p| !p.is_empty()) {
        return Some(path.into());
    }
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".config")))?;
    Some(base.join("agend").join("tui-theme"))
}
pub fn load(path: &Path) -> io::Result<Theme> {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Theme::default()),
        Err(e) => return Err(e),
    };
    let mut value = String::new();
    file.take(65).read_to_string(&mut value)?;
    if value.len() > 64 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "theme preference exceeds 64 bytes",
        ));
    }
    Theme::parse(value.trim())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "unknown theme preference"))
}
pub fn save(path: &Path, theme: Theme) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let temp = parent.join(format!(
        ".agend-theme-{}-{}",
        std::process::id(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    ));
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temp)?;
    let result = (|| {
        writeln!(file, "{}", theme.id())?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}
