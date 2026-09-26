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
//! - Retention: 30 days by `created_at_unix_ms` (D31).
//!
//! Must NOT: change a row's id, sender, target, task, body or level after
//! it was inserted.

use agend_core::model::DeliveryState;
use agend_core::policy::busy::BusyLevel;
use rusqlite::{Connection, OptionalExtension, Row};

use super::StoreError;

/// A message to insert.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewMessage {
    pub id: String,
    pub from_instance: String,
    pub to_instance: String,
    pub task_id: Option<String>,
    pub body: String,
    pub level: BusyLevel,
}

/// A stored message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub seq: i64,
    pub id: String,
    pub from_instance: String,
    pub to_instance: String,
    pub task_id: Option<String>,
    pub body: String,
    pub level: BusyLevel,
    pub state: DeliveryState,
    pub turn_id: Option<String>,
    pub created_at_unix_ms: u64,
    pub updated_at_unix_ms: u64,
}

impl Message {
    /// Same id, sender, target, task, body and level as `new`.
    pub fn same_as(&self, new: &NewMessage) -> bool {
        self.id == new.id
            && self.from_instance == new.from_instance
            && self.to_instance == new.to_instance
            && self.task_id == new.task_id
            && self.body == new.body
            && self.level == new.level
    }
}

/// What [`claim`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Claim {
    /// A new id: inserted as `queued`.
    Inserted(Message),
    /// The id exists with the same content: nothing changed.
    Existing(Message),
    /// The id exists with other content: nothing changed (`invalid_request`).
    Different(Message),
}

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
                       turn_id, created_at_unix_ms, updated_at_unix_ms";

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
