-- 0004_messages (gate 7 P2, P3, P5): the messages table (delivery model),
-- the agent's process group for the codex sweep, and the codex instances
-- left from before gate 7. Never edit this file once it has shipped.

-- One row per message the daemon delivers (retention: 30 days by
-- `created_at_unix_ms`, D31). This table is the only idempotency layer: a
-- message id is inserted once. `seq` is the explicit order (an INTEGER
-- PRIMARY KEY is kept by VACUUM / VACUUM INTO; a bare rowid may not be); AUTOINCREMENT
-- never gives a number twice, even after a prune empties the table (gate 9's
-- `inbox --after`). `attempted_at_unix_ms`: set just before the first RPC that
-- sends it; a `queued` row with it set may have reached codex and is only sent
-- again after the thread history and queue show it did not.
-- `state`: `queued` (in the DB, no backend reply yet), `sent` (the backend
-- accepted it), `confirmed` (it is in the agent's thread), `failed` (the
-- backend refused it, or the instance was failed). `turn_id`: the backend
-- turn it went into, when known.
CREATE TABLE messages (
    seq                INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    id                 TEXT    NOT NULL UNIQUE,
    from_instance      TEXT    NOT NULL,
    to_instance        TEXT    NOT NULL,
    task_id            TEXT,
    body               TEXT    NOT NULL,
    level              TEXT    NOT NULL CHECK (level IN ('queue', 'steer', 'interrupt')),
    state              TEXT    NOT NULL CHECK (state IN ('queued', 'sent', 'confirmed', 'failed')),
    turn_id            TEXT,
    attempted_at_unix_ms INTEGER CHECK (attempted_at_unix_ms IS NULL OR attempted_at_unix_ms >= 0),
    created_at_unix_ms INTEGER NOT NULL CHECK (created_at_unix_ms >= 0),
    updated_at_unix_ms INTEGER NOT NULL CHECK (updated_at_unix_ms >= 0)
) STRICT;

CREATE INDEX messages_by_target ON messages (to_instance, seq);
CREATE INDEX messages_by_time ON messages (created_at_unix_ms);

-- The pid of the agent a holder started (`Spawned.process_id`; it is its
-- own process group). Written at `Spawned`, cleared after the sweep that
-- follows a holder's death (P2).
ALTER TABLE instances ADD COLUMN agent_pid INTEGER CHECK (agent_pid IS NULL OR agent_pid > 0);

-- 1 for a codex instance this migration found without a thread id that may
-- hold a conversation (P3): a human decides, it is never started again.
ALTER TABLE instances ADD COLUMN legacy_no_thread INTEGER NOT NULL DEFAULT 0
    CHECK (legacy_no_thread IN (0, 1));

-- Decided once, here: a codex row from gate 6 with no thread id whose
-- session started is a bare codex TUI with a real conversation. `new` rows
-- (never started) and rows that failed before their first `Spawn`
-- (`session_started = 0`) are left alone.
UPDATE instances SET legacy_no_thread = 1, status = 'failed'
WHERE backend = 'codex' AND session_id IS NULL AND status IN ('running', 'failed')
  AND session_started = 1;
