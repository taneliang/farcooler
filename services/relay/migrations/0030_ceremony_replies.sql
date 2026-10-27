-- A sealed enrollment-ceremony reply, waiting for the device it is for (ov-769).
--
-- `POST /v1/ceremony/reply` writes one row per ceremony and
-- `GET /v1/ceremony/reply/<ceremony>` deletes it as it reads it
-- (`src/ceremony-reply.ts`). A row nobody read is gone after 120 seconds, by
-- the read's own check, and the five-minute cron deletes it.
--
-- `sealed` is HPKE ciphertext the relay has no key for: never a runner, a host
-- key or a token in the clear. `account_id` is the offer's account, the only
-- one that may read the row; `posted_by` is who posted it.
--
-- Additive only: a new table the previous worker never reads. Both foreign
-- keys say ON DELETE CASCADE, so removing an account still goes through on the
-- old worker.
CREATE TABLE ceremony_replies (
  ceremony TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  posted_by TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  sealed TEXT NOT NULL,
  created_at INTEGER NOT NULL
);
CREATE INDEX ceremony_replies_by_created ON ceremony_replies (created_at);
