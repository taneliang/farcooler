/// How each finished agent's last turn ended, for `/v1/pulse` (ov-239).
///
/// The watch hears from the relay over the pulse route and no other, so a
/// failure the runner has since resolved can only clear there if the answer
/// carries the outcome. One entry per `done` row on the account whose runner
/// said (`failed` set), newest first: an opaque terminal id, whether the turn
/// failed, and when the relay filed it (`at`, epoch ms, the clock the card's
/// `updatedAt` uses). It names no runner, label or path.
///
/// Additive: a client that predates it reads `runners` and drops the rest, and
/// one that finds none draws what it had. A row that went back to work has
/// `failed` NULL (migration 0018), so it is absent, and a row whose runner
/// said nothing is absent: no word is not "finished well".

/// The most outcomes one pulse answer carries: a fleet far larger than anyone
/// runs, so the answer stays small however long rows linger.
export const PULSE_TURNS_SHOWN = 100

export interface PulseTurn {
  terminal: string
  failed: boolean
  at: number
}

export async function pulseTurns(db: D1Database, account: string): Promise<PulseTurn[]> {
  const finished = await db
    .prepare(
      `SELECT terminal, failed, updated_at FROM live_activities
       WHERE account_id = ? AND status = 'done' AND failed IS NOT NULL
       ORDER BY updated_at DESC LIMIT ?`,
    )
    .bind(account, PULSE_TURNS_SHOWN)
    .all<{ terminal: string; failed: number; updated_at: number }>()
  return (finished.results ?? []).map(row => ({
    terminal: row.terminal,
    failed: row.failed === 1,
    at: row.updated_at,
  }))
}
