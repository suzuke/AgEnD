-- user_version = 3

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
    CHECK (session_started IN (0, 1))) STRICT;

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
