/// The client shell served the way the Worker serves it, for a real browser.
///
/// Headers and files come from the Worker's own code (src/headers.ts,
/// src/assets.ts), so the CSP a browser enforces here is the one production
/// sends, on the real bundle from build.sh. What isn't the Worker's is only
/// what decides WHO gets the shell (sign-in and the flag, covered by vitest):
/// every visitor here is an admitted account, `acct-test`.

import { asset, isAsset, version } from '../../src/assets'
import type { WebEnv } from '../../src/env'
import { shellCsp, withHeaders } from '../../src/headers'

export interface Reports {
  /// Bodies posted to /csp, as the browser sent them.
  csp: string[]
}

export function shellHandler(dist: (path: string) => Uint8Array | null, env: Pick<WebEnv, 'DERP_HOSTS' | 'DERP_MAP_ORIGIN'>, reports: Reports) {
  const assets = {
    async fetch(input: URL | string): Promise<Response> {
      const bytes = dist(new URL(input).pathname)
      return bytes ? new Response(bytes) : new Response(null, { status: 404 })
    },
  } as unknown as Fetcher

  return async (request: Request): Promise<Response> => {
    const url = new URL(request.url)
    const path = url.pathname
    if (path.startsWith('/a/')) return (isAsset(path) && (await asset(assets, url, request.method))) || new Response(null, { status: 404 })
    if (path === '/version.json') return await version(assets, url)
    if (path === '/csp') {
      reports.csp.push(await request.text())
      return new Response(null, { status: 204 })
    }
    // Signing out ends at the sign-in page. What the page deletes before it
    // posts here is what the check looks for; the Worker's own Clear-Site-Data
    // is vitest's (test/sign-in.test.ts).
    if (path === '/sign-out' && request.method === 'POST') return new Response(null, { status: 303, headers: { location: '/signed-out' } })
    if (path === '/signed-out') return withHeaders('signed out', 200, { 'content-type': 'text/plain' })
    if (path === '/api/relay/directory/me') {
      return withHeaders(JSON.stringify({ user: { id: 'acct-test', email: 'owner@example.com' } }), 200, { 'content-type': 'application/json' })
    }
    if (path === '/') {
      const shell = dist('/index.html')
      return withHeaders(shell, 200, {
        'cache-control': 'no-store',
        'content-type': 'text/html; charset=utf-8',
        'content-security-policy': shellCsp(env as WebEnv),
        'reporting-endpoints': 'csp="/csp"',
        'x-frame-options': 'DENY',
      })
    }
    return new Response(null, { status: 404 })
  }
}
