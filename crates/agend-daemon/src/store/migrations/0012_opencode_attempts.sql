-- Attempt attribution survives instance removal and normal message retention.
-- Only terminal messages may age out, cascading their attribution with them.
CREATE TABLE opencode_attempts (
    message_id TEXT NOT NULL PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    session_id TEXT NOT NULL,
    backend_message_id TEXT NOT NULL
) STRICT;
INSERT INTO opencode_attempts(message_id,session_id,backend_message_id)
SELECT id,substr(turn_id,1,instr(turn_id,'|')-1),substr(turn_id,instr(turn_id,'|')+1)
FROM messages WHERE attempted_at_unix_ms IS NOT NULL AND turn_id GLOB 'ses*|msg*';
