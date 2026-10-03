-- A thread that admitted operator input must never use text receipt matching again.
-- No instance foreign key: the thread can be resumed under another instance.
CREATE TABLE codex_input_threads (
    thread_id TEXT PRIMARY KEY NOT NULL CHECK (length(thread_id) > 0)
) STRICT;
