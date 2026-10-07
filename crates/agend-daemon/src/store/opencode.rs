//! OpenCode attempt ownership in the existing messages idempotency layer.
//! The backend reference binds the attempt to both session and message id.
use super::{Message, StoreError, messages};
use agend_core::model::DeliveryState;
use rusqlite::{Connection, params};

impl super::SqliteStore {
    pub async fn opencode_events(
        &self,
        instance: &str,
        after: i64,
    ) -> Result<Vec<agend_core::traits::DriverEvent>, StoreError> {
        let instance = instance.to_owned();
        self.call(move |conn| {
            let mut q = conn.prepare("SELECT seq,kind,payload FROM driver_events WHERE instance_id=?1 AND seq>?2 AND kind IN ('OpenCodeConfirmed','OpenCodeState') ORDER BY seq LIMIT 1024")?;
            let rows = q.query_map(params![instance,after], |r| Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?)))?;
            rows.map(|row| {
                let (seq, kind, payload) = row?;
                let v: serde_json::Value = serde_json::from_str(&payload).map_err(|e| StoreError::Invalid(e.to_string()))?;
                let invalid = || StoreError::Invalid("invalid stored OpenCode event".into());
                let kind = if kind == "OpenCodeConfirmed" {
                    agend_core::traits::DriverEventKind::MessageConfirmed { message_id:v["message_id"].as_str().ok_or_else(invalid)?.into() }
                } else { agend_core::traits::DriverEventKind::BusyChanged { busy:v["busy"].as_bool().ok_or_else(invalid)? } };
                Ok(agend_core::traits::DriverEvent {cursor:format!("opencode:{seq}"),kind})
            }).collect()
        }).await
    }
    pub async fn begin_opencode_attempt(
        &self,
        id: &str,
        instance: &str,
        session: &str,
        backend_id: &str,
        now: u64,
    ) -> Result<bool, StoreError> {
        let (id, instance, session, backend_id) = (
            id.to_owned(),
            instance.to_owned(),
            session.to_owned(),
            backend_id.to_owned(),
        );
        self.call(move |conn| begin(conn, &id, &instance, &session, &backend_id, now))
            .await
    }

    pub async fn confirm_opencode_attempt(
        &self,
        id: &str,
        session: &str,
        backend_id: &str,
        now: u64,
    ) -> Result<Option<Message>, StoreError> {
        let (id, session, backend_id) = (id.to_owned(), session.to_owned(), backend_id.to_owned());
        self.call(move |conn| confirm(conn, &id, &session, &backend_id, now))
            .await
    }
}

pub fn reference(session: &str, message: &str) -> String {
    format!("{session}|{message}")
}

fn event(
    conn: &Connection,
    instance: &str,
    session: &str,
    kind: &str,
    payload: serde_json::Value,
    now: u64,
) -> Result<(), StoreError> {
    let id = super::instances::new_session_id().map_err(|e| StoreError::Invalid(e.to_string()))?;
    let now =
        i64::try_from(now).map_err(|_| StoreError::Invalid("event time out of range".into()))?;
    conn.execute("INSERT INTO driver_events(id,instance_id,session_id,kind,payload,occurred_at_unix_ms,ingested_at_unix_ms,replayed) VALUES(?1,?2,?3,?4,?5,?6,?6,0)",params![id,instance,session,kind,payload.to_string(),now])?;
    Ok(())
}

/// Only the first caller can authorize a write. A daemon crash after this
/// commit leaves an unknown attempt that REST can confirm, never auto-replay.
pub(crate) fn begin(
    conn: &Connection,
    id: &str,
    instance: &str,
    session: &str,
    backend_id: &str,
    now: u64,
) -> Result<bool, StoreError> {
    let now =
        i64::try_from(now).map_err(|_| StoreError::Invalid("attempt time out of range".into()))?;
    Ok(conn.execute(
        "UPDATE messages SET attempted_at_unix_ms=?5, turn_id=?4, updated_at_unix_ms=?5 \
         WHERE id=?1 AND to_instance=?2 AND state='queued' AND attempted_at_unix_ms IS NULL \
         AND EXISTS (SELECT 1 FROM instances WHERE id=?2 AND backend='opencode' \
         AND delivery='push' AND session_id=?3 AND status='running')",
        params![id, instance, session, reference(session, backend_id), now],
    )? == 1)
}

