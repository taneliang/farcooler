/// The headers on every response the web client's Worker makes, and the two
/// content security policies (plan-763 §1, "Headers on every response").
///
/// Two policies, because there are two kinds of page:
/// - The Worker's own pages (sign-in, "Not available yet", errors) run no
///   script at all, under the site's /app policy.
/// - The client shell runs this origin's script and wasm, and connects to this
///   origin, Tailcat's DERP map and the DERP relays, and nothing else.

import type { WebEnv } from './env'

/// On every response, page or file.
export const BASE_HEADERS: Record<string, string> = {
  'cross-origin-opener-policy': 'same-origin',
  'cross-origin-resource-policy': 'same-origin',
  'permissions-policy':
    'camera=(self), publickey-credentials-create=(self), publickey-credentials-get=(self), microphone=(), geolocation=()',
  // Nothing leaves the origin in a Referer, as on www. Not no-referrer: under
  // no-referrer a browser serializes the Origin of this page's own POSTs, a
  // fetch() as well as a form, as `null` (Fetch, "serialize a request
  // origin"), and the API's exact-Origin check and Sign Out would refuse the
  // shell itself (ov-602 found this on www; test/browser/ checks it in Chromium).
  'referrer-policy': 'same-origin',
  'strict-transport-security': 'max-age=31536000',
  'x-content-type-options': 'nosniff',
}

/// The Worker's own pages: no script, no connections, inline style only.
export const PAGE_CSP =
  "default-src 'none'; style-src 'unsafe-inline'; img-src 'self'; form-action 'self' https://api.workos.com; frame-ancestors 'none'; base-uri 'none'"

/// A hostname, optionally with `*.` in front. Nothing that could end a CSP
/// source or start a new directive gets through.
const HOST = /^(\*\.)?[a-z0-9-]+(\.[a-z0-9-]+)+$/
/// An https origin with no path.
const ORIGIN = /^https:\/\/[a-z0-9-]+(\.[a-z0-9-]+)+$/

/// The client shell's policy. `wasm-unsafe-eval` compiles the wasm modules and
/// is not `unsafe-eval`; Trusted Types refuse any string-to-script sink but
/// through the one policy named `fc`. A DERP host or map origin that isn't
/// shaped like one is left out, never passed through.
///
/// Each DERP host is allowed twice: `wss://` for the relay connection itself,
/// and `https://` because Tailcat's js build also fetches the relay's
/// `/derp/probe` to measure it (found in Chromium through a real tunnel, ov-764;
/// plan-763 §1 listed only `wss://`).
export function shellCsp(env: WebEnv): string {
  const derp = (env.DERP_HOSTS ?? '')
    .split(/\s+/)
    .filter(h => HOST.test(h))
    .flatMap(h => [`https://${h}`, `wss://${h}`])
  const map = ORIGIN.test(env.DERP_MAP_ORIGIN ?? '') ? [env.DERP_MAP_ORIGIN!] : []
  return [
    "default-src 'none'",
    "script-src 'self' 'wasm-unsafe-eval'",
    "style-src 'self'",
    "img-src 'self' data: blob:",
    "font-src 'self'",
    ['connect-src', "'self'", ...map, ...derp].join(' '),
    "worker-src 'self'",
    "manifest-src 'self'",
    "form-action 'self' https://api.workos.com",
    "frame-ancestors 'none'",
    "base-uri 'none'",
    "require-trusted-types-for 'script'",
    'trusted-types fc',
    'report-to csp',
    // Firefox reads only report-uri.
    'report-uri /csp',
  ].join('; ')
}

/// A response with the base headers and anything else given.
export function withHeaders(body: BodyInit | null, status: number, extra: Record<string, string> = {}, cookies: string[] = []): Response {
  const headers = new Headers({ ...BASE_HEADERS, ...extra })
  for (const value of cookies) headers.append('set-cookie', value)
  return new Response(body, { status, headers })
}

/// One of the Worker's own pages.
export function page(html: string, status: number, extra: Record<string, string> = {}, cookies: string[] = []): Response {
  return withHeaders(
    html,
    status,
    { 'cache-control': 'no-store', 'content-type': 'text/html; charset=utf-8', 'content-security-policy': PAGE_CSP, 'x-frame-options': 'DENY', ...extra },
    cookies,
  )
}

export const redirect = (location: string, cookies: string[] = []) => withHeaders(null, 303, { 'cache-control': 'no-store', location }, cookies)
