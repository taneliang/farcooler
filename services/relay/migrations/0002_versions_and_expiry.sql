-- What version each thing is running, and when a machine's token stops working.
--
-- Additive only, which is the rule for this service: the previous worker is
-- still serving requests while a deploy rolls out, and App Store builds months
-- old still call these routes. Every column here is nullable with no default
-- behaviour change, so a client that never sends one keeps working exactly as
-- it did.

-- The version string the app last reported. Purely so someone can open the
-- devices screen and see at a glance which of their machines is behind —
-- "0.2.0 (beta 3) · 1284" — instead of going to each one to find out.
ALTER TABLE devices ADD COLUMN version TEXT;

-- The same for a paired machine, reported by the daemon when it notifies. A
-- machine that has never notified has never reported, which is correct: the
-- column says what it last ran, not what is installed right now.
ALTER TABLE daemons ADD COLUMN version TEXT;

-- When a daemon's bearer token stops being accepted.
--
-- It used to be never. Combined with a token that has to travel to the machine
-- somehow, "valid forever unless the user happens to notice and revoke it" is a
-- long time for a credential nobody is watching. A year is long enough that
-- re-pairing is rare and short enough that an old token is not a permanent one.
--
-- NULL means no expiry, which is what every token issued before this migration
-- has. Backfilling them would log people out of a feature they just set up; the
-- next pairing gets the new behaviour.
ALTER TABLE daemons ADD COLUMN expires_at INTEGER;
