/// Keep `sessions_seen` from growing for ever (ov-568).

import type { Env } from './index'

/// How long a session id can still come back. WorkOS sets a session's maximum
/// length and inactivity timeout per environment in its dashboard and doesn't
/// publish a ceiling, so this is a generous documented guess: a row older than
/// this belongs to a session that has ended, and a missing row only matters for
/// a session that is still alive. If an environment is ever set to outlast it,
/// raise this first.
export const SESSION_MAX_LIFETIME_MS = 400 * 24 * 60 * 60 * 1000

/// Delete the rows no check can still need. A row is needed while its session
/// can come back (younger than `SESSION_MAX_LIFETIME_MS`), and also while the
/// account's cutoff is at or after it: `requireSession` refuses a `sid` first
/// seen at or before `sessions_revoked_at`, and with the row gone the `sid`
/// would read as first seen now. A session old enough to prune is dead at
/// WorkOS, so for the cutoff case the row is kept only while the cutoff itself
/// is inside the lifetime; past that nothing it covers is alive.
export async function pruneSessionsSeen(env: Env, now = Date.now()): Promise<void> {
  const horizon = now - SESSION_MAX_LIFETIME_MS
  await env.DB.prepare(
    `DELETE FROM sessions_seen
     WHERE first_seen < ?1
       AND NOT EXISTS (
         SELECT 1 FROM accounts a
         WHERE a.id = sessions_seen.account_id
           AND a.sessions_revoked_at IS NOT NULL
           AND a.sessions_revoked_at >= ?1
           AND sessions_seen.first_seen <= a.sessions_revoked_at
       )`,
  )
    .bind(horizon)
    .run()
}
