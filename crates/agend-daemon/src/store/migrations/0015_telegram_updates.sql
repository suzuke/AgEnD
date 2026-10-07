-- A row exists before an operator operation can start. NULL outcome is unknown.
CREATE TABLE telegram_updates (
    bot_id INTEGER NOT NULL CHECK(bot_id > 0),
    update_id INTEGER NOT NULL CHECK(update_id >= 0),
    fingerprint TEXT NOT NULL,
    outcome TEXT,
    delivery_id TEXT UNIQUE REFERENCES telegram_outbox(id),
    PRIMARY KEY(bot_id,update_id)
) STRICT;

-- A monotonic failure episode survives boot and distinguishes identical recurrences.
CREATE TABLE instance_failures (
    instance_id TEXT PRIMARY KEY REFERENCES instances(id) ON DELETE CASCADE,
    reason TEXT NOT NULL,
    since_ms INTEGER NOT NULL CHECK(since_ms >= 0)
) STRICT;

ALTER TABLE tasks ADD COLUMN attention_revision INTEGER NOT NULL DEFAULT 0 CHECK(attention_revision >= 0);
