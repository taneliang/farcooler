CREATE TABLE web_push (
  endpoint TEXT PRIMARY KEY,
  account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE
);
CREATE INDEX web_push_account ON web_push(account_id);
