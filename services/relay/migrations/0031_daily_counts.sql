-- Anonymous daily totals (ov-504): one number per day and metric, no account,
-- device or runner id anywhere. `label` is a fixed word from the code (today
-- only ''), never data from a caller.
--
-- Additive only: a new table the previous worker never reads. No foreign key,
-- so nothing about an account can ever be reached from it.
CREATE TABLE daily_counts (
  day TEXT NOT NULL,
  metric TEXT NOT NULL,
  label TEXT NOT NULL DEFAULT '',
  value INTEGER NOT NULL,
  PRIMARY KEY (day, metric, label)
);
