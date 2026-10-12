//! Bounded driver event history. This log is not delivery recovery state.
use super::{SqliteStore, StoreError, claude::ms};
use agend_core::protocol::client::is_uuid_v4;
use agend_core::runtime_records::{DriverEvent, DriverEventAppend, NewDriverEvent};
use rusqlite::{Connection, OptionalExtension, Row};

fn row(r: &Row<'_>) -> rusqlite::Result<DriverEvent> {
    Ok(DriverEvent {
        seq: r.get(0)?,
        event: NewDriverEvent {
            id: r.get(1)?,
            instance_id: r.get(2)?,
            session_id: r.get(3)?,
            kind: r.get(4)?,
            payload: r.get(5)?,
            occurred_at_unix_ms: r.get(6)?,
            replayed: r.get(8)?,
        },
        ingested_at_unix_ms: r.get(7)?,
    })
}
const COLUMNS: &str = "seq, id, instance_id, session_id, kind, payload, occurred_at_unix_ms, ingested_at_unix_ms, replayed";
pub(super) fn append(
    conn: &Connection,
    event: &NewDriverEvent,
    now: u64,
) -> Result<DriverEventAppend, StoreError> {
    let invalid = |reason: &str| StoreError::Invalid(format!("driver event: {reason}"));
    if !is_uuid_v4(&event.id) || !is_uuid_v4(&event.session_id) {
        return Err(invalid("event and session ids must be UUID v4"));
    }
    super::instances::validate_id(&event.instance_id).map_err(StoreError::Invalid)?;
    if event.kind.is_empty()
        || event.kind.len() > 64
        || !event
            .kind
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
    {
        return Err(invalid("invalid event kind"));
    }
    if event.payload.len() > 1024 * 1024 {
        return Err(invalid("payload exceeds 1 MiB"));
    }
    let object: bool = conn.query_row(
        "SELECT CASE WHEN json_valid(?1) THEN json_type(?1) = 'object' ELSE 0 END",
        [&event.payload],
        |r| r.get(0),
    )?;
    if !object {
        return Err(invalid("payload must be a JSON object"));
    }
    ms(event.occurred_at_unix_ms)?;
    ms(now)?;
    let old = conn
        .query_row(
            &format!("SELECT {COLUMNS} FROM driver_events WHERE id = ?1"),
            [&event.id],
            row,
        )
        .optional()?;
    if let Some(old) = old {
        let mut comparable = event.clone();
        // Replay is transport metadata. A lost commit reply may change it.
        comparable.replayed = old.event.replayed;
        if comparable != old.event {
            return Err(invalid("event id already has different content"));
        }
        return Ok(DriverEventAppend {
            event: old,
            inserted: false,
        });
    }
    conn.execute("INSERT INTO driver_events(id, instance_id, session_id, kind, payload, \
        occurred_at_unix_ms, ingested_at_unix_ms, replayed) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![event.id, event.instance_id, event.session_id, event.kind, event.payload,
            ms(event.occurred_at_unix_ms)?, ms(now)?, event.replayed])?;
    let event = conn.query_row(
        &format!("SELECT {COLUMNS} FROM driver_events WHERE id = ?1"),
        [&event.id],
        row,
    )?;
    Ok(DriverEventAppend {
        event,
        inserted: true,
    })
}
impl SqliteStore {
    /// One DB-thread snapshot; absent/expired/ambiguous history is not completion.
    pub async fn claude_message_outcome(
        &self,
        message_id: &str,
    ) -> Result<Option<String>, StoreError> {
        let id = message_id.to_owned();
        self.call(move |conn| {
            let Some(message) = super::messages::get(conn, &id)? else { return Ok(None); };
            if message.state != agend_core::model::DeliveryState::Confirmed { return Ok(None); }
            let Some(delivery) = super::claude::get(conn, &id)? else { return Ok(None); };
            let Some(attempt) = delivery.attempt.as_ref() else { return Ok(None); };
            let current = super::instances::get(conn, &message.to_instance)?;
            if !current.is_some_and(|i| i.backend == agend_core::model::Backend::Claude
                && i.status == super::InstanceStatus::Running && i.session_id.as_deref() == Some(&attempt.session_id)
                && i.id == delivery.instance_id) { return Ok(None); }
            let mut query = conn.prepare(&format!("SELECT {COLUMNS} FROM driver_events WHERE instance_id=?1 AND session_id=?2 AND occurred_at_unix_ms>=?3 AND kind IN ('AgendAck','PostToolUse','Stop','SessionEnd','SessionStart') ORDER BY seq LIMIT 65"))?;
            let events = query.query_map(rusqlite::params![delivery.instance_id,attempt.session_id,ms(attempt.started_at_unix_ms)?], row)?.collect::<Result<Vec<_>,_>>()?;
            if events.len() > 64 { return Ok(None); }
            Ok(crate::driver::claude::outcome::completed(&delivery, &events))
        }).await
    }

