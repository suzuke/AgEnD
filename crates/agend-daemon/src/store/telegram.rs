//! Whole notification enqueue and per-part persist-before-send receipts.
use super::{SqliteStore, StoreError};
use agend_core::telegram::{TelegramDelivery, TelegramStore};
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
}
