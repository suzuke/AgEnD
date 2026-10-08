//! Explicit operator publication, preserving original bytes and refusing replacement config.
use crate::{
    cli::{Failure, Output},
    service::files,
};
use agend_core::{config::Config, telegram::pairing::PairingRecord};
use agend_daemon::notifier::config;
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

const LIMIT: u64 = 1024 * 1024;
struct Snapshot {
    bytes: String,
    dev: u64,
    ino: u64,
}
fn snapshot(path: &Path) -> Result<Option<Snapshot>, String> {
    let Some(file) = files::regular(path)? else {
        return Ok(None);
    };
    let metadata = file
        .metadata()
        .map_err(|_| "cannot inspect configuration")?;
    let mut bytes = String::new();
    file.take(LIMIT + 1)
        .read_to_string(&mut bytes)
        .map_err(|_| "cannot read configuration as UTF-8")?;
    if bytes.len() as u64 > LIMIT {
        return Err("configuration exceeds 1 MiB".into());
    }
    Ok(Some(Snapshot {
        bytes,
        dev: metadata.dev(),
        ino: metadata.ino(),
    }))
}
fn unchanged(path: &Path, original: &Snapshot) -> Result<(), String> {
    let current = snapshot(path)?.ok_or("configuration disappeared; nothing published")?;
    if current.dev != original.dev || current.ino != original.ino || current.bytes != original.bytes
    {
        return Err("configuration changed during apply; inspect it and retry".into());
    }
    Ok(())
}
struct Temporary(PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

pub(super) fn run(home: &Path, record: &PairingRecord) -> Result<Output, Failure> {
    apply(home, record).map_err(|message| Failure::new("telegram_config", message))
}
fn apply(home: &Path, record: &PairingRecord) -> Result<Output, String> {
    if !agend_core::protocol::client::is_uuid_v4(&record.session.id) {
        return Err("invalid pairing receipt identity".into());
    }
    let telegram = record.configuration()?;
    // Shares the installation lock, preventing our uninstall/config writers from racing.
    let _lock = files::lock(home)?;
    let path = home.join("config.toml");
    let original = snapshot(&path)?;
    let text = original.as_ref().map_or("", |s| s.bytes.as_str());
    let current = config::parse(text)?;
    if current.telegram.as_ref() == Some(&telegram) {
        return Ok(Output::new(
            vec!["Telegram configuration already matches; no file changed".into()],
            serde_json::json!({"applied":false,"already_matches":true,"pairing_id":record.session.id}),
        ));
    }
    if current.telegram.is_some() {
        return Err(
            "config.toml already configures a different Telegram destination; preserved".into(),
        );
    }
    let encoded = config::encode(&Config {
        registry_checks: None,
        backend_version_checks: None,
        telegram: Some(telegram.clone()),
    })?;
    let combined = format!("{text}\n{encoded}");
    if combined.len() as u64 > LIMIT {
        return Err("resulting configuration exceeds 1 MiB".into());
    }
    if config::parse(&combined)?.telegram != Some(telegram) {
        return Err("generated Telegram configuration did not round-trip".into());
    }
    let temporary = Temporary(home.join(format!(
        ".config-telegram-{}.tmp",
        super::new_id().map_err(|_| "cannot generate configuration temporary name")?
    )));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&temporary.0)
        .map_err(|_| "cannot create configuration temporary")?;
    file.write_all(combined.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|_| "cannot write configuration temporary")?;
    drop(file);
    let backup = if let Some(original) = &original {
        unchanged(&path, original)?;
        let backup = home.join(format!("config.before-telegram-{}.toml", record.session.id));
        // A hard link retains the old inode, including edits through an already-open fd.
        fs::hard_link(&path, &backup).map_err(|_| "cannot reserve original configuration backup; inspect existing backup before retrying")?;
        File::open(home)
            .and_then(|dir| dir.sync_all())
            .map_err(|_| "cannot sync original configuration backup; nothing published")?;
        unchanged(&backup, original)?;
        unchanged(&path, original)?;
        fs::rename(&temporary.0, &path)
            .map_err(|_| "cannot publish configuration; original backup retained")?;
        Some(backup)
    } else {
        fs::hard_link(&temporary.0, &path)
            .map_err(|_| "configuration appeared during apply; preserved")?;
        None
    };
    File::open(home).and_then(|dir| dir.sync_all()).map_err(|_| "configuration published but directory sync failed; inspect config.toml before retrying")?;
    let mut lines =
        vec!["Telegram configuration applied; restart the daemon to activate it".into()];
    if let Some(backup) = &backup {
        lines.push(format!(
            "Original configuration retained at {}",
            backup.display()
        ));
    }
    Ok(Output::new(
        lines,
        serde_json::json!({"applied":true,"pairing_id":record.session.id,"config":path,"backup":backup,"restart_required":true}),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use agend_core::{config::SecretRef, telegram::pairing::*};
    use agend_testkit::{fakes::FakeClock, tempdir::TempDir};
    use std::os::unix::fs::symlink;
    fn confirmed() -> PairingRecord {
        let clock = FakeClock::new(1000);
        let mut session = TelegramPairing::new(
            "11111111-1111-4111-8111-111111111111".into(),
            SecretRef::File("/private/unused-token".into()),
            123,
            "test_bot".into(),
            &clock,
        )
        .unwrap();
        let candidate = PairingCandidate {
            chat_id: -10042,
            user_id: 42,
            topic_id: Some(7),
        };
        let command = session.command();
        session
            .observe(candidate.clone(), &command, 1, &clock)
            .unwrap();
        session.confirm(&candidate, &clock).unwrap();
        PairingRecord {
            session,
            phase: PairingPhase::Confirmed,
        }
    }
    #[test]
    fn preserves_original_comments_and_is_idempotent_without_resolving_token() {
        let dir = TempDir::new("telegram-apply").unwrap();
        let path = dir.path().join("config.toml");
        let old = "# human notes 繁中\nregistry_checks = false\nbackend_version_checks = false\n# no inline secrets\n";
        fs::write(&path, old).unwrap();
        let record = confirmed();
        apply(dir.path(), &record).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.starts_with(old));
        assert_eq!(config::parse(&text).unwrap().registry_checks, Some(false));
        assert_eq!(
            config::parse(&text).unwrap().backend_version_checks,
            Some(false)
        );
        assert_eq!(
            config::parse(&text).unwrap().telegram,
            Some(record.configuration().unwrap())
        );
        assert_eq!(
            fs::read_to_string(
                dir.path()
                    .join(format!("config.before-telegram-{}.toml", record.session.id))
            )
            .unwrap(),
            old
        );
        assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
        apply(dir.path(), &record).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), text);
        assert!(fs::read_dir(dir.path()).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp")
        }));
    }
    #[test]
    fn refuses_unconfirmed_foreign_config_and_symlinks_without_changes() {
        let dir = TempDir::new("telegram-apply").unwrap();
        let mut record = confirmed();
        record.phase = PairingPhase::Pending;
        assert!(apply(dir.path(), &record).is_err());
        assert!(!dir.path().join("config.toml").exists());
        record.phase = PairingPhase::Confirmed;
        let outside = dir.path().join("outside");
        fs::write(&outside, "# preserve me\n").unwrap();
        let path = dir.path().join("config.toml");
        symlink(&outside, &path).unwrap();
        assert!(apply(dir.path(), &record).is_err());
        assert_eq!(fs::read_to_string(&outside).unwrap(), "# preserve me\n");
        fs::remove_file(&path).unwrap();
        let mut foreign = record.configuration().unwrap();
        foreign.chat_id -= 1;
        let text = config::encode(&Config {
            registry_checks: None,
            backend_version_checks: None,
            telegram: Some(foreign),
        })
        .unwrap();
        fs::write(&path, &text).unwrap();
        assert!(apply(dir.path(), &record).is_err());
        assert_eq!(fs::read_to_string(path).unwrap(), text);
    }
    #[test]
    fn missing_file_publication_and_install_lock_are_exclusive() {
        let dir = TempDir::new("telegram-apply").unwrap();
        let record = confirmed();
        let lock = files::lock(dir.path()).unwrap();
        assert!(apply(dir.path(), &record).is_err());
        assert!(!dir.path().join("config.toml").exists());
        drop(lock);
        apply(dir.path(), &record).unwrap();
        assert_eq!(
            config::parse(&fs::read_to_string(dir.path().join("config.toml")).unwrap())
                .unwrap()
                .telegram,
            Some(record.configuration().unwrap())
        );
    }
    #[test]
    fn existing_backup_is_preserved_and_failed_apply_removes_temporary() {
        let dir = TempDir::new("telegram-apply").unwrap();
        let record = confirmed();
        let path = dir.path().join("config.toml");
        fs::write(&path, "# original\n").unwrap();
        let backup = dir
            .path()
            .join(format!("config.before-telegram-{}.toml", record.session.id));
        fs::write(&backup, "foreign backup").unwrap();
        assert!(apply(dir.path(), &record).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), "# original\n");
        assert_eq!(fs::read_to_string(&backup).unwrap(), "foreign backup");
        assert!(fs::read_dir(dir.path()).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp")
        }));
    }
    #[test]
    fn snapshot_checks_reject_in_place_edits_and_replacements() {
        let dir = TempDir::new("telegram-apply").unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "# old\n").unwrap();
        let original = snapshot(&path).unwrap().unwrap();
        fs::write(&path, "# edited\n").unwrap();
        assert!(unchanged(&path, &original).is_err());
        fs::remove_file(&path).unwrap();
        fs::write(&path, "# old\n").unwrap();
        // Keep the old inode alive so the replacement cannot recycle it.
        // The content check above already covers in-place edits.
        let before = snapshot(&path).unwrap().unwrap();
        let held = File::open(&path).unwrap();
        fs::remove_file(&path).unwrap();
        fs::write(&path, "# old\n").unwrap();
        assert!(unchanged(&path, &before).is_err());
        drop(held);
    }
}
