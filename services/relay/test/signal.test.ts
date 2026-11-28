import { env } from 'cloudflare:test'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import worker from '../src/index'
import { RUNNER_NAMESPACE, runnerStatement } from '../src/directory/runners'
import { messageOf } from '../src/signal'
import { STUN_URLS, TURN_TTL_S } from '../src/turn'
import { makeKey } from './sshsig-signer'

// WebRTC signaling (ov-1534): a browser with an enrolled device meets its
// runner at the runner's Durable Object. In workerd, against the real
// migration chain, with the runner's real WebSocket into a real object. The
// only fake is Cloudflare's TURN credential endpoint.

const signing = (await crypto.subtle.generateKey(
  { name: 'RSASSA-PKCS1-v1_5', modulusLength: 2048, publicExponent: new Uint8Array([1, 0, 1]), hash: 'SHA-256' },
  true,
  ['sign', 'verify'],
)) as CryptoKeyPair
const publicJwk = { ...(await crypto.subtle.exportKey('jwk', signing.publicKey)), kid: 'test-key' }

function base64Url(value: string | Uint8Array): string {
  const bytes = typeof value === 'string' ? new TextEncoder().encode(value) : value
  return btoa(String.fromCharCode(...bytes)).replaceAll('+', '-').replaceAll('/', '_').replaceAll('=', '')
}

async function sessionFor(user: string): Promise<string> {
  const now = Math.floor(Date.now() / 1000)
  const claims = { sub: user, iss: (env as any).WORKOS_ISSUER, client_id: (env as any).WORKOS_CLIENT_ID, sid: `s-${user}`, iat: now, exp: now + 3600 }
  const header = base64Url(JSON.stringify({ alg: 'RS256', kid: 'test-key' }))
  const payload = base64Url(JSON.stringify(claims))
  const signature = await crypto.subtle.sign('RSASSA-PKCS1-v1_5', signing.privateKey, new TextEncoder().encode(`${header}.${payload}`))
  return `${header}.${payload}.${base64Url(new Uint8Array(signature))}`
}

// MARK: - Cloudflare, faked

/// The fake TURN endpoint's account key and what it mints. The token is the
/// value that must never appear anywhere but the Authorization header.
const TURN_KEY_ID = 'turn-key-id-test'
const TURN_TOKEN = 'TURN-ACCOUNT-TOKEN-must-never-leave-the-worker'
const withTurn = { TURN_KEY_ID, TURN_KEY_API_TOKEN: TURN_TOKEN }

interface Minted {
  url: string
  authorization: string | null
  body: any
}
let minted: Minted[] = []
let turnReply: () => Response = () => new Response('{}')
let logged: string[] = []

beforeEach(() => {
  minted = []
  logged = []
  let n = 0
  turnReply = () => {
    n += 1
    return new Response(
      JSON.stringify({
        iceServers: {
          urls: ['stun:stun.cloudflare.com:3478', 'turn:turn.cloudflare.com:3478?transport=udp', 'turns:turn.cloudflare.com:5349?transport=tcp'],
          username: `minted-user-${n}`,
          credential: `minted-credential-${n}`,
        },
      }),
    )
  }
  vi.stubGlobal('fetch', async (input: any, init: any = {}) => {
    const url = typeof input === 'string' ? input : input.url
    if (url.includes('/sso/jwks/')) return new Response(JSON.stringify({ keys: [publicJwk] }))
    if (url.startsWith('https://rtc.live.cloudflare.com/')) {
      const headers = new Headers(init.headers)
      minted.push({ url, authorization: headers.get('authorization'), body: JSON.parse(init.body) })
      return turnReply()
    }
    return new Response('{}')
  })
  for (const level of ['log', 'info', 'warn', 'error', 'debug'] as const) {
    vi.spyOn(console, level).mockImplementation((...args: unknown[]) => {
      logged.push(args.map(a => (typeof a === 'string' ? a : JSON.stringify(a) ?? String(a))).join(' '))
    })
  }
})

const sockets: WebSocket[] = []
afterEach(() => {
  vi.restoreAllMocks()
  for (const ws of sockets.splice(0)) {
    try {
      ws.close(1000, 'test over')
    } catch {}
  }
})

// MARK: - Calling the relay

const RELAY = 'https://relay.test'
const ctx = { waitUntil() {}, passThroughOnException() {} } as never

