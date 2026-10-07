# Telegram producer fixture

`get-me.json` is a real Telegram Bot API getMe response captured on 2026-10-07.
Only bot identity values (id, first_name, username) were replaced. Shape and
capability fields are preserved. No token or human chat data is retained.

The capture made one read-only request using the authorized dedicated test bot;
it sent no messages. Capture plan and digest are kept outside Git under
`AgEnD-ops/g12d-telegram-20261007/`.

`message.json` is a real sendMessage receipt from the third bounded text probe.
Bot/chat identity values were replaced, while message shape and authored text
were preserved. All three probe messages were deleted, with successful replies.
Plain text lost surrounding whitespace; visible framing preserved it exactly.
