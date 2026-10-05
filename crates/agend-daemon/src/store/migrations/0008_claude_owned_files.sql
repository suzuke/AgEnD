-- D40 P2: ownership is retained when an instance is removed; its workspace stays.
CREATE TABLE claude_owned_files (
    path TEXT NOT NULL PRIMARY KEY,
    instance_id TEXT NOT NULL,
    sha256 TEXT NOT NULL CHECK (length(sha256) = 64)
) STRICT;
