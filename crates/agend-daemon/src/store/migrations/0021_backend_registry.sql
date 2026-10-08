-- At most one durable public registry observation per supported backend.
CREATE TABLE backend_registry (
    backend TEXT NOT NULL PRIMARY KEY CHECK(backend IN ('claude','codex','opencode')),
    record TEXT NOT NULL CHECK(json_valid(record) AND json_type(record) = 'object')
) STRICT;
