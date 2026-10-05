-- One startup per instance; reconnecting never clears uncertain key intents.
CREATE TABLE claude_startup (
    instance_id TEXT NOT NULL PRIMARY KEY REFERENCES instances(id) ON DELETE CASCADE,
    session_id TEXT NOT NULL,
    launch_id TEXT NOT NULL,
    generation TEXT,
    halted INTEGER NOT NULL CHECK (halted IN (0, 1)),
    keys TEXT NOT NULL CHECK (json_valid(keys) AND json_type(keys) = 'object')
) STRICT;