/// Call only after exact REST identity/body verification. The transaction
/// refuses a receipt for another session or a row which was never attempted.
pub(crate) fn confirm(
    conn: &Connection,
    id: &str,
    session: &str,
    backend_id: &str,
    now: u64,
) -> Result<Option<Message>, StoreError> {
    let tx = conn.unchecked_transaction()?;
    let Some(mut row) = messages::get(&tx, id)? else {
        return Ok(None);
    };
    let reference = reference(session, backend_id);
    if row.attempted_at_unix_ms.is_none()
        || row.turn_id.as_deref() != Some(&reference)
        || row.state == DeliveryState::Failed
    {
        return Ok(None);
    }
    if row.state == DeliveryState::Queued {
        row = messages::advance(&tx, id, DeliveryState::Sent, Some(&reference), now)?
            .ok_or_else(|| StoreError::Invalid("OpenCode attempted message disappeared".into()))?;
    }
    if row.state == DeliveryState::Sent {
        row = messages::advance(&tx, id, DeliveryState::Confirmed, Some(&reference), now)?
            .ok_or_else(|| StoreError::Invalid("OpenCode sent message disappeared".into()))?;
        event(
            &tx,
            &row.to_instance,
            session,
            "OpenCodeConfirmed",
            serde_json::json!({"message_id":id}),
            now,
        )?;
    }
    tx.commit()?;
    Ok(Some(row))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{Instance, InstanceStatus, NewMessage, SqliteStore};
    use agend_core::{model::Backend, policy::busy::BusyLevel};
    use agend_testkit::{block_on, tempdir::TempDir};

    #[test]
    fn native_database_attempt_survives_reopen_and_cannot_replay_or_change_session() {
        let dir = TempDir::new("opencode-attempt").unwrap();
        {
            let store = SqliteStore::open(dir.path(), 0).unwrap();
            block_on(store.add_instance(&Instance {
                id: "open-1".into(),
                backend: Backend::Opencode,
                program: "unused".into(),
                args: vec![],
                working_directory: dir.path().display().to_string(),
                session_id: Some("ses_native".into()),
                status: InstanceStatus::Running,
                session_started: true,
                agent_pid: None,
                legacy_no_thread: false,
                delivery: "push".into(),
            }))
            .unwrap();
            block_on(store.claim_message(
                &NewMessage {
                    id: "message-1".into(),
                    from_instance: "sender".into(),
                    to_instance: "open-1".into(),
                    task_id: None,
                    body: "preserve me".into(),
                    level: BusyLevel::Queue,
                },
                1,
            ))
            .unwrap();
            store
                .call_blocking(|conn| {
                    assert!(confirm(conn, "message-1", "ses_native", "msg_native", 2)?.is_none());
                    assert!(!begin(
                        conn,
                        "message-1",
                        "open-1",
                        "ses_foreign",
                        "msg_native",
                        2
                    )?);
                    assert!(begin(
                        conn,
                        "message-1",
                        "open-1",
                        "ses_native",
                        "msg_native",
                        3
                    )?);
                    assert!(!begin(
                        conn,
                        "message-1",
                        "open-1",
                        "ses_native",
                        "msg_native",
                        4
                    )?);
                    Ok(())
                })
                .unwrap();
        }
        let store = SqliteStore::open(dir.path(), 5).unwrap();
        store
            .call_blocking(|conn| {
                assert!(!begin(
                    conn,
                    "message-1",
                    "open-1",
                    "ses_native",
                    "msg_native",
                    6
                )?);
                assert!(confirm(conn, "message-1", "ses_foreign", "msg_native", 6)?.is_none());
                assert!(confirm(conn, "message-1", "ses_native", "msg_other", 6)?.is_none());
                let row = confirm(conn, "message-1", "ses_native", "msg_native", 7)?.unwrap();
                assert_eq!(row.state, DeliveryState::Confirmed);
                assert_eq!(row.attempted_at_unix_ms, Some(3));
                assert_eq!(row.body, "preserve me");
                Ok(())
            })
            .unwrap();
    }
}
