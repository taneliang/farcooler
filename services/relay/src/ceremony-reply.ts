/// The enrollment ceremony's reply, carried sealed for a device that can't
/// scan it (ov-769).
///
/// A browser has no camera to scan the granting phone's reply with, so the
/// phone seals the reply to a key the browser's offer named and posts it here,
/// and the browser takes it. The seal is HPKE, done in Rust on both ends
/// (`crates/ceremony/src/seal.rs`). This relay holds ciphertext and who posted
/// it; never a runner, a host key or a token, because it never holds the key.
///
/// Two routes:
///
/// - `POST /v1/ceremony/reply`, `{ceremony, account, sealed}`. The poster is
///   signed in as `account`, the offer's account, or is its teammate by
///   `TEAMMATE_RULE` below. The body is at most `REPLY_MAX` bytes. Each poster
///   gets one reply per ceremony (a second gets 409), and one ceremony and
///   account holds at most `REPLIES_MAX`, so nobody's junk post can take the
///   slot from the real one.
/// - `GET /v1/ceremony/reply/<ceremony>`. Only the offer's account may read,
///   and it gets every reply for it, each with who posted it, and the read
///   deletes them all. After `REPLY_TTL_MS` a reply is gone. Every refusal is
///   the same 404, so a reader learns nothing about a ceremony that isn't
///   theirs. The five-minute cron deletes what nobody read.
///
/// ## What the seal does not stop
///
/// A reply from anyone who saw the offer, and may post, OPENS. The offer is
/// public, so it gives them the reply key, the ceremony, the account, the
/// channel and the Key A, which is all a valid reply needs; a forged one can
/// list the forger's machines with their host keys pinned, and it passes
/// `accept_manifest` too. The ceremony's module note names this risk for the
/// QR leg ("filmed the first code, is in the room"), and the relay removes "in
/// the room": the posters are the account itself and its teammates. What stops
/// a forged reply is the browser's comparison code (slice 8): six characters
/// over the manifest that the person matches against the granting phone's
/// screen before anything is kept. `posted_by` is there so the browser can say
/// who a reply is from; it proves nothing about the runners inside.

import { requireAccount } from './session'
import { readLimited } from './heartbeat-body'
import { json, type Env } from './index'

/// The largest body `POST` takes. A manifest at the ceremony's byte budget
/// (`BUDGET`, 1800) seals to under 2.5 KB of base64url; the Rust test
/// `a_manifest_over_budget_is_not_sealed_and_one_at_budget_fits_the_relay`
/// holds the two numbers together.
export const REPLY_MAX = 4096

/// How many replies one ceremony and account holds, and so the most one read
/// returns. One per poster: the account itself and up to three teammates.
export const REPLIES_MAX = 4

/// How long a reply waits to be read: the ceremony's `FRESHNESS`, 120 s, the
/// longest the browser will wait for one on its own clock.
export const REPLY_TTL_MS = 120_000

export const REPLY_PATH = '/v1/ceremony/reply'

/// A ceremony id: 128 bits as lowercase hex (`fresh_ceremony_id`).
const CEREMONY = /^[0-9a-f]{32}$/

/// The smallest sealed reply: a 32-byte encapsulated key, a 16-byte tag and at
/// least one byte. Unpadded base64url only.
const SEALED = /^[A-Za-z0-9_-]+$/
const SEALED_MIN_BYTES = 32 + 16 + 1

/// What only a reply in the clear would hold: a host key's fingerprint, or
/// the manifest's runner list. A phone with a bug that posts the reply
/// unsealed is refused rather than stored. Ciphertext holds either by chance
/// with odds near 2^-50 per position, and that is a retry, not a leak.
const PLAINTEXT = ['SHA256:', '"runners"']

/// Who may post a reply for another account: someone who shares a team with
/// it that isn't a personal one, both of them `admin` or `member` there.
/// `billing` grants nothing, so it neither vouches nor is vouched for.
///
/// The same rule as `teammates` in `crates/ceremony/src/teammate.rs` (its
/// `ROLES`), which decides whose code a runner admin's phone may grant. The
/// two must agree: a phone that may grant a teammate's browser and then gets
/// 403 here falls back to its QR code, which the browser can't scan.
const TEAMMATE_RULE = `SELECT 1 FROM memberships poster
  JOIN memberships member ON member.team_id = poster.team_id
  JOIN teams t ON t.id = poster.team_id
  WHERE poster.account_id = ? AND member.account_id = ? AND t.personal_of IS NULL
    AND poster.role IN ('admin', 'member') AND member.role IN ('admin', 'member')
  LIMIT 1`

