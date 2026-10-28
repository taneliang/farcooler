import { expect, vi } from 'vitest'

import worker, { type WebEnv } from '../src/index'

// The relay and the ASSETS binding, played by each test. The relay's `me`
// shape is services/relay/src/directory/teams.ts's.

export const HOST = 'app.canary.farcooler.com'
export const ORIGIN = `https://${HOST}`
export const RELAY = 'https://relay.example'
export const OWNER = 'user_OWNER'
export const SHELL = '<!doctype html><title>Far Cooler</title><script type="module" src="/a/app.0123456789abcdef.js"></script>'
export const JS = 'console.log("app")'
export const WASM = new Uint8Array([0x0b, 0x02, 0x80, 0x00, 0x61, 0x73, 0x6d, 0x03])

const b64 = (value: unknown) => btoa(JSON.stringify(value)).replaceAll('+', '-').replaceAll('/', '_').replace(/=+$/, '')
const jwt = (claims: Record<string, unknown>) => `${b64({ alg: 'RS256' })}.${b64(claims)}.c2lnbmF0dXJl`
export const ACCESS = jwt({ sub: OWNER, sid: 'session_01ABC', exp: Math.floor(Date.now() / 1000) + 600 })
export const SESSION = `__Host-fc_session=${b64({ a: ACCESS, r: 'refresh_SECRET', c: 'csrf-TEST' })}`

/// Files by path, as build.sh would leave them in dist/.
export const FILES: Record<string, BodyInit> = {
  '/index.html': SHELL,
  '/version.json': '{"build":"abc","channel":"canary","files":{}}',
  '/a/app.0123456789abcdef.js': JS,
  '/a/tailcat.0123456789abcdef.wasm.br': WASM,
  '/a/icon.0123456789abcdef.svg': '<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>',
}

export const ASSETS = {
  fetch: async (input: RequestInfo | URL) => {
    const path = new URL(input instanceof Request ? input.url : String(input)).pathname
    return path in FILES ? new Response(FILES[path]) : new Response('not found', { status: 404 })
  },
} as unknown as Fetcher

/// Canary with the flag on for OWNER, unless `over` says otherwise.
export function envWith(over: Partial<WebEnv> = {}): WebEnv {
  return {
    ASSETS,
    APP_HOST: HOST,
    APP_RELAY_URL: RELAY,
    APP_WORKOS_CLIENT_ID: 'client_TEST',
    WEB_CLIENT: 'owner',
    WEB_ACCOUNTS: OWNER,
    DERP_MAP_ORIGIN: 'https://tailcat.dev',
    DERP_HOSTS: 'tc301a.ipn.dev tc302a.ipn.dev',
    ...over,
  }
}

export interface Call {
  path: string
  method: string | undefined
  authorization: string | null
  body: any
}

/// Play the relay: `me` answers for ACCESS as `account`; `routes` adds more.
export function relay(account = OWNER, routes: Record<string, (call: Call) => Response> = {}): Call[] {
  const calls: Call[] = []
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = new URL(String(input))
      expect(url.origin).toBe(RELAY)
      const call = { path: url.pathname, method: init?.method, authorization: new Headers(init?.headers).get('authorization'), body: init?.body ? JSON.parse(String(init.body)) : null }
      calls.push(call)
      if (url.pathname === '/v1/directory/me') {
        return call.authorization === `Bearer ${ACCESS}`
          ? Response.json({ user: { id: account, email: 'owner@example.com' }, teams: [], devices: [] })
          : Response.json({ error: 'unauthorized' }, { status: 401 })
      }
      const route = routes[url.pathname]
      return route ? route(call) : Response.json({ error: 'not found' }, { status: 404 })
    }),
  )
  return calls
}

export async function get(path: string, env: WebEnv, init: RequestInit = {}, origin = ORIGIN): Promise<Response> {
  return await worker.fetch(new Request(`${origin}${path}`, init), env)
}
