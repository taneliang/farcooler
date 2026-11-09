/// A browser's sealed copy of what it keeps about itself (ov-779).
///
/// A browser's runner list, and the node key of one enrolled before ov-779,
/// live in its site storage, which Safari wipes after seven days without a
/// visit and anyone can clear. So the page also keeps a copy here, sealed
/// (AES-GCM) under a key it derives from its passkey's PRF output. After a
/// wipe, one touch of the passkey gets the key back, and the copy restores the
/// browser with no new enrollment.
///
/// This relay never sees the key, so it can't read a runner, a host key or a
/// token, and it can't forge a copy the page would open. It can withhold one,
/// which costs the person a new enrollment, or hand back an older one, which
/// lists runners as they were then.
///
/// One route, `/v1/web/vault`, for the signed-in account only:
///
/// - `POST {credential, uv, sealed}` keeps the copy for that passkey, replacing
///   the one before. At most `VAULTS_MAX` passkeys per account; a new one past
///   that gets 409 `full`.
/// - `GET` answers `{vaults: [{credential, uv, sealed}]}`, every copy the
///   account keeps, so the page can name the passkeys in `allowCredentials`.

import { readLimited } from './heartbeat-body'
import { json, type Env } from './index'
import { requireAccount } from './session'

export const VAULT_PATH = '/v1/web/vault'

/// The longest credential id, as base64url: WebAuthn's 1023 bytes is 1364.
export const CREDENTIAL_MAX = 1400

/// The longest sealed copy, as base64url. A copy is a passkey's public key and
/// a runner list, about 600 bytes per runner sealed, so 12 KB is about 20
/// runners: more than any person's computer has.
export const VAULT_MAX = 12_288

/// How many passkeys' copies one account keeps: one per browser profile or
/// synced passkey provider it uses.
export const VAULTS_MAX = 8

const BASE64URL = /^[A-Za-z0-9_-]+$/

/// The smallest copy: a 12-byte nonce, a 16-byte tag and at least one byte.
const SEALED_MIN_BYTES = 12 + 16 + 1

/// What only a copy in the clear would hold, as in ceremony-reply.ts: a page
/// with a bug that posts it unsealed is refused rather than stored.
const PLAINTEXT = ['SHA256:', '"runners"']

const NO_STORE = { 'cache-control': 'no-store' }

/// The route, or `null` for another path, so `fetch` goes on.
export async function webVault(request: Request, env: Env, pathname: string): Promise<Response | null> {
  if (pathname !== VAULT_PATH) return null
  if (request.method === 'GET') return await readVaults(request, env)
  if (request.method === 'POST') return await saveVault(request, env)
  return json({ error: 'method' }, 405)
}

async function readVaults(request: Request, env: Env): Promise<Response> {
  const account = await requireAccount(request, env, env.CEREMONY_REPLY_LIMIT)
  if (account instanceof Response) return account
  const rows = await env.DB.prepare(`SELECT credential, uv, sealed FROM web_vaults WHERE account_id = ? ORDER BY updated_at DESC`)
    .bind(account)
    .all<{ credential: string; uv: number; sealed: string }>()
  return json({ vaults: rows.results.map(row => ({ credential: row.credential, uv: row.uv === 1, sealed: row.sealed })) }, 200, NO_STORE)
}

async function saveVault(request: Request, env: Env): Promise<Response> {
  const account = await requireAccount(request, env, env.CEREMONY_REPLY_LIMIT)
  if (account instanceof Response) return account

  // The body's JSON wraps the copy and the credential id.
  const text = await readLimited(request, VAULT_MAX + CREDENTIAL_MAX + 256)
  if (text === null) return json({ error: 'too large' }, 413)
  let body: unknown
  try {
    body = JSON.parse(text)
  } catch {
    return json({ error: 'body' }, 400)
  }
  const { credential, uv, sealed } = (body ?? {}) as Record<string, unknown>
  if (typeof credential !== 'string' || !BASE64URL.test(credential) || credential.length > CREDENTIAL_MAX) return json({ error: 'credential' }, 400)
  if (typeof uv !== 'boolean') return json({ error: 'uv' }, 400)
  if (typeof sealed !== 'string') return json({ error: 'sealed' }, 400)
  if (sealed.length > VAULT_MAX) return json({ error: 'too large' }, 413)
  if (!BASE64URL.test(sealed) || !looksSealed(sealed)) return json({ error: 'sealed' }, 400)

  // A passkey already kept is always saved again; a new one only while the
  // account keeps fewer than VAULTS_MAX.
  const written = await env.DB.prepare(
    `INSERT INTO web_vaults (account_id, credential, uv, sealed, updated_at)
     SELECT ?1, ?2, ?3, ?4, ?5
     WHERE EXISTS (SELECT 1 FROM web_vaults WHERE account_id = ?1 AND credential = ?2)
        OR (SELECT COUNT(*) FROM web_vaults WHERE account_id = ?1) < ?6
     ON CONFLICT (account_id, credential) DO UPDATE SET uv = ?3, sealed = ?4, updated_at = ?5`,
  )
    .bind(account, credential, uv ? 1 : 0, sealed, Date.now(), VAULTS_MAX)
    .run()
  if (!written.meta.changes) return json({ error: 'full' }, 409)
  return json({ ok: true })
}

/// Long enough to be a sealed copy, and holding nothing only plaintext would.
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
