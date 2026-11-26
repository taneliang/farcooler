/// Web Push sends an empty wake-up message. The service worker shows generic
/// copy, so no runner identity or question leaves for the push provider.
import { requireAccount } from './session'
import { json, type Env } from './index'
import { readLimited } from './heartbeat-body'

const providers = new Map([
  ['fcm.googleapis.com', /^\/fcm\/send\/[A-Za-z0-9_.~-]+(?::[A-Za-z0-9_.~-]+)?$/],
  ['updates.push.services.mozilla.com', /^\/wpush\/v2\/[A-Za-z0-9_.~-]+$/],
  ['web.push.apple.com', /^\/[A-Za-z0-9_.~-]+$/],
])

function endpointOf(value: unknown): URL | null {
  if (typeof value !== 'string' || value.length > 2048) return null
  try {
    const url = new URL(value)
    const path = providers.get(url.hostname)
    return path && url.origin === `https://${url.hostname}` && !url.username && !url.password
      && !url.search && !url.hash && path.test(url.pathname) ? url : null
  } catch {
    return null
  }
}

export async function configureWebPush(request: Request, env: Env, action: string): Promise<Response> {
  const account = await requireAccount(request, env)
  if (account instanceof Response) return account
  const text = await readLimited(request, 4096)
  let endpoint: URL | null
  try { endpoint = endpointOf(text && JSON.parse(text)?.endpoint) } catch { endpoint = null }
  if (action === 'key') {
    const enrolled = endpoint && await env.DB.prepare('SELECT 1 FROM web_push WHERE endpoint = ? AND account_id = ?')
      .bind(endpoint.href, account).first()
    return json({ key: env.WEB_PUSH_PRIVATE_JWK ? env.WEB_PUSH_PUBLIC_KEY ?? null : null, enrolled: Boolean(enrolled) })
  }
  if (!endpoint) return json({ error: 'endpoint' }, 400)
  if (action === 'remove') {
    await env.DB.prepare('DELETE FROM web_push WHERE account_id = ? AND endpoint = ?').bind(account, endpoint.href).run()
    return json({ ok: true })
  }
  if (!env.WEB_PUSH_PUBLIC_KEY || !env.WEB_PUSH_PRIVATE_JWK) return json({ error: 'unavailable' }, 503)
  const inserted = await env.DB.prepare(`
    INSERT INTO web_push (endpoint, account_id)
    SELECT ?1, ?2 WHERE (SELECT COUNT(*) FROM web_push WHERE account_id = ?2) < 16
    ON CONFLICT (endpoint) DO NOTHING
  `).bind(endpoint.href, account).run()
  if (inserted.meta.changes || await env.DB.prepare('SELECT 1 FROM web_push WHERE endpoint = ? AND account_id = ?')
    .bind(endpoint.href, account).first()) return json({ ok: true })
  return json({ error: 'subscription refused' }, 409)
}

function base64url(bytes: Uint8Array): string {
  return btoa(String.fromCharCode(...bytes)).replaceAll('+', '-').replaceAll('/', '_').replace(/=+$/, '')
}

async function authorize(key: CryptoKey, origin: string): Promise<string> {
  const encode = (value: unknown) => base64url(new TextEncoder().encode(JSON.stringify(value)))
  const claims = `${encode({ typ: 'JWT', alg: 'ES256' })}.${encode({ aud: origin, exp: Math.floor(Date.now() / 1000) + 600 })}`
  const signature = await crypto.subtle.sign({ name: 'ECDSA', hash: 'SHA-256' }, key, new TextEncoder().encode(claims))
  return `${claims}.${base64url(new Uint8Array(signature))}`
}

export async function deliverWebPush(env: Env, account: string, send = fetch): Promise<void> {
  if (!env.WEB_PUSH_PUBLIC_KEY || !env.WEB_PUSH_PRIVATE_JWK) return
  const { results } = await env.DB.prepare('SELECT endpoint FROM web_push WHERE account_id = ?').bind(account).all<{ endpoint: string }>()
  if (!results.length) return
  const key = await crypto.subtle.importKey('jwk', JSON.parse(env.WEB_PUSH_PRIVATE_JWK), { name: 'ECDSA', namedCurve: 'P-256' }, false, ['sign'])
  await Promise.allSettled(results.map(async ({ endpoint }) => {
    const url = endpointOf(endpoint)
    if (!url) return
    const token = await authorize(key, url.origin)
    const response = await send(url.href, {
      method: 'POST',
      redirect: 'error',
      signal: AbortSignal.timeout(3000),
      headers: { ttl: '300', topic: 'needs-you', authorization: `vapid t=${token}, k=${env.WEB_PUSH_PUBLIC_KEY}` },
    })
    if (response.status === 404 || response.status === 410) {
      await env.DB.prepare('DELETE FROM web_push WHERE account_id = ? AND endpoint = ?').bind(account, endpoint).run()
    }
  }))
}
