import { describe, expect, it } from 'vitest'

import webToml from '../wrangler.toml?raw'
import relayToml from '../../relay/wrangler.toml?raw'
import siteToml from '../../site/wrangler.toml?raw'

// wrangler.toml: one Worker per channel, on its own host, and no second origin.

const uncommented = (toml: string) => toml.replace(/^\s*#.*$/gm, '')

/// `[vars]` per environment, top level as `top`, and the top-level keys.
function blocks(toml: string, top: string): { vars: Record<string, Record<string, string>>; topLevel: Record<string, string> } {
  const vars: Record<string, Record<string, string>> = {}
  const topLevel: Record<string, string> = {}
  let current: Record<string, string> | null = null
  let atTop = true
  for (const line of uncommented(toml).split('\n')) {
    const header = line.match(/^\[(?:env\.([A-Za-z0-9_]+)\.)?vars\]\s*$/)
    if (header) current = vars[header[1] ?? top] = {}
    else if (line.startsWith('[')) {
      current = null
      atTop = false
    } else {
      const pair = line.match(/^([A-Za-z_]+)\s*=\s*(.+?)\s*$/)
      if (pair && current) current[pair[1]] = pair[2].replace(/^"(.*)"$/, '$1')
      else if (pair && atTop) topLevel[pair[1]] = pair[2]
    }
  }
  return { vars, topLevel }
}

const namespaces = (toml: string) => [...uncommented(toml).matchAll(/^\s*namespace_id\s*=\s*"(\d+)"/gm)].map(m => m[1])

describe("the web client's wrangler.toml", () => {
  const web = blocks(webToml, 'production')
  const relay = blocks(relayToml, 'stable')

  it('turns off workers.dev and preview URLs, which canary inherits', () => {
    expect(web.topLevel.workers_dev).toBe('false')
    expect(web.topLevel.preview_urls).toBe('false')
    // An env that set either would override the top level.
    expect(uncommented(webToml).match(/workers_dev|preview_urls/g)).toHaveLength(2)
  })

  it('declares production and canary only, each on its own host and channel relay', () => {
    expect(Object.keys(web.vars).sort()).toEqual(['canary', 'production'])
    const pairs: [string, string, string, string][] = [
      ['production', 'stable', 'app.farcooler.com', 'https://relay.farcooler.com'],
      ['canary', 'canary', 'app.canary.farcooler.com', 'https://relay-canary.farcooler.com'],
    ]
    for (const [env, relayEnv, host, relayUrl] of pairs) {
      const vars = web.vars[env]
      expect(vars.APP_HOST, env).toBe(host)
      expect(vars.APP_RELAY_URL, env).toBe(relayUrl)
      expect(vars.APP_WORKOS_CLIENT_ID, env).toBe(relay.vars[relayEnv].WORKOS_CLIENT_ID)
      expect(webToml, env).toContain(`{ pattern = "${host}", custom_domain = true }`)
    }
  })

  it('keeps the client off on stable', () => {
    expect(web.vars.production.WEB_CLIENT).toBe('')
    expect(web.vars.production.WEB_ACCOUNTS).toBe('')
  })

  it('opens canary to everyone, with no account list', () => {
    expect(web.vars.canary.WEB_CLIENT).toBe('everyone')
    expect(web.vars.canary.WEB_ACCOUNTS).toBe('')
  })

  it("names Tailcat's relays one by one, never a wildcard over its domain", () => {
    for (const env of ['production', 'canary']) {
      const hosts = web.vars[env].DERP_HOSTS.split(/\s+/)
      expect(hosts, env).toEqual(['tc301a.ipn.dev', 'tc302a.ipn.dev', 'tc303a.ipn.dev', 'tc304a.ipn.dev'])
    }
  })

  it('gives each rate limit a namespace no other Worker holds', () => {
    const mine = namespaces(webToml)
    expect(mine).toHaveLength(2)
    const all = [...mine, ...namespaces(siteToml), ...namespaces(relayToml)]
    expect(new Set(all).size).toBe(all.length)
  })
})
