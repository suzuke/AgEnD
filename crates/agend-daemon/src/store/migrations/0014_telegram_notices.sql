-- Last observed exception survives a boot-local Fleet/event cursor reset.
CREATE TABLE telegram_notices (
    source_id TEXT PRIMARY KEY NOT NULL,
    delivery_id TEXT NOT NULL REFERENCES telegram_outbox(id),
    active INTEGER NOT NULL CHECK(active IN (0,1))
) STRICT;
