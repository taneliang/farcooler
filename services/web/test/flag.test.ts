import { afterEach, describe, expect, it, vi } from 'vitest'

import { envWith, get, relay, SESSION, SHELL } from './helpers'

// Who gets the client shell (src/flag.ts), and what everyone else sees: a page
// with no script on it at all.

afterEach(() => vi.unstubAllGlobals())

const signedIn = { headers: { cookie: SESSION } }

async function notAvailable(res: Response) {
  const html = await res.text()
  expect(html).toContain('Not available yet')
  expect(html).not.toMatch(/<script/i)
  expect(res.headers.get('content-security-policy')).toContain("default-src 'none'")
  expect(res.headers.get('content-security-policy')).not.toContain('script-src')
  return html
}

describe('the flag', () => {
  it('gives the owner the shell', async () => {
    relay()
    const res = await get('/', envWith(), signedIn)
    expect(res.status).toBe(200)
    expect(await res.text()).toBe(SHELL)
  })

  it("gives a signed-in account that isn't listed Not available yet, with a Sign Out and no script", async () => {
    relay('user_SOMEONE_ELSE')
    const res = await get('/', envWith(), signedIn)
    expect(res.status).toBe(200)
    expect(await notAvailable(res)).toContain('Sign Out')
  })

  it('is off, for everyone and with no sign-in, when WEB_CLIENT is empty: as app.farcooler.com is', async () => {
    const calls = relay()
    const env = envWith({ WEB_CLIENT: '', WEB_ACCOUNTS: '' })
    for (const path of ['/', '/sign-in', '/callback?code=x&state=y']) {
      const res = await get(path, env, signedIn)
      const html = await notAvailable(res)
      expect(html, path).not.toContain('Sign In')
      expect(res.headers.get('location'), path).toBeNull()
    }
    expect((await get('/api/relay/directory/me', env, { method: 'POST', headers: { cookie: SESSION, 'x-fc': '1', origin: 'https://app.canary.farcooler.com' } })).status).toBe(404)
    // Off means the relay is never asked anything.
    expect(calls).toEqual([])
  })

  it('fails closed on any value but "owner" and "everyone", and on an empty account list', async () => {
    for (const over of [{ WEB_CLIENT: 'Owner' }, { WEB_CLIENT: 'Everyone' }, { WEB_CLIENT: 'everyone ' }, { WEB_CLIENT: ' everyone' }, { WEB_CLIENT: 'EVERYONE' }, { WEB_CLIENT: 'owner ' }, { WEB_CLIENT: undefined }, { WEB_ACCOUNTS: '' }, { WEB_ACCOUNTS: ' , ' }]) {
      relay()
      await notAvailable(await get('/', envWith(over), signedIn))
    }
  })

  it('admits any signed-in account for "everyone", listed or not, with an empty list too', async () => {
    for (const accounts of ['', 'user_OWNER', 'user_SOMEONE_ELSE_ENTIRELY']) {
      relay('user_STRANGER')
      const res = await get('/', envWith({ WEB_CLIENT: 'everyone', WEB_ACCOUNTS: accounts }), signedIn)
      expect(await res.text(), accounts).toBe(SHELL)
    }
  })

  it('still requires sign-in for "everyone"', async () => {
    const calls = relay()
    const env = envWith({ WEB_CLIENT: 'everyone', WEB_ACCOUNTS: '' })
    const html = await (await get('/', env)).text()
    expect(html).toContain('href="/sign-in"')
    expect(html).not.toMatch(/<script/i)
    expect(calls).toEqual([])
    // The API stays closed without a session, and for an account the relay can't name.
    const api = { method: 'POST', headers: { 'x-fc': '1', origin: 'https://app.canary.farcooler.com' } }
    expect((await get('/api/relay/directory/me', env, api)).status).not.toBe(200)
    relay('')
    await notAvailable(await get('/', env, signedIn))
  })

  it('refuses "everyone" when the relay returns an empty, missing or non-string user id', async () => {
    const env = envWith({ WEB_CLIENT: 'everyone', WEB_ACCOUNTS: '' })
    const api = { method: 'POST', headers: { cookie: SESSION, 'x-fc': '1', origin: 'https://app.canary.farcooler.com' } }
    for (const user of [{ id: '' }, {}, { id: 7 }, { id: null }]) {
      vi.stubGlobal('fetch', vi.fn(async () => Response.json({ user, teams: [], devices: [] })))
      await notAvailable(await get('/', env, signedIn))
      expect((await get('/api/relay/directory/me', env, api)).status, JSON.stringify(user)).toBe(404)
    }
  })

  it('reads the account list split on commas and spaces, and matches ids exactly', async () => {
    relay()
    expect(await (await get('/', envWith({ WEB_ACCOUNTS: 'user_A, user_OWNER user_B' }), signedIn)).text()).toBe(SHELL)
    relay()
    await notAvailable(await get('/', envWith({ WEB_ACCOUNTS: 'user_OWNER2' }), signedIn))
    relay()
    await notAvailable(await get('/', envWith({ WEB_ACCOUNTS: 'user_OWNE' }), signedIn))
  })

  it('shows sign-in to a signed-out visitor while the flag is on, and never the shell', async () => {
    const calls = relay()
    const res = await get('/', envWith())
    const html = await res.text()
    expect(html).toContain('href="/sign-in"')
    expect(html).not.toMatch(/<script/i)
    expect(calls).toEqual([])
  })

  it('never serves the shell by its file name', async () => {
    relay()
    for (const path of ['/index.html', '/a/index.html', '/a/../index.html', '/version.json/../index.html']) {
      const res = await get(path, envWith(), signedIn)
      expect(await res.text(), path).not.toBe(SHELL)
    }
  })

  it('answers no host but its own, so *.workers.dev and preview URLs serve nothing', async () => {
    relay()
    for (const origin of ['https://farcooler-web-canary.someone.workers.dev', 'https://abc123-farcooler-web-canary.someone.workers.dev', 'https://app.farcooler.com', 'http://app.canary.farcooler.com']) {
      const res = await get('/', envWith(), signedIn, origin)
      expect(res.status, origin).toBe(404)
      expect(await res.text(), origin).toBe('')
    }
    expect((await get('/', envWith({ APP_HOST: '' }), signedIn)).status).toBe(404)
  })
})
