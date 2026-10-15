-- Old-way tokens a runner's new token is to retire (ov-529).
--
-- Additive only, same as 0002 through 0022: the previous worker is still
-- serving requests while a deploy rolls out, and it never writes this column.
--
-- A runner paired the old way (`/v1/daemons`) keeps that token for a year, and
-- the account lists the runner twice. Registering, rotating or claiming a
-- runner issues a new token, but the old ones are not revoked then: the new
-- token exists only in the Mac app until it is written onto the box, and a
-- failed write must not leave the runner silent. So the register or claim
-- leaves a note here, and the new token's first heartbeat (which proves it was
-- installed) acts on it and clears it.
--
-- `old_token_accounts` is a JSON array of `{account, team}`: whose old-way
-- tokens for this runner's install to revoke, and in which team's log to say
-- so. NULL is nothing pending.
ALTER TABLE runners ADD COLUMN old_token_accounts TEXT;
CREATE INDEX IF NOT EXISTS runners_by_daemon ON runners (daemon_id);