/// Both routes. `null` for a path that is neither, so `fetch` goes on.
export async function ceremonyReply(request: Request, env: Env, pathname: string): Promise<Response | null> {
  if (pathname === REPLY_PATH) {
    return request.method === 'POST' ? await postReply(request, env) : json({ error: 'method' }, 405)
  }
  if (pathname.startsWith(`${REPLY_PATH}/`)) {
    return request.method === 'GET' ? await takeReplies(request, env, pathname.slice(REPLY_PATH.length + 1)) : json({ error: 'method' }, 405)
  }
  return null
}

async function postReply(request: Request, env: Env): Promise<Response> {
  const poster = await requireAccount(request, env, env.CEREMONY_REPLY_LIMIT)
  if (poster instanceof Response) return poster

  const text = await readLimited(request, REPLY_MAX)
  if (text === null) return json({ error: 'too large' }, 413)
  let body: unknown
  try {
    body = JSON.parse(text)
  } catch {
    return json({ error: 'body' }, 400)
  }
  const { ceremony, account, sealed } = (body ?? {}) as Record<string, unknown>
  if (typeof ceremony !== 'string' || !CEREMONY.test(ceremony)) return json({ error: 'ceremony' }, 400)
  if (typeof account !== 'string' || !account || account.length > 128) return json({ error: 'account' }, 400)
  if (typeof sealed !== 'string' || !SEALED.test(sealed) || !looksSealed(sealed)) return json({ error: 'sealed' }, 400)

  if (!(await mayPostFor(env, poster, account))) return json({ error: 'forbidden' }, 403)

  // One reply per poster and ceremony, and at most REPLIES_MAX live ones for
  // this ceremony and account. A poster's own reply that has outlived its
  // window, and not yet been swept, is replaced: nobody can read it any more.
  const now = Date.now()
  const cutoff = now - REPLY_TTL_MS
  const written = await env.DB.prepare(
    `INSERT INTO ceremony_replies (ceremony, account_id, posted_by, sealed, created_at)
     SELECT ?1, ?2, ?3, ?4, ?5
     WHERE (SELECT COUNT(*) FROM ceremony_replies
            WHERE ceremony = ?1 AND account_id = ?2 AND posted_by != ?3 AND created_at >= ?6) < ?7
     ON CONFLICT (ceremony, posted_by) DO UPDATE SET account_id = ?2, sealed = ?4, created_at = ?5
     WHERE ceremony_replies.created_at < ?6`,
  )
    .bind(ceremony, account, poster, sealed, now, cutoff, REPLIES_MAX)
    .run()
  if (!written.meta.changes) return json({ error: 'taken' }, 409)
  return json({ ok: true })
}

/// The offer's own account, or a teammate by `TEAMMATE_RULE`.
async function mayPostFor(env: Env, poster: string, account: string): Promise<boolean> {
  if (poster === account) return true
  return (await env.DB.prepare(TEAMMATE_RULE).bind(poster, account).first()) !== null
}

async function takeReplies(request: Request, env: Env, ceremony: string): Promise<Response> {
  const reader = await requireAccount(request, env, env.CEREMONY_REPLY_LIMIT)
  if (reader instanceof Response) return reader
  const gone = json({ error: 'not found' }, 404, NO_STORE)
  if (!CEREMONY.test(ceremony)) return gone

  // Deleted in the read, so two reads at once can't both have one.
  const rows = await env.DB.prepare(
    `DELETE FROM ceremony_replies WHERE ceremony = ? AND account_id = ? RETURNING sealed, posted_by, created_at`,
  )
    .bind(ceremony, reader)
    .all<{ sealed: string; posted_by: string; created_at: number }>()
  const now = Date.now()
  const replies = rows.results
    .filter(row => now - row.created_at <= REPLY_TTL_MS)
    .sort((a, b) => a.created_at - b.created_at)
    .slice(0, REPLIES_MAX)
    .map(row => ({ sealed: row.sealed, posted_by: row.posted_by }))
  if (replies.length === 0) return gone
  return json({ replies }, 200, NO_STORE)
}

/// A read that deletes must never be answered from a cache.
const NO_STORE = { 'cache-control': 'no-store' }

/// The five-minute cron's share: every reply nobody read in time. The read
/// refuses those itself, so this is tidiness, not the TTL.
export async function sweepCeremonyReplies(env: Env): Promise<void> {
  await env.DB.prepare(`DELETE FROM ceremony_replies WHERE created_at < ?`)
    .bind(Date.now() - REPLY_TTL_MS)
    .run()
}

/// Long enough to be a sealed reply, and holding nothing only plaintext would.
function looksSealed(sealed: string): boolean {
  let bytes: string
  try {
    const padded = sealed.replaceAll('-', '+').replaceAll('_', '/')
    bytes = atob(padded + '='.repeat((4 - (padded.length % 4)) % 4))
  } catch {
    return false
  }
  return bytes.length >= SEALED_MIN_BYTES && !PLAINTEXT.some(marker => bytes.includes(marker))
}
