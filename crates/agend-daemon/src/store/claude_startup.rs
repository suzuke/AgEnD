//! A new holder creates a startup; daemon reconnect never does. Each key
//! is reserved before IO. Only an explicit pre-write refusal releases it.
use super::{SqliteStore, StoreError};
use agend_core::runtime_records::{ClaudeStartup, ClaudeStartupKey};
use rusqlite::OptionalExtension;

impl SqliteStore {
    pub async fn begin_claude_startup(&self, id: &str, session: &str) -> Result<bool, StoreError> {
        let id = id.to_owned();
        let session = session.to_owned();
        let launch = super::instances::new_session_id().map_err(StoreError::Io)?;
        self.call(move |conn| {
            conn.execute("INSERT INTO claude_startup (instance_id,session_id,launch_id,generation,halted,keys) \
                VALUES (?1,?2,?3,NULL,0,'{}') ON CONFLICT(instance_id) DO UPDATE SET \
                session_id=excluded.session_id, launch_id=excluded.launch_id, generation=NULL, halted=claude_startup.manual, keys='{}'",
                rusqlite::params![id,session,launch])?;
            Ok(conn.query_row("SELECT manual=0 FROM claude_startup WHERE instance_id=?1", [&id], |r| r.get(0))?)
        }).await
    }

    /// Diagnostic captures own all startup input. Register before starting
    /// their private daemon; ordinary holder starts never clear this policy.
    pub async fn manual_claude_startup(&self, id: &str, session: &str) -> Result<(), StoreError> {
        let id = id.to_owned();
        let session = session.to_owned();
        let launch = super::instances::new_session_id().map_err(StoreError::Io)?;
        self.call(move |conn| {
            let changed = conn.execute("INSERT INTO claude_startup (instance_id,session_id,launch_id,generation,halted,manual,keys) \
                SELECT ?1,?2,?3,NULL,1,1,'{}' WHERE EXISTS (SELECT 1 FROM instances WHERE id=?1 AND session_id=?2 AND backend='claude') \
                ON CONFLICT(instance_id) DO UPDATE SET manual=1,halted=1", rusqlite::params![id,session,launch])?;
            if changed != 1 { return Err(StoreError::Invalid("diagnostic Claude session does not match".into())); }
            Ok(())
        }).await
    }

    pub async fn claude_startup(&self, id: &str) -> Result<Option<ClaudeStartup>, StoreError> {
        let id = id.to_owned();
        self.call(move |conn| {
            Ok(conn.query_row("SELECT session_id,launch_id,generation,halted FROM claude_startup WHERE instance_id=?1", [&id], |r| {
                Ok(ClaudeStartup {instance:id.clone(),session:r.get(0)?,launch:r.get(1)?,generation:r.get(2)?,halted:r.get(3)?})
            }).optional()?)
        }).await
    }

    pub async fn halt_claude_startup(&self, id: &str, session: &str) -> Result<(), StoreError> {
        let id = id.to_owned();
        let session = session.to_owned();
        self.call(move |conn| {
            conn.execute(
                "UPDATE claude_startup SET halted=1 WHERE instance_id=?1 AND session_id=?2",
                rusqlite::params![id, session],
            )?;
            Ok(())
        })
        .await
    }

    pub async fn reserve_claude_startup_key(
        &self,
        key: ClaudeStartupKey,
    ) -> Result<bool, StoreError> {
        if !matches!(
            key.prompt.as_str(),
            "trust_no" | "trust_yes" | "development"
        ) {
            return Err(StoreError::Invalid("unsupported startup key".into()));
        }
        self.call(move |conn| {
            let tx=conn.transaction()?;
            if super::backend_switch::startup_paused(&tx, &key.startup.instance)? {
                return Ok(false);
            }
            let path=format!("$.{}",key.prompt);
            let changed=tx.execute("UPDATE claude_startup SET generation=?1,keys=json_set(keys,?2,json(?3)) \
                WHERE instance_id=?4 AND session_id=?5 AND launch_id=?6 AND halted=0 \
                AND (generation IS NULL OR generation=?1) AND json_extract(keys,?2) IS NULL \
                AND NOT EXISTS (SELECT 1 FROM json_each(keys) WHERE json_extract(value,'$.state')='intent') \
                AND EXISTS (SELECT 1 FROM instances WHERE id=?4 AND session_id=?5 \
                    AND backend='claude' AND delivery='push' AND status='running' AND session_started=1)",
                rusqlite::params![key.generation,path,serde_json::json!({"attempt":key.attempt,"state":"intent"}).to_string(),
                    key.startup.instance,key.startup.session,key.startup.launch])?;
            tx.commit()?;
            Ok(changed==1)
        }).await
    }

    pub async fn finish_claude_startup_key(
        &self,
        key: ClaudeStartupKey,
        written: bool,
    ) -> Result<(), StoreError> {
        let path = format!("$.{}", key.prompt);
        self.call(move |conn| {
            let sql=if written {"UPDATE claude_startup SET keys=json_set(keys,?1,json(?2))"}
                else {"UPDATE claude_startup SET keys=json_remove(keys,?1)"};
            conn.execute(&format!("{sql} WHERE instance_id=?3 AND session_id=?4 AND launch_id=?5 AND generation=?6 \
                AND json_extract(keys,?1 || '.attempt')=?7 AND json_extract(keys,?1 || '.state')='intent'"),
                rusqlite::params![path,serde_json::json!({"attempt":key.attempt,"state":"written"}).to_string(),
                    key.startup.instance,key.startup.session,key.startup.launch,key.generation,key.attempt])?;
            Ok(())
        }).await
    }
}
