import { describe, expect, it } from 'vitest'

import siteToml from '../wrangler.toml?raw'
import relayToml from '../../relay/wrangler.toml?raw'
import worker, { type Env } from '../src/index'

// One site per relay channel (ov-521): www is production, and canary and
// preview serve the web app alone, sending everything else to www.

const ASSETS = { fetch: async () => new Response('marketing page') } as unknown as Fetcher
const www = { APPCAST_URL: 'x', ASSETS } as Env
const canary = { ...www, MARKETING_ORIGIN: 'https://www.farcooler.com' } as Env

describe('the canary and preview sites', () => {
  it('send the marketing page and anything else outside /app to www, path and query kept', async () => {
    for (const [path, to] of [
      ['/', 'https://www.farcooler.com/'],
      ['/pricing?from=x', 'https://www.farcooler.com/pricing?from=x'],
      ['/apple', 'https://www.farcooler.com/apple'],
      ['/downloads', 'https://www.farcooler.com/downloads'],
    ]) {
      const res = await worker.fetch(new Request(`https://canary.farcooler.com${path}`), canary)
      expect(res.status, path).toBe(302)
      expect(res.headers.get('location'), path).toBe(to)
    }
  })

  it('serve /app and /download themselves', async () => {
    // Unconfigured here, so /app answers "Not available yet" rather than a redirect.
    expect((await worker.fetch(new Request('https://canary.farcooler.com/app'), canary)).status).toBe(503)
    expect((await worker.fetch(new Request('https://canary.farcooler.com/app/teams/x'), canary)).status).toBe(503)
    const download = await worker.fetch(new Request('https://canary.farcooler.com/download/canary', { method: 'POST' }), canary)
    expect(download.status).toBe(405)
  })

  it('leave www serving the marketing page', async () => {
    const res = await worker.fetch(new Request('https://www.farcooler.com/'), www)
    expect(res.status).toBe(200)
    expect(await res.text()).toBe('marketing page')
  })
})

/// `[vars]` and rate-limit namespaces per environment, top level as `production`.
function blocks(toml: string, top: string): { vars: Record<string, Record<string, string>>; namespaces: string[] } {
  const vars: Record<string, Record<string, string>> = {}
  const namespaces: string[] = []
  let current: Record<string, string> | null = null
  for (const line of toml.split('\n')) {
    const header = line.match(/^\[(?:env\.([A-Za-z0-9_]+)\.)?vars\]\s*$/)
    if (header) current = vars[header[1] ?? top] = {}
    else if (line.startsWith('[')) current = null
    else {
      const pair = line.match(/^([A-Z_]+)\s*=\s*"([^"]*)"\s*$/)
      if (current && pair) current[pair[1]] = pair[2]
    }
    const namespace = line.match(/^namespace_id\s*=\s*"(\d+)"/)
    if (namespace) namespaces.push(namespace[1])
  }
  return { vars, namespaces }
}

describe("each channel's site in wrangler.toml", () => {
  const site = blocks(siteToml, 'production')
  const relay = blocks(relayToml, 'stable')
  const pairs: [string, string, string][] = [
    ['production', 'stable', 'https://relay.farcooler.com'],
    ['preview', 'preview', 'https://relay-preview.farcooler.com'],
    ['canary', 'canary', 'https://relay-canary.farcooler.com'],
  ]

  it("reads that channel's relay and signs in through that relay's own WorkOS client", () => {
    expect(Object.keys(site.vars).sort()).toEqual(['canary', 'preview', 'production'])
    for (const [channel, relayChannel, host] of pairs) {
      const vars = site.vars[channel]
      expect(vars.APP_RELAY_URL, channel).toBe(host)
      expect(vars.APP_WORKOS_CLIENT_ID, channel).toBe(relay.vars[relayChannel].WORKOS_CLIENT_ID)
      expect(relayToml, channel).toContain(`pattern = "${new URL(host).hostname}"`)
    }
  })

  it('keeps the marketing page on www alone', () => {
    expect(site.vars.production.MARKETING_ORIGIN).toBeUndefined()
    expect(site.vars.preview.MARKETING_ORIGIN).toBe('https://www.farcooler.com')
    expect(site.vars.canary.MARKETING_ORIGIN).toBe('https://www.farcooler.com')
  })

  it('gives every rate limit its own namespace, apart from the relay', () => {
    const all = [...site.namespaces, ...relay.namespaces]
    expect(site.namespaces).toHaveLength(3)
    expect(new Set(all).size).toBe(all.length)
  })
})
