/// Daily active runners, kept as one number per UTC day (ov-504).
///
/// This relay serves one channel, so its database is the channel and the table
/// needs no channel column. The count is read from `daemons.last_seen_at`, which
/// the relay already keeps, and only the total is written: no id, no hash, no
/// salt. Each five-minute run rewrites today's row with the number of distinct
/// runners seen since midnight UTC, so the row only grows through the day, a
/// retried or doubled run writes the same value, and the day's last run leaves
/// its final count.
///
/// Read with: wrangler d1 execute farcooler --remote --command
///   "SELECT day, value FROM daily_counts WHERE metric = 'runner.active' ORDER BY day DESC"

import type { Env } from './index'

export const RUNNER_ACTIVE = 'runner.active'
const DAY_MS = 24 * 60 * 60 * 1000

export async function recordDailyActiveRunners(env: Env, now = Date.now()): Promise<void> {
  const midnight = now - (now % DAY_MS)
  const day = new Date(midnight).toISOString().slice(0, 10)
  // One runner can hold several tokens (re-pairs); `install_id` names the
  // runner when the daemon sent one, else the token row stands for itself.
  // The write is skipped when the value is unchanged, so most runs cost a read.
  await env.DB.prepare(
    `INSERT INTO daily_counts (day, metric, label, value)
     SELECT ?1, ?2, '', COUNT(DISTINCT COALESCE(install_id, id)) FROM daemons WHERE last_seen_at >= ?3
     ON CONFLICT (day, metric, label) DO UPDATE SET value = excluded.value WHERE value != excluded.value`,
  )
    .bind(day, RUNNER_ACTIVE, midnight)
    .run()
}