async function call(path: string, body: unknown, bearer?: string, bindings: Record<string, unknown> = {}) {
  const response = await worker.fetch(
    new Request(`${RELAY}${path}`, { method: 'POST', headers: bearer ? { authorization: `Bearer ${bearer}` } : {}, body: JSON.stringify(body) }),
    { ...env, ...bindings } as never,
    ctx,
  )
  const text = await response.text()
  return { status: response.status, body: text ? JSON.parse(text) : null, text }
}

async function as(user: string, route: string, body: unknown = {}) {
  return await call(`/v1/directory/${route}`, body, await sessionFor(user))
}

/// A browser of `user`'s, signaling through the relay.
function browser(user: string, bindings: Record<string, unknown> = {}) {
  return {
    open: async (runner: string, device: string) => await call('/v1/rtc/open', { runner, device }, await sessionFor(user), bindings),
    send: async (runner: string, device: string, message: unknown) =>
      await call('/v1/rtc/send', { runner, device, message }, await sessionFor(user), bindings),
    poll: async (runner: string, session: string, after = 0, wait = 0) =>
      await call('/v1/rtc/poll', { runner, session, after, wait }, await sessionFor(user), bindings),
  }
}

/// A runner's end of its WebSocket, with everything it has heard.
interface Line {
  ws: WebSocket
  heard: any[]
  next(): Promise<any>
  send(message: unknown): void
}

async function connect(token: string): Promise<Line> {
  const response = await worker.fetch(
    new Request(`${RELAY}/v1/rtc/runner`, { headers: { upgrade: 'websocket', authorization: `Bearer ${token}` } }),
    env as never,
    ctx,
  )
  expect(response.status).toBe(101)
  const ws = response.webSocket!
  ws.accept()
  sockets.push(ws)
  const heard: any[] = []
  let read = 0
  let waiting: (() => void) | null = null
  ws.addEventListener('message', event => {
    heard.push(JSON.parse(event.data as string))
    waiting?.()
  })
  return {
    ws,
    heard,
    async next() {
      while (read >= heard.length) {
        await new Promise<void>((resolve, reject) => {
          const timer = setTimeout(() => reject(new Error(`runner heard nothing more (has ${JSON.stringify(heard)})`)), 2000)
          waiting = () => {
            clearTimeout(timer)
            waiting = null
            resolve()
          }
        })
      }
      return heard[read++]
    },
    send(message: unknown) {
      ws.send(JSON.stringify(message))
    },
  }
}

/// Let messages already sent cross the socket.
const settle = () => new Promise(resolve => setTimeout(resolve, 50))

// MARK: - The directory, set up the way the product sets it up

let installs = 0

/// A runner in a fresh team `user` administers, registered as the daemon
/// registers: the box signs.
async function runnerFor(user: string): Promise<{ id: string; token: string; team: string }> {
  const install = `install-${++installs}-${crypto.randomUUID().slice(0, 8)}`
  const team = (await as(user, 'teams/create', { name: `${user}'s team` })).body.team.id
  const box = await makeKey(`box-${install}`)
  const issuedAt = Date.now()
  const made = await as(user, 'runners/register', {
    team,
    install,
    name: 'Studio',
    runnerKey: box.line,
    issuedAt,
    proof: await box.sign(RUNNER_NAMESPACE, runnerStatement('register', user, team, install, RELAY, issuedAt)),
  })
  expect(made.status).toBe(200)
  return { id: made.body.runner, token: made.body.token, team: made.body.team }
}

/// Another runner, in `team`.
async function runnerIn(user: string, team: string): Promise<{ id: string; token: string }> {
  const install = `install-${++installs}-${crypto.randomUUID().slice(0, 8)}`
  const box = await makeKey(`box-${install}`)
  const issuedAt = Date.now()
  const made = await as(user, 'runners/register', {
    team,
    install,
    name: 'Second',
    runnerKey: box.line,
    issuedAt,
    proof: await box.sign(RUNNER_NAMESPACE, runnerStatement('register', user, team, install, RELAY, issuedAt)),
  })
  expect(made.status).toBe(200)
  return { id: made.body.runner, token: made.body.token }
}

/// `user` joins `team` by its admin's invite, as the product does it.
async function joinTeam(team: string, user: string, admin = 'alice') {
  const invite = await as(admin, 'teams/invites/create', { team, uses: 1, role: 'member' })
  expect((await as(user, 'invites/accept', { code: invite.body.code })).status).toBe(200)
}

