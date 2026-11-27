-- A one-use ticket a signed-in browser makes for `farcooler join` on a runner
-- (ov-1490, `src/join.ts`).
--
-- The ticket lets one command post its sealed reply for the ceremony it was
-- made for. The relay answers that reply with a claim, and the claim adds the
-- runner to the account once the person has approved it in the browser. Only
-- hashes of both are kept. A row is gone when its runner is added or when it
-- expires.
--
-- Additive only: a new table the previous worker never reads. The foreign key
-- says ON DELETE CASCADE, so removing an account still goes through on the old
-- worker.
CREATE TABLE join_tickets (
  ticket_hash TEXT PRIMARY KEY NOT NULL,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  team_id TEXT NOT NULL,
  ceremony TEXT NOT NULL,
  claim_hash TEXT,
  approved_at INTEGER,
  created_at INTEGER NOT NULL,
  expires_at INTEGER NOT NULL
);
CREATE INDEX join_tickets_by_account ON join_tickets (account_id, ceremony);
CREATE INDEX join_tickets_by_claim ON join_tickets (claim_hash);
CREATE INDEX join_tickets_by_expiry ON join_tickets (expires_at);
