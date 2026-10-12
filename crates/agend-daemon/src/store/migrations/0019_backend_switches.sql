-- Last explicit transition per instance, retained for reconciliation/rollback.
CREATE TABLE backend_switches (
    instance_id TEXT NOT NULL PRIMARY KEY REFERENCES instances(id) ON DELETE CASCADE,
    id TEXT NOT NULL UNIQUE,
    record TEXT NOT NULL CHECK(json_valid(record) AND json_type(record) = 'object')
) STRICT;
