//! Persist a daily attempt before network I/O and reject stale completions/acks.
use super::{SqliteStore, StoreError};
use agend_core::{
    model::Backend,
    setup::backend::{
        PublishedBackend, npm_package,
        observation::{REGISTRY_INTERVAL_MS, RegistryObservation},
        valid_version,
    },
};
use rusqlite::{Connection, OptionalExtension, params};

fn invalid() -> StoreError {
    StoreError::Invalid("invalid backend registry observation".into())
}
fn read(conn: &Connection, backend: &str) -> Result<Option<RegistryObservation>, StoreError> {
    let json: Option<String> = conn
        .query_row(
            "SELECT record FROM backend_registry WHERE backend=?1",
            [backend],
            |r| r.get(0),
        )
        .optional()?;
    json.map(|json| {
        let row: RegistryObservation = serde_json::from_str(&json).map_err(|_| invalid())?;
        let kind = Backend::parse(backend).ok_or_else(invalid)?;
        if row.latest.as_ref().is_some_and(|r| {
            r.backend != backend || r.package != npm_package(kind) || !valid_version(&r.version)
        }) || row
            .error
            .as_ref()
            .is_some_and(|e| e.is_empty() || e.len() > 512)
            || row.backend != backend
            || row.attempt == 0
            || row.acknowledged_revision > row.revision
            || row.completed_ms.is_some_and(|at| at < row.started_ms)
        {
            return Err(invalid());
        }
        Ok(row)
    })
    .transpose()
}
fn write(conn: &Connection, row: &RegistryObservation) -> Result<(), StoreError> {
    let json = serde_json::to_string(row).map_err(|_| invalid())?;
    conn.execute("INSERT INTO backend_registry(backend,record) VALUES(?1,?2) ON CONFLICT(backend) DO UPDATE SET record=excluded.record", params![row.backend,json])?;
    Ok(())
}
impl SqliteStore {
    pub async fn registry_observation(
        &self,
        backend: Backend,
    ) -> Result<Option<RegistryObservation>, StoreError> {
        self.call(move |conn| read(conn, backend.as_str())).await
    }
    /// Reserve once per day, including across crashes. Clock rollback never
    /// makes an attempt due early. A missing completion is not a success.
    pub async fn begin_registry_check(
        &self,
        backend: Backend,
        now: u64,
    ) -> Result<Option<RegistryObservation>, StoreError> {
        self.call(move |conn| {
            let tx = conn.transaction()?;
            let old = read(&tx, backend.as_str())?;
            if old
                .as_ref()
                .is_some_and(|r| now.saturating_sub(r.started_ms) < REGISTRY_INTERVAL_MS)
            {
                return Ok(None);
            }
            let mut row = old.unwrap_or(RegistryObservation {
                backend: backend.as_str().into(),
                attempt: 0,
                started_ms: now,
                completed_ms: None,
                latest: None,
                error: None,
                changed_ms: None,
                revision: 0,
                acknowledged_revision: 0,
            });
            row.attempt = row.attempt.checked_add(1).ok_or_else(invalid)?;
            row.started_ms = now;
            row.completed_ms = None;
            write(&tx, &row)?;
            tx.commit()?;
            Ok(Some(row))
        })
        .await
    }
    pub async fn finish_registry_check(
        &self,
        backend: Backend,
        attempt: u64,
        now: u64,
        result: Result<PublishedBackend, String>,
    ) -> Result<bool, StoreError> {
        match &result {
            Ok(r)
                if r.backend == backend.as_str()
                    && r.package == npm_package(backend)
                    && valid_version(&r.version) => {}
            Err(e) if !e.is_empty() && e.len() <= 512 => (),
            _ => return Err(invalid()),
        }
        self.call(move |conn| {
            let tx = conn.transaction()?;
            let Some(mut row) = read(&tx, backend.as_str())? else {
                return Ok(false);
            };
            if row.attempt != attempt || row.completed_ms.is_some() || now < row.started_ms {
                return Ok(false);
            }
            let previous = (row.latest.clone(), row.error.clone());
            match result {
                Ok(value) => {
                    row.latest = Some(value);
                    row.error = None;
                }
                Err(error) => row.error = Some(error),
            }
            if previous != (row.latest.clone(), row.error.clone()) {
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
    pub async fn acknowledge_registry_observation(
        &self,
        backend: Backend,
        revision: u64,
    ) -> Result<bool, StoreError> {
        self.call(move |conn| {
            let tx = conn.transaction()?;
            let Some(mut row) = read(&tx, backend.as_str())? else {
                return Ok(false);
            };
            if row.revision != revision || revision == 0 || row.acknowledged_revision == revision {
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
    use agend_testkit::{block_on, tempdir::TempDir};
    fn release() -> PublishedBackend {
        let native: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../tests/fixtures/backend_registry/codex.json"
        ))
        .unwrap();
        PublishedBackend {
            backend: "codex".into(),
            package: native["name"].as_str().unwrap().into(),
            version: native["version"].as_str().unwrap().into(),
        }
    }
    #[test]
    fn durable_schedule_stale_completion_and_exact_ack_survive_reopen() {
        let dir = TempDir::new("registry-store").unwrap();
        let home = dir.path().join("home");
        let now = 1_800_000_000_000;
        let backend = Backend::Codex;
        let store = SqliteStore::open(&home, now).unwrap();
        let first = block_on(store.begin_registry_check(backend, now))
            .unwrap()
            .unwrap();
        assert!(
            block_on(store.begin_registry_check(backend, now))
                .unwrap()
                .is_none()
        );
        drop(store);
        let store = SqliteStore::open(&home, now + 1).unwrap();
        for at in [now - 1, now + 1, now + REGISTRY_INTERVAL_MS - 1] {
            assert!(
                block_on(store.begin_registry_check(backend, at))
                    .unwrap()
                    .is_none()
            );
        }
        let second = block_on(store.begin_registry_check(backend, now + REGISTRY_INTERVAL_MS))
            .unwrap()
            .unwrap();
        assert!(
            !block_on(store.finish_registry_check(
                backend,
                first.attempt,
                now + REGISTRY_INTERVAL_MS,
                Ok(release())
            ))
            .unwrap()
        );
        assert!(
            block_on(store.finish_registry_check(
                backend,
                second.attempt,
                now + REGISTRY_INTERVAL_MS,
                Ok(release())
            ))
            .unwrap()
        );
        assert!(
            !block_on(store.finish_registry_check(
                backend,
                second.attempt,
                now + REGISTRY_INTERVAL_MS,
                Err("late".into())
            ))
            .unwrap()
        );
        let row = block_on(store.registry_observation(backend))
            .unwrap()
            .unwrap();
        assert_eq!(row.revision, 1);
        assert!(block_on(store.acknowledge_registry_observation(backend, row.revision)).unwrap());
        drop(store);
        let store = SqliteStore::open(&home, now + REGISTRY_INTERVAL_MS).unwrap();
        assert_eq!(
            block_on(store.registry_observation(backend))
                .unwrap()
                .unwrap()
                .acknowledged_revision,
            1
        );
        let at = now + REGISTRY_INTERVAL_MS * 2;
        let third = block_on(store.begin_registry_check(backend, at))
            .unwrap()
            .unwrap();
        assert!(
            block_on(store.finish_registry_check(
                backend,
                third.attempt,
                at,
                Err("registry unavailable".into())
            ))
            .unwrap()
        );
        let failed = block_on(store.registry_observation(backend))
            .unwrap()
            .unwrap();
        assert_eq!(failed.latest, Some(release()));
        assert_eq!(failed.revision, 2);
        assert!(!block_on(store.acknowledge_registry_observation(backend, 1)).unwrap());
        assert!(block_on(store.acknowledge_registry_observation(backend, 2)).unwrap());
        let at = at + REGISTRY_INTERVAL_MS;
        let fourth = block_on(store.begin_registry_check(backend, at))
            .unwrap()
            .unwrap();
        assert!(
            block_on(store.finish_registry_check(
                backend,
                fourth.attempt,
                at,
                Err("registry unavailable".into())
            ))
            .unwrap()
        );
        assert_eq!(
            block_on(store.registry_observation(backend))
                .unwrap()
                .unwrap()
                .revision,
            2
        );
    }
    #[test]
    fn invalid_identity_and_backdated_completion_cannot_publish() {
        let dir = TempDir::new("registry-invalid").unwrap();
        let store = SqliteStore::open(&dir.path().join("home"), 100).unwrap();
        let row = block_on(store.begin_registry_check(Backend::Codex, 100))
            .unwrap()
            .unwrap();
        let mut wrong = release();
        wrong.package = "foreign".into();
        assert!(
            block_on(store.finish_registry_check(Backend::Codex, row.attempt, 100, Ok(wrong)))
                .is_err()
        );
        assert!(
            !block_on(store.finish_registry_check(Backend::Codex, row.attempt, 99, Ok(release())))
                .unwrap()
        );
        assert!(!block_on(store.acknowledge_registry_observation(Backend::Codex, 0)).unwrap());
        assert!(
            block_on(store.registry_observation(Backend::Codex))
                .unwrap()
                .unwrap()
                .latest
                .is_none()
        );
    }
}
