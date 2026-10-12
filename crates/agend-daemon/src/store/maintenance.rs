//! Home exclusion spanning database creation, daemon startup and uninstall.
//! The lock inode is retained permanently; deleting it would split waiters.

use std::fs::{self, File, OpenOptions};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;

use rusqlite::Connection;

use super::{DB_FILE, StoreError};

const LOCK: &str = ".agend-maintenance.lock";

pub(super) fn reader(home: &Path) -> Result<File, StoreError> {
    acquire(home, libc::LOCK_SH)
}

fn acquire(home: &Path, mode: libc::c_int) -> Result<File, StoreError> {
    super::create_private_dir(home)?;
    let path = home.join(LOCK);
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(&path)?;
    let meta = file.metadata()?;
    // SAFETY: getuid reads this process's uid without side effects.
    if !meta.is_file()
        || meta.uid() != unsafe { libc::getuid() }
        || meta.mode() & 0o077 != 0
        || meta.nlink() != 1
    {
        return Err(StoreError::Refused {
            path,
            reason: "maintenance lock must be a private owned regular file with one link",
        });
    }
    // SAFETY: file owns the descriptor; dropping it releases flock.
    if unsafe { libc::flock(file.as_raw_fd(), mode | libc::LOCK_NB) } != 0 {
        let error = std::io::Error::last_os_error();
        return if error.raw_os_error() == Some(libc::EWOULDBLOCK) {
            Err(StoreError::InUse)
        } else {
            Err(error.into())
        };
    }
    Ok(file)
}

/// A local setup operation that may publish home data while the daemon runs.
/// Its shared lease prevents an uninstall from deleting data during publication.
pub struct Activity {
    _home: File,
}
impl Activity {
    pub fn acquire(home: &Path) -> Result<Self, StoreError> {
        Ok(Self {
            _home: reader(home)?,
        })
    }
}

/// Excludes new store opens even before a database or daemon socket exists.
/// Also holds SQLite's native lock to refuse a pre-upgrade live daemon.
/// No schema migration, task update or missing database creation is performed.
pub struct Maintenance {
    _database: Option<Connection>,
    _home: File,
}

impl Maintenance {
    pub fn acquire(home: &Path) -> Result<Self, StoreError> {
        let guard = acquire(home, libc::LOCK_EX)?;
        let path = home.join(DB_FILE);
        let database = match fs::symlink_metadata(&path) {
            Ok(meta) if meta.is_file() && meta.nlink() == 1 => {
                super::check_existing(home, &path)?;
                Some(super::lock(&path)?)
            }
            Ok(_) => {
                return Err(StoreError::Refused {
                    path,
                    reason: "maintenance requires a regular database with one link",
                });
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.into()),
        };
        Ok(Self {
            _database: database,
            _home: guard,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::SqliteStore;
    use agend_testkit::tempdir::TempDir;

    #[test]
    fn publication_can_share_the_daemon_home_but_excludes_data_removal() {
        let root = TempDir::new("g13-maintenance-activity").unwrap();
        let activity = Activity::acquire(root.path()).unwrap();
        let store = SqliteStore::open(root.path(), 0).unwrap();
        drop(store);
        assert!(matches!(
            Maintenance::acquire(root.path()),
            Err(StoreError::InUse)
        ));
        drop(activity);
        let maintenance = Maintenance::acquire(root.path()).unwrap();
        assert!(matches!(
            Activity::acquire(root.path()),
            Err(StoreError::InUse)
        ));
        drop(maintenance);
        assert!(Activity::acquire(root.path()).is_ok());
    }

    #[test]
    fn exclusion_covers_a_missing_database_and_the_whole_store_lifetime() {
        let root = TempDir::new("g13-maintenance").unwrap();
        let maintenance = Maintenance::acquire(root.path()).unwrap();
        assert!(!root.path().join(DB_FILE).exists());
        assert!(matches!(
            SqliteStore::open(root.path(), 0),
            Err(StoreError::InUse)
        ));
        drop(maintenance);
        let store = SqliteStore::open(root.path(), 0).unwrap();
        // A store can already own the home while the daemon has no socket.
        assert!(!root.path().join("run/daemon.sock").exists());
        assert!(matches!(
            Maintenance::acquire(root.path()),
            Err(StoreError::InUse)
        ));
        drop(store);
        let maintenance = Maintenance::acquire(root.path()).unwrap();
        assert!(matches!(
            SqliteStore::open(root.path(), 0),
            Err(StoreError::InUse)
        ));
        drop(maintenance);
        drop(SqliteStore::open(root.path(), 0).unwrap());
    }

    #[test]
    fn native_sqlite_owner_without_the_new_home_lock_is_still_refused() {
        let root = TempDir::new("g13-maintenance-legacy").unwrap();
        drop(SqliteStore::open(root.path(), 0).unwrap());
        let legacy = super::super::lock(&root.path().join(DB_FILE)).unwrap();
        assert!(Maintenance::acquire(root.path()).is_err());
        drop(legacy);
        assert!(Maintenance::acquire(root.path()).is_ok());
    }
}
