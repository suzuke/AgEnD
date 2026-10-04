//! Claude push attempt and ACK transactions, serialized on the sole DB thread.
//! Transport may start only after `Started`; `Existing` is never a send permit.
use super::{SqliteStore, StoreError, instances, messages};
use agend_core::model::{Backend, DeliveryState};
use agend_core::protocol::client::is_uuid_v4;
use agend_core::runtime_records::{
    ClaudeAck, ClaudeAckResult, ClaudeAttempt, ClaudeDelivery, ClaudeReservation, ClaudeRoute,
    InstanceStatus, NewClaudeDelivery,
};
use rusqlite::{Connection, OptionalExtension};

pub(super) fn ms(value: u64) -> Result<i64, StoreError> {
    i64::try_from(value).map_err(|_| invalid("time out of range"))
}
fn invalid(what: &str) -> StoreError {
    StoreError::Invalid(format!("Claude delivery: {what}"))
}

pub(crate) fn get(conn: &Connection, id: &str) -> Result<Option<ClaudeDelivery>, StoreError> {
    conn.query_row(
        "SELECT instance_id, delivery_id, session_id, route, started_at_unix_ms, \
         sent_at_unix_ms, confirmed_at_unix_ms, abandoned_at_unix_ms, abandonment_reason \
         FROM claude_deliveries WHERE message_id = ?1",
        [id],
        |r| {
            let delivery_id: Option<String> = r.get(1)?;
            let attempt = if let Some(delivery_id) = delivery_id {
                let route: String = r.get(3)?;
                let Some(route) = ClaudeRoute::parse(&route) else {
                    return Ok(Err(invalid("invalid stored route")));
                };
                Some(ClaudeAttempt {
                    delivery_id,
                    session_id: r.get(2)?,
                    route,
                    started_at_unix_ms: r.get(4)?,
                    sent_at_unix_ms: r.get(5)?,
                    confirmed_at_unix_ms: r.get(6)?,
                })
            } else {
                None
            };
            Ok(Ok(ClaudeDelivery {
                message_id: id.into(),
                instance_id: r.get(0)?,
                attempt,
                abandoned_at_unix_ms: r.get(7)?,
                abandonment_reason: r.get(8)?,
            }))
        },
    )
    .optional()?
    .transpose()
}
fn required(conn: &Connection, id: &str) -> Result<ClaudeDelivery, StoreError> {
    get(conn, id)?.ok_or_else(|| invalid("not a retained Claude push message"))
}
fn attempt_matches(d: &ClaudeDelivery, new: &NewClaudeDelivery) -> bool {
    d.instance_id == new.instance_id
        && d.attempt.as_ref().is_some_and(|a| {
            a.delivery_id == new.delivery_id
                && a.session_id == new.session_id
                && a.route == new.route
        })
}

pub(crate) fn reserve(
    conn: &mut Connection,
    new: &NewClaudeDelivery,
    now: u64,
) -> Result<ClaudeReservation, StoreError> {
    if !is_uuid_v4(&new.delivery_id) || !is_uuid_v4(&new.session_id) {
        return Err(invalid("delivery and session ids must be UUID v4"));
    }
    let tx = conn.transaction()?;
    let stored = required(&tx, &new.message_id)?;
    if stored.attempt.is_some() {
        return if attempt_matches(&stored, new) {
            Ok(ClaudeReservation::Existing(stored))
        } else {
            Err(invalid("message already has a different attempt"))
        };
    }
    let message = messages::get(&tx, &new.message_id)?.ok_or_else(|| invalid("missing message"))?;
    if !stored.may_start(&message) || stored.instance_id != new.instance_id {
        return Err(invalid(
            "message is closed or a previous outcome is unknown",
        ));
    }
    let instance =
        instances::get(&tx, &new.instance_id)?.ok_or_else(|| invalid("missing instance"))?;
    if instance.backend != Backend::Claude
        || instance.delivery != "push"
        || instance.status != InstanceStatus::Running
        || !instance.session_started
        || instance.session_id.as_deref() != Some(&new.session_id)
    {
        return Err(invalid(
            "instance is not running Claude push in this session",
        ));
    }
    tx.execute(
        "UPDATE claude_deliveries SET delivery_id = ?2, session_id = ?3, route = ?4, \
        started_at_unix_ms = ?5 WHERE message_id = ?1",
        rusqlite::params![
            new.message_id,
            new.delivery_id,
            new.session_id,
            new.route.as_str(),
            ms(now)?
        ],
    )?;
    messages::mark_attempted(&tx, &new.message_id, now)?;
    let result = required(&tx, &new.message_id)?;
    tx.commit()?;
    Ok(ClaudeReservation::Started(result))
}

