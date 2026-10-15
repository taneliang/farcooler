-- When an account last ended every one of its sessions (ov-564).
--
-- Additive only, same as 0002 through 0025: one new nullable column and one new
-- table, which the previous worker never reads or writes while a deploy rolls
-- out.
--
-- "Sign out everywhere" sets this to the relay's clock. A WorkOS access token
-- is a signed JWT the relay checks without calling WorkOS, so revoking the
-- session upstream does not stop a copy of its access token until it expires.
-- This column does: a token whose `iat` is at or before this time is refused,
-- on every route that needs a session. NULL is an account that never did.
ALTER TABLE accounts ADD COLUMN sessions_revoked_at INTEGER;

-- The first time the relay saw each WorkOS session id (`sid`).
--
-- A refresh keeps the session's `sid` and mints a fresh `iat`, so the stamp
-- above can't stop a refresh token. This can: a token whose `sid` was first
-- seen at or before the stamp belongs to a session that existed when the
-- person signed out everywhere, whatever its `iat` says. `requireSession`
-- records each `sid` on its first sight, and sign-out records every session
-- WorkOS lists, so one the relay had never heard from is covered too.
CREATE TABLE sessions_seen (
  sid TEXT NOT NULL,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  first_seen INTEGER NOT NULL,
  PRIMARY KEY (account_id, sid)
);
