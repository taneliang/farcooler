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
-- **`devices.pulse_hash`** is the SHA-256 of the device's pulse token, which a
-- widget posts to `/v1/pulse`. Never the token. Minted by `/v1/devices` when
-- the registration asks, replaced by every registration that asks again, and
-- gone with the row when the device is revoked. It reads the account's runner
-- labels and how long ago each beat, and nothing else.
ALTER TABLE daemons ADD COLUMN beat_every INTEGER;
ALTER TABLE devices ADD COLUMN pulse_hash TEXT;
CREATE UNIQUE INDEX devices_pulse ON devices (pulse_hash) WHERE pulse_hash IS NOT NULL;
