//! CAS ownership ledger; stale writers cannot change a task's remote identity.
use super::{SqliteStore, StoreError};
use agend_core::github::{GithubChange, GithubStore, VersionedGithubChange};
use rusqlite::{Connection, OptionalExtension, params};

fn load(c: &Connection, task: &str) -> Result<Option<VersionedGithubChange>, StoreError> {
    let row: Option<(u64, String, String)> = c
        .query_row(
            "SELECT revision,identity,change FROM github_changes WHERE task_id=?1",
            [task],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    row.map(|(revision, identity, body)| {
        let change: GithubChange =
            serde_json::from_str(&body).map_err(|e| StoreError::Invalid(e.to_string()))?;
        let stored_identity =
            serde_json::from_str(&identity).map_err(|e| StoreError::Invalid(e.to_string()))?;
        if change.identity.task_id != task || change.identity != stored_identity {
            return Err(StoreError::Invalid(
                "GitHub ownership ledger identity mismatch".into(),
            ));
        }
        Ok(VersionedGithubChange { revision, change })
    })
    .transpose()
}

impl GithubStore for SqliteStore {
    type Error = StoreError;
    async fn github_change(&self, task: &str) -> Result<Option<VersionedGithubChange>, StoreError> {
        let task = task.to_owned();
        self.call(move |c| load(c, &task)).await
    }
    async fn save_github_change(
        &self,
        expected: Option<u64>,
        change: &GithubChange,
    ) -> Result<bool, StoreError> {
        let change = change.clone();
        self.call(move |c| {
            let tx = c.transaction()?;
            let old = load(&tx, &change.identity.task_id)?;
            if old.as_ref().map(|row| row.revision) != expected {
                return Ok(false);
            }
            if !change.follows(old.as_ref().map(|row| &row.change)) {
                return Err(StoreError::Invalid("invalid GitHub ownership transition".into()));
            }
            let revision = expected.unwrap_or(0).checked_add(1)
                .filter(|n| *n <= i64::MAX as u64)
                .ok_or_else(|| StoreError::Invalid("GitHub ownership revision exhausted".into()))?;
            let identity = serde_json::to_string(&change.identity).map_err(|e| StoreError::Invalid(e.to_string()))?;
            let body = serde_json::to_string(&change).map_err(|e| StoreError::Invalid(e.to_string()))?;
            tx.execute("INSERT INTO github_changes(task_id,revision,identity,change,repository_id,branch) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(task_id) DO UPDATE SET revision=excluded.revision,change=excluded.change", params![change.identity.task_id,revision,identity,body,change.identity.repository_id,change.identity.branch])?;
            tx.commit()?;
            Ok(true)
        }).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agend_core::{github::GithubIdentity, pipeline::task::Task, traits::Store};
    use agend_testkit::{block_on, tempdir::TempDir};

    fn initial(task: &str) -> GithubChange {
        GithubChange {
            identity: GithubIdentity {
                task_id: task.into(),
                local_repo: "/repo".into(),
                repository: "owner/repo".into(),
                repository_id: 42,
                base: "main".into(),
                branch: "agend/task".into(),
                nonce: "owned-nonce".into(),
            },
            pull_number: None,
            pushed_head: None,
            push_intent: None,
            create_attempted: false,
            cleanup: Default::default(),
        }
    }
    #[test]
    fn ownership_and_unknown_attempts_survive_native_database_reopen() {
        let dir = TempDir::new("github-ledger").unwrap();
        let mut change = initial("t-1");
        {
            let store = SqliteStore::open(dir.path(), 0).unwrap();
            block_on(store.create_task(&Task::new("t-1", "github", "team", "code", 1))).unwrap();
            assert!(block_on(store.save_github_change(None, &change)).unwrap());
            change.push_intent = Some("a".repeat(40));
            assert!(block_on(store.save_github_change(Some(1), &change)).unwrap());
        }
        let store = SqliteStore::open(dir.path(), 1).unwrap();
        assert_eq!(
            block_on(store.github_change("t-1"))
                .unwrap()
                .unwrap()
                .change,
            change
        );
        assert!(!block_on(store.save_github_change(Some(1), &change)).unwrap());
        let mut invalid = change.clone();
        invalid.push_intent = Some("b".repeat(40));
        assert!(block_on(store.save_github_change(Some(2), &invalid)).is_err());
        invalid = change.clone();
        invalid.identity.repository_id += 1;
        assert!(block_on(store.save_github_change(Some(2), &invalid)).is_err());
        change.pushed_head = change.push_intent.take();
        assert!(block_on(store.save_github_change(Some(2), &change)).unwrap());
        change.create_attempted = true;
        assert!(block_on(store.save_github_change(Some(3), &change)).unwrap());
        drop(store);
        let store = SqliteStore::open(dir.path(), 2).unwrap();
        let restored = block_on(store.github_change("t-1")).unwrap().unwrap();
        assert!(restored.change.create_attempted);
        assert_eq!(restored.revision, 4);
        invalid = change.clone();
        invalid.create_attempted = false;
        assert!(block_on(store.save_github_change(Some(4), &invalid)).is_err());
        change.pull_number = Some(123);
        assert!(block_on(store.save_github_change(Some(4), &change)).unwrap());
        invalid = change.clone();
        invalid.pull_number = Some(124);
        assert!(block_on(store.save_github_change(Some(5), &invalid)).is_err());
        assert_eq!(
            block_on(store.github_change("t-1"))
                .unwrap()
                .unwrap()
                .change,
            change
        );
        change.cleanup.close_attempted = true;
        assert!(block_on(store.save_github_change(Some(5), &change)).unwrap());
        change.cleanup.delete_attempted = true;
        assert!(block_on(store.save_github_change(Some(6), &change)).unwrap());
        drop(store);
        let store = SqliteStore::open(dir.path(), 3).unwrap();
        assert_eq!(
            block_on(store.github_change("t-1"))
                .unwrap()
                .unwrap()
                .change,
            change
        );
        for close in [true, false] {
            invalid = change.clone();
            if close {
                invalid.cleanup.close_attempted = false;
            } else {
                invalid.cleanup.delete_attempted = false;
            }
            assert!(block_on(store.save_github_change(Some(7), &invalid)).is_err());
        }
        change.cleanup.complete = true;
        assert!(block_on(store.save_github_change(Some(7), &change)).unwrap());
        invalid = change.clone();
        invalid.push_intent = Some("b".repeat(40));
        assert!(block_on(store.save_github_change(Some(8), &invalid)).is_err());
        block_on(store.create_task(&Task::new("t-2", "github", "team", "code", 1))).unwrap();
        assert!(block_on(store.save_github_change(None, &initial("t-2"))).is_err());
    }
}
