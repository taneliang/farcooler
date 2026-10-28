import { env } from 'cloudflare:test'
import { beforeEach, describe, expect, it } from 'vitest'

import worker from '../src/index'
import { recordDailyActiveRunners } from '../src/usage-counts'

// Daily active runners (ov-504), against the real migration chain.
const DAY = 24 * 60 * 60 * 1000
const noon = Date.UTC(2026, 9, 28, 12)

async function runner(id: string, seen: number | null, install: string | null = null) {
  await env.DB.prepare(`INSERT INTO accounts (id, created_at) VALUES ('a', 0) ON CONFLICT DO NOTHING`).run()
  await env.DB.prepare(
    `INSERT INTO daemons (id, account_id, token_hash, label, created_at, last_seen_at, install_id) VALUES (?, 'a', ?, 'Mac', 0, ?, ?)`,
  ).bind(id, `h-${id}`, seen, install).run()
}
const rows = async () => (await env.DB.prepare(`SELECT * FROM daily_counts`).all()).results

beforeEach(async () => {
  await env.DB.prepare(`DELETE FROM daily_counts`).run()
  await env.DB.prepare(`DELETE FROM accounts`).run()
})

describe('daily active runners', () => {
  it('counts runners seen today, once per install, and not yesterday or never', async () => {
    await runner('r1', noon - 1000)
    await runner('r2', noon - 2000, 'inst')
    await runner('r3', noon - 3000, 'inst')
    await runner('old', noon - DAY)
    await runner('never', null)
    await recordDailyActiveRunners(env as any, noon)
    expect(await rows()).toEqual([{ day: '2026-10-28', metric: 'runner.active', label: '', value: 2 }])
  })

  it('is idempotent and grows through the day', async () => {
    await runner('r1', noon - 1000)
    await recordDailyActiveRunners(env as any, noon)
    await recordDailyActiveRunners(env as any, noon + 1000)
    await runner('r2', noon + 500)
    await recordDailyActiveRunners(env as any, noon + 2000)
    expect((await rows()).map((r: any) => r.value)).toEqual([2])
  })

  it('stores nothing that names an account, device or runner', async () => {
    await runner('r1', noon - 1000)
    await recordDailyActiveRunners(env as any, noon)
    const cols = (await env.DB.prepare(`PRAGMA table_info(daily_counts)`).all()).results.map((c: any) => c.name)
    expect(cols).toEqual(['day', 'metric', 'label', 'value'])
    expect(JSON.stringify(await rows())).not.toMatch(/r1|"a"|h-r1/)
  })

  it('runs on the cron', async () => {
    await runner('r1', Date.now())
    await worker.scheduled({} as any, env as any)
    expect(((await rows()) as any[])[0].value).toBe(1)
  })
})
