-- A runner's heartbeat, and the credential a phone's widget reads it with.
--
-- Additive only, same as 0002 through 0014: the previous worker is still
-- serving requests while a deploy rolls out, and it never reads or writes
-- these columns.
--
-- **`daemons.beat_every`** is how often, in seconds, this token's runner has
-- promised to beat (`/v1/heartbeat`). NULL is a runner too old to beat, and it
-- is what keeps such a runner out of `/v1/pulse`: its silence says nothing,
-- because it was never going to speak unprompted. The beat itself lands on
-- `last_seen_at`, the column 0001 already has and `/v1/notify` already stamps.
--
-- A runner unpaired on purpose (Stop Notifying, `push forget`) sends one last
-- beat marked withdrawn, which sets `beat_every` back to NULL: it leaves the
-- pulse rather than reading as lost.
--
-- **`daemons.name`** is what the runner calls itself (the computer's name on
-- a Mac, its short hostname elsewhere), carried on every beat. The phone
-- names a quiet runner by it before the pairing label, which is "This Mac"
-- for every Mac's own runner and names nothing on a phone.
--
-- **`devices.pulse_hash`** is the SHA-256 of the device's pulse token, which a
-- widget posts to `/v1/pulse`. Never the token. The PHONE makes the token,
-- once, and sends it on every registration; the relay stores its hash, so
-- registrations are idempotent in any order. A new token replaces the old,
-- a device that changes accounts loses it, and it goes with the row when the
-- device is revoked. It reads the account's runner names and how long ago
-- each beat, and nothing else.
ALTER TABLE daemons ADD COLUMN beat_every INTEGER;
ALTER TABLE daemons ADD COLUMN name TEXT;
ALTER TABLE devices ADD COLUMN pulse_hash TEXT;
CREATE UNIQUE INDEX devices_pulse ON devices (pulse_hash) WHERE pulse_hash IS NOT NULL;
