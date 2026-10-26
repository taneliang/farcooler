import { afterEach, describe, expect, it, vi } from 'vitest'

import { ACCESS, envWith, get, ORIGIN, RELAY, SESSION, SHELL } from './helpers'

// The relay limits directory/me per account (ov-815; DIRECTORY_ME_LIMIT in
// services/relay/wrangler.toml). The Worker calls it on every GET / and every
// /api/relay/directory/me, and a refusal there means "wait a minute": the
// limited page and a 429, not "Far Cooler couldn't be reached".

afterEach(() => vi.unstubAllGlobals())

/// The relay, refusing directory/me with a 429 while `limited` holds.
function relayLimiting(limited: () => boolean): string[] {
  const asked: string[] = []
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = new URL(String(input))
      expect(url.origin).toBe(RELAY)
      asked.push(url.pathname)
      expect(new Headers(init?.headers).get('authorization')).toBe(`Bearer ${ACCESS}`)
      if (limited()) return Response.json({ error: 'slow down' }, { status: 429 })
      return Response.json({ user: { id: 'user_OWNER', email: 'owner@example.com' }, teams: [], devices: [] })
    }),
  )
  return asked
}

describe("the relay's per-account limit, through the Worker", () => {
  it('shows the limited page on GET /, and the shell again once the relay answers', async () => {
    let limited = true
    const asked = relayLimiting(() => limited)
    const refused = await get('/', envWith(), { headers: { cookie: SESSION } })
    expect(refused.status).toBe(429)
    expect(refused.headers.get('retry-after')).toBe('60')
    expect(await refused.text()).toContain('Too many attempts')
    // Asked once, and no refresh: a limit is not an expired session.
    expect(asked).toEqual(['/v1/directory/me'])
    expect(refused.headers.get('set-cookie')).toBeNull()

    limited = false
    const served = await get('/', envWith(), { headers: { cookie: SESSION } })
    expect(served.status).toBe(200)
    expect(await served.text()).toBe(SHELL)
  })

  it("answers the page's /api/relay/directory/me with 429 limited", async () => {
    relayLimiting(() => true)
    const res = await get('/api/relay/directory/me', envWith(), { method: 'POST', headers: { 'x-fc': '1', origin: ORIGIN, cookie: SESSION } })
    expect(res.status).toBe(429)
    expect(res.headers.get('retry-after')).toBe('60')
    expect(await res.json()).toEqual({ error: 'limited' })
  })
})