/// A SHA-256 fingerprint, as `ssh-keygen -lf` prints one.
async function fp(seed: string): Promise<string> {
  const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(seed)))
  return `SHA256:${btoa(String.fromCharCode(...digest)).replaceAll('=', '')}`
}

/// A device of `user`'s the directory knows, holding a key with `fingerprint`.
async function device(user: string, fingerprint: string, state = 'certified'): Promise<string> {
  await as(user, 'me')
  const id = crypto.randomUUID()
  await env.DB.batch([
    env.DB.prepare(
      `INSERT INTO device_keys (id, account_id, label, key_a, key_a_fingerprint, state, created_at) VALUES (?, ?, 'Browser', ?, ?, ?, ?)`,
    ).bind(id, user, `ecdsa ${id}`, fingerprint, state, Date.now()),
    env.DB.prepare(`INSERT INTO device_fingerprints (fingerprint, device_id) VALUES (?, ?)`).bind(fingerprint, id),
  ])
  return id
}

/// The runner reports these keys on its authorized_keys, as its heartbeat does.
async function onRunner(token: string, fingerprints: string[]) {
  const entries = fingerprints.map(fingerprint => ({ fingerprint, comment: '', inside: true, scope: 'shell', file: 'authorized_keys' }))
  const hash = [...new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(fingerprints.join()))).values()]
    .map(b => b.toString(16).padStart(2, '0'))
    .join('')
  const beat = await call('/v1/heartbeat', { beatEvery: 300, install: 'x', inventory: { hash, unparsed: 0, entries } }, token)
  expect(beat.status).toBe(200)
}

/// A runner of `user`'s, online, with one device of theirs enrolled on it.
async function scene(user: string) {
  const runner = await runnerFor(user)
  const key = await fp(`${user}-browser-${runner.id}`)
  const dev = await device(user, key)
  await onRunner(runner.token, [key])
  const line = await connect(runner.token)
  return { runner, device: dev, key, line, browser: browser(user) }
}

const OFFER = 'v=0\r\no=- 1 2 IN IP4 127.0.0.1\r\ns=-\r\na=fingerprint:sha-256 AA:BB\r\n'
const ANSWER = 'v=0\r\no=- 3 4 IN IP4 127.0.0.1\r\ns=-\r\na=fingerprint:sha-256 CC:DD\r\n'
const CANDIDATE = 'candidate:1 1 udp 2122260223 192.0.2.10 54321 typ host'

// MARK: - The exchange

