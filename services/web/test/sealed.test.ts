// A phone enrolls a browser with no camera (src/client/sealed.ts, ov-771): the
// wait for the sealed reply, the code, and the directory registration. Every
// relay, wasm and browser call is played by the test, and time is a number.

import { describe, expect, it } from 'vitest'

import type { Signed } from '../src/client/device'
import { type Answer, confirmCodes, type Opened, POLL_MS, type PollDeps, type RegisterDeps, registerDevice, WAIT_MS, waitForReply } from '../src/client/sealed'

const CEREMONY = '0123456789abcdef0123456789abcdef'
const GOOD: Opened = { code: 'K7M2QX', granted_by: null, runners: [{ id: 'r1', label: 'studio', user: 'owner', host_key: 'SHA256:real', token: 't', why: '' }] }
const FORGED: Opened = { code: 'ZZ99AB', granted_by: null, runners: [{ ...GOOD.runners[0], host_key: 'SHA256:forged' }] }

function poller(answers: (Answer | 'throws')[], opens: (Opened | Error)[] = [GOOD]) {
  let clock = 0
  const log: string[] = []
  const refused: string[] = []
  let reads = 0
  let cancelled = false
  const deps: PollDeps = {
    read: async ceremony => {
      log.push(`read ${ceremony} at ${clock}`)
      const next = answers[Math.min(reads++, answers.length - 1)]
      if (next === 'throws') throw new Error('offline')
      return next
    },
    open: json => {
      log.push(`open ${json}`)
      const next = opens.shift() ?? new Error('unopened')
      if (next instanceof Error) throw next
      return next
    },
    now: () => clock,
    sleep: async ms => void (clock += ms),
    cancelled: () => cancelled,
    refused: word => void refused.push(word),
  }
  return { deps, log, refused, cancel: () => void (cancelled = true), clock: () => clock }
}

const none: Answer = { status: 404, body: { error: 'not found' } }
const replies = (...sealed: string[]): Answer => ({ status: 200, body: { replies: sealed.map(s => ({ sealed: s, posted_by: 'user_OWNER' })) } })

describe('waiting for the sealed reply', () => {
  it('asks every two seconds and opens the relay answer as it came', async () => {
    const t = poller([none, none, replies('AAAA', 'BBBB')])
    const waited = await waitForReply(t.deps, CEREMONY)
    expect(waited).toEqual({ kind: 'opened', opened: GOOD })
    expect(t.log.filter(l => l.startsWith('read'))).toEqual([`read ${CEREMONY} at 0`, `read ${CEREMONY} at ${POLL_MS}`, `read ${CEREMONY} at ${2 * POLL_MS}`])
    // Both replies go to wasm in the relay's order: it takes the earliest, not this.
    expect(t.log.at(-1)).toBe(`open ${JSON.stringify({ replies: [{ sealed: 'AAAA', posted_by: 'user_OWNER' }, { sealed: 'BBBB', posted_by: 'user_OWNER' }] })}`)
  })

  it('gives up after the 120 s window', async () => {
    const t = poller([none])
    expect(await waitForReply(t.deps, CEREMONY)).toEqual({ kind: 'timeout' })
    expect(t.clock()).toBe(WAIT_MS)
    expect(t.log.filter(l => l.startsWith('read'))).toHaveLength(WAIT_MS / POLL_MS)
  })

  it('goes on after a dropped request or a reply that did not open, and says why', async () => {
    const t = poller(['throws', replies('JUNK'), replies('REAL')], [new Error('unopened'), GOOD])
    expect(await waitForReply(t.deps, CEREMONY)).toEqual({ kind: 'opened', opened: GOOD })
    expect(t.refused).toEqual(['unopened'])
  })

  it('stops for a code past its window or one already taken', async () => {
    for (const word of ['stale', 'already_taken'] as const) {
      const t = poller([replies('AAAA')], [new Error(word)])
      expect(await waitForReply(t.deps, CEREMONY)).toEqual({ kind: 'ended', word })
      expect(t.refused).toEqual([])
    }
  })

  it('stops when the offer on screen is gone, without opening what it read', async () => {
    const t = poller([replies('AAAA')])
    t.cancel()
    expect(await waitForReply(t.deps, CEREMONY)).toEqual({ kind: 'cancelled' })
    expect(t.log).toEqual([])
    // Cancelled while a read was in flight: the reply is not opened either.
    const late = poller([replies('AAAA')])
    const read = late.deps.read
    late.deps.read = async c => {
      late.cancel()
      return await read(c)
    }
    expect(await waitForReply(late.deps, CEREMONY)).toEqual({ kind: 'cancelled' })
    expect(late.log.some(l => l.startsWith('open'))).toBe(false)
  })
})

