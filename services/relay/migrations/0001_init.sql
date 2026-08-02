-- Accounts, devices, and the machines paired to them.
--
-- No transcripts, no repositories, no terminal output: the relay's whole job is
-- knowing where to deliver a notification, and anything else it stored would be
-- something to leak.

CREATE TABLE accounts (
  -- The WorkOS user id. Identity is not this service's job, so the id it issues
  -- is the one used here rather than a second one to keep in step.
  id TEXT PRIMARY KEY,
  created_at INTEGER NOT NULL,
  -- Nullable on purpose: a provider may not release one, and a notification
  -- does not need it. Present only so a pro tier has something to bill.
  email TEXT
);

CREATE TABLE devices (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  -- 'apns' or 'fcm'. The daemon never learns which: it asks for its user to be
  -- notified and the relay picks the transport.
  platform TEXT NOT NULL,
  push_token TEXT NOT NULL,
  -- Shown when revoking, so a list of devices is readable by a person.
  label TEXT NOT NULL,
  updated_at INTEGER NOT NULL,
  UNIQUE (platform, push_token)
);

CREATE TABLE daemons (
  id TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  -- The SHA-256 of the bearer token, never the token. A relay database that
  -- leaks should not hand anyone the ability to notify its users.
  token_hash TEXT NOT NULL UNIQUE,
  label TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  last_seen_at INTEGER
);

CREATE INDEX devices_by_account ON devices (account_id);
CREATE INDEX daemons_by_account ON daemons (account_id);
