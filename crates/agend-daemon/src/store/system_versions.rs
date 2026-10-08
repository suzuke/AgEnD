//! Durable external-version checks, scoped to the configured instance.
use super::{SqliteStore, StoreError};
use agend_core::setup::backend::observation::{SystemBackendVersion, SystemVersionObservation};
use rusqlite::{Connection, OptionalExtension, params};

pub const CHECK_INTERVAL_MS: u64 = 60_000;
fn invalid() -> StoreError {
    StoreError::Invalid("invalid system version observation".into())
}
fn scope(conn: &Connection, id: &str) -> Result<Option<(String, String, String)>, StoreError> {
    Ok(conn
        .query_row(
            "SELECT backend,program,working_directory FROM instances WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?)
}
fn read(conn: &Connection, id: &str) -> Result<Option<SystemVersionObservation>, StoreError> {
    let json: Option<String> = conn
        .query_row(
            "SELECT record FROM system_versions WHERE instance_id=?1",
            [id],
            |r| r.get(0),
        )
        .optional()?;
    json.map(|json| {
        let row: SystemVersionObservation = serde_json::from_str(&json).map_err(|_| invalid())?;
        if row.instance_id != id
            || row.generation.is_empty()
            || row.attempt == 0
            || row.acknowledged_revision > row.revision
        {
            return Err(invalid());
        }
        Ok(row)
    })
    .transpose()
}
fn write(conn: &Connection, row: &SystemVersionObservation) -> Result<(), StoreError> {
    let json = serde_json::to_string(row).map_err(|_| invalid())?;
    conn.execute("INSERT INTO system_versions(instance_id,record) VALUES(?1,?2) ON CONFLICT(instance_id) DO UPDATE SET record=excluded.record",params![row.instance_id,json])?;
    Ok(())
}
impl SqliteStore {
    pub async fn system_version_observation(
        &self,
        id: &str,
    ) -> Result<Option<SystemVersionObservation>, StoreError> {
        let id = id.to_owned();
        self.call(move |conn| read(conn, &id)).await
    }
    /// Reserve before probing; crashes and clock rollback do not cause a tight loop.
    pub async fn begin_system_version_check(
        &self,
        id: &str,
        now: u64,
    ) -> Result<Option<SystemVersionObservation>, StoreError> {
        let id = id.to_owned();
        self.call(move |conn| {
            let tx = conn.transaction()?;
            let Some((backend, program, working_directory)) = scope(&tx, &id)? else {
                return Ok(None);
            };
            let old = read(&tx, &id)?.filter(|r| {
                r.backend == backend
                    && r.program == program
                    && r.working_directory == working_directory
            });
            if old
                .as_ref()
                .is_some_and(|r| now.saturating_sub(r.started_ms) < CHECK_INTERVAL_MS)
            {
                return Ok(None);
            }
            let mut row = match old {
                Some(row) => row,
                None => SystemVersionObservation {
                    instance_id: id.clone(),
                    generation: super::instances::new_session_id()?,
                    backend: backend.clone(),
                    program: program.clone(),
                    working_directory: working_directory.clone(),
                    attempt: 0,
                    started_ms: now,
                    completed_ms: None,
                    latest: None,
                    error: None,
                    changed_ms: None,
                    revision: 0,
                    acknowledged_revision: 0,
                },
            };
            row.backend = backend;
            row.program = program;
            row.working_directory = working_directory;
            row.attempt = row.attempt.checked_add(1).ok_or_else(invalid)?;
            row.started_ms = now;
            row.completed_ms = None;
            write(&tx, &row)?;
            tx.commit()?;
            Ok(Some(row))
        })
        .await
    }
    pub async fn finish_system_version_check(
        &self,
        ticket: &SystemVersionObservation,
        now: u64,
        result: Result<SystemBackendVersion, String>,
    ) -> Result<bool, StoreError> {
        match &result {
            Ok(v)
                if v.backend == ticket.backend
                    && v.configured_program == ticket.program
                    && std::path::Path::new(&v.resolved_program).is_absolute()
                    && !v.version_output.is_empty()
                    && v.version_output.len() <= 65536
                    && v.sha256.len() == 64
                    && v.sha256.bytes().all(|b| b.is_ascii_hexdigit()) => {}
            Err(e) if !e.is_empty() && e.len() <= 512 => (),
            _ => return Err(invalid()),
        }
        let ticket = ticket.clone();
        self.call(move |conn| {
            let tx = conn.transaction()?;
            let Some(mut row) = read(&tx, &ticket.instance_id)? else {
                return Ok(false);
            };
            if row.generation != ticket.generation
                || row.attempt != ticket.attempt
                || row.backend != ticket.backend
                || row.program != ticket.program
                || row.working_directory != ticket.working_directory
                || row.started_ms != ticket.started_ms
                || row.completed_ms.is_some()
                || now < row.started_ms
                || scope(&tx, &row.instance_id)?
                    != Some((
                        row.backend.clone(),
                        row.program.clone(),
                        row.working_directory.clone(),
                    ))
            {
                return Ok(false);
            }
            let before = (row.latest.clone(), row.error.clone());
            match result {
                Ok(v) => {
                    row.latest = Some(v);
                    row.error = None
                }
                Err(e) => row.error = Some(e),
            }
            // The first successful sample establishes a quiet baseline. A
            // failure, recovery or later identity change needs an exact ack.
            if before != (row.latest.clone(), row.error.clone())
                && (before.0.is_some() || before.1.is_some() || row.error.is_some())
            {
                row.revision = row.revision.checked_add(1).ok_or_else(invalid)?;
                row.changed_ms = Some(now);
            }
            row.completed_ms = Some(now);
            write(&tx, &row)?;
            tx.commit()?;
            Ok(true)
        })
        .await
    }
    pub async fn acknowledge_system_version(
        &self,
        id: &str,
        generation: &str,
        revision: u64,
    ) -> Result<bool, StoreError> {
        let id = id.to_owned();
        let generation = generation.to_owned();
        self.call(move |conn| {
            let tx = conn.transaction()?;
            let Some(mut row) = read(&tx, &id)? else {
                return Ok(false);
            };
            if row.generation != generation
                || row.revision != revision
                || revision == 0
                || row.acknowledged_revision == revision
                || scope(&tx, &id)?
                    != Some((
                        row.backend.clone(),
                        row.program.clone(),
                        row.working_directory.clone(),
                    ))
            {
                return Ok(false);
            }
            row.acknowledged_revision = revision;
            write(&tx, &row)?;
            tx.commit()?;
            Ok(true)
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agend_core::{
        model::Backend,
        runtime_records::{Instance, InstanceStatus},
    };
    use agend_testkit::{block_on, tempdir::TempDir};
    use std::{collections::BTreeMap, fs, os::unix::fs::PermissionsExt};
    fn instance(home: &std::path::Path) -> Instance {
        Instance {
            id: "version-test".into(),
            backend: Backend::Codex,
            program: home.join("backend").to_str().unwrap().into(),
            args: vec![],
            working_directory: home.to_str().unwrap().into(),
            session_id: None,
            status: InstanceStatus::New,
            session_started: false,
            agent_pid: None,
            legacy_no_thread: false,
            delivery: "push".into(),
        }
    }
    fn native(home: &std::path::Path, version: &str) -> SystemBackendVersion {
        let path = home.join("backend");
        fs::write(
            &path,
            format!("#!/bin/sh\n[ \"$1\" = --version ] || exit 2\nprintf '{version}\\n'\n"),
        )
        .unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        crate::backend_versions::system_version::observe(
            home,
            Backend::Codex,
            path.to_str().unwrap(),
            home,
            &BTreeMap::from([("PATH".into(), "/usr/bin:/bin".into())]),
        )
        .unwrap()
        .unwrap()
    }
    #[test]
    fn native_baseline_drift_failure_recovery_and_exact_ack_survive_reopen() {
        let dir = TempDir::new("system-version-store").unwrap();
        let home = dir.path().canonicalize().unwrap();
        let store = SqliteStore::open(&home, 100).unwrap();
        block_on(store.add_instance(&instance(&home))).unwrap();
        let first = block_on(store.begin_system_version_check("version-test", 100))
            .unwrap()
            .unwrap();
        let v1 = native(&home, "codex 1");
        assert!(block_on(store.finish_system_version_check(&first, 100, Ok(v1.clone()))).unwrap());
        let baseline = block_on(store.system_version_observation("version-test"))
            .unwrap()
            .unwrap();
        assert_eq!(baseline.revision, 0);
        drop(store);
        let store = SqliteStore::open(&home, 101).unwrap();
        assert!(
            block_on(store.begin_system_version_check("version-test", 99))
                .unwrap()
                .is_none()
        );
        assert!(
            block_on(store.begin_system_version_check("version-test", 101))
                .unwrap()
                .is_none()
        );
        let second = block_on(store.begin_system_version_check("version-test", 60100))
            .unwrap()
            .unwrap();
        let v2 = native(&home, "codex 2");
        assert!(!block_on(store.finish_system_version_check(&first, 60100, Ok(v1))).unwrap());
        assert!(
            block_on(store.finish_system_version_check(&second, 60100, Ok(v2.clone()))).unwrap()
        );
        assert!(
            !block_on(store.finish_system_version_check(&second, 60100, Ok(v2.clone()))).unwrap()
        );
        assert!(
            block_on(store.acknowledge_system_version("version-test", &second.generation, 1))
                .unwrap()
        );
        let third = block_on(store.begin_system_version_check("version-test", 120100))
            .unwrap()
            .unwrap();
        assert!(
            block_on(store.finish_system_version_check(&third, 120100, Err("unavailable".into())))
                .unwrap()
        );
        let failed = block_on(store.system_version_observation("version-test"))
            .unwrap()
            .unwrap();
        assert_eq!(failed.latest, Some(v2.clone()));
        assert_eq!(failed.revision, 2);
        assert!(
            !block_on(store.acknowledge_system_version("version-test", &third.generation, 1))
                .unwrap()
        );
        let fourth = block_on(store.begin_system_version_check("version-test", 180100))
            .unwrap()
            .unwrap();
        assert!(
            block_on(store.finish_system_version_check(&fourth, 180100, Ok(v2.clone()))).unwrap()
        );
        let fifth = block_on(store.begin_system_version_check("version-test", 240100))
            .unwrap()
            .unwrap();
        assert!(block_on(store.finish_system_version_check(&fifth, 240100, Ok(v2))).unwrap());
        drop(store);
        let store = SqliteStore::open(&home, 240101).unwrap();
        let row = block_on(store.system_version_observation("version-test"))
            .unwrap()
            .unwrap();
        assert_eq!(row.revision, 3);
        assert_eq!(row.changed_ms, Some(180100));
        assert_eq!(row.acknowledged_revision, 1);
    }
    #[test]
    fn removal_recreation_changed_scope_and_forged_ticket_cannot_publish_or_ack() {
        let dir = TempDir::new("system-version-identity").unwrap();
        let home = dir.path().canonicalize().unwrap();
        let store = SqliteStore::open(&home, 100).unwrap();
        let instance = instance(&home);
        block_on(store.add_instance(&instance)).unwrap();
        let first = block_on(store.begin_system_version_check("version-test", 100))
            .unwrap()
            .unwrap();
        let value = native(&home, "codex 1");
        let mut forged = first.clone();
        forged.working_directory = "/foreign".into();
        assert!(
            !block_on(store.finish_system_version_check(&forged, 100, Ok(value.clone()))).unwrap()
        );
        block_on(store.call(|conn| {
            conn.execute(
                "UPDATE instances SET working_directory='/foreign' WHERE id='version-test'",
                [],
            )?;
            Ok(())
        }))
        .unwrap();
        assert!(
            !block_on(store.finish_system_version_check(&first, 100, Ok(value.clone()))).unwrap()
        );
        assert!(block_on(store.remove_instance("version-test")).unwrap());
        assert!(
            block_on(store.system_version_observation("version-test"))
                .unwrap()
                .is_none()
        );
        block_on(store.add_instance(&instance)).unwrap();
        let new = block_on(store.begin_system_version_check("version-test", 101))
            .unwrap()
            .unwrap();
        assert_ne!(new.generation, first.generation);
        assert!(!block_on(store.finish_system_version_check(&first, 101, Ok(value))).unwrap());
        assert!(
            block_on(store.finish_system_version_check(&new, 101, Err("missing".into()))).unwrap()
        );
        assert!(
            !block_on(store.acknowledge_system_version("version-test", &first.generation, 1))
                .unwrap()
        );
        assert!(
            block_on(store.acknowledge_system_version("version-test", &new.generation, 1)).unwrap()
        );
    }
    #[test]
    fn changed_scope_with_the_same_error_never_inherits_an_old_ack() {
        let dir = TempDir::new("system-version-scope").unwrap();
        let home = dir.path().canonicalize().unwrap();
        let store = SqliteStore::open(&home, 100).unwrap();
        block_on(store.add_instance(&instance(&home))).unwrap();
        let old = block_on(store.begin_system_version_check("version-test", 100))
            .unwrap()
            .unwrap();
        assert!(
            block_on(store.finish_system_version_check(&old, 100, Err("unavailable".into())))
                .unwrap()
        );
        assert!(
            block_on(store.acknowledge_system_version("version-test", &old.generation, 1)).unwrap()
        );
        block_on(store.call(|conn| {
            conn.execute(
                "UPDATE instances SET working_directory='/foreign' WHERE id='version-test'",
                [],
            )?;
            Ok(())
        }))
        .unwrap();
        assert!(
            !block_on(store.acknowledge_system_version("version-test", &old.generation, 1))
                .unwrap()
        );
        let new = block_on(store.begin_system_version_check("version-test", 101))
            .unwrap()
            .unwrap();
        assert_ne!(new.generation, old.generation);
        assert!(new.latest.is_none());
        assert!(
            block_on(store.finish_system_version_check(&new, 101, Err("unavailable".into())))
                .unwrap()
        );
        assert!(
            !block_on(store.acknowledge_system_version("version-test", &old.generation, 1))
                .unwrap()
        );
        let row = block_on(store.system_version_observation("version-test"))
            .unwrap()
            .unwrap();
        assert_eq!(row.revision, 1);
        assert_eq!(row.acknowledged_revision, 0);
        let cwd = home.to_str().unwrap().to_owned();
        block_on(store.call(move |conn| {
            conn.execute(
                "UPDATE instances SET working_directory=?1 WHERE id='version-test'",
                [cwd],
            )?;
            Ok(())
        }))
        .unwrap();
        // B is still unacknowledged: scope changed, but no A reservation yet.
        assert!(
            !block_on(store.acknowledge_system_version("version-test", &new.generation, 1))
                .unwrap()
        );
        let restored = block_on(store.begin_system_version_check("version-test", 102))
            .unwrap()
            .unwrap();
        assert_ne!(restored.generation, old.generation);
        assert_ne!(restored.generation, new.generation);
        let value = native(&home, "codex 1");
        assert!(block_on(store.finish_system_version_check(&restored, 102, Ok(value))).unwrap());
        let row = block_on(store.system_version_observation("version-test"))
            .unwrap()
            .unwrap();
        assert_eq!(row.revision, 0);
        assert!(row.error.is_none());
    }
}
