//! The `messages` table (gate 7 P5, migration 0004): every message the
//! daemon delivers, its delivery state and order. It is the only
//! idempotency layer (V1-LESSONS #1): [`claim`] looks the id up, compares
//! and inserts in one DB-thread closure, so two deliveries of one id at the
//! same moment insert it once.
//!
//! - Order is `seq` (an explicit `INTEGER PRIMARY KEY`, unchanged by
//!   `VACUUM INTO` snapshots and restores).
//! - States move only as `agend_core::model::DeliveryState::can_transition_to`
//!   allows ([`advance`]).
//! - Retention: 30 days by creation (D31); D40 retains unresolved Claude push
//!   messages and starts terminal retention at the final state update.
//!
//! Must NOT: change a row's id, sender, target, task, body or level after
//! it was inserted.

use agend_core::model::DeliveryState;
use agend_core::policy::busy::BusyLevel;
use rusqlite::{Connection, OptionalExtension, Row};

use super::StoreError;

pub use agend_core::runtime_records::{Claim, Message, NewMessage};

pub fn level_text(level: BusyLevel) -> &'static str {
    match level {
        BusyLevel::Queue => "queue",
        BusyLevel::Steer => "steer",
        BusyLevel::Interrupt => "interrupt",
    }
}

pub fn parse_level(text: &str) -> Option<BusyLevel> {
    [BusyLevel::Queue, BusyLevel::Steer, BusyLevel::Interrupt]
        .into_iter()
        .find(|l| level_text(*l) == text)
}

pub fn state_text(state: DeliveryState) -> &'static str {
    match state {
        DeliveryState::Queued => "queued",
        DeliveryState::Sent => "sent",
        DeliveryState::Confirmed => "confirmed",
        DeliveryState::Failed => "failed",
    }
}

pub fn parse_state(text: &str) -> Option<DeliveryState> {
    use DeliveryState::*;
    [Queued, Sent, Confirmed, Failed]
        .into_iter()
        .find(|s| state_text(*s) == text)
}

const COLUMNS: &str = "seq, id, from_instance, to_instance, task_id, body, level, state, \
                       turn_id, created_at_unix_ms, updated_at_unix_ms, attempted_at_unix_ms";

fn from_row(row: &Row<'_>) -> rusqlite::Result<Result<Message, StoreError>> {
    let id: String = row.get(1)?;
    let level: String = row.get(6)?;
    let state: String = row.get(7)?;
    let created: i64 = row.get(9)?;
    let updated: i64 = row.get(10)?;
    let seq = row.get(0)?;
    let from_instance = row.get(2)?;
    let to_instance = row.get(3)?;
    let task_id = row.get(4)?;
    let body = row.get(5)?;
    let turn_id = row.get(8)?;
    let attempted: Option<i64> = row.get(11)?;
    Ok((|| {
        let invalid = |what: String| StoreError::Invalid(format!("message {id}: {what}"));
        Ok(Message {
            level: parse_level(&level).ok_or_else(|| invalid(format!("level {level:?}")))?,
            state: parse_state(&state).ok_or_else(|| invalid(format!("state {state:?}")))?,
            created_at_unix_ms: u64::try_from(created)
                .map_err(|_| invalid(format!("created_at {created}")))?,
            updated_at_unix_ms: u64::try_from(updated)
                .map_err(|_| invalid(format!("updated_at {updated}")))?,
            seq,
            from_instance,
            to_instance,
            task_id,
            body,
            turn_id,
            attempted_at_unix_ms: attempted.and_then(|a| u64::try_from(a).ok()),
            id: id.clone(),
        })
    })())
}

fn ms(value: u64) -> Result<i64, StoreError> {
    i64::try_from(value).map_err(|_| StoreError::Invalid(format!("time {value} out of range")))
}

pub(crate) fn get(conn: &Connection, id: &str) -> Result<Option<Message>, StoreError> {
    conn.query_row(
        &format!("SELECT {COLUMNS} FROM messages WHERE id = ?1"),
        [id],
        from_row,
    )
    .optional()?
    .transpose()
}

/// The messages to `to_instance`, in `seq` order.
pub(crate) fn to_instance(conn: &Connection, to: &str) -> Result<Vec<Message>, StoreError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM messages WHERE to_instance = ?1 ORDER BY seq"
    ))?;
    let rows = stmt.query_map([to], from_row)?;
    rows.map(|row| row?).collect()
}

/// Pending OpenCode writes and reconciliation use separate bounded pages so
/// a prefix of ambiguous attempts cannot starve new queued work.
pub(crate) fn opencode_page(
    conn: &Connection,
    to: &str,
    attempted: bool,
    after: i64,
) -> Result<Vec<Message>, StoreError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM messages WHERE to_instance=?1 AND seq>?2 \
         AND state IN ('queued','sent') AND (attempted_at_unix_ms IS NOT NULL)=?3 \
         ORDER BY seq LIMIT ?4"
    ))?;
    let rows = stmt.query_map(
        rusqlite::params![to, after, attempted, if attempted { 8 } else { 32 }],
        from_row,
    )?;
    rows.map(|row| row?).collect()
}