describe('a browser and its runner', () => {
  it('trade offer, answer and trickled candidates through the runner’s object', async () => {
    const { runner, device: dev, key, line, browser: alice } = await scene('alice')

    const opened = await alice.open(runner.id, dev)
    expect(opened.status).toBe(200)
    const session = opened.body.session
    expect(session).toMatch(/^[0-9a-f]{32}$/)
    expect(opened.body.ice_servers).toMatchObject({ session, type: 'ice_servers', ttl: TURN_TTL_S })

    // The runner is handed the same servers for the same session.
    expect(await line.next()).toEqual(opened.body.ice_servers)

    // The offer, with the device the relay checked.
    expect((await alice.send(runner.id, dev, { session, type: 'offer', sdp: OFFER })).status).toBe(200)
    expect(await line.next()).toEqual({ session, type: 'offer', sdp: OFFER, device: { id: dev, account: 'alice', fingerprints: [key] } })

    // The browser trickles a candidate, and the end-of-candidates marker.
    await alice.send(runner.id, dev, { session, type: 'candidate', candidate: CANDIDATE, sdpMid: '0', sdpMLineIndex: 0 })
    expect(await line.next()).toEqual({ session, type: 'candidate', candidate: CANDIDATE, sdpMid: '0', sdpMLineIndex: 0 })
    await alice.send(runner.id, dev, { session, type: 'candidate', candidate: '', sdpMid: '0', sdpMLineIndex: 0 })
    expect(await line.next()).toMatchObject({ type: 'candidate', candidate: '' })

    // The runner answers and trickles back.
    line.send({ session, type: 'answer', sdp: ANSWER })
    line.send({ session, type: 'candidate', candidate: CANDIDATE, sdpMid: '0', sdpMLineIndex: 0 })
    await settle()
    const first = await alice.poll(runner.id, session)
    expect(first.status).toBe(200)
    expect(first.body.messages).toEqual([
      { session, type: 'answer', sdp: ANSWER },
      { session, type: 'candidate', candidate: CANDIDATE, sdpMid: '0', sdpMLineIndex: 0 },
    ])
    expect(first.body.ended).toBe(false)

    // Polling past a cursor acknowledges what came before it.
    line.send({ session, type: 'candidate', candidate: `${CANDIDATE} 2`, sdpMid: '0', sdpMLineIndex: 0 })
    await settle()
    const second = await alice.poll(runner.id, session, first.body.cursor)
    expect(second.body.messages).toEqual([{ session, type: 'candidate', candidate: `${CANDIDATE} 2`, sdpMid: '0', sdpMLineIndex: 0 }])

    // The runner ends it; the browser reads that, and can send no more.
    line.send({ session, type: 'end', reason: 'closed' })
    await settle()
    const last = await alice.poll(runner.id, session, second.body.cursor)
    expect(last.body).toEqual({ messages: [{ session, type: 'end', reason: 'closed' }], cursor: second.body.cursor + 1, ended: true })
    expect((await alice.send(runner.id, dev, { session, type: 'candidate', candidate: CANDIDATE })).status).toBe(410)
  })

  it('wakes a waiting poll the moment the runner answers', async () => {
    const { runner, device: dev, line, browser: alice } = await scene('alice')
    const { session } = (await alice.open(runner.id, dev)).body
    await alice.send(runner.id, dev, { session, type: 'offer', sdp: OFFER })

    const started = Date.now()
    const waiting = alice.poll(runner.id, session, 0, 10)
    await settle()
    line.send({ session, type: 'answer', sdp: ANSWER })
    const polled = await waiting
    expect(polled.body.messages).toEqual([{ session, type: 'answer', sdp: ANSWER }])
    expect(Date.now() - started).toBeLessThan(5000)
  })

  it('carries the browser’s end to the runner and forgets the session', async () => {
    const { runner, device: dev, line, browser: alice } = await scene('alice')
    const { session } = (await alice.open(runner.id, dev)).body
    await line.next()
    expect((await alice.send(runner.id, dev, { session, type: 'end', reason: 'closed tab' })).status).toBe(200)
    expect(await line.next()).toEqual({ session, type: 'end', reason: 'closed tab' })
    expect((await alice.poll(runner.id, session)).status).toBe(404)
  })

  it('passes on only the fields it checked, and only what each side may send', async () => {
    const { runner, device: dev, line, browser: alice } = await scene('alice')
    const { session } = (await alice.open(runner.id, dev)).body
    await line.next()

    await alice.send(runner.id, dev, { session, type: 'offer', sdp: OFFER, device: { id: 'forged' }, extra: 'smuggled' })
    const offer = await line.next()
    expect(offer.extra).toBeUndefined()
    expect(offer.device.id).toBe(dev)

    // A browser can't answer, or hand out servers.
    expect((await alice.send(runner.id, dev, { session, type: 'answer', sdp: ANSWER })).status).toBe(400)
    expect((await alice.send(runner.id, dev, { session, type: 'ice_servers', urls: ['turn:evil.example'] })).status).toBe(400)
    // A runner can't offer, or hand out servers either.
    line.send({ session, type: 'offer', sdp: OFFER })
    line.send({ session, type: 'ice_servers', urls: ['turn:evil.example'], username: 'u', credential: 'c' })
    line.send({ session, type: 'answer', sdp: ANSWER, injected: true })
    await settle()
    expect((await alice.poll(runner.id, session)).body.messages).toEqual([{ session, type: 'answer', sdp: ANSWER }])
    await settle()
    expect(line.heard).toHaveLength(2)

    expect(messageOf({ session, type: 'candidate', candidate: 'x'.repeat(2000) }, 'browser')).toBeNull()
    expect(messageOf({ session: 'not-a-session', type: 'end' }, 'browser')).toBeNull()
  })

  it('says the runner is offline rather than holding the offer, and mints nothing for it', async () => {
    const runner = await runnerFor('alice')
    const key = await fp('offline-browser')
    const dev = await device('alice', key)
    await onRunner(runner.token, [key])
    const opened = await browser('alice', withTurn).open(runner.id, dev)
    expect(opened.status).toBe(503)
    expect(opened.body.error).toBe('runner_offline')
    expect(minted).toHaveLength(0)
  })

  it('lets a runner’s second connection replace its first', async () => {
    const { runner, device: dev, line: old, browser: alice } = await scene('alice')
    const fresh = await connect(runner.token)
    await settle()
    expect(old.ws.readyState).not.toBe(WebSocket.OPEN)
    const { session } = (await alice.open(runner.id, dev)).body
    expect((await fresh.next()).session).toBe(session)
    expect(old.heard).toHaveLength(0)
  })
})

