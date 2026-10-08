-- One observation per instance; explicit removal discards the receipt.
CREATE TABLE system_versions (
    instance_id TEXT NOT NULL PRIMARY KEY REFERENCES instances(id) ON DELETE CASCADE,
    record TEXT NOT NULL CHECK(json_valid(record) AND json_type(record) = 'object')
) STRICT;
