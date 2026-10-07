-- Durable remote ownership must outlive transient failures and restarts.
CREATE TABLE github_changes (
    task_id TEXT NOT NULL PRIMARY KEY REFERENCES tasks(id),
    repository_id INTEGER NOT NULL CHECK(repository_id > 0),
    branch TEXT NOT NULL,
    revision INTEGER NOT NULL CHECK(revision >= 1),
    identity TEXT NOT NULL CHECK(json_valid(identity)),
    change TEXT NOT NULL CHECK(json_valid(change)),
    UNIQUE(repository_id, branch)
) STRICT;
