-- The keys on a runner, as the runner last reported them (ov-510; design
-- ov-480 §3, §7 step 2).
--
-- A runner reads its whole `authorized_keys`, inside Far Cooler's fence and
-- out, and says on each heartbeat what it holds: a hash always, and the list
-- itself only when the hash changed or this row doesn't match it. So there is
-- one row per runner, replaced whole.
--
-- Fingerprints, comments, options and scopes only; never a key's bytes, and
-- never the text of a line that isn't a key. `entries` is a JSON array of
-- `{fingerprint, comment, inside, scope, file, options, firstSeenAt}`; the
-- relay stamps `firstSeenAt` itself, from the previous report, which is how a
-- key that appeared after the first report is told from one that was always
-- there. `baseline_at` (and `baseline_unparsed`, the unreadable lines then) is
-- that first report's; it never moves. Both are NULL until a list has been
-- read: a runner whose first report is an error has no baseline yet.
--
-- `error_file` is the file the error is about. A report with an error about
-- `authorized_keys` leaves the held list and its history alone (what it lists
-- is incomplete or mislabeled), unless there is none yet.
--
-- `total` is how many keys the runner read, which may be more than `entries`
-- holds. `unparsed` counts lines no key could be read from. `error` is why
-- the runner could not read its files completely (`damaged`, `refused`,
-- `too_large`, `unreadable`), with when; a clean report clears it.
-- `reported_at` is the last time keys were read.
--
-- What each key IS to the directory (known, unknown, orphaned, planted) is not
-- stored: it is worked out when read, against the devices and roles of that
-- moment, so a key's label follows the directory without a rewrite.
--
-- Additive only, as 0002 through 0021: a new table, cascading with its runner.
CREATE TABLE runner_inventory (
  runner_id TEXT PRIMARY KEY REFERENCES runners(id) ON DELETE CASCADE,
  hash TEXT NOT NULL,
  unparsed INTEGER NOT NULL DEFAULT 0,
  baseline_unparsed INTEGER NOT NULL DEFAULT 0,
  total INTEGER NOT NULL DEFAULT 0,
  entries TEXT NOT NULL DEFAULT '[]',
  error TEXT,
  error_file TEXT,
  error_at INTEGER,
  baseline_at INTEGER,
  reported_at INTEGER
);
