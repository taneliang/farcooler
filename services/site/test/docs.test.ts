import { env } from 'cloudflare:test'
import { describe, expect, it } from 'vitest'

import worker, { type Env } from '../src/index'

// The docs pages (public/docs), served by the real asset handler that
// vitest.config.ts points at public/, through the Worker as www serves them.
// The pages repeat their header, list of pages and pager by hand, so this is
// what notices a page that isn't served, a title that drifted, or a link,
// image or anchor that leads nowhere.

const files = import.meta.glob('../public/docs/*.html', { query: '?raw', import: 'default', eager: true }) as Record<string, string>
const home = (import.meta.glob('../public/index.html', { query: '?raw', import: 'default', eager: true }) as Record<string, string>)['../public/index.html']

/** Each docs page by the path it's served at: `index.html` at /docs/, `x.html` at /docs/x. */
const pages = Object.entries(files).map(([file, html]) => {
  const name = file.split('/').pop()!.replace(/\.html$/, '')
  return { path: name === 'index' ? '/docs/' : `/docs/${name}`, html }
})

const TITLES: Record<string, string> = {
  '/docs/': 'Far Cooler Docs',
  '/docs/getting-started': 'Getting Started – Far Cooler',
  '/docs/orchestrator': 'The Orchestrator and the Plan – Far Cooler',
  '/docs/lanes': 'Lanes and Landing – Far Cooler',
  '/docs/needs-you': 'Needs You and Rulings – Far Cooler',
  '/docs/agents': 'Agents – Far Cooler',
  '/docs/phones': 'Phones and the Watch – Far Cooler',
  '/docs/teams': 'Teams – Far Cooler',
  '/docs/member-logins': 'Member Logins – Far Cooler',
  '/docs/trust': 'What Runs Where – Far Cooler',
}

/** Paths the Worker answers in code rather than from public/. */
const ROUTES = new Set(['/download/canary', '/app'])

const serve = (path: string) =>
  worker.fetch(new Request(`https://www.farcooler.com${path}`), { ASSETS: (env as unknown as { ASSETS: Fetcher }).ASSETS } as Env)

/** Every same-site reference on a page: links, image sources and stylesheets. */
function references(html: string): string[] {
  const refs = [...html.matchAll(/\s(?:href|src|srcset)="([^"]+)"/g)].map((m) => m[1])
  return refs.filter((r) => !/^[a-z]+:/i.test(r))
}

describe('docs pages', () => {
  it('are all here, and only these', () => {
    expect(pages.map((p) => p.path).sort()).toEqual(Object.keys(TITLES).sort())
  })

  it.each(Object.entries(TITLES))('serves %s with its title', async (path, title) => {
    const res = await serve(path)
    expect(res.status).toBe(200)
    expect(res.headers.get('content-type')).toContain('text/html')
    const html = await res.text()
    expect(html).toContain(`<title>${title}</title>`)
    expect(html).toContain('<link rel="stylesheet" href="/docs/docs.css">')
  })

  it('answers /docs, without the slash, with the docs home', async () => {
    const res = await serve('/docs')
    if (res.status === 200) {
      expect(await res.text()).toContain(`<title>${TITLES['/docs/']}</title>`)
    } else {
      expect([301, 307, 308]).toContain(res.status)
      expect(new URL(res.headers.get('location')!, 'https://www.farcooler.com').pathname).toBe('/docs/')
    }
  })

  it('serves its stylesheet', async () => {
    const res = await serve('/docs/docs.css')
    expect(res.status).toBe(200)
    expect(res.headers.get('content-type')).toContain('text/css')
  })

  it.each(pages.map((p) => [p.path, p.html]))('%s has no dead internal link, image or anchor', async (path, html) => {
    const refs = references(html)
    expect(refs.length).toBeGreaterThan(0)
    for (const ref of refs) {
      const url = new URL(ref, `https://www.farcooler.com${path}`)
      if (ROUTES.has(url.pathname)) continue
      const res = await serve(url.pathname)
      expect(res.status, `${path} links to ${ref}`).toBe(200)
      if (url.hash) {
        const target = url.pathname === path ? html : await res.text()
        expect(target, `${path} links to ${ref}`).toContain(`id="${url.hash.slice(1)}"`)
      } else {
        await res.arrayBuffer()
      }
    }
  })

  it.each(pages.map((p) => [p.path, p.html]))('%s lists every docs page, and marks itself', (path, html) => {
    for (const other of Object.keys(TITLES)) expect(html).toContain(`href="${other}"`)
    expect(html).toMatch(new RegExp(`href="${path.replace(/\//g, '\\/')}" aria-current="page"`))
  })

  it.each(pages.map((p) => [p.path, p.html]))('%s describes every image', (path, html) => {
    for (const img of html.match(/<img\b[^>]*>/g) ?? []) {
      expect(img, path).toMatch(/\salt="[^"]{20,}"/)
      expect(img, path).toMatch(/\swidth="\d+" height="\d+"/)
    }
  })
})

describe('the home page', () => {
  it('links to the docs from its bar and its hero', () => {
    expect(home.match(/href="\/docs\/"/g)?.length).toBe(2)
  })

  it('has no dead internal link to the docs', async () => {
    for (const ref of references(home).filter((r) => r.startsWith('/docs'))) {
      const res = await serve(new URL(ref, 'https://www.farcooler.com/').pathname)
      expect(res.status, ref).toBe(200)
      await res.arrayBuffer()
    }
  })
})