describe('the comparison code', () => {
  it('keeps the reply only when the person says the codes match', async () => {
    const kept: Opened[] = []
    expect(await confirmCodes(GOOD, true, async o => void kept.push(o))).toBe(true)
    expect(kept).toEqual([GOOD])
  })

  it('"They don\'t match" keeps nothing, for a forged reply or an honest one', async () => {
    const kept: Opened[] = []
    for (const opened of [FORGED, GOOD]) expect(await confirmCodes(opened, false, async o => void kept.push(o))).toBe(false)
    expect(kept).toEqual([])
  })
})

const SIGNED: Signed = { authenticatorData: new Uint8Array([1]).buffer, clientDataJSON: new Uint8Array([2]).buffer, signature: new Uint8Array([3]).buffer }
const CHALLENGE = 'ab'.repeat(32)

function registrar(answers: Record<string, Answer | 'throws'>, sign: () => Promise<Signed> = async () => SIGNED) {
  const log: string[] = []
  const bodies: Record<string, any> = {}
  const deps: RegisterDeps = {
    call: async (route, body) => {
      log.push(route)
      bodies[route] = body
      const answer = answers[route]
      if (!answer || answer === 'throws') throw new Error('offline')
      return answer
    },
    sign: async data => {
      log.push(`sign ${Array.from(data).join(',')}`)
      return await sign()
    },
    data: (account, challenge, key) => new Uint8Array([account.length, challenge.length, key.length]),
    proof: (account, challenge, key) => `proof for ${account} ${challenge.slice(0, 4)} ${key}`,
  }
  return { deps, log, bodies }
}

describe('registering this browser with the directory', () => {
  const KEY = 'sk-ecdsa-sha2-nistp256@openssh.com AAAAKEY farcooler-web'
  const good = { 'directory/devices/challenge': { status: 200, body: { challenge: CHALLENGE } }, 'directory/devices/register': { status: 200, body: { device: {} } } }

  it('mints a challenge, signs its data with one touch, and registers the proof for the bare key', async () => {
    const t = registrar(good)
    expect(await registerDevice(t.deps, 'user_OWNER', KEY, 'Chrome on Mac')).toEqual({ ok: true })
    expect(t.log).toEqual(['directory/devices/challenge', 'sign 10,64,42', 'directory/devices/register'])
    expect(t.bodies['directory/devices/register']).toEqual({
      label: 'Chrome on Mac',
      challenge: CHALLENGE,
      keyA: 'sk-ecdsa-sha2-nistp256@openssh.com AAAAKEY',
      proofA: 'proof for user_OWNER abab sk-ecdsa-sha2-nistp256@openssh.com AAAAKEY',
    })
  })

  it('registers nothing when the person declines the touch', async () => {
    const t = registrar(good, async () => Promise.reject(new Error('declined')))
    expect(await registerDevice(t.deps, 'user_OWNER', KEY, 'x')).toEqual({ ok: false, word: 'declined' })
    expect(t.log).not.toContain('directory/devices/register')
  })

  it('says unreachable for no challenge or a relay that is down, and claimed or revoked for a key that is', async () => {
    expect(await registerDevice(registrar({ 'directory/devices/challenge': 'throws' }).deps, 'a', KEY, 'x')).toEqual({ ok: false, word: 'unreachable' })
    expect(await registerDevice(registrar({ ...good, 'directory/devices/register': { status: 502, body: {} } }).deps, 'a', KEY, 'x')).toEqual({ ok: false, word: 'unreachable' })
    expect(await registerDevice(registrar({ ...good, 'directory/devices/register': { status: 409, body: { error: 'claimed' } } }).deps, 'a', KEY, 'x')).toEqual({ ok: false, word: 'claimed' })
    expect(await registerDevice(registrar({ ...good, 'directory/devices/register': { status: 409, body: { error: 'revoked' } } }).deps, 'a', KEY, 'x')).toEqual({ ok: false, word: 'revoked' })
    expect(await registerDevice(registrar({ ...good, 'directory/devices/register': { status: 403, body: { error: 'proofA' } } }).deps, 'a', KEY, 'x')).toEqual({ ok: false, word: 'refused' })
  })
})