/// Bounded unattempted Claude messages; retained history is never loaded
/// just to find the next dispatch batch.
pub(crate) fn pending_claude(
    conn: &Connection,
    to: &str,
    queue_only: bool,
) -> Result<Vec<Message>, StoreError> {
    pending_claude_filtered(conn, to, u8::from(queue_only))
}
pub(crate) fn pending_claude_filtered(
    conn: &Connection,
    to: &str,
    filter: u8,
) -> Result<Vec<Message>, StoreError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM messages WHERE to_instance = ?1 AND state = 'queued' \
         AND (?2 = 0 OR (?2 = 1 AND level = 'queue') OR (?2 = 2 AND level != 'queue')) AND attempted_at_unix_ms IS NULL AND id IN \
         (SELECT message_id FROM claude_deliveries WHERE instance_id = ?1 \
         AND delivery_id IS NULL AND abandoned_at_unix_ms IS NULL) ORDER BY seq LIMIT 32"
    ))?;
    let rows = stmt.query_map(rusqlite::params![to, filter], from_row)?;
    rows.map(|row| row?).collect()
}

/// Page only unresolved attempts, before LIMIT; unrelated message history
/// must not hide an unknown result. The caller wraps the sequence cursor.
pub(crate) fn unknown_claude_after(
    conn: &Connection,
    after: i64,
    before: u64,
) -> Result<Vec<Message>, StoreError> {
    let columns = COLUMNS
        .split(", ")
        .map(|s| format!("m.{s}"))
        .collect::<Vec<_>>()
        .join(", ");
    let mut query = conn.prepare(&format!(
        "SELECT {columns} FROM messages m JOIN claude_deliveries d ON d.message_id=m.id \
         WHERE m.seq>?1 AND m.state='queued' AND m.attempted_at_unix_ms IS NOT NULL \
         AND m.attempted_at_unix_ms<=?2 AND d.abandoned_at_unix_ms IS NULL \
         AND d.sent_at_unix_ms IS NULL AND d.confirmed_at_unix_ms IS NULL \
         ORDER BY m.seq LIMIT 32"
    ))?;
    query
        .query_map(rusqlite::params![after, ms(before)?], from_row)?
        .map(|r| r?)
        .collect()
}

/// The messages to `to` after its message `after` (by `seq`); `None` when
/// `to` has no message with id `after` (unknown, pruned, or someone else's).
pub(crate) fn to_instance_after(
    conn: &Connection,
    to: &str,
    after: &str,
) -> Result<Option<Vec<Message>>, StoreError> {
    let seq: Option<i64> = conn
        .query_row(
            "SELECT seq FROM messages WHERE id = ?1 AND to_instance = ?2",
            [after, to],
            |r| r.get(0),
        )
        .optional()?;
    let Some(seq) = seq else {
        return Ok(None);
    };
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLUMNS} FROM messages WHERE to_instance = ?1 AND seq > ?2 ORDER BY seq"
    ))?;
    let rows = stmt.query_map(rusqlite::params![to, seq], from_row)?;
    rows.map(|row| row?)
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

/// Looks `new.id` up, compares, and inserts it as `queued` when it is new;
/// all in the caller's closure on the DB thread (P5).
pub(crate) fn claim(conn: &Connection, new: &NewMessage, now: u64) -> Result<Claim, StoreError> {
    if let Some(found) = get(conn, &new.id)? {
        return Ok(if found.same_as(new) {
            Claim::Existing(found)
        } else {
            Claim::Different(found)
        });
    }
    let now = ms(now)?;
    conn.execute(
        "INSERT INTO messages (id, from_instance, to_instance, task_id, body, level, state, \
         turn_id, created_at_unix_ms, updated_at_unix_ms) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'queued', NULL, ?7, ?7)",
        rusqlite::params![
            new.id,
            new.from_instance,
            new.to_instance,
            new.task_id,
            new.body,
            level_text(new.level),
            now
        ],
    )?;
    let inserted = get(conn, &new.id)?
        .ok_or_else(|| StoreError::Invalid(format!("message {} vanished", new.id)))?;
    Ok(Claim::Inserted(inserted))
}

/// Moves message `id` to `next` when its current state allows it
/// (`DeliveryState::can_transition_to`), recording `turn_id` when given.
/// Returns the row after the change, or `None` when nothing changed (no
/// such message, or the move is not allowed).
pub(crate) fn advance(
    conn: &Connection,
    id: &str,
    next: DeliveryState,
    turn_id: Option<&str>,
    now: u64,
) -> Result<Option<Message>, StoreError> {
    if super::claude::get(conn, id)?.is_some() {
        return Err(StoreError::Invalid(
            "Claude push requires an explicit delivery or ACK transaction".into(),
        ));
    }
    advance_claude(conn, id, next, turn_id, now)
}

/// Shared forward transitions used within the Claude attribution transaction.
pub(super) fn advance_claude(
    conn: &Connection,
    id: &str,
    next: DeliveryState,
    turn_id: Option<&str>,
    now: u64,
) -> Result<Option<Message>, StoreError> {
    let Some(current) = get(conn, id)? else {
        return Ok(None);
    };
    if !current.state.can_transition_to(next) {
        return Ok(None);
    }
    conn.execute(
        "UPDATE messages SET state = ?2, turn_id = COALESCE(?3, turn_id), \
         updated_at_unix_ms = ?4 WHERE id = ?1",
        rusqlite::params![id, state_text(next), turn_id, ms(now)?],
    )?;
    get(conn, id)
}

/// Records that message `id` is about to be sent (only while `queued`).
pub(crate) fn mark_attempted(conn: &Connection, id: &str, now: u64) -> Result<(), StoreError> {
    conn.execute(
        "UPDATE messages SET attempted_at_unix_ms = COALESCE(attempted_at_unix_ms, ?2) \
         WHERE id = ?1 AND state = 'queued'",
        rusqlite::params![id, ms(now)?],
    )?;
    Ok(())
}
