//! Durable G4 receipts, separate from action outcomes and task state.
use super::{SqliteStore, StoreError};
use agend_core::attention_read::AttentionReadStore;
impl AttentionReadStore for SqliteStore {
    type Error = StoreError;
    async fn attention_read_keys(&self) -> Result<Vec<String>, StoreError> {
        self.call(|c| {
            let mut q = c.prepare("SELECT read_key FROM attention_reads ORDER BY read_key")?;
            Ok(q.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?)
        })
        .await
    }
    async fn mark_attention_read(&self, key: &str, now: u64) -> Result<(), StoreError> {
        let key = key.to_owned();
        self.call(move |c| {
            if key.is_empty() || key.len() > 4096 {
                return Err(StoreError::Invalid("invalid attention read key".into()));
            }
            c.execute(
                "INSERT INTO attention_reads(read_key,read_at_unix_ms) VALUES(?1,?2) ON CONFLICT(read_key) DO NOTHING",
                rusqlite::params![key, now],
            )?;
            Ok(())
        }).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn read_receipts_survive_database_reopen_and_duplicate_marks() {
        let dir = agend_testkit::tempdir::TempDir::new("attention-reads").unwrap();
        let store = SqliteStore::open(dir.path(), 0).unwrap();
        store.mark_attention_read("ask-1#1", 1).await.unwrap();
        store.mark_attention_read("ask-1#1", 2).await.unwrap();
        drop(store);
        let reopened = SqliteStore::open(dir.path(), 0).unwrap();
        assert_eq!(
            reopened.attention_read_keys().await.unwrap(),
            vec!["ask-1#1"]
        );
        assert!(
            !reopened
                .attention_read_keys()
                .await
                .unwrap()
                .contains(&"ask-1#2".to_owned())
        );
    }
}
