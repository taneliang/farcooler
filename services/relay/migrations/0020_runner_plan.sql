-- The plan's glance each runner last sent (ov-310): per board with a plan,
-- its name, its Needs You count, up to two Now lanes by name and state, and
-- the lane next up. JSON as `planGlanceOf` cleaned it; never card text.
--
-- Additive only, same as 0002 through 0019: the previous worker is still
-- serving requests while a deploy rolls out, and it never reads or writes
-- these columns. A runner older than them never sends `plan`, and NULL is
-- what such a runner has: no card and no pulse carries a plan for it.
--
-- On `daemons`, beside `needs_you`, because it is the runner's own word, and
-- overwritten, never merged, by the count notice that carries it. `plan_at`
-- is the relay's clock when it was stored: the newest one leads the glance,
-- and one older than a day is read as none.
ALTER TABLE daemons ADD COLUMN plan TEXT;
ALTER TABLE daemons ADD COLUMN plan_at INTEGER;
