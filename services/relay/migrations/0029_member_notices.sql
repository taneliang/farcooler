-- A push to one team member's own devices, sent by a runner (ov-646).
--
-- `/v1/notify/member` writes one row per push it sends: the runner, the member
-- it reached, what the push was about (`kind`, only `login` so far), the
-- daemon's event id and when. It is the route's whole memory:
--
--   - a row for this runner and member newer than six hours means no push;
--   - a row with this event id means no push, so a retried request sends
--     nothing new.
--
-- Nothing in a row is content: the push's words are the relay's fixed
-- template, never stored, and `event` is a hash the daemon made.
--
-- Additive only: a new table the previous worker never reads. Both foreign
-- keys say ON DELETE CASCADE, so removing a runner or an account still goes
-- through on the old worker.
CREATE TABLE member_notices (
  runner_id TEXT NOT NULL REFERENCES runners(id) ON DELETE CASCADE,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  kind TEXT NOT NULL,
  event TEXT NOT NULL,
  sent_at INTEGER NOT NULL,
  PRIMARY KEY (runner_id, account_id, event)
);
CREATE INDEX member_notices_by_sent ON member_notices (runner_id, account_id, sent_at);
