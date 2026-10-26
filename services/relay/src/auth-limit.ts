/// The auth routes' rate limit (ov-811): who a caller is, for `AUTH_LIMIT`.
///
/// Out of index.ts for its size budget; index.ts re-exports `visitorKey` for
/// the tests.

import { VISITOR_CONTEXT } from '../../common/visitor'
import type { Env } from './index'

/// Whether this caller may make another auth request.
///
/// Keyed on the connecting IP, which is the only thing an unauthenticated
/// caller has, or on the visitor the web or site Worker signs for, whose own
/// IP is Cloudflare's (`visitorKey`). Not a strong identity — a botnet has
/// many — but it is what stops one client burning the WorkOS quota, which is
/// the realistic failure.
///
/// Fails OPEN when the binding is absent. A local `wrangler dev` has no rate
/// limiter, and a relay that refused every sign-in because a binding was
/// missing would be a worse outage than the one being prevented. Production
/// declares it in wrangler.toml; if it is ever missing there, sign-in works and
/// the protection is gone, which is the right way round for this specific
/// guard.
export async function withinRate(request: Request, env: Env): Promise<boolean> {
  if (!env.AUTH_LIMIT) return true
  const key = (await visitorKey(request, env)) ?? request.headers.get('cf-connecting-ip') ?? 'unknown'
  const { success } = await env.AUTH_LIMIT.limit({ key })
  return success
}

/// The visitor the web or site Worker names in `x-fc-visitor`, when
/// `x-fc-visitor-mac` is that name's HMAC-SHA256 under `VISITOR_SECRET`; null
/// otherwise, and the caller is keyed on its address as before.
///
/// Only a holder of the secret can choose a bucket: anyone else's header is
/// ignored, so a forged one leaves them in their own address's bucket. The
/// native apps send neither header. `crypto.subtle.verify` compares in
/// constant time.
export async function visitorKey(request: Request, env: Env): Promise<string | null> {
  const secret = env.VISITOR_SECRET?.trim()
  const visitor = request.headers.get('x-fc-visitor')
  const mac = request.headers.get('x-fc-visitor-mac')
  if (!secret || !visitor || !mac || visitor.length > 64 || !/^[0-9a-f]{64}$/.test(mac)) return null
  const key = await crypto.subtle.importKey('raw', new TextEncoder().encode(secret), { name: 'HMAC', hash: 'SHA-256' }, false, ['verify'])
  const bytes = new Uint8Array(mac.match(/../g)!.map(pair => parseInt(pair, 16)))
  const good = await crypto.subtle.verify('HMAC', key, bytes, new TextEncoder().encode(`${VISITOR_CONTEXT}${visitor}`))
  return good ? visitor : null
}
