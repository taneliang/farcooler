-- The challenges a device signs to register its keys (ov-549).
--
-- Additive only, same as 0002 through 0023: one new table, which the previous
-- worker never reads or writes while a deploy rolls out.
--
-- A device proves it holds Key A by signing a statement that names this
-- account, its keys and one of these: 32 random bytes, as hex, that the relay
-- chose. Each is spent by the registration that uses it, and dies after a few
-- minutes unspent, so a captured proof registers nothing a second time.
--
-- Not secret: the device is told it in the clear. What matters is that it is
-- the relay's choice, used once, and soon.
CREATE TABLE device_challenges (
  nonce TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  created_at INTEGER NOT NULL,
  expires_at INTEGER NOT NULL
);
CREATE INDEX device_challenges_by_account ON device_challenges (account_id, created_at);
-- Every mint drops the expired rows first, which without this is a full scan.
CREATE INDEX device_challenges_by_expiry ON device_challenges (expires_at);
