-- Reserved before native spawn; reconnect must prove the original holder UUID.
-- Explicit instance removal discards its receipt so ID reuse cannot adopt it.
CREATE TABLE managed_launches (
    instance_id TEXT NOT NULL PRIMARY KEY REFERENCES instances(id) ON DELETE CASCADE,
    binding TEXT NOT NULL UNIQUE,
    intent TEXT NOT NULL CHECK(json_valid(intent) AND json_type(intent) = 'object')
) STRICT;
