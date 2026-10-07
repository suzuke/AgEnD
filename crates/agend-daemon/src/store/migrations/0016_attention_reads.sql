CREATE TABLE attention_reads (read_key TEXT PRIMARY KEY, read_at_unix_ms INTEGER NOT NULL CHECK(read_at_unix_ms >= 0)) STRICT;