// MARK: - Who may signal

describe('who may reach a runner', () => {
  /// Each refusal below is asserted twice: the browser is refused, and the
  /// runner, listening the whole time, heard nothing.
  async function refusedBeforeRunner(line: Line, attempt: () => Promise<{ status: number }>, status = 404) {
    expect((await attempt()).status).toBe(status)
    await settle()
    expect(line.heard).toEqual([])
    expect(minted).toHaveLength(0)
  }

  it('refuses a caller with no session, or a forged one', async () => {
    const { runner, device: dev, line } = await scene('alice')
    await refusedBeforeRunner(line, () => call('/v1/rtc/open', { runner: runner.id, device: dev }), 401)
    await refusedBeforeRunner(line, () => call('/v1/rtc/open', { runner: runner.id, device: dev }, 'not.a.token'), 401)
  })

  it('refuses a device that is someone else’s, teammate or not', async () => {
    const { runner, device: dev, line } = await scene('alice')
    await as('mallory', 'me')
    await refusedBeforeRunner(line, () => browser('mallory', withTurn).open(runner.id, dev))
    // In the runner's team, with alice's enrolled device's id: still not hers.
    await joinTeam(runner.team, 'mallory')
    await refusedBeforeRunner(line, () => browser('mallory', withTurn).open(runner.id, dev))
  })

  it('refuses a device that is pending or revoked', async () => {
    const { runner, line, browser: alice } = await scene('alice')
    for (const state of ['pending', 'revoked']) {
      const key = await fp(`alice-${state}`)
      const dev = await device('alice', key, state)
      await onRunner(runner.token, [key])
      await refusedBeforeRunner(line, () => alice.open(runner.id, dev))
    }
  })

  it('refuses a device whose key is not on that runner', async () => {
    const { runner, line, browser: alice } = await scene('alice')
    const elsewhere = await device('alice', await fp('alice-key-not-on-runner'))
    await refusedBeforeRunner(line, () => alice.open(runner.id, elsewhere))
  })

  it('refuses a device whose key is on another runner of the same team, not this one', async () => {
    const { runner, line, browser: alice } = await scene('alice')
    const key = await fp('alice-key-on-sibling')
    const dev = await device('alice', key)
    // A second runner in the same team, carrying the key; this one doesn't.
    const sibling = await runnerIn('alice', runner.team)
    await onRunner(sibling.token, [key])
    await refusedBeforeRunner(line, () => alice.open(runner.id, dev))
  })

  it('refuses someone outside the runner’s team even with their key still on it', async () => {
    const { runner, line } = await scene('alice')
    const key = await fp('bob-left-team')
    const bobs = await device('bob', key)
    await onRunner(runner.token, [key])
    await refusedBeforeRunner(line, () => browser('bob').open(runner.id, bobs))

    // Let in, the same device and key are enough.
    await joinTeam(runner.team, 'bob')
    expect((await browser('bob').open(runner.id, bobs)).status).toBe(200)
  })

  it('refuses every later send from someone who is not enrolled, even naming a live session', async () => {
    const { runner, device: dev, line, browser: alice } = await scene('alice')
    const { session } = (await alice.open(runner.id, dev)).body
    await line.next()
    line.heard.length = 0
    await as('mallory', 'me')
    const mallorys = await device('mallory', await fp('mallory'))
    await refusedBeforeRunner(line, () => browser('mallory').send(runner.id, mallorys, { session, type: 'offer', sdp: OFFER }))
    await refusedBeforeRunner(line, () => browser('mallory').send(runner.id, dev, { session, type: 'offer', sdp: OFFER }))
  })

  it('refuses an enrolled caller naming a session that is not theirs', async () => {
    const { runner, device: dev, line, browser: alice } = await scene('alice')
    const { session } = (await alice.open(runner.id, dev)).body
    await line.next()

    // Mallory, in the team with her own key on the runner, enrolled in every
    // way, names alice's session.
    await joinTeam(runner.team, 'mallory')
    const key = await fp('mallory-enrolled')
    const mallorys = await device('mallory', key)
    await onRunner(runner.token, [await fp(`alice-browser-${runner.id}`), key])
    expect((await browser('mallory').send(runner.id, mallorys, { session, type: 'offer', sdp: OFFER })).status).toBe(404)
    // Alice's own second device, enrolled too, names the first one's session.
    const second = await fp('alice-second')
    const secondDevice = await device('alice', second)
    await onRunner(runner.token, [await fp(`alice-browser-${runner.id}`), key, second])
    expect((await alice.send(runner.id, secondDevice, { session, type: 'offer', sdp: OFFER })).status).toBe(404)
    await settle()
    // Only the session's `ice_servers`, from its open.
    expect(line.heard.map(m => m.type)).toEqual(['ice_servers'])
    // The session's own device still gets through.
    expect((await alice.send(runner.id, dev, { session, type: 'offer', sdp: OFFER })).status).toBe(200)
  })

  it('answers a poll only to the account that opened the session', async () => {
    const { runner, device: dev, line, browser: alice } = await scene('alice')
    const { session } = (await alice.open(runner.id, dev)).body
    line.send({ session, type: 'answer', sdp: ANSWER })
    await settle()
    const stolen = await browser('mallory').poll(runner.id, session)
    expect(stolen.status).toBe(404)
    expect(stolen.text).not.toContain(ANSWER)
    expect((await alice.poll(runner.id, session)).body.messages).toHaveLength(1)
  })

  it('refuses a WebSocket from a token that belongs to no runner, or to none at all', async () => {
    const legacy = 'legacy-daemon-token'
    await env.DB.prepare(`INSERT INTO accounts (id, created_at) VALUES ('old', 0) ON CONFLICT DO NOTHING`).run()
    const hash = [...new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(legacy)))]
      .map(b => b.toString(16).padStart(2, '0'))
      .join('')
    await env.DB.prepare(`INSERT INTO daemons (id, account_id, token_hash, label, created_at) VALUES (?, 'old', ?, 'Mac', 0)`)
      .bind(crypto.randomUUID(), hash)
      .run()
    for (const [token, status] of [
      [legacy, 403],
      ['no-such-token', 401],
    ] as const) {
      const response = await worker.fetch(
        new Request(`${RELAY}/v1/rtc/runner`, { headers: { upgrade: 'websocket', authorization: `Bearer ${token}` } }),
        env as never,
        ctx,
      )
      expect(response.status).toBe(status)
      expect(response.webSocket).toBeNull()
    }
  })
})

