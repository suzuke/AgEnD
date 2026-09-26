-- 0002_instances (gate 6 P2): the instances the daemon keeps running. Never
-- edit this file once it has shipped.

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
