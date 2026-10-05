PRAGMA foreign_keys = OFF;
BEGIN TRANSACTION;
CREATE TABLE ask_turns (
    seq INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    ask_id TEXT NOT NULL REFERENCES asks(id),
    turn TEXT NOT NULL CHECK(json_valid(turn)),
    delivered INTEGER NOT NULL DEFAULT 0 CHECK(delivered IN (0,1))
) STRICT;
CREATE TABLE asks (
    id TEXT NOT NULL PRIMARY KEY,
    instance_id TEXT NOT NULL,
    task_id TEXT,
    thread TEXT NOT NULL CHECK(json_valid(thread)),
    created_at_unix_ms INTEGER NOT NULL CHECK(created_at_unix_ms >= 0)
) STRICT;
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
INSERT INTO "claude_deliveries" VALUES('m-fixture','fixture-1',NULL,NULL,NULL,NULL,NULL,NULL,NULL,NULL);
INSERT INTO "claude_deliveries" VALUES('m-claude-fixture','fixture-1','22222222-2222-4222-8222-222222222222','11111111-1111-4111-8111-111111111111','stop',1,NULL,NULL,NULL,NULL);
CREATE TABLE codex_input_threads (
    thread_id TEXT PRIMARY KEY NOT NULL CHECK (length(thread_id) > 0)
) STRICT;
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
INSERT INTO "driver_events" VALUES(1,'22222222-2222-4222-8222-222222222222','fixture-1','11111111-1111-4111-8111-111111111111','Stop','{}',1,2,1);
CREATE TABLE "instances" (
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
INSERT INTO "instances" VALUES('fixture-1','claude','/bin/bash','["-c","exit 0"]','/tmp','s-fixture','running',1,NULL,0,'general','','push');
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
INSERT INTO "messages" VALUES(1,'m-fixture','operator','fixture-1','T-fixture','hello','queue','confirmed','t-fixture',NULL,1790000000000,1790000000001);
INSERT INTO "messages" VALUES(2,'m-claude-fixture','operator','fixture-1',NULL,'fixture push','queue','queued',NULL,1,1,1);
CREATE TABLE reminders (
    seq INTEGER NOT NULL PRIMARY KEY AUTOINCREMENT,
    task_id TEXT NOT NULL REFERENCES tasks(id),
    due_at_unix_ms INTEGER NOT NULL CHECK(due_at_unix_ms >= 0)
) STRICT;
CREATE TABLE task_events (
    seq                 INTEGER NOT NULL PRIMARY KEY,
    task_id             TEXT    NOT NULL REFERENCES tasks (id),
    event_id            TEXT    NOT NULL,
    occurred_at_unix_ms INTEGER NOT NULL CHECK (occurred_at_unix_ms >= 0),
    kind                TEXT    NOT NULL,
    detail              TEXT    NOT NULL
) STRICT;
INSERT INTO "task_events" VALUES(1,'T-fixture','e-1',1790000000000,'created','');
INSERT INTO "task_events" VALUES(2,'T-fixture','e-2',1790000000001,'stage_completed','work');
CREATE TABLE "tasks" (
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
INSERT INTO "tasks" VALUES('T-fixture','fixture task','general','code',1,'T-parent','["T-a","T-b"]',NULL,'dev-1','blocked',1,NULL,3,NULL,0,NULL,NULL,NULL,0);
CREATE TABLE teams (
    id TEXT NOT NULL PRIMARY KEY,
    repo TEXT,
    default_workflow TEXT NOT NULL
) STRICT;
INSERT INTO "teams" VALUES('general',NULL,'research');
CREATE TABLE workflows (
    id      TEXT    NOT NULL,
    version INTEGER NOT NULL CHECK (version >= 0),
    toml    TEXT    NOT NULL,
    PRIMARY KEY (id, version)
) STRICT;
INSERT INTO "workflows" VALUES('code',1,'id = "code"
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
CREATE INDEX messages_by_target ON messages (to_instance, seq);
CREATE INDEX messages_by_time ON messages (created_at_unix_ms);
CREATE INDEX task_events_by_task ON task_events (task_id, seq);
CREATE INDEX task_events_by_time ON task_events (occurred_at_unix_ms);
CREATE INDEX driver_events_by_time ON driver_events (ingested_at_unix_ms);
CREATE TRIGGER capture_claude_message AFTER INSERT ON messages
WHEN EXISTS (SELECT 1 FROM instances WHERE id = NEW.to_instance AND backend = 'claude' AND delivery = 'push')
BEGIN
    INSERT INTO claude_deliveries(message_id, instance_id) VALUES (NEW.id, NEW.to_instance);
END;
DELETE FROM "sqlite_sequence";
INSERT INTO "sqlite_sequence" VALUES('messages',2);
INSERT INTO "sqlite_sequence" VALUES('driver_events',1);
-- D40 P2: ownership is retained when an instance is removed; its workspace stays.
CREATE TABLE claude_owned_files (
    path TEXT NOT NULL PRIMARY KEY,
    instance_id TEXT NOT NULL,
    sha256 TEXT NOT NULL CHECK (length(sha256) = 64)
) STRICT;

INSERT INTO claude_owned_files VALUES('/fixture/CLAUDE.md', 'fixture-1', 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa');
-- One startup per instance; reconnecting never clears uncertain key intents.
CREATE TABLE claude_startup (
    instance_id TEXT NOT NULL PRIMARY KEY REFERENCES instances(id) ON DELETE CASCADE,
    session_id TEXT NOT NULL,
    launch_id TEXT NOT NULL,
    generation TEXT,
    halted INTEGER NOT NULL CHECK (halted IN (0, 1)),
    manual INTEGER NOT NULL DEFAULT 0 CHECK (manual IN (0, 1)),
    keys TEXT NOT NULL CHECK (json_valid(keys) AND json_type(keys) = 'object')
) STRICT;

COMMIT;
PRAGMA user_version = 9;
PRAGMA foreign_keys = ON;
