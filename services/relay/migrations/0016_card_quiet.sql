-- The runners a Live Activity last named as quiet, and which runner a beat
-- is from (ov-71).
--
-- Additive only, same as 0002 through 0015: the previous worker is still
-- serving requests while a deploy rolls out, and it never reads or writes
-- this column.
--
-- **`install_cards.quiet`** is the JSON list of runner names the card was last
-- pushed with under `quiet`: the runners on the card that stopped beating
-- (`/v1/heartbeat`, migration 0015). The cron sweep (`sweepQuiet`) compares
-- what the fleet says now with this, and pushes the card only when the two
-- differ, so a runner that went quiet is said once and a runner that beats
-- again is taken back once. NULL is a card no push has recorded one for,
-- which reads as the empty list.
ALTER TABLE install_cards ADD COLUMN quiet TEXT;

-- **`daemons.runner_key`** is `sha256("runner:" + account + ":" + runner id)`,
-- the runner id being the one a phone reads as `Host.runner_id`, sent on
-- every beat. Never the id itself, for `install_id`'s reason. `/v1/pulse`
-- hands it back, and a watch hashes the ids its phone learned the same way to
-- tell which agents belong to a runner that went quiet. NULL from a runner
-- too old to send one.
ALTER TABLE daemons ADD COLUMN runner_key TEXT;
