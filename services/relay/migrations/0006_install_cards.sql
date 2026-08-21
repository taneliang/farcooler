-- One live card per app install, replacing one per terminal.
--
-- Additive only, same as 0002 through 0005 and for the same reason: the previous
-- worker is still serving requests while a deploy rolls out, and an App Store
-- build from months ago still calls these routes. That rule is exactly why this
-- is a NEW table rather than a reshaping of `live_activities`. Three things
-- forbid the in-place version:
--
--   * `live_activities` carries `UNIQUE (account_id, terminal)`, and one card per
--     install needs `UNIQUE (account_id)`. SQLite cannot drop or loosen a
--     constraint in place; the only in-place route is a rebuild, which is the
--     one thing an additive-only migration set does not do.
--   * the rows already there would violate the new uniqueness the moment it was
--     declared — an account with four agents has four of them — so the migration
--     would have to DELETE somebody's in-flight cards to succeed.
--   * and those deletions would land while the OLD worker is still serving. That
--     worker addresses a card through `live_activities.update_token`, so taking
--     its rows away mid-rollout leaves live cards on people's lock screens that
--     nothing can end, which is the precise failure the whole card lifecycle has
--     been built around avoiding.
--
-- So `live_activities` is left exactly as it is and simply stops being read. Its
-- rows expire on their own: every card it can address carries the hour-long
-- `stale-date` its start already gave it, and the app ends every card but one on
-- its next launch anyway — see `LiveActivities.reapDuplicates`. A later migration
-- may drop the table once no deployed worker reads it; dropping it in the same
-- one that stops reading it would be the mid-rollout deletion again, wearing a
-- different hat.

-- The card this install currently has up, and how to reach it.
--
-- One row per ACCOUNT, which is what "per install" costs in a schema that has
-- never had an install identity: the push-to-start token is per install and
-- lives on `devices`, but the update token is reported through a route that
-- authenticates a PERSON, so the relay has no way to tell which of an account's
-- phones filed it. Where somebody has two installs, both get a card started and
-- the relay can address whichever reported last — exactly the behavior
-- `live_activities` had, and the count of rows it has to guess between drops
-- from one per terminal to one.
CREATE TABLE install_cards (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,

  -- The update token for the one running activity, or `TOKEN_UNKNOWN` — the
  -- empty string — for a card the relay started while the app was not running
  -- and therefore cannot address yet. NOT NULL for the same reason it was in
  -- 0003: a row HAS to exist the moment a card is push-started, or the next
  -- push finds none and starts another, and SQLite cannot loosen a column in
  -- place later so the sentinel is the durable choice.
  update_token TEXT NOT NULL,

  -- The environment of the app install that started the activity, read the same
  -- way as devices.environment: NULL is production.
  environment TEXT,

  -- Which agent the card is currently LEADING with, and what it is showing for
  -- it.
  --
  -- New here, and the reason this table exists at all. With a card per terminal
  -- the relay never had to choose: every push was about its own card. With one
  -- card and four agents it has to, on every push, and the choice is the same
  -- precedence the rest of the product uses — blocked outranks working, and a
  -- working agent never bumps a blocked one off the card. That decision needs
  -- two facts remembered between requests, because this worker holds nothing in
  -- memory between them.
  --
  -- `leader_status` also does the job `live_activities.blind_status` did, and
  -- generalizes it: that column said what a card the relay could not address was
  -- showing, and went NULL as soon as the app filed a real token. This one says
  -- what the card is showing whether or not it can be reached, which is a
  -- superset, and it is what keeps a `blocked` escalation to exactly ONE
  -- replacement card.
  --
  -- Both NULL together, and only in one case: the app files an update token for
  -- a card the relay holds no row for, so there is an address but nothing is
  -- known about what is on it. A NULL leader is adopted by the next push that
  -- arrives, which is the honest reading of "there is a card and we do not know
  -- what it says".
  leader_terminal TEXT,
  leader_status TEXT,

  -- When the person swiped the card away, in Unix milliseconds. NULL is "still
  -- up". Carried over unchanged from `live_activities` — see 0005 — and now
  -- scoped to the install, which is the scope the person was actually acting on:
  -- there is one card, and swiping it away is a statement about that card rather
  -- than about one agent.
  dismissed_at INTEGER,

  updated_at INTEGER NOT NULL,

  -- One card per install. A second row would be a second lock-screen card that
  -- the relay believes in and the app has no way to tell it about, which is the
  -- shape every card bug in this service has had.
  UNIQUE (account_id)
);
