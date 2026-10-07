//! Native inbound update ledger: claim before operator dispatch, never replay.
use super::{SqliteStore, StoreError};
use agend_core::telegram::{TelegramDelivery, TelegramInboundStore};
use rusqlite::{OptionalExtension, params};
fn invalid() -> StoreError {
    StoreError::Invalid("invalid Telegram update identity".into())
}
impl TelegramInboundStore for SqliteStore {
    type Error = StoreError;
    async fn telegram_offset(&self, bot: u64) -> Result<i64, StoreError> {
        self.call(move |c| {
            Ok(c.query_row(
                "SELECT COALESCE(MAX(update_id)+1,0) FROM telegram_updates WHERE bot_id=?1",
                [bot],
                |r| r.get(0),
            )?)
        })
        .await
    }
    async fn claim_telegram_update(
        &self,
        bot: u64,
        update: i64,
        fingerprint: &str,
    ) -> Result<bool, StoreError> {
        let hash = fingerprint.to_owned();
        self.call(move |c| {
            if bot == 0
                || bot >= (1 << 52)
                || update < 0
                || update == i64::MAX
                || hash.len() != 64
                || !hash.bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err(invalid());
            }
            let old: Option<String> = c
                .query_row(
                    "SELECT fingerprint FROM telegram_updates WHERE bot_id=?1 AND update_id=?2",
                    params![bot, update],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(old) = old {
                if old != hash {
                    return Err(invalid());
                }
                return Ok(false);
            }
            c.execute(
                "INSERT INTO telegram_updates(bot_id,update_id,fingerprint) VALUES(?1,?2,?3)",
                params![bot, update, hash],
            )?;
            Ok(true)
        })
        .await
    }
    async fn claim_telegram_action(
        &self,
        bot: u64,
        update: i64,
        delivery: &str,
    ) -> Result<bool, StoreError> {
        let delivery = delivery.to_owned();
        self.call(move |c| {
            Ok(c.execute(
                "UPDATE telegram_updates SET delivery_id=?3
                WHERE bot_id=?1 AND update_id=?2 AND outcome IS NULL AND delivery_id IS NULL
                AND NOT EXISTS(SELECT 1 FROM telegram_updates WHERE delivery_id=?3)
                AND EXISTS(SELECT 1 FROM telegram_notices WHERE delivery_id=?3 AND active=1)",
                params![bot, update, delivery],
            )? == 1)
        })
        .await
    }
    async fn finish_telegram_update(
        &self,
        bot: u64,
        update: i64,
        outcome: &str,
    ) -> Result<bool, StoreError> {
        let outcome = outcome.to_owned();
        self.call(move |c| {
            if outcome.is_empty() || outcome.len()>4096 { return Err(invalid()); }
            let tx = c.transaction()?;
            let changed = tx.execute("UPDATE telegram_updates SET outcome=?3 WHERE bot_id=?1 AND update_id=?2 AND outcome IS NULL",params![bot,update,outcome])? == 1;
            if changed {
                tx.execute("UPDATE telegram_notices SET active=0 WHERE delivery_id=(SELECT delivery_id FROM telegram_updates WHERE bot_id=?1 AND update_id=?2)",params![bot,update])?;
            }
            tx.commit()?;
            Ok(changed)
        }).await
    }
    async fn telegram_message(
        &self,
        bot: u64,
        chat: i64,
        message: i64,
    ) -> Result<Option<TelegramDelivery>, StoreError> {
        self.call(move |c| {
            let mut query=c.prepare("SELECT o.delivery FROM telegram_outbox o JOIN telegram_notices n ON n.delivery_id=o.id AND n.active=1 WHERE NOT EXISTS(SELECT 1 FROM telegram_updates u WHERE u.delivery_id=o.id) AND json_extract(o.delivery,'$.destination.bot_id')=?1 AND json_extract(o.delivery,'$.destination.chat_id')=?2 AND EXISTS(SELECT 1 FROM json_each(o.delivery,'$.message_ids') WHERE value=?3) LIMIT 2")?;
            let rows:Vec<String>=query.query_map(params![bot,chat,message],|r|r.get(0))?.collect::<Result<_,_>>()?;
            if rows.len()>1 { return Err(invalid()); }
            let Some(text)=rows.into_iter().next() else {return Ok(None);};
            let row:TelegramDelivery=serde_json::from_str(&text).map_err(|_|invalid())?;
            if !row.valid() || row.abandoned {return Err(invalid());}
            Ok(Some(row))
        }).await
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use agend_testkit::{block_on, tempdir::TempDir};
    #[test]
    fn claimed_update_is_never_dispatched_twice_even_after_reopen_without_completion() {
        let dir = TempDir::new("telegram-update").unwrap();
        let store = SqliteStore::open(dir.path(), 0).unwrap();
        let hash = "a".repeat(64);
        assert!(block_on(store.claim_telegram_update(42, 123, &hash)).unwrap());
        drop(store);
        let store = SqliteStore::open(dir.path(), 1).unwrap();
        assert!(!block_on(store.claim_telegram_update(42, 123, &hash)).unwrap());
        assert_eq!(block_on(store.telegram_offset(42)).unwrap(), 124);
        assert_eq!(block_on(store.telegram_offset(43)).unwrap(), 0);
        assert!(block_on(store.claim_telegram_update(42, 123, &"b".repeat(64))).is_err());
        assert!(block_on(store.finish_telegram_update(42, 123, "accepted")).unwrap());
        assert!(!block_on(store.finish_telegram_update(42, 123, "replayed")).unwrap());
    }
}
