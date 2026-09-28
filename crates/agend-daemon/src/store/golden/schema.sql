-- user_version = 4

CREATE INDEX messages_by_target ON messages (to_instance, seq);

CREATE INDEX messages_by_time ON messages (created_at_unix_ms);

CREATE INDEX task_events_by_task ON task_events (task_id, seq);

CREATE INDEX task_events_by_time ON task_events (occurred_at_unix_ms);

CREATE TABLE instances (
    id                TEXT NOT NULL PRIMARY KEY
                      CHECK (length(id) BETWEEN 1 AND 24 AND id NOT GLOB '*[^a-z0-9-]*'),
    backend           TEXT NOT NULL CHECK (backend IN ('claude', 'codex', 'opencode')),
    program           TEXT NOT NULL,
    args              TEXT NOT NULL CHECK (json_valid(args) AND json_type(args) = 'array'),
    working_directory TEXT NOT NULL,
    session_id        TEXT,
    status            TEXT NOT NULL CHECK (status IN ('new', 'running', 'failed'))
, session_started INTEGER NOT NULL DEFAULT 0
    CHECK (session_started IN (0, 1)), agent_pid INTEGER CHECK (agent_pid IS NULL OR agent_pid > 0), legacy_no_thread INTEGER NOT NULL DEFAULT 0
    CHECK (legacy_no_thread IN (0, 1))) STRICT;

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

CREATE TABLE sqlite_sequence(name,seq);

CREATE TABLE task_events (
    seq                 INTEGER NOT NULL PRIMARY KEY,
    task_id             TEXT    NOT NULL REFERENCES tasks (id),
    event_id            TEXT    NOT NULL,
    occurred_at_unix_ms INTEGER NOT NULL CHECK (occurred_at_unix_ms >= 0),
    kind                TEXT    NOT NULL,
    detail              TEXT    NOT NULL
) STRICT;

CREATE TABLE tasks (
    id               TEXT    NOT NULL PRIMARY KEY,
    title            TEXT    NOT NULL,
    team_id          TEXT    NOT NULL,
    workflow_id      TEXT    NOT NULL,
    workflow_version INTEGER NOT NULL CHECK (workflow_version >= 0),
    parent           TEXT,
    depends_on       TEXT    NOT NULL CHECK (json_valid(depends_on) AND json_type(depends_on) = 'array'),
    superseded_by    TEXT,
    assignee         TEXT,
    status           TEXT    NOT NULL CHECK (status IN ('open', 'running', 'blocked', 'done', 'superseded')),
    requires_repo    INTEGER NOT NULL CHECK (requires_repo IN (0, 1)),
    merge_commit     TEXT,
    version          INTEGER NOT NULL CHECK (version >= 1)
) STRICT;

CREATE TABLE workflows (
    id      TEXT    NOT NULL,
    version INTEGER NOT NULL CHECK (version >= 0),
    toml    TEXT    NOT NULL,
    PRIMARY KEY (id, version)
) STRICT;
