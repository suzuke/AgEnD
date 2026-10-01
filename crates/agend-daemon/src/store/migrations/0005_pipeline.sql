-- Gate 10: retain task ids, event order and all historical rows.
CREATE TEMP TABLE saved_events AS SELECT * FROM task_events;
DROP TABLE task_events;
CREATE TABLE tasks_new (
    id               TEXT    NOT NULL PRIMARY KEY,
    title            TEXT    NOT NULL,
    team_id          TEXT    NOT NULL,
    workflow_id      TEXT    NOT NULL,
    workflow_version INTEGER NOT NULL CHECK (workflow_version >= 0),
    parent           TEXT,
    depends_on       TEXT    NOT NULL CHECK (json_valid(depends_on) AND json_type(depends_on) = 'array'),
    superseded_by    TEXT,
    assignee         TEXT,
    status           TEXT    NOT NULL CHECK (status IN ('open', 'running', 'blocked', 'done', 'superseded', 'failed', 'cancelled')),
    requires_repo    INTEGER NOT NULL CHECK (requires_repo IN (0, 1)),
    merge_commit     TEXT,
    version          INTEGER NOT NULL CHECK (version >= 1),
    pipeline TEXT CHECK (pipeline IS NULL OR json_valid(pipeline)),
    stage_entered_at_unix_ms INTEGER NOT NULL DEFAULT 0 CHECK (stage_entered_at_unix_ms >= 0),
    merge_intent TEXT,
    block_reason TEXT,
    attention_reason TEXT,
    failure_acknowledged INTEGER NOT NULL DEFAULT 0 CHECK (failure_acknowledged IN (0,1))
) STRICT;

INSERT INTO tasks_new (id,title,team_id,workflow_id,workflow_version,parent,depends_on,superseded_by,assignee,status,requires_repo,merge_commit,version)
SELECT id,title,team_id,workflow_id,workflow_version,parent,depends_on,superseded_by,assignee,status,requires_repo,merge_commit,version FROM tasks;
DROP TABLE tasks;
ALTER TABLE tasks_new RENAME TO tasks;
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
INSERT INTO task_events SELECT * FROM saved_events;
DROP TABLE saved_events;
CREATE TABLE teams (
    id TEXT NOT NULL PRIMARY KEY,
    repo TEXT,
    default_workflow TEXT NOT NULL
) STRICT;
INSERT INTO teams VALUES ('general', NULL, 'research');
-- Rebuild instead of ALTER ADD REFERENCES: SQLite refuses a non-NULL
-- REFERENCES default when old instances already exist and foreign_keys is ON.
CREATE TABLE instances_new (
    id TEXT NOT NULL PRIMARY KEY CHECK (length(id) BETWEEN 1 AND 24 AND id NOT GLOB '*[^a-z0-9-]*'),
    backend TEXT NOT NULL CHECK (backend IN ('claude','codex','opencode')),
    program TEXT NOT NULL,
    args TEXT NOT NULL CHECK (json_valid(args) AND json_type(args) = 'array'),
    working_directory TEXT NOT NULL,
    session_id TEXT,
    status TEXT NOT NULL CHECK (status IN ('new','running','failed')),
    session_started INTEGER NOT NULL DEFAULT 0 CHECK (session_started IN (0,1)),
    agent_pid INTEGER CHECK (agent_pid IS NULL OR agent_pid > 0),
    legacy_no_thread INTEGER NOT NULL DEFAULT 0 CHECK (legacy_no_thread IN (0,1)),
    team_id TEXT NOT NULL DEFAULT 'general' REFERENCES teams(id),
    role TEXT NOT NULL DEFAULT '',
    delivery TEXT NOT NULL DEFAULT 'push' CHECK (delivery IN ('push','inbox'))
) STRICT;
INSERT INTO instances_new (id,backend,program,args,working_directory,session_id,status,session_started,agent_pid,legacy_no_thread)
SELECT id,backend,program,args,working_directory,session_id,status,session_started,agent_pid,legacy_no_thread FROM instances;
DROP TABLE instances;
ALTER TABLE instances_new RENAME TO instances;
CREATE TABLE bindings (
    instance_id TEXT NOT NULL PRIMARY KEY REFERENCES instances(id),
    task_id TEXT NOT NULL REFERENCES tasks(id),
    kind TEXT NOT NULL CHECK(kind IN ('work','review')),
    worktree TEXT NOT NULL UNIQUE,
    branch TEXT,
    head TEXT,
    ticket TEXT NOT NULL,
    status TEXT NOT NULL CHECK(status IN ('pending','ready'))
) STRICT;
CREATE TABLE asks (
    id TEXT NOT NULL PRIMARY KEY,
    instance_id TEXT NOT NULL,
    task_id TEXT,
    thread TEXT NOT NULL CHECK(json_valid(thread)),
    created_at_unix_ms INTEGER NOT NULL CHECK(created_at_unix_ms >= 0)
) STRICT;
CREATE TABLE ask_turns (
    seq INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    ask_id TEXT NOT NULL REFERENCES asks(id),
    turn TEXT NOT NULL CHECK(json_valid(turn)),
    delivered INTEGER NOT NULL DEFAULT 0 CHECK(delivered IN (0,1))
) STRICT;
CREATE TABLE reminders (
    seq INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    task_id TEXT NOT NULL REFERENCES tasks(id),
    due_at_unix_ms INTEGER NOT NULL CHECK(due_at_unix_ms >= 0)
) STRICT;