pub(crate) fn written(
    conn: &mut Connection,
    id: &str,
    delivery: &str,
    now: u64,
) -> Result<ClaudeDelivery, StoreError> {
    let tx = conn.transaction()?;
    let stored = required(&tx, id)?;
    let attempt = stored
        .attempt
        .as_ref()
        .ok_or_else(|| invalid("attempt not started"))?;
    if attempt.delivery_id != delivery || stored.abandoned_at_unix_ms.is_some() {
        return Err(invalid("wrong delivery id or abandoned message"));
    }
    let message = messages::get(&tx, id)?.ok_or_else(|| invalid("missing message"))?;
    if message.state == DeliveryState::Failed {
        return Err(invalid("message failed"));
    }
    if message.state == DeliveryState::Confirmed && attempt.confirmed_at_unix_ms.is_none() {
        return Err(invalid("confirmed message lacks ACK attribution"));
    }
    if message.state == DeliveryState::Queued {
        messages::advance_claude(&tx, id, DeliveryState::Sent, None, now)?;
    }
    tx.execute(
        "UPDATE claude_deliveries SET sent_at_unix_ms = COALESCE(sent_at_unix_ms, ?2) \
        WHERE message_id = ?1",
        rusqlite::params![id, ms(now)?],
    )?;
    let result = required(&tx, id)?;
    tx.commit()?;
    Ok(result)
}

pub(crate) fn acknowledge(
    conn: &mut Connection,
    ack: &ClaudeAck,
    now: u64,
) -> Result<ClaudeAckResult, StoreError> {
    let tx = conn.transaction()?;
    let stored = required(&tx, &ack.message_id)?;
    let attempt = stored
        .attempt
        .as_ref()
        .ok_or_else(|| invalid("ACK has no started attempt"))?;
    // The delivery's session is authoritative, even after instance replacement.
    if stored.instance_id != ack.instance_id
        || attempt.delivery_id != ack.delivery_id
        || attempt.session_id != ack.session_id
        || stored.abandoned_at_unix_ms.is_some()
    {
        return Err(invalid("ACK identity mismatch or abandoned message"));
    }
    let message = messages::get(&tx, &ack.message_id)?.ok_or_else(|| invalid("missing message"))?;
    if message.state == DeliveryState::Failed {
        return Err(invalid("message failed"));
    }
    if attempt.confirmed_at_unix_ms.is_some() {
        return if message.state == DeliveryState::Confirmed {
            Ok(ClaudeAckResult::AlreadyConfirmed)
        } else {
            Err(invalid("inconsistent ACK state"))
        };
    }
    if message.state == DeliveryState::Confirmed {
        return Err(invalid("confirmed message lacks ACK attribution"));
    }
    if message.state == DeliveryState::Queued {
        messages::advance_claude(&tx, &ack.message_id, DeliveryState::Sent, None, now)?;
    }
    if message.state != DeliveryState::Confirmed {
        messages::advance_claude(&tx, &ack.message_id, DeliveryState::Confirmed, None, now)?;
    }
    tx.execute(
        "UPDATE claude_deliveries SET sent_at_unix_ms = COALESCE(sent_at_unix_ms, ?2), \
        confirmed_at_unix_ms = ?2 WHERE message_id = ?1",
        rusqlite::params![ack.message_id, ms(now)?],
    )?;
    // A delivery id and a native hook event id inhabit different domains.
    // ACK idempotency is the transaction's confirmed_at guard, not reusing
    // a delivery UUID as an event UUID (which can collide with a saved hook).
    super::driver_events::append(&tx, &agend_core::runtime_records::NewDriverEvent {
        id: instances::new_session_id().map_err(|e| invalid(&format!("cannot allocate ACK event id: {e}")))?,
        instance_id: ack.instance_id.clone(), session_id: ack.session_id.clone(),
        kind: "AgendAck".into(), payload: serde_json::json!({"message_id":ack.message_id,"delivery_id":ack.delivery_id,"session_id":ack.session_id}).to_string(),
        occurred_at_unix_ms: now, replayed: false,
    }, now)?;
    tx.commit()?;
    Ok(ClaudeAckResult::Confirmed)
}

