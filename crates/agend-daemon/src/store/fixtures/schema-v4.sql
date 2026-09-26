-- Schema version 4 as shipped (gate 7), with sample rows. Never edit: the
-- test upgrades this to the latest schema and compares it with
-- golden/schema.sql, so a changed old migration fails. The next migration's
-- PR adds schema-v5.sql the same way.

-- One row per task; every field of agend_core's `Task` is a column. The row
-- is only rewritten by compare-and-swap on `version`, which starts at 1 and
-- grows by 1 per write. Rows are never deleted (retention: forever).
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

-- One row per saved workflow version (D19, D21): the workflow as D19 TOML.
-- A saved version never changes (retention: forever).
CREATE TABLE workflows (
    id      TEXT    NOT NULL,
    version INTEGER NOT NULL CHECK (version >= 0),
    toml    TEXT    NOT NULL,
    PRIMARY KEY (id, version)
) STRICT;

-- Task history for people (gate 5 P4: never replayed). Ordered by `seq`;
-- pruned after 14 days by `occurred_at_unix_ms` (D31).
CREATE TABLE task_events (
    seq                 INTEGER NOT NULL PRIMARY KEY,
    task_id             TEXT    NOT NULL REFERENCES tasks (id),
    event_id            TEXT    NOT NULL,
    occurred_at_unix_ms INTEGER NOT NULL CHECK (occurred_at_unix_ms >= 0),
    kind                TEXT    NOT NULL,
    detail              TEXT    NOT NULL
) STRICT;

CREATE INDEX task_events_by_task ON task_events (task_id, seq);
CREATE INDEX task_events_by_time ON task_events (occurred_at_unix_ms);

-- One row per instance (retention: forever). `id` names the holder's files
-- under run/holders/, so it is short and file-name safe. `args` is a JSON
-- array of the agent's base arguments; the daemon appends the backend's
-- session arguments (`--session-id` / `--resume`) itself. `status`: `new`
-- (never started), `running` (the daemon keeps it running; every later
-- start resumes), `failed` (the daemon gave up; a human decides).
CREATE TABLE instances (
    id                TEXT NOT NULL PRIMARY KEY
                      CHECK (length(id) BETWEEN 1 AND 24 AND id NOT GLOB '*[^a-z0-9-]*'),
    backend           TEXT NOT NULL CHECK (backend IN ('claude', 'codex', 'opencode')),
    program           TEXT NOT NULL,
    args              TEXT NOT NULL CHECK (json_valid(args) AND json_type(args) = 'array'),
    working_directory TEXT NOT NULL,
    session_id        TEXT,
    status            TEXT NOT NULL CHECK (status IN ('new', 'running', 'failed'))
) STRICT;

-- 0003_session_started
ALTER TABLE instances ADD COLUMN session_started INTEGER NOT NULL DEFAULT 0
    CHECK (session_started IN (0, 1));

-- 0004_messages
CREATE TABLE messages (
    seq                INTEGER NOT NULL PRIMARY KEY,
    id                 TEXT    NOT NULL UNIQUE,
    from_instance      TEXT    NOT NULL,
    to_instance        TEXT    NOT NULL,
    task_id            TEXT,
    body               TEXT    NOT NULL,
    level              TEXT    NOT NULL CHECK (level IN ('queue', 'steer', 'interrupt')),
    state              TEXT    NOT NULL CHECK (state IN ('queued', 'sent', 'confirmed', 'failed')),
    turn_id            TEXT,
    created_at_unix_ms INTEGER NOT NULL CHECK (created_at_unix_ms >= 0),
    updated_at_unix_ms INTEGER NOT NULL CHECK (updated_at_unix_ms >= 0)
) STRICT;

CREATE INDEX messages_by_target ON messages (to_instance, seq);
CREATE INDEX messages_by_time ON messages (created_at_unix_ms);

ALTER TABLE instances ADD COLUMN agent_pid INTEGER CHECK (agent_pid IS NULL OR agent_pid > 0);
ALTER TABLE instances ADD COLUMN legacy_no_thread INTEGER NOT NULL DEFAULT 0
    CHECK (legacy_no_thread IN (0, 1));

PRAGMA user_version = 4;

INSERT INTO tasks (id, title, team_id, workflow_id, workflow_version, parent, depends_on,
                   superseded_by, assignee, status, requires_repo, merge_commit, version)
VALUES ('T-fixture', 'fixture task', 'general', 'code', 1, 'T-parent', '["T-a","T-b"]',
        NULL, 'dev-1', 'blocked', 1, NULL, 3);

INSERT INTO workflows (id, version, toml) VALUES ('code', 1, 'id = "code"
version = 1
requires = ["repo"]
allow_unreviewed = false

[[stages]]
id = "work"

[stages.stage]
kind = "work"
role = "dev"
instructions = ""
output = "branch"

[[stages]]
id = "submit"

[stages.stage]
kind = "submit"
forge = "local"

[[stages]]
id = "checks"
timeout_ms = 300000

[stages.stage]
kind = "command"
command = "cargo test"

[[stages]]
id = "review"

[stages.stage]
kind = "approval"
count = 1
bind_head = true

[stages.stage.by]
role = "reviewer"

[[stages]]
id = "merge"

[stages.stage]
kind = "merge"
');

INSERT INTO task_events (task_id, event_id, occurred_at_unix_ms, kind, detail)
VALUES ('T-fixture', 'e-1', 1790000000000, 'created', ''),
       ('T-fixture', 'e-2', 1790000000001, 'stage_completed', 'work');

INSERT INTO instances (id, backend, program, args, working_directory, session_id, status,
                       session_started, agent_pid, legacy_no_thread)
VALUES ('fixture-1', 'claude', '/bin/bash', '["-c","exit 0"]', '/tmp', 's-fixture', 'running', 1,
        NULL, 0);

INSERT INTO messages (id, from_instance, to_instance, task_id, body, level, state, turn_id,
                      created_at_unix_ms, updated_at_unix_ms)
VALUES ('m-fixture', 'operator', 'fixture-1', 'T-fixture', 'hello', 'queue', 'confirmed',
        't-fixture', 1790000000000, 1790000000001);
