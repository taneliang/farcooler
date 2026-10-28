import { afterEach, describe, expect, it, vi } from 'vitest'

import { ROUTES } from '../src/api'
import { ACCESS, envWith, get, ORIGIN, OWNER, relay, SESSION } from './helpers'

// /api/relay: the page's only way to the relay (src/api.ts). Each of its
// three checks has a case here that goes red without it.

afterEach(() => vi.unstubAllGlobals())

const good = { 'x-fc': '1', origin: ORIGIN, cookie: SESSION }
const post = (path: string, headers: Record<string, string>, body?: string) => get(path, envWith(), { method: 'POST', headers, body })

describe('/api/relay', () => {
  it("forwards an allowed route with the session's bearer, and answers the relay's body", async () => {
    const calls = relay()
    const res = await post('/api/relay/directory/me', good)
    expect(res.status).toBe(200)
    expect((await res.json<any>()).user.id).toBe('user_OWNER')
    expect(calls.map(c => c.path)).toEqual(['/v1/directory/me'])
    expect(res.headers.get('cache-control')).toBe('no-store')
  })

  it('refuses a request without x-fc: 1, before asking the relay anything', async () => {
    for (const value of [undefined, '0', 'true']) {
      const calls = relay()
      const headers: Record<string, string> = { origin: ORIGIN, cookie: SESSION }
      if (value) headers['x-fc'] = value
      expect((await post('/api/relay/directory/me', headers)).status, String(value)).toBe(403)
      expect(calls).toEqual([])
    }
  })

  it("refuses an Origin that isn't exactly this host, or none", async () => {
    for (const origin of [undefined, 'null', 'https://www.farcooler.com', 'https://app.farcooler.com', `${ORIGIN}.evil.com`, 'http://app.canary.farcooler.com']) {
      const calls = relay()
      const headers: Record<string, string> = { 'x-fc': '1', cookie: SESSION }
      if (origin) headers.origin = origin
      expect((await post('/api/relay/directory/me', headers)).status, String(origin)).toBe(403)
      expect(calls).toEqual([])
    }
  })

  it('forwards nothing off the allowlist, however the path is spelled', async () => {
    expect([...ROUTES]).toEqual(['directory/me', 'directory/devices/challenge', 'directory/devices/register', 'ceremony/reply'])
    for (const path of ['directory/teams/members', 'ceremony/reply/0123456789abcdef0123456789abcdef', 'ceremony', 'directory/devices/revoke', 'directory/devices/certify', 'account/signout-all', 'auth/token', 'directory/me/', 'directory/me/../teams', 'directory//me', '../v1/directory/me']) {
      const calls = relay()
      const res = await post(`/api/relay/${path}`, good)
      // 404, or 405 for a path that normalized out of /api/relay/.
      expect([404, 405], path).toContain(res.status)
      expect(calls, path).toEqual([])
    }
  })

  it('takes POST only', async () => {
    relay()
    expect((await get('/api/relay/directory/me', envWith(), { headers: good })).status).toBe(405)
  })

  it('answers 401 signed out, and 404 to an account the flag does not admit', async () => {
    relay()
    expect((await post('/api/relay/directory/me', { 'x-fc': '1', origin: ORIGIN })).status).toBe(401)
    relay('user_SOMEONE_ELSE')
    expect((await post('/api/relay/directory/me', good)).status).toBe(404)
  })

  // ov-771: the sealed reply is read through the Worker, as a GET at the relay.
  describe('the sealed reply (ceremony/reply)', () => {
    const ceremony = '0123456789abcdef0123456789abcdef'
    const read = (body: unknown) => post('/api/relay/ceremony/reply', { ...good, 'content-type': 'application/json' }, JSON.stringify(body))

    it("reads the relay's reply for that ceremony as a GET with the session's bearer", async () => {
      const calls = relay(OWNER, { [`/v1/ceremony/reply/${ceremony}`]: () => Response.json({ replies: [{ sealed: 'AAAA', posted_by: OWNER }] }) })
      const res = await read({ ceremony })
      expect(res.status).toBe(200)
      expect(await res.json()).toEqual({ replies: [{ sealed: 'AAAA', posted_by: OWNER }] })
      const asked = calls.find(c => c.path.startsWith('/v1/ceremony/'))
      expect(asked).toMatchObject({ path: `/v1/ceremony/reply/${ceremony}`, method: 'GET', authorization: `Bearer ${ACCESS}` })
      expect(asked?.body).toBeNull()
    })

    it('answers 404 while there is no reply, as the relay does', async () => {
      relay()
      expect((await read({ ceremony })).status).toBe(404)
    })

    it('never puts anything but a ceremony id in the relay path', async () => {
      for (const bad of ['../directory/me', `${ceremony}/../../me`, `${ceremony}?x=1`, ceremony.toUpperCase(), '', 7, null]) {
        const calls = relay()
        expect((await read({ ceremony: bad })).status, String(bad)).toBe(400)
        expect(calls.filter(c => c.path.startsWith('/v1/ceremony/')), String(bad)).toEqual([])
      }
    })
  })

  it('forwards the device challenge and registration to the directory, body and all', async () => {
    const calls = relay(OWNER, {
      '/v1/directory/devices/challenge': () => Response.json({ challenge: 'ab'.repeat(32), expiresAt: 1 }),
      '/v1/directory/devices/register': () => Response.json({ device: { id: 'd1' } }),
    })
    expect((await post('/api/relay/directory/devices/challenge', { ...good, 'content-type': 'application/json' }, '{}')).status).toBe(200)
    const registration = { label: 'Chrome on Mac', challenge: 'ab'.repeat(32), keyA: 'sk-ecdsa x', proofA: 'p' }
    expect((await post('/api/relay/directory/devices/register', { ...good, 'content-type': 'application/json' }, JSON.stringify(registration))).status).toBe(200)
    const sent = calls.filter(c => c.path.startsWith('/v1/directory/devices/'))
    expect(sent.map(c => [c.path, c.method])).toEqual([
      ['/v1/directory/devices/challenge', 'POST'],
      ['/v1/directory/devices/register', 'POST'],
    ])
    expect(sent[1].body).toEqual(registration)
  })
})
