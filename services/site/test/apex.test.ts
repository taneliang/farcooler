import { describe, expect, it } from 'vitest'

import worker, { type Env } from '../src/index'

const env = { ASSETS: { fetch: () => Promise.resolve(new Response('page')) } } as unknown as Env

describe('farcooler.com', () => {
  it('sends the bare domain to www, keeping the path and query', async () => {
    const res = await worker.fetch(new Request('http://farcooler.com/download/canary?from=x'), env)
    expect(res.status).toBe(301)
    expect(res.headers.get('location')).toBe('https://www.farcooler.com/download/canary?from=x')
  })

  it('serves www itself', async () => {
    const res = await worker.fetch(new Request('https://www.farcooler.com/'), env)
    expect(res.status).toBe(200)
  })
})