pub(crate) fn abandon(
    conn: &mut Connection,
    id: &str,
    reason: &str,
    now: u64,
) -> Result<ClaudeDelivery, StoreError> {
    if reason.trim().is_empty() || reason.len() > 4096 {
        return Err(invalid("invalid abandonment reason"));
    }
    let tx = conn.transaction()?;
    let stored = required(&tx, id)?;
    let message = messages::get(&tx, id)?.ok_or_else(|| invalid("missing message"))?;
    if message.state == DeliveryState::Confirmed {
        return Err(invalid("message already confirmed"));
    }
    if stored.abandoned_at_unix_ms.is_some() {
        return Ok(stored);
    }
    if message.state != DeliveryState::Failed {
        messages::advance_claude(&tx, id, DeliveryState::Failed, None, now)?;
    }
    tx.execute(
        "UPDATE claude_deliveries SET abandoned_at_unix_ms = ?2, abandonment_reason = ?3 \
        WHERE message_id = ?1",
        rusqlite::params![id, ms(now)?, reason],
    )?;
    let result = required(&tx, id)?;
    tx.commit()?;
    Ok(result)
}

impl SqliteStore {
    pub(crate) async fn unknown_claude_after(
        &self,
        after: i64,
        before: u64,
    ) -> Result<Vec<agend_core::runtime_records::Message>, StoreError> {
        self.call(move |conn| messages::unknown_claude_after(conn, after, before))
            .await
    }
    pub(crate) async fn claude_outcome_unknown(&self, id: &str) -> Result<bool, StoreError> {
        let id = id.to_owned();
        self.call(move |conn| {
            Ok(match (get(conn, &id)?, messages::get(conn, &id)?) {
                (Some(d), Some(m)) => d.outcome_unknown(&m),
                _ => false,
            })
        })
        .await
    }
    /// Recheck on the serialized DB thread before explicit abandonment, so
    /// a concurrent valid Written/ACK receipt cannot be changed to Failed.
    pub(crate) async fn abandon_unknown_claude(
        &self,
        id: &str,
        reason: &str,
        now: u64,
    ) -> Result<(), StoreError> {
        let (id, reason) = (id.to_owned(), reason.to_owned());
        self.call(move |conn| {
            let d = required(conn, &id)?;
            let m = messages::get(conn, &id)?.ok_or_else(|| invalid("missing message"))?;
            if !d.outcome_unknown(&m) {
                return Err(invalid(
                    "delivery no longer has an unknown outcome; nothing changed",
                ));
            }
            abandon(conn, &id, &reason, now)?;
            Ok(())
        })
        .await
    }
    pub(crate) async fn pending_claude_interrupts(
        &self,
        instance: &str,
    ) -> Result<Vec<agend_core::runtime_records::Message>, StoreError> {
        let instance = instance.to_owned();
        self.call(move |conn| messages::pending_claude_filtered(conn, &instance, 2))
            .await
    }
    /// Undo only a reservation whose daemon key was definitively refused
    /// before any PTY write. Timeouts, lost responses and write failures MUST
    /// retain the intent. This is not permission to retry a content writer.
    pub(crate) async fn release_refused_claude_key(
        &self,
        message: &str,
        delivery: &str,
    ) -> Result<(), StoreError> {
        let message = message.to_owned();
        let delivery = delivery.to_owned();
        self.call(move |conn| {
            let tx = conn.transaction()?;
            let d = required(&tx, &message)?;
            let m = messages::get(&tx, &message)?.ok_or_else(|| invalid("missing message"))?;
            if d.abandoned_at_unix_ms.is_some() || m.state != DeliveryState::Queued || !d.attempt.as_ref().is_some_and(|a| a.delivery_id == delivery && a.sent_at_unix_ms.is_none() && a.confirmed_at_unix_ms.is_none()) {
                return Err(invalid("key reservation can no longer be released"));
            }
            tx.execute("UPDATE claude_deliveries SET delivery_id=NULL, session_id=NULL, route=NULL, started_at_unix_ms=NULL WHERE message_id=?1", [&message])?;
            tx.execute("UPDATE messages SET attempted_at_unix_ms=NULL WHERE id=?1", [&message])?;
            tx.commit()?;
            Ok(())
        }).await
    }
    pub async fn claude_delivery(&self, id: &str) -> Result<Option<ClaudeDelivery>, StoreError> {
        let id = id.to_owned();
        self.call(move |conn| get(conn, &id)).await
    }
    /// Commit the attempt before content IO. An existing attempt never permits replay.
    pub async fn reserve_claude_delivery(
        &self,
        new: NewClaudeDelivery,
        now: u64,
    ) -> Result<ClaudeReservation, StoreError> {
        self.call(move |conn| reserve(conn, &new, now)).await
    }
    pub async fn claude_delivery_written(
        &self,
        id: &str,
        delivery: &str,
        now: u64,
    ) -> Result<ClaudeDelivery, StoreError> {
        let id = id.to_owned();
        let delivery = delivery.to_owned();
        self.call(move |conn| written(conn, &id, &delivery, now))
            .await
    }
    /// Callers must authenticate the supplied instance before invoking this API.
    pub async fn acknowledge_claude(
        &self,
        ack: ClaudeAck,
        now: u64,
    ) -> Result<ClaudeAckResult, StoreError> {
        self.call(move |conn| acknowledge(conn, &ack, now)).await
    }
    /// Explicit operator abandonment; this is not an automatic failure policy.
    pub async fn abandon_claude_delivery(
        &self,
        id: &str,
        reason: &str,
        now: u64,
    ) -> Result<ClaudeDelivery, StoreError> {
        let id = id.to_owned();
        let reason = reason.to_owned();
        self.call(move |conn| abandon(conn, &id, &reason, now))
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agend_core::{
        policy::busy::BusyLevel,
        runtime_records::{Instance, NewMessage},
    };
    use agend_testkit::{block_on, tempdir::TempDir};
    const SESSION: &str = "11111111-1111-4111-8111-111111111111";
    const DELIVERY: &str = "22222222-2222-4222-8222-222222222222";
    fn setup(store: &SqliteStore) {
        block_on(store.add_instance(&Instance {
            id: "claude".into(),
            backend: Backend::Claude,
            program: "/bin/false".into(),
            args: vec![],
            working_directory: "/tmp".into(),
            session_id: Some(SESSION.into()),
            status: InstanceStatus::Running,
            session_started: true,
            agent_pid: None,
            legacy_no_thread: false,
            delivery: "push".into(),
        }))
        .unwrap();
        block_on(store.claim_message(
            &NewMessage {
                id: "m".into(),
                from_instance: "operator".into(),
                to_instance: "claude".into(),
                task_id: None,
                body: "body".into(),
                level: BusyLevel::Queue,
            },
            0,
        ))
        .unwrap();
    }
    fn new() -> NewClaudeDelivery {
        NewClaudeDelivery {
            message_id: "m".into(),
            delivery_id: DELIVERY.into(),
            session_id: SESSION.into(),
            instance_id: "claude".into(),
            route: ClaudeRoute::Channel,
        }
    }
    #[test]
    fn metadata_failure_rolls_back_message_transition_and_ack() {
        let dir = TempDir::new("claude-rollback").unwrap();
        let store = SqliteStore::open(dir.path(), 0).unwrap();
        setup(&store);
        store.call_blocking(|conn| {
            conn.execute_batch("CREATE TEMP TRIGGER fail_attempt BEFORE UPDATE OF attempted_at_unix_ms ON messages BEGIN SELECT RAISE(ABORT, 'injected attempt failure'); END;")?;
            assert!(reserve(conn,&new(),1).is_err());
            assert!(required(conn,"m")?.attempt.is_none());
            assert!(messages::get(conn,"m")?.unwrap().attempted_at_unix_ms.is_none());
            conn.execute_batch("DROP TRIGGER fail_attempt;")?;
            reserve(conn,&new(),1)?;
            conn.execute_batch("CREATE TEMP TRIGGER fail_ack BEFORE UPDATE OF confirmed_at_unix_ms ON claude_deliveries BEGIN SELECT RAISE(ABORT, 'injected ACK failure'); END;")?;
            let ack=ClaudeAck { message_id:"m".into(),delivery_id:DELIVERY.into(),instance_id:"claude".into(),session_id:SESSION.into() };
            assert!(acknowledge(conn,&ack,2).is_err());
            assert_eq!(messages::get(conn,"m")?.unwrap().state,DeliveryState::Queued);
            assert!(required(conn,"m")?.attempt.unwrap().sent_at_unix_ms.is_none());
            conn.execute_batch("DROP TRIGGER fail_ack;")?;
            conn.execute_batch("CREATE TEMP TRIGGER fail_write BEFORE UPDATE OF sent_at_unix_ms ON claude_deliveries BEGIN SELECT RAISE(ABORT, 'injected write failure'); END;")?;
            assert!(written(conn,"m",DELIVERY,2).is_err());
            assert_eq!(messages::get(conn,"m")?.unwrap().state,DeliveryState::Queued);
            conn.execute_batch("DROP TRIGGER fail_write; CREATE TEMP TRIGGER fail_abandon BEFORE UPDATE OF abandoned_at_unix_ms ON claude_deliveries BEGIN SELECT RAISE(ABORT, 'injected abandonment failure'); END;")?;
            assert!(abandon(conn,"m","human",2).is_err());
            assert_eq!(messages::get(conn,"m")?.unwrap().state,DeliveryState::Queued);
            assert!(required(conn,"m")?.abandoned_at_unix_ms.is_none());
            conn.execute_batch("DROP TRIGGER fail_abandon;")?;
            assert_eq!(acknowledge(conn,&ack,3)?,ClaudeAckResult::Confirmed);
            Ok(())
        }).unwrap();
    }
    #[test]
    fn generic_receipts_cannot_confirm_claude_push_and_legacy_attempt_is_unknown() {
        let dir = TempDir::new("claude-legacy").unwrap();
        let store = SqliteStore::open(dir.path(), 0).unwrap();
        setup(&store);
        store
            .call_blocking(|conn| {
                assert!(messages::advance(conn, "m", DeliveryState::Sent, None, 1).is_err());
                messages::mark_attempted(conn, "m", 1)?;
                let message = messages::get(conn, "m")?.unwrap();
                let record = required(conn, "m")?;
                assert!(record.outcome_unknown(&message));
                assert!(!record.may_start(&message));
                assert!(reserve(conn, &new(), 2).is_err());
                abandon(conn, "m", "operator chose to close legacy outcome", 3)?;
                Ok(())
            })
            .unwrap();
    }
}
