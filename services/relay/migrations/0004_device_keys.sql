-- Which key a device holds, and whether a ceremony ever said so.
--
-- Additive only, same as 0002 and 0003 and for the same reason: the previous
-- worker is still serving requests while a deploy rolls out, and an App Store
-- build from months ago still calls /v1/devices. Nothing here changes what a
-- client that sends none of it gets.

-- The relay stores a fingerprint, never a key.
--
-- Its only use for a device key is the account lookup, and a hash answers that:
-- an ed25519 public key is 32 random bytes, so `SHA256:t7Xq...9Vd` is matchable
-- and not reversible. Storing the key would make "never install a key the relay
-- handed you" a rule to remember; storing a fingerprint makes it a fact about
-- the schema, because there is no key here to hand anyone. It is the same
-- string `ssh-keygen -lf` prints, which is the string a person reads off a
-- screen at the confirmation.
ALTER TABLE devices ADD COLUMN key_a_fingerprint TEXT;

-- 'pending' or 'verified'. A row is created `pending` by the NEW device, which
-- is the only party that can prove possession of the key, and promoted by a
-- trusted device once a ceremony has actually enrolled it.
--
-- Existing rows default to 'verified' and carry a NULL fingerprint. They predate
-- this and were created by a flow that had no pending state, and the lookup
-- matches on the fingerprint, so a default of 'verified' grants them nothing
-- they did not already have: they are invisible to every ceremony until the app
-- on that device re-registers with a key.
ALTER TABLE devices ADD COLUMN state TEXT NOT NULL DEFAULT 'verified';

-- One row per key per account.
--
-- Partial, because NULL is the normal state for every device registered before
-- this migration and for every client that never sends a key -- and SQLite
-- would happily allow many NULLs in a plain unique index, but saying so in the
-- predicate is what makes the intent survive someone reading it later. Scoped
-- to the account rather than global: two accounts registering one public key is
-- a case the lookup is built to survive, not one to refuse at write time.
CREATE UNIQUE INDEX devices_account_fingerprint
  ON devices (account_id, key_a_fingerprint)
  WHERE key_a_fingerprint IS NOT NULL;
