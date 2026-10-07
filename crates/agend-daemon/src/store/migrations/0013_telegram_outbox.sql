-- A claimed part has an unknown remote outcome until its exact receipt commits.
-- Do not age unknown attempts out into a fresh send.
CREATE TABLE telegram_outbox (
    seq INTEGER PRIMARY KEY AUTOINCREMENT,
    id TEXT NOT NULL UNIQUE,
    delivery TEXT NOT NULL CHECK(json_valid(delivery))
) STRICT;
