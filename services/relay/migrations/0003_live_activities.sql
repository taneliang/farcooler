-- Which APNs environment a device belongs to, and the tokens a Live Activity
-- needs that the device token cannot supply.
--
-- Additive only, same as 0002 and for the same reason: the previous worker is
-- still serving requests while a deploy rolls out, and an App Store build from
-- months ago still calls these routes. Every column added here is nullable and
-- changes nothing for a client that never sends it.

-- 'development' or 'production', and NULL means production.
--
-- A locally-signed build carries `aps-environment: development`, so APNs issues
-- it a SANDBOX token, and production APNs answers a sandbox token with
-- BadDeviceToken — push was simply broken for every dev-signed build. NULL
-- rather than a default because every already-registered device was registered
-- by a build that could only have been talking to production, and backfilling
-- them to a literal 'production' would be a behavior change dressed up as a
-- migration. The host choice reads NULL as production, which is what they are.
ALTER TABLE devices ADD COLUMN environment TEXT;

-- The push-to-start token: one per app install, stable for its lifetime, and
-- the only way to CREATE a Live Activity while the app is not running. That is
-- the entire point of the feature — an agent goes blocked while the phone is in
-- a pocket, and nothing on the device is awake to start an activity itself.
--
-- Not the same string as push_token and not interchangeable with it: sending an
-- alert to this one, or a liveactivity push to the device token, is rejected.
ALTER TABLE devices ADD COLUMN live_activity_start_token TEXT;

-- The update token for one running activity.
--
-- Separate from devices because its lifetime is the activity's, not the app
-- install's: APNs issues a fresh one every time an activity starts, it only
-- works for THAT activity, and it is worthless the moment the activity ends.
-- Keyed by terminal because that is what an activity is about — one agent, one
-- lock-screen card — and the app reports the token once the activity is
-- running, which is why this cannot be filled in at registration time.
CREATE TABLE live_activities (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  terminal TEXT NOT NULL,
  update_token TEXT NOT NULL,
  -- The environment of the app install that started the activity, read the same
  -- way as devices.environment: NULL is production.
  environment TEXT,
  updated_at INTEGER NOT NULL,
  -- One activity per agent. A second card for the same terminal would be two
  -- lock-screen entries disagreeing about one agent's state, and the app has no
  -- way to dismiss the one the relay forgot about.
  UNIQUE (account_id, terminal)
);

CREATE INDEX live_activities_by_account ON live_activities (account_id);
