//! Exact-snapshot pairing and cursor publication on the sole DB thread.
use super::{SqliteStore, StoreError};
use agend_core::{
    telegram::pairing::{PairingCandidate, PairingPhase, PairingRecord, TelegramPairing},
    traits::Clock,
};
use rusqlite::{Connection, OptionalExtension, params};

struct At(u64);
impl Clock for At {
    fn now_unix_ms(&self) -> u64 {
        self.0
    }
}
fn invalid(message: impl Into<String>) -> StoreError {
    StoreError::Invalid(message.into())
}
fn validate(session: &TelegramPairing) -> Result<(), StoreError> {
    let mut rebuilt = TelegramPairing::new(
        session.id.clone(),
        session.token.clone(),
        session.bot_id,
        session.bot_username.clone(),
        &At(session.created_at_ms),
    )
    .map_err(invalid)?;
    if session.offset < 0 {
        return Err(invalid("negative pairing cursor"));
    }
    rebuilt.offset = session.offset;
    rebuilt.candidate = session.candidate.clone();
    if rebuilt != *session {
        return Err(invalid("pairing identity or expiry changed"));
    }
    if let Some(candidate) = &session.candidate {
        session
            .confirm(candidate, &At(session.created_at_ms))
            .map_err(invalid)?;
    }
    Ok(())
}
fn read(conn: &Connection) -> Result<Option<PairingRecord>, StoreError> {
    let row: Option<(String, String)> = conn
        .query_row(
            "SELECT id,record FROM telegram_pairing WHERE slot=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    row.map(|(id, json)| {
        let record: PairingRecord =
            serde_json::from_str(&json).map_err(|_| invalid("invalid pairing record"))?;
        validate(&record.session)?;
        if id != record.session.id
            || (record.phase == PairingPhase::Confirmed && record.session.candidate.is_none())
        {
            return Err(invalid("inconsistent pairing identity or confirmation"));
        }
        Ok(record)
    })
    .transpose()
}
fn write(conn: &Connection, record: &PairingRecord) -> Result<(), StoreError> {
    let json =
        serde_json::to_string(record).map_err(|_| invalid("cannot encode pairing record"))?;
    conn.execute("INSERT INTO telegram_pairing(slot,id,record) VALUES(1,?1,?2) ON CONFLICT(slot) DO UPDATE SET id=excluded.id,record=excluded.record",
        params![record.session.id, json])?;
    Ok(())
}
impl SqliteStore {
    pub async fn telegram_pairing(&self) -> Result<Option<PairingRecord>, StoreError> {
        self.call(|conn| read(conn)).await
    }

    pub async fn begin_telegram_pairing(
        &self,
        session: &TelegramPairing,
        previous: Option<&str>,
        now: u64,
    ) -> Result<PairingRecord, StoreError> {
        validate(session)?;
        session.check_time(&At(now)).map_err(invalid)?;
        if session.offset != 0 || session.candidate.is_some() {
            return Err(invalid("pairing must begin without observations"));
        }
        let session = session.clone();
        let previous = previous.map(str::to_owned);
        self.call(move |conn| {
            let tx = conn.transaction()?;
            let current = read(&tx)?;
            if current.as_ref().map(|r| &r.session.id) != previous.as_ref() {
                return Err(invalid("pairing changed; query status before beginning"));
            }
            if let Some(current) = current
                && (current.session.id == session.id
                    || (current.phase == PairingPhase::Pending
                        && now < current.session.expires_at_ms))
            {
                return Err(invalid("pairing is still pending or its id was reused"));
            }
            let record = PairingRecord {
                session,
                phase: PairingPhase::Pending,
            };
            write(&tx, &record)?;
            tx.commit()?;
            Ok(record)
        })
        .await
    }

    /// Persist the candidate and the next cursor together before another poll.
    pub async fn observe_telegram_pairing(
        &self,
        expected: &PairingRecord,
        observed: &TelegramPairing,
        now: u64,
    ) -> Result<PairingRecord, StoreError> {
        validate(observed)?;
        observed.check_time(&At(now)).map_err(invalid)?;
        let mut identity = observed.clone();
        identity.offset = expected.session.offset;
        identity.candidate = expected.session.candidate.clone();
        if identity != expected.session
            || observed.offset < expected.session.offset
            || (expected.session.candidate.is_some()
                && observed.candidate != expected.session.candidate)
            || (observed.candidate != expected.session.candidate
                && observed.offset == expected.session.offset)
        {
            return Err(invalid("pairing identity, candidate or cursor changed"));
        }
        let expected = expected.clone();
        let observed = observed.clone();
        self.call(move |conn| {
            let tx = conn.transaction()?;
            if expected.phase != PairingPhase::Pending || read(&tx)?.as_ref() != Some(&expected) {
                return Err(invalid("pairing observation is stale or closed"));
            }
            let record = PairingRecord {
                session: observed,
                phase: PairingPhase::Pending,
            };
            write(&tx, &record)?;
            tx.commit()?;
            Ok(record)
        })
        .await
    }

    pub async fn confirm_telegram_pairing(
        &self,
        expected: &PairingRecord,
        candidate: &PairingCandidate,
        now: u64,
    ) -> Result<PairingRecord, StoreError> {
        expected
            .session
            .confirm(candidate, &At(now))
            .map_err(invalid)?;
        self.close_telegram_pairing(expected, PairingPhase::Confirmed)
            .await
    }
    pub async fn cancel_telegram_pairing(
        &self,
        expected: &PairingRecord,
    ) -> Result<PairingRecord, StoreError> {
        self.close_telegram_pairing(expected, PairingPhase::Cancelled)
            .await
    }
    async fn close_telegram_pairing(
        &self,
        expected: &PairingRecord,
        phase: PairingPhase,
    ) -> Result<PairingRecord, StoreError> {
        let expected = expected.clone();
        self.call(move |conn| {
            let tx = conn.transaction()?;
            if expected.phase != PairingPhase::Pending || read(&tx)?.as_ref() != Some(&expected) {
                return Err(invalid(
                    "pairing changed or is already closed; query status",
                ));
            }
            let record = PairingRecord { phase, ..expected };
            write(&tx, &record)?;
            tx.commit()?;
            Ok(record)
        })
        .await
    }
}
