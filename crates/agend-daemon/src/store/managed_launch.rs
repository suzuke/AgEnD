//! Durable pre-spawn reservation. Only the supervisor may replace an intent,
//! after proving the previous holder is gone; reconnect only reads it.
use super::{SqliteStore, StoreError, instances};
use agend_core::{
    runtime_records::{Instance, ManagedLaunchIntent},
    setup::backend::{ImportedBackend, valid_version},
    traits::HolderLaunch,
};
use rusqlite::{Connection, OptionalExtension, params};

fn get(conn: &Connection, id: &str) -> Result<Option<ManagedLaunchIntent>, StoreError> {
    let row: Option<(String, String)> = conn
        .query_row(
            "SELECT binding,intent FROM managed_launches WHERE instance_id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    row.map(|(binding, json)| {
        let intent: ManagedLaunchIntent =
            serde_json::from_str(&json).map_err(|e| StoreError::Invalid(e.to_string()))?;
        if intent.instance_id != id
            || intent.binding != binding
            || !agend_core::protocol::client::is_uuid_v4(&binding)
        {
            return Err(StoreError::Invalid(
                "managed launch identity is inconsistent".into(),
            ));
        }
        Ok(intent)
    })
    .transpose()
}

impl SqliteStore {
    pub async fn managed_launch(
        &self,
        id: &str,
    ) -> Result<Option<ManagedLaunchIntent>, StoreError> {
        let id = id.to_owned();
        self.call(move |conn| get(conn, &id)).await
    }

    /// Compare-and-swap the previous reservation, before any SpawnBound IO.
    /// A lost call result must be reconciled with managed_launch, never blindly retried.
    pub async fn prepare_managed_launch(
        &self,
        instance: &Instance,
        launch: &HolderLaunch,
        artifact: ImportedBackend,
        expected_binding: Option<&str>,
    ) -> Result<ManagedLaunchIntent, StoreError> {
        if launch.instance_id != instance.id
            || launch.backend != instance.backend
            || launch.working_directory != instance.working_directory
            || artifact.backend != instance.backend.as_str()
            || artifact.format != 1
            || !valid_version(&artifact.version)
            || artifact.sha256.len() != 64
            || !artifact.sha256.bytes().all(|b| b.is_ascii_hexdigit())
            || artifact.bytes == 0
            || launch.executable.is_empty()
        {
            return Err(StoreError::Invalid(
                "managed launch does not match the instance and artifact".into(),
            ));
        }
        let intent = ManagedLaunchIntent {
            binding: instances::new_session_id().map_err(StoreError::Io)?,
            instance_id: instance.id.clone(),
            artifact,
            configured_program: instance.program.clone(),
            configured_args: instance.args.clone(),
            session_id: instance.session_id.clone(),
            delivery: instance.delivery.clone(),
            executable: launch.executable.clone(),
            args: launch.args.clone(),
            working_directory: launch.working_directory.clone(),
        };
        let expected = expected_binding.map(str::to_owned);
        let instance = instance.clone();
        self.call(move |conn| {
            let tx = conn.transaction()?;
            if instances::get(&tx, &instance.id)?.as_ref() != Some(&instance) {
                return Err(StoreError::Invalid("instance changed before managed launch reservation".into()));
            }
            let previous = get(&tx, &instance.id)?;
            if previous.as_ref().map(|p| p.binding.as_str()) != expected.as_deref() {
                return Err(StoreError::Invalid("managed launch reservation changed; reconcile before spawning".into()));
            }
            let json = serde_json::to_string(&intent).map_err(|e| StoreError::Invalid(e.to_string()))?;
            tx.execute("INSERT INTO managed_launches(instance_id,binding,intent) VALUES (?1,?2,?3) \
                ON CONFLICT(instance_id) DO UPDATE SET binding=excluded.binding,intent=excluded.intent",
                params![instance.id, intent.binding, json])?;
            tx.commit()?;
            Ok(intent)
        }).await
    }
}
