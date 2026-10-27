-- A sealed enrollment-ceremony reply, waiting for the device it is for (ov-769).
--
-- `POST /v1/ceremony/reply` writes one row per ceremony and poster, so one
-- poster can't take the slot from another, and at most four for one ceremony
-- and account. `GET /v1/ceremony/reply/<ceremony>` deletes every row for that
-- ceremony and its own account as it reads them (`src/ceremony-reply.ts`). A
-- row nobody read is gone after 120 seconds, by the read's own check, and the
-- five-minute cron deletes it.
--
-- `sealed` is HPKE ciphertext the relay has no key for: never a runner, a host
-- key or a token in the clear. `account_id` is the offer's account, the only
-- one that may read the row; `posted_by` is who posted it.
--
-- Additive only: a new table the previous worker never reads. Both foreign
-- keys say ON DELETE CASCADE, so removing an account still goes through on the
-- old worker.
CREATE TABLE ceremony_replies (
  ceremony TEXT NOT NULL,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  posted_by TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  sealed TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  PRIMARY KEY (ceremony, posted_by)
);
CREATE INDEX ceremony_replies_by_reader ON ceremony_replies (ceremony, account_id);
CREATE INDEX ceremony_replies_by_created ON ceremony_replies (created_at);
