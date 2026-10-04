-- D40: event history is separate from unresolved delivery recovery data.
CREATE TABLE driver_events (
    seq INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    instance_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    payload TEXT NOT NULL CHECK (json_valid(payload) AND json_type(payload) = 'object'),
    occurred_at_unix_ms INTEGER NOT NULL CHECK (occurred_at_unix_ms >= 0),
    ingested_at_unix_ms INTEGER NOT NULL CHECK (ingested_at_unix_ms >= 0),
    replayed INTEGER NOT NULL CHECK (replayed IN (0, 1))
) STRICT;
CREATE INDEX driver_events_by_time ON driver_events (ingested_at_unix_ms);

-- A marker is captured atomically with message insertion, before any attempt.
-- No instance FK: removal must not erase pending ACK attribution or retention.
CREATE TABLE claude_deliveries (
    message_id TEXT NOT NULL PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    instance_id TEXT NOT NULL,
    delivery_id TEXT UNIQUE,
    session_id TEXT,
    route TEXT CHECK (route IN ('channel', 'stop')),
    started_at_unix_ms INTEGER CHECK (started_at_unix_ms >= 0),
    sent_at_unix_ms INTEGER CHECK (sent_at_unix_ms >= 0),
    confirmed_at_unix_ms INTEGER CHECK (confirmed_at_unix_ms >= 0),
    abandoned_at_unix_ms INTEGER CHECK (abandoned_at_unix_ms >= 0),
    abandonment_reason TEXT,
    CHECK ((delivery_id IS NULL AND session_id IS NULL AND route IS NULL AND started_at_unix_ms IS NULL)
        OR (delivery_id IS NOT NULL AND session_id IS NOT NULL AND route IS NOT NULL AND started_at_unix_ms IS NOT NULL)),
    CHECK (sent_at_unix_ms IS NULL OR started_at_unix_ms IS NOT NULL),
    CHECK (confirmed_at_unix_ms IS NULL OR sent_at_unix_ms IS NOT NULL),
    CHECK (confirmed_at_unix_ms IS NULL OR abandoned_at_unix_ms IS NULL),
    CHECK ((abandoned_at_unix_ms IS NULL) = (abandonment_reason IS NULL))
) STRICT;
INSERT INTO claude_deliveries(message_id, instance_id)
SELECT m.id, m.to_instance FROM messages m JOIN instances i ON i.id = m.to_instance
WHERE i.backend = 'claude' AND i.delivery = 'push';
CREATE TRIGGER capture_claude_message AFTER INSERT ON messages
WHEN EXISTS (SELECT 1 FROM instances WHERE id = NEW.to_instance AND backend = 'claude' AND delivery = 'push')
BEGIN
    INSERT INTO claude_deliveries(message_id, instance_id) VALUES (NEW.id, NEW.to_instance);
END;
