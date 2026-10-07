//! Whole notification enqueue and per-part persist-before-send receipts.
use super::{SqliteStore, StoreError};
use agend_core::telegram::{TelegramDelivery, TelegramDestination, TelegramNotice, TelegramStore};
use rusqlite::{Connection, OptionalExtension, params};

fn invalid() -> StoreError {
    StoreError::Invalid("invalid Telegram delivery state".into())
}
fn load(c: &Connection, id: &str) -> Result<Option<TelegramDelivery>, StoreError> {
    let text: Option<String> = c
        .query_row(
            "SELECT delivery FROM telegram_outbox WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .optional()?;
    text.map(|text| {
        let row: TelegramDelivery = serde_json::from_str(&text).map_err(|_| invalid())?;
        if row.id != id || !row.valid() {
            return Err(invalid());
        }
        Ok(row)
    })
    .transpose()
}
fn save(c: &Connection, row: &TelegramDelivery) -> Result<(), StoreError> {
    if !row.valid() {
        return Err(invalid());
    }
    c.execute(
        "UPDATE telegram_outbox SET delivery=?2 WHERE id=?1",
        params![row.id, serde_json::to_string(row).map_err(|_| invalid())?],
    )?;
    Ok(())
}
impl TelegramStore for SqliteStore {
    type Error = StoreError;
    async fn observe_telegram(
        &self,
        notices: &[TelegramNotice],
        destination: &TelegramDestination,
        now: u64,
    ) -> Result<Vec<TelegramDelivery>, StoreError> {
        let notices = notices.to_vec();
        let destination = destination.clone();
        self.call(move |c| {
            let mut keys = std::collections::BTreeSet::new();
            if notices.iter().any(|n| n.key.is_empty() || !keys.insert(n.key.clone())) {
                return Err(invalid());
            }
            let tx = c.transaction()?;
            let previous: Vec<(String, String, bool)> = {
                let mut q = tx.prepare("SELECT source_id,delivery_id,active FROM telegram_notices")?;
                q.query_map([], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?)))?
                    .collect::<Result<_,_>>()?
            };
            let mut pending = Vec::new();
            for notice in notices {
                let old = previous.iter().find(|(id,_,_)| *id == notice.key);
                let existing = old.map(|(_,id,_)| load(&tx,id)).transpose()?.flatten();
                if let (Some((_,_,true)), Some(row)) = (old, &existing)
                    && row.notification == notice.notification
                    && row.attention == notice.attention
                    && row.task_version == notice.task_version {
                        // A config/token change must not create another copy.
                        pending.push(row.clone());
                        continue;
                    }
                if let Some(mut old) = existing
                    && !old.complete() {
                        old.abandoned = true;
                        save(&tx, &old)?;
                    }
                let id = super::instances::new_session_id().map_err(|e| StoreError::Invalid(e.to_string()))?;
                let mut row = TelegramDelivery::new(id.clone(), destination.clone(), notice.notification, now);
                row.attention = notice.attention;
                row.task_version = notice.task_version;
                if !row.valid() { return Err(invalid()); }
                tx.execute("INSERT INTO telegram_outbox(id,delivery) VALUES(?1,?2)",
                    params![id,serde_json::to_string(&row).map_err(|_| invalid())?])?;
                tx.execute("INSERT INTO telegram_notices(source_id,delivery_id,active) VALUES(?1,?2,1) ON CONFLICT(source_id) DO UPDATE SET delivery_id=excluded.delivery_id,active=1",
                    params![notice.key,id])?;
                pending.push(row);
            }
            for (key,id,active) in previous {
                if active && !keys.contains(&key) {
                    if let Some(mut row) = load(&tx,&id)?
                        && !row.complete() { row.abandoned=true; save(&tx,&row)?; }
                    tx.execute("UPDATE telegram_notices SET active=0 WHERE source_id=?1", [key])?;
                }
            }
            tx.commit()?;
            Ok(pending)
        }).await
    }
    async fn enqueue_telegram(
        &self,
        delivery: &TelegramDelivery,
    ) -> Result<TelegramDelivery, StoreError> {
        let incoming = delivery.clone();
        self.call(move |c| {
            if !incoming.valid()
                || incoming.next_part != 0
                || incoming.in_flight
                || incoming.abandoned
            {
                return Err(invalid());
            }
            if let Some(old) = load(c, &incoming.id)? {
                if old.destination != incoming.destination
                    || old.notification != incoming.notification
                    || old.parts != incoming.parts
                    || old.attention != incoming.attention
                    || old.task_version != incoming.task_version
                {
                    return Err(invalid());
                }
                return Ok(old);
            }
            c.execute(
                "INSERT INTO telegram_outbox(id,delivery) VALUES(?1,?2)",
                params![
                    incoming.id,
                    serde_json::to_string(&incoming).map_err(|_| invalid())?
                ],
            )?;
            Ok(incoming)
        })
        .await
    }
    async fn telegram_delivery(&self, id: &str) -> Result<Option<TelegramDelivery>, StoreError> {
        let id = id.to_owned();
        self.call(move |c| load(c, &id)).await
    }
    async fn claim_telegram_part(&self, id: &str, part: usize) -> Result<bool, StoreError> {
        let id = id.to_owned();
        self.call(move |c| {
            let Some(mut row) = load(c, &id)? else {
                return Ok(false);
            };
            if row.next_part != part || row.in_flight || row.complete() || row.abandoned {
                return Ok(false);
            }
            row.in_flight = true;
            save(c, &row)?;
            Ok(true)
        })
        .await
    }
    async fn confirm_telegram_part(
        &self,
        id: &str,
        part: usize,
        message_id: i64,
    ) -> Result<bool, StoreError> {
        let id = id.to_owned();
        self.call(move |c| {
            let Some(mut row) = load(c, &id)? else {
                return Ok(false);
            };
            if row.next_part != part
                || !row.in_flight
                || row.abandoned
                || message_id <= 0
                || row.message_ids.contains(&message_id)
            {
                return Ok(false);
            }
            row.next_part += 1;
            row.in_flight = false;
            row.message_ids.push(message_id);
            save(c, &row)?;
            Ok(true)
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agend_core::{
        telegram::TelegramDestination,
        traits::{Notification, NotificationSeverity},
    };
    use agend_testkit::{block_on, tempdir::TempDir};
    #[test]
    fn partial_receipts_and_unknown_attempt_survive_reopen_without_replay() {
        let dir = TempDir::new("telegram-outbox").unwrap();
        let row = TelegramDelivery::new(
            "notice-1".into(),
            TelegramDestination {
                bot_id: 123456789,
                chat_id: 42,
                topic_id: None,
            },
            Notification {
                severity: NotificationSeverity::Attention,
                title: "Title".into(),
                body: " 繁中 é ✅\n".repeat(1500),
                task_id: Some("t-1".into()),
            },
            1,
        );
        assert!(row.parts.len() > 2);
        let store = SqliteStore::open(dir.path(), 0).unwrap();
        block_on(store.enqueue_telegram(&row)).unwrap();
        assert!(!block_on(store.confirm_telegram_part(&row.id, 0, 1)).unwrap());
        assert!(block_on(store.claim_telegram_part(&row.id, 0)).unwrap());
        assert!(!block_on(store.claim_telegram_part(&row.id, 0)).unwrap());
        assert!(block_on(store.confirm_telegram_part(&row.id, 0, 1)).unwrap());
        assert!(!block_on(store.confirm_telegram_part(&row.id, 0, 1)).unwrap());
        assert!(block_on(store.claim_telegram_part(&row.id, 1)).unwrap());
        drop(store);
        let store = SqliteStore::open(dir.path(), 1).unwrap();
        let resumed = block_on(store.enqueue_telegram(&row)).unwrap();
        assert_eq!(resumed.next_part, 1);
        assert!(resumed.in_flight);
        assert!(!block_on(store.claim_telegram_part(&row.id, 1)).unwrap());
        let mut changed = row.clone();
        changed.destination.chat_id = 43;
        assert!(block_on(store.enqueue_telegram(&changed)).is_err());
        assert!(!block_on(store.confirm_telegram_part(&row.id, 1, 1)).unwrap());
        assert!(block_on(store.confirm_telegram_part(&row.id, 1, 2)).unwrap());
        assert!(block_on(store.claim_telegram_part(&row.id, 2)).unwrap());
    }
    #[test]
    fn exception_identity_survives_restart_and_reopens_only_after_resolution_or_change() {
        let dir = TempDir::new("telegram-observe").unwrap();
        let destination = TelegramDestination {
            bot_id: 123456789,
            chat_id: 42,
            topic_id: None,
        };
        let mut notice = TelegramNotice {
            task_version: None,
            key: "approval:t-1/approve/1".into(),
            attention: None,
            notification: Notification {
                severity: NotificationSeverity::Attention,
                title: "Approve".into(),
                body: "head abc".into(),
                task_id: Some("t-1".into()),
            },
        };
        let store = SqliteStore::open(dir.path(), 0).unwrap();
        let first = block_on(store.observe_telegram(&[notice.clone()], &destination, 1))
            .unwrap()
            .remove(0);
        assert!(block_on(store.claim_telegram_part(&first.id, 0)).unwrap());
        assert!(block_on(store.confirm_telegram_part(&first.id, 0, 1)).unwrap());
        drop(store);
        let store = SqliteStore::open(dir.path(), 2).unwrap();
        let same = block_on(store.observe_telegram(&[notice.clone()], &destination, 2))
            .unwrap()
            .remove(0);
        assert_eq!(same.id, first.id);
        assert!(same.complete());
        assert!(
            block_on(store.observe_telegram(&[notice.clone(), notice.clone()], &destination, 3))
                .is_err()
        );
        assert_eq!(
            block_on(store.observe_telegram(&[notice.clone()], &destination, 4)).unwrap()[0].id,
            first.id
        );
        notice.notification.body = "head def".into();
        let changed = block_on(store.observe_telegram(&[notice.clone()], &destination, 5))
            .unwrap()
            .remove(0);
        assert_ne!(changed.id, first.id);
        block_on(store.observe_telegram(&[], &destination, 6)).unwrap();
        assert!(
            block_on(store.telegram_delivery(&changed.id))
                .unwrap()
                .unwrap()
                .abandoned
        );
        let reopened = block_on(store.observe_telegram(&[notice], &destination, 7))
            .unwrap()
            .remove(0);
        assert_ne!(reopened.id, changed.id);
        assert!(!reopened.abandoned);
    }
}
