import { describe, expect, it } from 'vitest'

import worker, { type Env } from '../src/index'

const TOKEN = '0123456789abcdef0123456789abcdef'
const page = (headers: Record<string, string> = { 'content-type': 'text/html; charset=utf-8' }) =>
  ({ fetch: async () => new Response('<!doctype html><html><body><p>hi</p></body></html>', { headers }) }) as unknown as Fetcher

async function render(token: string | undefined, assets = page()) {
  const env = { APPCAST_URL: 'x', ASSETS: assets, ANALYTICS_TOKEN: token } as Env
  return worker.fetch(new Request('https://www.farcooler.com/'), env)
}

describe('analytics beacon', () => {
  it('is rendered, with the token, when one is set', async () => {
    const html = await (await render(TOKEN)).text()
    expect(html).toContain('static.cloudflareinsights.com/beacon.min.js')
    expect(html).toContain(`data-cf-beacon='{"token": "${TOKEN}"}'`)
    expect(html.indexOf('beacon.min.js')).toBeGreaterThan(html.indexOf('<p>hi</p>'))
  })

  it.each([undefined, '', '   '])('is absent when the token is %j', async (token) => {
    expect(await (await render(token)).text()).not.toContain('cloudflareinsights')
  })

  it('is absent, not injected, when the token is not a token', async () => {
    const html = await (await render(`'}"><script>alert(1)</script>`)).text()
    expect(html).not.toContain('cloudflareinsights')
    expect(html).not.toContain('alert(1)')
  })

  it('leaves non-HTML assets alone', async () => {
    const res = await render(TOKEN, page({ 'content-type': 'image/webp' }))
    expect(await res.text()).not.toContain('cloudflareinsights')
  })
})
