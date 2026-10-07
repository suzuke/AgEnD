-- REST can backfill the same terminal message long after event retention.
CREATE TABLE opencode_observed (
    instance_id TEXT NOT NULL REFERENCES instances(id) ON DELETE CASCADE,
    session_id TEXT NOT NULL,
    message_id TEXT NOT NULL,
    PRIMARY KEY(instance_id,session_id,message_id)
) STRICT;
