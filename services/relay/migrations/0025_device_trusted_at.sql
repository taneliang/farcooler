-- When a device was trusted on first use (ov-561, ruling R-75).
--
-- Additive only, same as 0002 through 0024: two nullable columns, which the
-- previous worker never reads and leaves NULL on the rows it writes while a
-- deploy rolls out.
--
-- An account's first device is trusted on first use, and so, since R-75, is
-- the next device to register once the account has no trusted or certified
-- device left: a new key, or one of its pending devices signing in again. A
-- stolen session can cause that by removing every confirmed device first, so
-- the moment is recorded here and `directory/me` returns it, for each app's
-- banner to announce. No route clears it.
ALTER TABLE device_keys ADD COLUMN trusted_at INTEGER;
-- 1 when that trust was R-75's reset rather than an account's first device:
-- the apps announce a reset, a new device's included, as a device confirmed
-- with none of the others confirmed, never as "the first device".
ALTER TABLE device_keys ADD COLUMN trust_reset INTEGER;
