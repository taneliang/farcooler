import { afterEach, describe, expect, it, vi } from 'vitest'

import { envWith, get, JS, relay, WASM } from './helpers'

// The hashed files and /version.json (src/assets.ts).

afterEach(() => vi.unstubAllGlobals())

describe('the client files', () => {
  it('serves a hashed file for a year, immutable, with its type', async () => {
    const res = await get('/a/app.0123456789abcdef.js', envWith())
    expect(res.status).toBe(200)
    expect(await res.text()).toBe(JS)
    expect(res.headers.get('cache-control')).toBe('public, max-age=31536000, immutable')
    expect(res.headers.get('content-type')).toBe('text/javascript; charset=utf-8')
  })

  it('serves a wasm module from its .br, as application/wasm with content-encoding br, bytes untouched', async () => {
    const res = await get('/a/tailcat.0123456789abcdef.wasm', envWith())
    expect(res.status).toBe(200)
    expect(res.headers.get('content-type')).toBe('application/wasm')
    expect(res.headers.get('content-encoding')).toBe('br')
    expect(new Uint8Array(await res.arrayBuffer())).toEqual(WASM)
  })

  it('serves nothing under /a/ that is not a hashed file of a known type', async () => {
    for (const path of ['/a/tailcat.0123456789abcdef.wasm.br', '/a/app.js', '/a/app.0123.js', '/a/app.0123456789abcdef.html', '/a/', '/a/x/../../index.html']) {
      expect((await get(path, envWith())).status, path).toBe(404)
    }
    expect((await get('/a/app.0123456789abcdef.js', envWith(), { method: 'POST' })).status).toBe(405)
  })

  it('serves the files whether or not the flag is on: they are public code', async () => {
    expect((await get('/a/app.0123456789abcdef.js', envWith({ WEB_CLIENT: '' }))).status).toBe(200)
  })

  it('serves /version.json never cached', async () => {
    const res = await get('/version.json', envWith())
    expect(res.headers.get('cache-control')).toBe('no-store')
    expect((await res.json<any>()).build).toBe('abc')
  })
})

describe('/csp', () => {
  it('logs only the directive and the blocked host, from either report format', async () => {
    const log = vi.spyOn(console, 'log').mockImplementation(() => {})
    try {
      const reportTo = JSON.stringify([
        {
          type: 'csp-violation',
          url: 'https://app.canary.farcooler.com/?secret=1',
          body: { documentURL: 'https://app.canary.farcooler.com/?secret=1', effectiveDirective: 'connect-src', blockedURL: 'wss://derp9.example.net/derp?k=SECRET', sample: 'SECRET' },
        },
        { type: 'csp-violation', body: { effectiveDirective: 'style-src-elem', blockedURL: 'inline' } },
      ])
      const reportUri = JSON.stringify({ 'csp-report': { 'document-uri': 'https://x/?secret', 'violated-directive': 'script-src-elem', 'blocked-uri': 'eval' } })
      for (const body of [reportTo, reportUri, 'not json', JSON.stringify([{ body: { effectiveDirective: 'x; drop', blockedURL: 'a' } }])]) {
        const res = await get('/csp', envWith(), { method: 'POST', body, headers: { 'content-type': 'application/reports+json' } })
        expect(res.status).toBe(204)
      }
      expect(log.mock.calls.map(c => c.join(' '))).toEqual(['csp connect-src derp9.example.net', 'csp style-src-elem inline', 'csp script-src-elem eval'])
    } finally {
      log.mockRestore()
    }
  })
})
