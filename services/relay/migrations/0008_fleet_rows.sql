-- What the relay remembers about every agent on an account, so the card can
-- draw a row for each of them.
--
-- Additive only, same as 0002 through 0007 and for the same reason: the previous
-- worker is still serving requests while a deploy rolls out, and an App Store
-- build from months ago still calls these routes. Every column here is nullable
-- and a row that predates them behaves exactly as it did.
--
-- **Why the relay stores a fleet at all**, having spent five migrations not
-- doing so. The lock-screen card grew a row per agent — `auth-refactor
-- force-push?  +142 −37  4 commits`, under a header of `2 need you / 3 to
-- review · 3 in flight` — and nothing else in the system can compose that. A
-- fleet spans several runners, each with its own daemon pushing independently,
-- so no daemon sees more than its own runner; the phone's App Group snapshot
-- sees the whole fleet but only while the app has run, which is the case the
-- product is NOT about. This worker sees every runner's notices for one
-- account, and it is the only thing that does.
--
-- **The table is `live_activities`, which is not a new name for a new idea.**
-- It has been keyed `(account_id, terminal)` since 0003 — one row per agent —
-- and 0006 superseded it with `install_cards` for the one-card-per-install
-- rekey and left it in place, unread, its rows to expire on their own. That
-- per-agent row is exactly the shape a roster needs, so it is reused rather
-- than duplicated beside itself under a better name.
--
-- Three of its columns are now inert and are deliberately left alone rather than
-- repurposed, because a column meaning two things over its life is how a
-- migration set stops being readable:
--
--   * `update_token` is `''` on every roster row — `TOKEN_UNKNOWN`, which
--     already means "not an address". The column is NOT NULL and SQLite cannot
--     loosen one in place, so the sentinel is what an additive migration has.
--   * `blind_status` was 0005's answer to a card the relay could not address.
--     `install_cards.leader_status` does that job now, for a card it can.
--   * `dismissed_at` is per-install and lives on `install_cards`. A person
--     swipes a card, never an agent.
--
-- Rows here are purged LAZILY, on write, per account — see `purgeQuiet`. There
-- are no cron triggers in this relay, so there is nowhere else to put it, and
-- the retention window is 24 hours because that is the design's own widest
-- trace window: past it a row cannot contribute to anything the card can draw,
-- so it has nothing left to say.

-- What the row says, in the words the card puts on it.
--
-- All four arrive on every notice already — `label` and `status` explicitly,
-- `machine` off the daemon's own name, `detail` as the composed `subtitle`. The
-- relay forwarded them and kept none; now it keeps exactly these, which is what
-- a row draws and nothing more.
--
-- `detail` is one line, cut on the host to `feed::WIDTH` before it ever leaves
-- the runner. Nothing here re-cuts it and nothing here logs it: this is the
-- narrowest widening of what the relay holds that lets the card exist at all,
-- and it is worth being explicit that it IS a widening. A composed question is
-- content in a way a count is not.
ALTER TABLE live_activities ADD COLUMN label TEXT;
ALTER TABLE live_activities ADD COLUMN machine TEXT;
ALTER TABLE live_activities ADD COLUMN status TEXT;
ALTER TABLE live_activities ADD COLUMN detail TEXT;

-- The numbers to the right of the name: `+142 −37  4 commits`.
--
-- NULL is not zero and the difference is the whole reason these are nullable. A
-- worktree the runner has not probed yet and one with no base to compare
-- against have both said nothing, and a card drawing `+0 −0` over either would
-- be reporting a measurement nobody made. The daemon omits the key, this stays
-- NULL, and the row draws no numbers — see `review::Counts` on the runner,
-- which has had three answers rather than two for exactly this reason.
--
-- `commits` counts what landed inside the trace's own window rather than what
-- is on the branch. `git log base..HEAD` would be a git call per notification
-- on the sampling loop's path; the commit ring is already in memory because the
-- trace's axis marks are drawn from it. See
-- `farcooler_core::trace::Trace::commits`.
ALTER TABLE live_activities ADD COLUMN insertions INTEGER;
ALTER TABLE live_activities ADD COLUMN deletions INTEGER;
ALTER TABLE live_activities ADD COLUMN commits INTEGER;

-- The thirteen buckets under the row, base64 of the wire's 66 bytes.
--
-- One fixed-width column rather than 26 columns or a bucket-per-row table, and
-- the reason is that it is bounded by construction: a trace is always thirteen
-- buckets of two channels plus the axis, so it is always 88 characters, and it
-- is purged with the row it belongs to rather than swept separately.
--
-- Opaque here. Nothing in this service decodes it — the encoding has two ends
-- already, `farcooler_core::trace::Trace::encode` and `AgentKit.ActivityTrace`,
-- and a relay that parsed it would be a third one to keep in step.
ALTER TABLE live_activities ADD COLUMN trace TEXT;

-- When this agent's turn began, in Unix milliseconds, for the row's own clock.
ALTER TABLE live_activities ADD COLUMN started_at INTEGER;

-- When this agent entered the tier it is in, in Unix milliseconds.
--
-- The ordering key, and not the same thing as `updated_at`. Rows sort blocked
-- first, then to-review, then working, and within a tier the LONGEST-WAITING
-- first — so the agent that has been stuck for an hour keeps its line while a
-- busy one that pushed a second ago does not take it. `updated_at` moves on
-- every notice and would sort by who spoke last, which is the opposite.
--
-- Written only when the status actually changes; a `working` agent pushing the
-- same tier every ten seconds leaves it exactly where it was.
ALTER TABLE live_activities ADD COLUMN status_since INTEGER;

-- When an activity push last went out for this install's card.
--
-- The coalescing clock, and its only reader. A fleet card changes whenever ANY
-- agent changes, which is strictly more updates than a leader-only card was —
-- four busy agents pushing every ten seconds is the case `leads()` used to
-- protect against by refusing three of them the card entirely. That protection
-- does not disappear when rows arrive; it moves here. A status change is news
-- and goes at once; `+142 −37` becoming `+147 −37` waits for the next tick.
--
-- Separate from `updated_at`, which says when the ROW last changed and is now
-- read as the age of an unaddressable claim. Two clocks because two questions,
-- each with exactly one reader; one column answering both would have to move on
-- writes the other one needs it not to.
ALTER TABLE install_cards ADD COLUMN pushed_at INTEGER;
