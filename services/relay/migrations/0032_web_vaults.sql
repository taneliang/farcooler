-- A browser's sealed copy of what it keeps about itself, one per passkey
-- (ov-779), so clearing the site's storage costs a touch, not an enrollment.
--
-- `sealed` is AES-GCM ciphertext under a key derived from the passkey's PRF
-- output, which this relay never sees: never a runner, a host key or a token
-- in the clear. `credential` is the passkey's credential id (base64url),
-- public: the page names it in `allowCredentials`. `uv` is whether the
-- assertion that sealed it was user-verified, because an authenticator's PRF
-- can differ with and without verification. `src/web-vault.ts` reads and
-- writes it; only `account_id` itself ever does.
--
-- Additive only: a new table the previous worker never reads. ON DELETE
-- CASCADE, so removing an account still goes through on the old worker.
CREATE TABLE web_vaults (
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  credential TEXT NOT NULL,
  uv INTEGER NOT NULL,
  sealed TEXT NOT NULL,
  updated_at INTEGER NOT NULL,
  PRIMARY KEY (account_id, credential)
);
