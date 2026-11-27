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
///   gets one reply per ceremony (a second gets 409). The account's own
///   post always gets in, and its teammates share `REPLIES_MAX - 1` slots, so
///   nobody's junk post can take the slot from the account's own phone.
/// - `GET /v1/ceremony/reply/<ceremony>`. Only the offer's account may read,
///   and it gets every reply for it, each with who posted it (`posted_by`, and
///   `posted_by_email`, the verified one or null, and directory authority),
///   and the read deletes them all. After `REPLY_TTL_MS` a reply is gone. Every refusal is
///   the same 404, so a reader learns nothing about a ceremony that isn't
///   theirs. The five-minute cron deletes what nobody read.
///
/// Teammate replies are device-signed and carry ceremony-bound runner
/// attestations (ov-1020). The read supplies independent current directory
/// authority; the browser checks every runner and the device signature before
/// offering the reply. Same-account replies still require the in-person code.

import { requireAccount } from './session'
import { holds, type HoldRow } from './directory/hold'
import { readLimited } from './heartbeat-body'
import { json, type Env } from './index'

/// Signed replies add one attestation per runner: at most 8 KB plaintext,
/// about 11 KB after HPKE and base64url, plus the request's identifiers.
export const REPLY_MAX = 12288

/// How many replies one ceremony and account holds, and so the most one read
/// returns. One per poster: the account itself, always let in, and up to three
/// teammates.
export const REPLIES_MAX = 4

/// How long a reply waits to be read: the ceremony's `FRESHNESS`, 120 s, the
/// longest the browser will wait for one on its own clock.
export const REPLY_TTL_MS = 120_000

export const REPLY_PATH = '/v1/ceremony/reply'

/// A ceremony id: 128 bits as lowercase hex (`fresh_ceremony_id`).
const CEREMONY = /^[0-9a-f]{32}$/

/// The smallest sealed reply: a 32-byte encapsulated key, a 16-byte tag and at
/// least one byte. Unpadded base64url only.
export const SEALED = /^[A-Za-z0-9_-]+$/
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

  // One reply per poster and ceremony. The account's own post is always let
  // in; its teammates share the other REPLIES_MAX - 1 slots, counting the
  // live replies of teammates other than this poster. So teammates can never
  // fill the slots against the account's own phone, and a read holds at most
  // REPLIES_MAX. A poster's own reply that has outlived its window, and not
  // yet been swept, is replaced: nobody can read it any more.
  const now = Date.now()
  const cutoff = now - REPLY_TTL_MS
  // D1 batches run in one transaction: a take must not delete the existing
  // reply between a refused insert and the lookup that explains the refusal.
  const [written, existing] = await env.DB.batch([env.DB.prepare(
    `INSERT INTO ceremony_replies (ceremony, account_id, posted_by, sealed, created_at)
     SELECT ?1, ?2, ?3, ?4, ?5
     WHERE ?3 = ?2 OR (SELECT COUNT(*) FROM ceremony_replies
            WHERE ceremony = ?1 AND account_id = ?2 AND posted_by NOT IN (?2, ?3) AND created_at >= ?6) < ?7 - 1
     ON CONFLICT (ceremony, posted_by) DO UPDATE SET account_id = ?2, sealed = ?4, created_at = ?5
     WHERE ceremony_replies.created_at < ?6`,
  )
    .bind(ceremony, account, poster, sealed, now, cutoff, REPLIES_MAX),
  env.DB.prepare(`SELECT 1 FROM ceremony_replies
      WHERE ceremony = ? AND account_id = ? AND posted_by = ? AND created_at >= ?`)
    .bind(ceremony, account, poster, cutoff)])
  if (!written.meta.changes) {
    return json({ error: existing.results.length ? 'already_posted' : 'slots_full' }, 409)
  }
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
  if (replies.length === 0) return gone
  // Each poster's email, as this relay has it: a teammate's reply names who
  // granted it, and the browser refuses one whose name isn't its poster's
  // (ov-778 R-113). Only `verified_email`: sign-in stores `email` unverified,
  // so a forger could set it to a teammate's. None on file is null, never a guess.
  const posters = [...new Set(replies.map(row => row.posted_by))]
  const emails = await env.DB.prepare(`SELECT id, verified_email AS email FROM accounts WHERE id IN (${posters.map(() => '?').join(', ')})`)
    .bind(...posters)
    .all<{ id: string; email: string | null }>()
  const emailOf = new Map(emails.results.map(row => [row.id, row.email]))
  const authority = new Map(await Promise.all(posters.filter(p => p !== reader).map(async p => [p, await replyAuthority(env, p, reader)] as const)))
  return json(
    { replies: replies.map(row => ({ authority: authority.get(row.posted_by), sealed: row.sealed, posted_by: row.posted_by, posted_by_email: emailOf.get(row.posted_by) ?? null })) },
    200,
    NO_STORE,
  )
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
export function looksSealed(sealed: string): boolean {
  let bytes: string
  try {
    const padded = sealed.replaceAll('-', '+').replaceAll('_', '/')
    bytes = atob(padded + '='.repeat((4 - (padded.length % 4)) % 4))
  } catch {
    return false
  }
  return bytes.length >= SEALED_MIN_BYTES && !PLAINTEXT.some(marker => bytes.includes(marker))
}

/// Current authority, read after taking the ciphertext. Runner identities are
/// their registered keys, not a phone's local ids. A device held after an
/// account reset is withheld here even if the feature flag for roster UI is off.
async function replyAuthority(env: Env, poster: string, reader: string): Promise<{ devices: string[]; runners: string[] }> {
  const devices = await env.DB.prepare(`SELECT id, state, trust_reset, trusted_at, certified_by, key_a_fingerprint
    FROM device_keys WHERE account_id = ?`).bind(poster).all<HoldRow & { key_a_fingerprint: string }>()
  const standing = holds(devices.results, Date.now())
  const eligible = devices.results.filter(d => (d.state === 'trusted' || d.state === 'certified')
    && !standing.unrooted.has(d.id) && !standing.since.has(d.id))
  const runners = await env.DB.prepare(`SELECT DISTINCT n.runner_key_fingerprint FROM runners n
    JOIN runner_roles r ON r.runner_id = n.id AND r.account_id = ? AND r.role IN ('owner', 'admin')
    JOIN memberships p ON p.team_id = n.team_id AND p.account_id = ? AND p.role IN ('admin', 'member')
    JOIN memberships m ON m.team_id = n.team_id AND m.account_id = ? AND m.role IN ('admin', 'member')
    JOIN teams t ON t.id = n.team_id AND t.personal_of IS NULL`).bind(poster, poster, reader)
    .all<{ runner_key_fingerprint: string }>()
  return { devices: eligible.map(d => d.key_a_fingerprint), runners: runners.results.map(r => r.runner_key_fingerprint) }
}