// MARK: - One runner's messages never reach another's

describe('two runners', () => {
  it('keep their sessions apart, both ways', async () => {
    const a = await scene('alice')
    const b = await scene('bob')
    const { session } = (await a.browser.open(a.runner.id, a.device)).body
    await a.line.next()

    // Runner B names A's session: dropped. A's browser hears only A.
    b.line.send({ session, type: 'answer', sdp: 'from-runner-b' })
    await settle()
    expect((await a.browser.poll(a.runner.id, session)).body.messages).toEqual([])
    a.line.send({ session, type: 'answer', sdp: ANSWER })
    await settle()
    expect((await a.browser.poll(a.runner.id, session)).body.messages).toEqual([{ session, type: 'answer', sdp: ANSWER }])

    // A's offer reaches A alone.
    await a.browser.send(a.runner.id, a.device, { session, type: 'offer', sdp: OFFER })
    expect((await a.line.next()).type).toBe('offer')
    await settle()
    expect(b.line.heard).toEqual([])

    // A's device is enrolled on A, not B: A's browser can't open B.
    expect((await a.browser.open(b.runner.id, a.device)).status).toBe(404)
    // And a session id carried to the other runner's object is no session there.
    expect((await a.browser.poll(b.runner.id, session)).status).toBe(404)
  })

  it('keep two runners of the same owner apart', async () => {
    const one = await scene('carol')
    const two = await runnerFor('carol')
    await onRunner(two.token, [one.key])
    const twoLine = await connect(two.token)

    const toOne = (await one.browser.open(one.runner.id, one.device)).body.session
    const toTwo = (await one.browser.open(two.id, one.device)).body.session
    expect((await one.line.next()).session).toBe(toOne)
    expect((await twoLine.next()).session).toBe(toTwo)

    // Runner two answering runner one's session reaches nobody.
    twoLine.send({ session: toOne, type: 'answer', sdp: 'wrong runner' })
    await settle()
    expect((await one.browser.poll(one.runner.id, toOne)).body.messages).toEqual([])
    await settle()
    expect(one.line.heard).toHaveLength(1)
    expect(twoLine.heard).toHaveLength(1)
  })
})

