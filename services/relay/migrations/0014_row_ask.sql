-- The headline's open hook ask, per roster row, so the Live Activity can
-- carry it and the lock screen can answer it while the app is suspended.
--
-- Additive only, same as 0002 through 0013: the previous worker is still
-- serving requests while a deploy rolls out, and it never reads or writes
-- these columns.
--
-- Three facts and no content: `ask_id` is the daemon's opaque `hook-ask-`
-- id, `ask_tool` is claude's tool name ("Bash"), and `ask_until` is when the
-- runner's hold ends, in Unix milliseconds. Never an option name, never a
-- command line.
--
-- Stored on the row, not only passed through, because the card's headline is
-- copied from a stored row whenever another agent's notice moves the card.
-- Written only on a blocked row; cleared when the row leaves blocked, when the
-- runner says the ask ended, and by `readFleet` once `ask_until` has passed.
-- They never outlive their row, which is itself gone after 24 hours.
ALTER TABLE live_activities ADD COLUMN ask_id TEXT;
ALTER TABLE live_activities ADD COLUMN ask_tool TEXT;
ALTER TABLE live_activities ADD COLUMN ask_until INTEGER;