    /// Receipt waiting may follow the latest daemon-derived state, but this
    /// observation never grants permission to write content or replay an intent.
    pub(crate) async fn claude_reported_idle(
        &self,
        instance: &str,
        session: &str,
    ) -> Result<bool, StoreError> {
        let (instance, session) = (instance.to_owned(), session.to_owned());
        self.call(move |conn| {
            Ok(conn
                .query_row(
                    "SELECT json_extract(payload, '$.busy') = 0 FROM driver_events \
                 WHERE instance_id=?1 AND session_id=?2 AND kind='AgendState' \
                 AND replayed=0 ORDER BY seq DESC LIMIT 1",
                    rusqlite::params![instance, session],
                    |r| r.get::<_, bool>(0),
                )
                .optional()?
                .unwrap_or(false))
        })
        .await
    }
    /// Filter before LIMIT so another instance or raw hook traffic cannot
    /// starve the adapter's cursor. Only daemon-derived state and durable ACKs
    /// are Driver events; historical hooks are observations, not current state.
    pub(crate) async fn claude_adapter_events(
        &self,
        instance: &str,
        seq: i64,
    ) -> Result<Vec<DriverEvent>, StoreError> {
        let instance = instance.to_owned();
        self.call(move |conn| {
            let mut query = conn.prepare(&format!("SELECT {COLUMNS} FROM driver_events WHERE instance_id = ?1 AND seq > ?2 AND (kind IN ('AgendState','AgendAck') OR (kind = 'Stop' AND json_extract(payload, '$.stop_hook_active') = 0)) ORDER BY seq LIMIT 1024"))?;
            query.query_map(rusqlite::params![instance,seq], row)?.map(|r| r.map_err(StoreError::from)).collect()
        }).await
    }
    /// A duplicate retained event returns its original seq without another insertion.
    /// Event deduplication expires with the 14-day log; it never confirms a message.
    pub async fn append_driver_event(
        &self,
        event: NewDriverEvent,
        now: u64,
    ) -> Result<DriverEventAppend, StoreError> {
        self.call(move |conn| append(conn, &event, now)).await
    }
    pub async fn driver_events_after(
        &self,
        seq: i64,
        limit: u32,
    ) -> Result<Vec<DriverEvent>, StoreError> {
        if seq < 0 || !(1..=1024).contains(&limit) {
            return Err(StoreError::Invalid(
                "invalid driver event cursor or limit".into(),
            ));
        }
        self.call(move |conn| {
            let mut query = conn.prepare(&format!(
                "SELECT {COLUMNS} FROM driver_events WHERE seq > ?1 ORDER BY seq LIMIT ?2"
            ))?;
            query
                .query_map(rusqlite::params![seq, limit], row)?
                .map(|r| r.map_err(StoreError::from))
                .collect()
        })
        .await
    }
}
