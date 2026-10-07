-- A permission answer is an operator decision, separate from message delivery.
-- Commit attempted_at before HTTP; unknown outcomes are never auto-replayed.
CREATE TABLE opencode_permissions (
    id TEXT NOT NULL PRIMARY KEY,
    instance_id TEXT NOT NULL REFERENCES instances(id) ON DELETE CASCADE,
    session_id TEXT NOT NULL,
    request_id TEXT NOT NULL,
    native TEXT NOT NULL CHECK(json_valid(native)),
    decision TEXT CHECK(decision IN ('once','reject')),
    attempted_at_unix_ms INTEGER CHECK(attempted_at_unix_ms >= 0),
    status TEXT NOT NULL CHECK(status IN ('pending','unknown','resolved','stale')),
    created_at_unix_ms INTEGER NOT NULL CHECK(created_at_unix_ms >= 0),
    CHECK(attempted_at_unix_ms IS NULL OR decision IS NOT NULL)
) STRICT;
CREATE INDEX opencode_permissions_instance ON opencode_permissions(instance_id,session_id,status);
