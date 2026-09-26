-- 0003_session_started (gate 8 P5): whether the backend session of an
-- instance was ever created, so `retry` of a `failed` instance knows whether
-- to resume it. Never edit this file once it has shipped.

-- 1 once the first `Spawn` was acknowledged (written with `running` in the
-- same statement); it never goes back to 0.
ALTER TABLE instances ADD COLUMN session_started INTEGER NOT NULL DEFAULT 0
    CHECK (session_started IN (0, 1));

-- Existing rows: `running` means the first `Spawn` was acknowledged. A
-- `failed` codex/opencode instance almost surely ran (gate 6 H2: it is
-- `running` right after its first `Spawn`, `failed` at its next death), so
-- it is never started fresh again; a `failed` claude instance stays 0,
-- which is safe (a `--session-id` start of an existing session is refused,
-- recorded `running`, and the next restart resumes it).
UPDATE instances SET session_started = 1
WHERE status = 'running' OR (status = 'failed' AND backend <> 'claude');
