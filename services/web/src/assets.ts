/// The client's files: hashed under /a/, plus /version.json.
///
/// A hashed name never changes content, so it is cached for a year. A wasm
/// module lives in the assets only as its brotli file (`x.<hash>.wasm.br`):
/// the Go Tailcat module is about 25.6 MiB raw, over the 25 MiB Workers Static
/// Assets allows per file, and 4.4 MB as brotli (plan-763 §0, spike-761 point
/// 1). The Worker serves it as `x.<hash>.wasm` with `content-encoding: br` and
/// `encodeBody: "manual"`, so Cloudflare passes the bytes through as they are
/// and the browser decodes them.

import { withHeaders } from './headers'

/// A hashed file: a name, a hash of 8 or more hex digits, and a known type.
/// Nothing else under /a/ is served, so no path can reach index.html or
/// version.json or climb out of a/.
const HASHED = /^\/a\/[A-Za-z0-9_-]+\.[0-9a-f]{8,64}\.(js|css|wasm|woff2|svg|png)$/

const TYPES: Record<string, string> = {
  js: 'text/javascript; charset=utf-8',
  css: 'text/css; charset=utf-8',
  wasm: 'application/wasm',
  woff2: 'font/woff2',
  svg: 'image/svg+xml',
  png: 'image/png',
}

const IMMUTABLE = 'public, max-age=31536000, immutable'
/// On every file, so one opened directly as a document (an SVG with a script
/// in it) runs nothing on the origin that holds a Control key.
const FILE_CSP = "default-src 'none'; style-src 'unsafe-inline'; sandbox"

export function isAsset(path: string): boolean {
  return HASHED.test(path)
}

/// One hashed file, or null for one that isn't there.
export async function asset(assets: Fetcher, url: URL, method: string): Promise<Response | null> {
  const found = HASHED.exec(url.pathname)
  if (!found) return null
  const kind = found[1]
  const stored = kind === 'wasm' ? `${url.pathname}.br` : url.pathname
  const upstream = await assets.fetch(new URL(stored, url.origin), { method: method === 'HEAD' ? 'HEAD' : 'GET' })
  if (upstream.status !== 200) return null
  const headers: Record<string, string> = { 'cache-control': IMMUTABLE, 'content-type': TYPES[kind], 'content-security-policy': FILE_CSP }
  if (kind !== 'wasm') return withHeaders(upstream.body, 200, headers)
  return new Response(upstream.body, {
    status: 200,
    headers: withHeaders(null, 200, { ...headers, 'content-encoding': 'br' }).headers,
    encodeBody: 'manual',
  })
}

/// The build that's deployed, for an open tab to compare with its own. Never
/// cached: a tab polls it on focus and offers a Reload when it changes.
export async function version(assets: Fetcher, url: URL): Promise<Response> {
  const upstream = await assets.fetch(new URL('/version.json', url.origin))
  if (upstream.status !== 200) return withHeaders(null, 404, { 'cache-control': 'no-store' })
  return withHeaders(upstream.body, 200, { 'cache-control': 'no-store', 'content-type': 'application/json' })
}
