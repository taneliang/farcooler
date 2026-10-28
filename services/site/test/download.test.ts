import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import worker, { type Env } from '../src/index'

const FEED = 'https://updates.farcooler.com/canary/appcast.xml'
const dmg = (n: number) => `https://updates.farcooler.com/canary/Far%20Cooler-${n}.dmg`

function item(url: string, version: number): string {
  return `<item><enclosure url="${url}" length="1" type="application/octet-stream" sparkle:version="${version}" sparkle:shortVersionString="0.1.0"/></item>`
}
const feed = (...items: string[]) => `<?xml version="1.0"?><rss><channel>${items.join('')}</channel></rss>`

const env = { APPCAST_URL: FEED, ASSETS: { fetch: async () => new Response('page') } } as unknown as Env

/** Serve `body` (or a failure) for the appcast. */
function appcastIs(body: string | 'down' | 500) {
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL) => {
      expect(String(input)).toBe(FEED)
      if (body === 'down') throw new Error('network')
      if (body === 500) return new Response('oops', { status: 500 })
      return new Response(body, { status: 200 })
    }),
  )
}

const get = () => worker.fetch(new Request('https://www.farcooler.com/download/canary'), env)

beforeEach(async () => {
  await (await caches.open('site-download')).delete(new Request('https://last-good.farcooler.invalid/canary'))
})
afterEach(() => vi.unstubAllGlobals())

describe('/download/canary', () => {
  it('redirects to the newest build, whatever order the feed lists them in', async () => {
    appcastIs(feed(item(dmg(3077), 3077), item(dmg(3079), 3079), item(dmg(3078), 3078)))
    const res = await get()
    expect(res.status).toBe(302)
    expect(res.headers.get('location')).toBe(dmg(3079))
    expect(res.headers.get('cache-control')).toBe('public, max-age=60')
  })

  it('compares build numbers as numbers: 10000 is newer than 9999', async () => {
    appcastIs(feed(item(dmg(9999), 9999), item(dmg(10000), 10000)))
    expect((await get()).headers.get('location')).toBe(dmg(10000))
  })

  it.each([
    ['another host', 'https://evil.example/canary/Far%20Cooler-9999.dmg'],
    ['a lookalike subdomain', 'https://updates.farcooler.com.evil.example/canary/x.dmg'],
    ['userinfo that spoofs the host', 'https://updates.farcooler.com@evil.example/canary/x.dmg'],
    ['plain http', 'http://updates.farcooler.com/canary/x.dmg'],
    ['another port', 'https://updates.farcooler.com:8443/canary/x.dmg'],
    ['a different folder', 'https://updates.farcooler.com/stable/x.dmg'],
    ['a path that climbs out of the folder', 'https://updates.farcooler.com/canary/../stable/x.dmg'],
    ['something that is not a disk image', 'https://updates.farcooler.com/canary/x.zip'],
    ['a javascript: URL', 'javascript:alert(1)'],
    ['no URL at all', ''],
  ])('refuses %s, even as the newest build', async (_what, bad) => {
    appcastIs(feed(item(dmg(3078), 3078), item(bad, 9999)))
    const res = await get()
    // No redirect to the bad URL, and none to the older good one either:
    // there is nothing remembered, so this is the friendly error.
    expect(res.status).toBe(503)
    expect(res.headers.get('location')).toBeNull()
  })

  it('falls back to the last build it read when the appcast is down', async () => {
    appcastIs(feed(item(dmg(3078), 3078)))
    expect((await get()).headers.get('location')).toBe(dmg(3078))
    for (const failure of ['down', 500, 'not xml at all'] as const) {
      appcastIs(failure)
      const res = await get()
      expect(res.status).toBe(302)
      expect(res.headers.get('location')).toBe(dmg(3078))
    }
  })

  it('does not fall back to a remembered build when the new feed is hostile', async () => {
    appcastIs(feed(item(dmg(3078), 3078)))
    await get()
    appcastIs(feed(item('https://evil.example/canary/x.dmg', 9999)))
    // Falls back to the REMEMBERED good URL, never the hostile one.
    expect((await get()).headers.get('location')).toBe(dmg(3078))
  })

  it('shows a friendly error, not a redirect, with nothing to fall back to', async () => {
    appcastIs('down')
    const res = await get()
    expect(res.status).toBe(503)
    expect(res.headers.get('retry-after')).toBe('120')
    expect(res.headers.get('location')).toBeNull()
    expect(await res.text()).toContain("The download isn't available right now.")
  })

  it('answers only GET and HEAD', async () => {
    appcastIs(feed(item(dmg(1), 1)))
    const res = await worker.fetch(new Request('https://www.farcooler.com/download/canary', { method: 'POST' }), env)
    expect(res.status).toBe(405)
  })
})

describe('download counting (ov-504)', () => {
  const rows: any[] = []
  const counted = { ...env, DOWNLOADS: { writeDataPoint: (e: any) => rows.push(e) } } as unknown as Env
  beforeEach(() => { rows.length = 0 })

  it('writes one row holding the build and nothing else per GET redirect', async () => {
    appcastIs(feed(item(dmg(3079), 3079)))
    await worker.fetch(new Request('https://www.farcooler.com/download/canary', { headers: { 'user-agent': 'Secret/1', 'cf-connecting-ip': '1.2.3.4' } }), counted)
    expect(rows).toEqual([{ indexes: ['download.click'], blobs: ['download.click', '3079'], doubles: [1] }])
  })

  it('counts nothing for HEAD or a 503', async () => {
    appcastIs(feed(item(dmg(3079), 3079)))
    await worker.fetch(new Request('https://www.farcooler.com/download/canary', { method: 'HEAD' }), counted)
    await (await caches.open('site-download')).delete(new Request('https://last-good.farcooler.invalid/canary'))
    appcastIs('down')
    await worker.fetch(new Request('https://www.farcooler.com/download/canary'), counted)
    expect(rows).toEqual([])
  })

  it('still redirects when the counter throws', async () => {
    appcastIs(feed(item(dmg(3079), 3079)))
    const broken = { ...env, DOWNLOADS: { writeDataPoint: () => { throw new Error('x') } } } as unknown as Env
    expect((await worker.fetch(new Request('https://www.farcooler.com/download/canary'), broken)).status).toBe(302)
  })
})
