-- One bounded operator pairing record; contains references, never token values.
CREATE TABLE telegram_pairing (
    slot INTEGER NOT NULL PRIMARY KEY CHECK(slot = 1),
    id TEXT NOT NULL UNIQUE,
    record TEXT NOT NULL CHECK(json_valid(record))
) STRICT;
