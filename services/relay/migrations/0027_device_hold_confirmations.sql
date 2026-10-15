-- A team admin's word that a device held after a trust reset is its person's
-- (ov-565, ruling R-76).
--
-- Additive only, same as 0002 through 0026: one new table, which the previous
-- worker never reads or writes while a deploy rolls out.
--
-- A device trusted by R-75's reset, and every device it certifies, is left
-- out of the `keys` a team's admins are sent until its hold ends
-- (`TRUST_RESET_HOLD_MS`) or an admin of that team, with its person in front
-- of them, confirms it here. One row per device per team: an admin vouches
-- for their own team, never for another team the person is also in. A
-- confirmation counts only for a hold that began before it, so one made after
-- an earlier reset never covers a later one.
CREATE TABLE device_hold_confirmations (
  device_id TEXT NOT NULL REFERENCES device_keys(id) ON DELETE CASCADE,
  team_id TEXT NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
  confirmed_by TEXT NOT NULL,
  confirmed_at INTEGER NOT NULL,
  PRIMARY KEY (device_id, team_id)
);
CREATE INDEX device_hold_confirmations_by_team ON device_hold_confirmations (team_id);
