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
///   signed in as `account`, the offer's account, or is an admin of a team
///   `account` is in: a runner admin granting a teammate's browser, as
///   `crates/ceremony/src/teammate.rs` lets them. The body is at most
///   `REPLY_MAX` bytes. One reply per ceremony: a second post gets 409, so a
///   squatter can't replace the real one, and the phone that's refused says
///   so and shows its QR code instead.
/// - `GET /v1/ceremony/reply/<ceremony>`. Only the offer's account may read
///   it, the read deletes it, and after `REPLY_TTL_MS` it's gone. Every
///   refusal is the same 404, so a reader learns nothing about a ceremony
///   that isn't theirs. The five-minute cron deletes what nobody read.

import { requireAccount } from './session'
import { readLimited } from './heartbeat-body'
import { json, type Env } from './index'

/// The largest body `POST` takes. A manifest at the ceremony's byte budget
/// (`BUDGET`, 1800) seals to under 2.5 KB of base64url; the Rust test
/// `a_manifest_over_budget_is_not_sealed_and_one_at_budget_fits_the_relay`
/// holds the two numbers together.
export const REPLY_MAX = 4096

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

/// Both routes. `null` for a path that is neither, so `fetch` goes on.
export async function ceremonyReply(request: Request, env: Env, pathname: string): Promise<Response | null> {
  if (pathname === REPLY_PATH) {
    return request.method === 'POST' ? await postReply(request, env) : json({ error: 'method' }, 405)
  }
  if (pathname.startsWith(`${REPLY_PATH}/`)) {
    return request.method === 'GET' ? await takeReply(request, env, pathname.slice(REPLY_PATH.length + 1)) : json({ error: 'method' }, 405)
  }
  return null
}

async function postReply(request: Request, env: Env): Promise<Response> {
  const poster = await requireAccount(request, env)
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

  // One reply per ceremony. One that has outlived its window and not yet
  // been swept is replaced, since nobody can read it any more.
  const now = Date.now()
  const written = await env.DB.prepare(
    `INSERT INTO ceremony_replies (ceremony, account_id, posted_by, sealed, created_at) VALUES (?1, ?2, ?3, ?4, ?5)
     ON CONFLICT (ceremony) DO UPDATE SET account_id = ?2, posted_by = ?3, sealed = ?4, created_at = ?5
     WHERE ceremony_replies.created_at < ?6`,
  )
    .bind(ceremony, account, poster, sealed, now, now - REPLY_TTL_MS)
    .run()
  if (!written.meta.changes) return json({ error: 'taken' }, 409)
  return json({ ok: true })
}

/// The offer's own account, or an admin of a team that account is in.
///
/// The second is the teammate case: the directory decides it, the same
/// directory the admin's phone asked before it granted anything.
async function mayPostFor(env: Env, poster: string, account: string): Promise<boolean> {
  if (poster === account) return true
  const shared = await env.DB.prepare(
    `SELECT 1 FROM memberships admin JOIN memberships member ON member.team_id = admin.team_id
     WHERE admin.account_id = ? AND admin.role = 'admin' AND member.account_id = ? LIMIT 1`,
  )
    .bind(poster, account)
    .first()
  return shared !== null
}

async function takeReply(request: Request, env: Env, ceremony: string): Promise<Response> {
  const reader = await requireAccount(request, env)
  if (reader instanceof Response) return reader
  const gone = json({ error: 'not found' }, 404, NO_STORE)
  if (!CEREMONY.test(ceremony)) return gone

  // Deleted in the read, so two reads at once can't both have it.
  const row = await env.DB.prepare(
    `DELETE FROM ceremony_replies WHERE ceremony = ? AND account_id = ? RETURNING sealed, created_at`,
  )
    .bind(ceremony, reader)
    .first<{ sealed: string; created_at: number }>()
  if (!row || Date.now() - row.created_at > REPLY_TTL_MS) return gone
  return json({ sealed: row.sealed }, 200, NO_STORE)
}

/// A read that deletes must never be answered from a cache.
const NO_STORE = { 'cache-control': 'no-store' }

/// The five-minute cron's share: every reply nobody read in time.
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