// MARK: - TURN credentials

describe('TURN credentials', () => {
  it('are minted per session, short-lived, from a key that never leaves the worker', async () => {
    const { runner, device: dev, line } = await scene('alice')
    const alice = browser('alice', withTurn)

    const first = await alice.open(runner.id, dev)
    const second = await alice.open(runner.id, dev)
    expect(first.status).toBe(200)
    expect(minted).toHaveLength(2)
    for (const call of minted) {
      expect(call.url).toBe(`https://rtc.live.cloudflare.com/v1/turn/keys/${TURN_KEY_ID}/credentials/generate`)
      expect(call.authorization).toBe(`Bearer ${TURN_TOKEN}`)
      expect(call.body.ttl).toBe(TURN_TTL_S)
    }
    expect(TURN_TTL_S).toBeLessThanOrEqual(3600)

    const ice = first.body.ice_servers
    expect(ice).toMatchObject({ type: 'ice_servers', username: 'minted-user-1', credential: 'minted-credential-1', ttl: TURN_TTL_S })
    expect(ice.stun_only).toBeUndefined()
    expect(ice.urls).toEqual(expect.arrayContaining([...STUN_URLS, 'turn:turn.cloudflare.com:3478?transport=udp']))
    expect(second.body.ice_servers.username).toBe('minted-user-2')

    // The runner gets each session's own.
    expect(await line.next()).toEqual(ice)
    expect((await line.next()).credential).toBe('minted-credential-2')

    // The account token appears in no reply, no runner message, no log line;
    // the minted credentials appear in no log line.
    const everything = [first.text, second.text, JSON.stringify(line.heard), ...logged].join('\n')
    expect(everything).not.toContain(TURN_TOKEN)
    expect(logged.join('\n')).not.toContain('minted-credential')
  })

  it('hand out STUN only, and say so, with no key', async () => {
    const { runner, device: dev, line, browser: alice } = await scene('alice')
    const opened = await alice.open(runner.id, dev)
    expect(opened.status).toBe(200)
    expect(minted).toHaveLength(0)
    expect(opened.body.ice_servers).toEqual({ session: opened.body.session, type: 'ice_servers', urls: [...STUN_URLS], ttl: TURN_TTL_S, stun_only: 'no_key' })
    expect(await line.next()).toEqual(opened.body.ice_servers)
    expect(logged.some(l => l.includes('STUN only'))).toBe(true)

    // Half a key is no key.
    const half = await browser('alice', { TURN_KEY_ID }).open(runner.id, dev)
    expect(half.body.ice_servers.stun_only).toBe('no_key')
    expect(minted).toHaveLength(0)
  })

  it('hand out STUN only, and say so, when Cloudflare refuses, without repeating what it said', async () => {
    const { runner, device: dev, browser: _ } = await scene('alice')
    turnReply = () => new Response(`{"error":"bad token ${TURN_TOKEN}"}`, { status: 401 })
    const refused = await browser('alice', withTurn).open(runner.id, dev)
    expect(refused.status).toBe(200)
    expect(refused.body.ice_servers.stun_only).toBe('refused')
    expect(refused.body.ice_servers.credential).toBeUndefined()
    expect(logged.some(l => l.includes('refused (401)'))).toBe(true)
    expect([refused.text, ...logged].join('\n')).not.toContain(TURN_TOKEN)

    // An answer with no TURN address in it is no credential either.
    turnReply = () => new Response(JSON.stringify({ iceServers: { urls: ['stun:x.example'], username: 'u', credential: 'c' } }))
    expect((await browser('alice', withTurn).open(runner.id, dev)).body.ice_servers.stun_only).toBe('refused')
  })
})
