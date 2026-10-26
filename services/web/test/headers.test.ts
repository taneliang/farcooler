import { afterEach, describe, expect, it, vi } from 'vitest'

import { shellCsp } from '../src/headers'
import { envWith, get, relay, SESSION } from './helpers'

// The shell's content security policy and the headers on every response
// (plan-763 §1). Each directive is asserted, so loosening any one goes red.

afterEach(() => vi.unstubAllGlobals())

function directives(csp: string | null): Record<string, string> {
  const out: Record<string, string> = {}
  for (const part of (csp ?? '').split(';')) {
    const [name, ...rest] = part.trim().split(/\s+/)
    if (name) out[name] = rest.join(' ')
  }
  return out
}

describe("the shell's CSP", () => {
  it('is exactly the plan\'s policy', async () => {
    relay()
    const res = await get('/', envWith(), { headers: { cookie: SESSION } })
    expect(directives(res.headers.get('content-security-policy'))).toEqual({
      'default-src': "'none'",
      'script-src': "'self' 'wasm-unsafe-eval'",
      'style-src': "'self'",
      'img-src': "'self' data: blob:",
      'font-src': "'self'",
      'connect-src': "'self' https://tailcat.dev wss://*.ipn.dev",
      'worker-src': "'self'",
      'manifest-src': "'self'",
      'form-action': "'self' https://api.workos.com",
      'frame-ancestors': "'none'",
      'base-uri': "'none'",
      'require-trusted-types-for': "'script'",
      'trusted-types': 'fc',
      'report-to': 'csp',
      'report-uri': '/csp',
    })
    expect(res.headers.get('reporting-endpoints')).toBe('csp="/csp"')
    expect(res.headers.get('cache-control')).toBe('no-store')
  })

  it('never allows unsafe-eval or unsafe-inline script', () => {
    const csp = directives(shellCsp(envWith()))
    expect(csp['default-src']).toBe("'none'")
    expect(csp['script-src']).not.toContain("'unsafe-eval'")
    expect(csp['script-src']).not.toContain("'unsafe-inline'")
  })

  it('lists the DERP hosts from DERP_HOSTS, and drops anything not shaped like a host', () => {
    const csp = directives(shellCsp(envWith({ DERP_HOSTS: 'derp.example.com *.ipn.dev evil.com;script-src * https://x.com', DERP_MAP_ORIGIN: "https://tailcat.dev 'unsafe-inline'" })))
    expect(csp['connect-src']).toBe("'self' wss://derp.example.com wss://*.ipn.dev")
    expect(csp['script-src']).toBe("'self' 'wasm-unsafe-eval'")
  })
})

describe('every response', () => {
  it('carries the base headers, on pages, files and refusals alike', async () => {
    relay()
    const env = envWith()
    const responses = [
      await get('/', env, { headers: { cookie: SESSION } }),
      await get('/', env),
      await get('/a/app.0123456789abcdef.js', env),
      await get('/a/tailcat.0123456789abcdef.wasm', env),
      await get('/version.json', env),
      await get('/nowhere', env),
      await get('/api/relay/directory/me', env, { method: 'POST' }),
    ]
    for (const res of responses) {
      expect(res.headers.get('cross-origin-opener-policy')).toBe('same-origin')
      expect(res.headers.get('cross-origin-resource-policy')).toBe('same-origin')
      expect(res.headers.get('permissions-policy')).toBe(
        'camera=(self), publickey-credentials-create=(self), publickey-credentials-get=(self), microphone=(), geolocation=()',
      )
      expect(res.headers.get('referrer-policy')).toBe('no-referrer')
      expect(res.headers.get('strict-transport-security')).toBe('max-age=31536000')
      expect(res.headers.get('x-content-type-options')).toBe('nosniff')
      // No COEP: nothing uses threads (plan-763 §1).
      expect(res.headers.get('cross-origin-embedder-policy')).toBeNull()
    }
  })

  it("gives the Worker's own pages a policy with no script at all", async () => {
    relay()
    for (const res of [await get('/', envWith()), await get('/', envWith({ WEB_CLIENT: '' })), await get('/nowhere', envWith())]) {
      const csp = directives(res.headers.get('content-security-policy'))
      expect(csp['default-src']).toBe("'none'")
      expect(csp['script-src']).toBeUndefined()
      expect(csp['connect-src']).toBeUndefined()
      expect(csp['frame-ancestors']).toBe("'none'")
    }
  })
})
