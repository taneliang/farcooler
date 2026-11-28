/// WebRTC signaling between a browser and a runner (ov-1534; design in
/// reports/ov-1502/report.md §5, ticket 1).
///
/// The relay is the meeting point and nothing more. Each runner has one
/// Durable Object, named by its directory id, and keeps a hibernating
/// WebSocket to it. A browser reaches that object over plain HTTPS calls: it
/// opens a session, posts its offer and trickled candidates, and long-polls
/// for the runner's answer and candidates. Neither side ever learns the
/// other's network address from the relay except inside those messages.
///
/// **The contract** (shared with the runner and web tickets; changed only by
/// telling the orchestrator). Every message is `{session, type, ...}`:
///
/// - `offer {sdp}`: browser to runner. The relay adds `device {id, account,
///   fingerprints}`, the device it checked, so the runner can check it too.
/// - `answer {sdp}`: runner to browser.
/// - `candidate {candidate, sdpMid, sdpMLineIndex}`: both ways, trickled.
/// - `end {reason}`: either way.
/// - `ice_servers {urls, username, credential, ttl}`: from the relay to both
///   sides, once per session; with `stun_only` and no credential when this
///   relay can't mint TURN (`turn.ts`).
///
/// The relay rebuilds every message from the fields it checked, so nothing
/// else a sender adds reaches the other side.
///
/// **Routes**, all under `/v1/rtc/`:
///
/// - `GET runner`: the runner's WebSocket, with its daemon token. Its object
///   is chosen by the runner row that token belongs to, never by anything the
///   request names, so a runner can only ever be its own.
/// - `POST open {runner, device}`: a session, and its ICE servers.
/// - `POST send {runner, device, message}`: an offer, a candidate or an end.
/// - `POST poll {runner, session, after?, wait?}`: what the runner sent since
///   `after`, waiting up to `wait` seconds (at most `POLL_WAIT_MAX_S`) for any.
///
/// **Who may signal a runner.** `open` and `send` take a WorkOS session
/// (`requireSession`), and refuse before anything reaches the runner unless
/// all of these hold (`enrolled`):
///
/// - `device` is one of the caller's own devices, trusted or certified: not
///   pending, not revoked;
/// - one of that device's keys is on the runner, as the runner last reported
///   its keys (`runner_inventory`, ov-510). That is what "enrolled on the
///   runner" means: the key a grant put there;
/// - the caller is a member of the runner's team, so a teammate who left is
///   cut off even if their key is still on the box.
///
/// Every refusal is the same 404, so a runner id says nothing to a stranger.
/// `poll` takes the session alone, and answers only the account that opened
/// it. Inside the object a session belongs to its runner: a runner's message
/// naming any other session is dropped.

import { DurableObject } from 'cloudflare:workers'

import { json, requireDaemon, type Env } from './index'
import { requireSession } from './session'
import { iceServers, type IceServers } from './turn'

export const RTC_PREFIX = '/v1/rtc/'

/// How long a signaling session lives after its last message. Signaling is
/// only the setup: once the data channel opens, nothing more crosses here
/// unless ICE restarts, which opens a new session.
export const SESSION_TTL_MS = 5 * 60 * 1000
/// Sessions one runner may have open at once.
export const MAX_SESSIONS = 16
/// Sessions one runner may be asked to open in a minute: each open mints TURN
/// credentials, and an open followed by an end frees the slot at once.
export const MAX_OPENS_PER_MINUTE = 20
/// Messages a session holds for the browser before it drops the runner's next.
/// A trickle is a dozen candidates or so.
export const MAX_QUEUE = 64
/// The longest a poll waits for a message. Under Cloudflare's idle limits, and
/// long enough that a browser waiting for an answer polls a handful of times.
export const POLL_WAIT_MAX_S = 25

const MAX_SDP = 32 * 1024
const MAX_CANDIDATE = 1024
const MAX_MID = 32
const MAX_REASON = 200
const ID = /^[A-Za-z0-9-]{1,64}$/
const SESSION = /^[0-9a-f]{32}$/

export type Message =
  | { session: string; type: 'offer'; sdp: string; device?: Device }
  | { session: string; type: 'answer'; sdp: string }
  | { session: string; type: 'candidate'; candidate: string; sdpMid: string | null; sdpMLineIndex: number | null }
  | { session: string; type: 'end'; reason: string }
  | ({ session: string; type: 'ice_servers' } & IceServers)

/// The device the relay checked, as the runner is told it.
export interface Device {
  id: string
  account: string
  fingerprints: string[]
}

/// A message from `from`, rebuilt from its checked fields, or null. A browser
/// sends offers, candidates and ends; a runner answers, candidates and ends.
/// Neither can send `ice_servers`, which only the relay writes.
export function messageOf(raw: unknown, from: 'browser' | 'runner'): Message | null {
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return null
  const m = raw as Record<string, unknown>
  if (typeof m.session !== 'string' || !SESSION.test(m.session)) return null
  const session = m.session
  switch (m.type) {
    case 'offer':
    case 'answer':
      if (m.type !== (from === 'browser' ? 'offer' : 'answer')) return null
      if (typeof m.sdp !== 'string' || !m.sdp || m.sdp.length > MAX_SDP) return null
      return { session, type: m.type, sdp: m.sdp }
    case 'candidate': {
      // An empty candidate is the end-of-candidates marker, and passes.
      if (typeof m.candidate !== 'string' || m.candidate.length > MAX_CANDIDATE) return null
      const mid = m.sdpMid ?? null
      if (mid !== null && (typeof mid !== 'string' || mid.length > MAX_MID)) return null
      const index = m.sdpMLineIndex ?? null
      if (index !== null && (!Number.isInteger(index) || (index as number) < 0 || (index as number) > 1023)) return null
      return { session, type: 'candidate', candidate: m.candidate, sdpMid: mid as string | null, sdpMLineIndex: index as number | null }
    }
    case 'end': {
      const reason = typeof m.reason === 'string' ? m.reason.slice(0, MAX_REASON) : ''
      return { session, type: 'end', reason }
    }
    default:
      return null
  }
}

// MARK: - The worker's half: who may signal

export async function rtc(request: Request, env: Env, path: string): Promise<Response> {
  const route = path.slice(RTC_PREFIX.length)
  if (route === 'runner') return await runnerSocket(request, env)
  if (request.method !== 'POST') return json({ error: 'method' }, 405)
  switch (route) {
    case 'open':
      return await open(request, env)
    case 'send':
      return await send(request, env)
    case 'poll':
      return await poll(request, env)
    default:
      return json({ error: 'not found' }, 404)
  }
}

/// The runner's own object: by the runner row its token belongs to.
async function runnerSocket(request: Request, env: Env): Promise<Response> {
  if (request.method !== 'GET') return json({ error: 'method' }, 405)
  if (request.headers.get('upgrade')?.toLowerCase() !== 'websocket') return json({ error: 'upgrade' }, 426)
  const daemon = await requireDaemon(request, env)
  if (daemon instanceof Response) return daemon
  const runner = await env.DB.prepare(`SELECT id FROM runners WHERE daemon_id = ?`).bind(daemon.id).first<{ id: string }>()
  // A token from before the directory (`/v1/daemons`) names no runner, and no
  // browser could ever be signaling it.
  if (!runner) return json({ error: 'not a runner' }, 403)
  // The token stays here: the object needs only to know it is the runner.
  const headers = new Headers(request.headers)
  headers.delete('authorization')
  return await objectFor(env, runner.id).fetch(new Request('https://signal/runner', { method: 'GET', headers }))
}

async function open(request: Request, env: Env): Promise<Response> {
  const caller = await enrolled(request, env)
  if (caller instanceof Response) return caller
  const session = newSession()
  const object = objectFor(env, caller.runner)
  const admitted = await object.fetch(internal('open', { session, account: caller.account, device: caller.device }))
  if (!admitted.ok) return admitted
  // Minted only once the runner is known to be there and to have room, so a
  // refused open costs Cloudflare nothing.
  const ice = await iceServers(env)
  const handed = await object.fetch(internal('ice', { session, ice }))
  if (!handed.ok) return handed
  return json({ session, ice_servers: { session, type: 'ice_servers', ...ice } })
}

async function send(request: Request, env: Env): Promise<Response> {
  const caller = await enrolled(request, env)
  if (caller instanceof Response) return caller
  const message = messageOf(caller.body.message, 'browser')
  if (!message) return json({ error: 'message' }, 400)
  return await objectFor(env, caller.runner).fetch(internal('send', { account: caller.account, device: caller.device.id, message }))
}

async function poll(request: Request, env: Env): Promise<Response> {
  const session = await requireSession(request, env)
  if (session instanceof Response) return session
  const body = await bodyOf(request)
  if (typeof body?.runner !== 'string' || !ID.test(body.runner)) return json({ error: 'runner' }, 400)
  if (typeof body.session !== 'string' || !SESSION.test(body.session)) return json({ error: 'session' }, 400)
  const after = Number.isSafeInteger(body.after) ? (body.after as number) : 0
  const wait = typeof body.wait === 'number' && body.wait > 0 ? Math.min(body.wait, POLL_WAIT_MAX_S) : 0
  return await objectFor(env, body.runner).fetch(internal('poll', { account: session.userId, session: body.session, after, wait }))
}

interface Enrolled {
  account: string
  runner: string
  device: Device
  body: Record<string, unknown>
}

/// The caller, their device and the runner, if that device is enrolled on it
/// (see the top of this file); otherwise the response to send instead.
async function enrolled(request: Request, env: Env): Promise<Enrolled | Response> {
  const session = await requireSession(request, env)
  if (session instanceof Response) return session
  const body = await bodyOf(request)
  if (typeof body?.runner !== 'string' || !ID.test(body.runner)) return json({ error: 'runner' }, 400)
  if (typeof body.device !== 'string' || !ID.test(body.device)) return json({ error: 'device' }, 400)
  const account = session.userId

  const found = await env.DB.prepare(
    `SELECT d.id FROM device_keys d JOIN runners n ON n.id = ?3
     WHERE d.id = ?1 AND d.account_id = ?2 AND d.state IN ('trusted', 'certified')
       AND EXISTS (SELECT 1 FROM memberships m WHERE m.team_id = n.team_id AND m.account_id = ?2)
       AND EXISTS (
         SELECT 1 FROM runner_inventory i, json_each(i.entries) e
         JOIN device_fingerprints f ON f.fingerprint = json_extract(e.value, '$.fingerprint')
         WHERE i.runner_id = n.id AND f.device_id = d.id)`,
  )
    .bind(body.device, account, body.runner)
    .first<{ id: string }>()
  if (!found) return json({ error: 'not found' }, 404)

  const prints = await env.DB.prepare(`SELECT fingerprint FROM device_fingerprints WHERE device_id = ? ORDER BY fingerprint`)
    .bind(found.id)
    .all<{ fingerprint: string }>()
  return {
    account,
    runner: body.runner,
    device: { id: found.id, account, fingerprints: prints.results.map(p => p.fingerprint) },
    body,
  }
}

function objectFor(env: Env, runner: string): DurableObjectStub {
  return env.SIGNAL.get(env.SIGNAL.idFromName(runner))
}

function internal(route: string, body: unknown): Request {
  return new Request(`https://signal/${route}`, { method: 'POST', body: JSON.stringify(body) })
}

function newSession(): string {
  return [...crypto.getRandomValues(new Uint8Array(16))].map(b => b.toString(16).padStart(2, '0')).join('')
}

async function bodyOf(request: Request): Promise<Record<string, unknown> | null> {
  try {
    const body = await request.json()
    return body && typeof body === 'object' && !Array.isArray(body) ? (body as Record<string, unknown>) : null
  } catch {
    return null
  }
}

// MARK: - The object's half: one runner's meeting point

interface Stored {
  account: string
  device: Device
  touched: number
  /// The last sequence number handed out, so a poll's `after` is a cursor.
  seq: number
  /// What the runner sent and the browser hasn't acknowledged by polling past.
  queue: { seq: number; message: Message }[]
  /// The runner ended it: the browser may still read what is queued.
  ended?: boolean
}

const RUNNER_TAG = 'runner'

/// One runner's signaling: its WebSocket, and the sessions browsers opened
/// with it. Reached only through the worker above, which has already decided
/// who is asking; the object only ever trusts its own storage for which
/// session belongs to whom.
export class Signal extends DurableObject<Env> {
  /// Polls waiting for a session's next message. In memory only: a waiting
  /// poll keeps the object awake, and one that arrives later reads storage.
  private waiters = new Map<string, Set<() => void>>()

  constructor(ctx: DurableObjectState, env: Env) {
    super(ctx, env)
    // Keepalives answered without waking the object.
    ctx.setWebSocketAutoResponse(new WebSocketRequestResponsePair('ping', 'pong'))
  }

  async fetch(request: Request): Promise<Response> {
    const route = new URL(request.url).pathname.slice(1)
    if (route === 'runner') return this.acceptRunner()
    const body = (await request.json()) as Record<string, any>
    switch (route) {
      case 'open':
        return await this.open(body.session, body.account, body.device)
      case 'ice':
        return await this.handIce(body.session, body.ice)
      case 'send':
        return await this.fromBrowser(body.account, body.device, body.message)
      case 'poll':
        return await this.poll(body.account, body.session, body.after, body.wait)
      default:
        return json({ error: 'not found' }, 404)
    }
  }

  /// The runner's socket. A second connection replaces the first, so there is
  /// never a doubt which socket hears a session.
  private acceptRunner(): Response {
    for (const old of this.ctx.getWebSockets(RUNNER_TAG)) {
      try {
        old.close(4000, 'replaced')
      } catch {}
    }
    const pair = new WebSocketPair()
    this.ctx.acceptWebSocket(pair[1], [RUNNER_TAG])
    return new Response(null, { status: 101, webSocket: pair[0] })
  }

  private runner(): WebSocket | undefined {
    return this.ctx.getWebSockets(RUNNER_TAG).find(ws => ws.readyState === WebSocket.OPEN)
  }

  private async open(session: string, account: string, device: Device): Promise<Response> {
    if (!this.runner()) return json({ error: 'runner_offline' }, 503)
    const now = Date.now()
    const live = await this.sessions(now)
    if (live.size >= MAX_SESSIONS) return json({ error: 'busy' }, 429)
    const opens = ((await this.ctx.storage.get<number[]>('opens')) ?? []).filter(t => t > now - 60_000)
    if (opens.length >= MAX_OPENS_PER_MINUTE) return json({ error: 'slow down' }, 429)
    await this.ctx.storage.put({
      opens: [...opens, now],
      [`s:${session}`]: { account, device, touched: now, seq: 0, queue: [] } satisfies Stored,
    })
    await this.schedule(now)
    return json({ ok: true })
  }

  private async handIce(session: string, ice: IceServers): Promise<Response> {
    const stored = await this.ctx.storage.get<Stored>(`s:${session}`)
    if (!stored) return json({ error: 'not found' }, 404)
    if (!this.toRunner({ session, type: 'ice_servers', ...ice })) return json({ error: 'runner_offline' }, 503)
    return json({ ok: true })
  }

  private async fromBrowser(account: string, device: string, message: Message): Promise<Response> {
    const key = `s:${message.session}`
    const stored = await this.ctx.storage.get<Stored>(key)
    // Someone else's session reads exactly as no session.
    if (!stored || stored.account !== account || stored.device.id !== device || this.expired(stored, Date.now())) {
      return json({ error: 'not found' }, 404)
    }
    if (stored.ended) return json({ error: 'ended' }, 410)
    const out: Message = message.type === 'offer' ? { ...message, device: stored.device } : message
    if (!this.toRunner(out)) return json({ error: 'runner_offline' }, 503)
    if (message.type === 'end') {
      await this.ctx.storage.delete(key)
      this.wake(message.session)
    } else {
      await this.ctx.storage.put(key, { ...stored, touched: Date.now() })
    }
    return json({ ok: true })
  }

  private toRunner(message: Message): boolean {
    const ws = this.runner()
    if (!ws) return false
    try {
      ws.send(JSON.stringify(message))
      return true
    } catch {
      return false
    }
  }

  private async poll(account: string, session: string, after: number, wait: number): Promise<Response> {
    const key = `s:${session}`
    const read = async () => {
      const stored = await this.ctx.storage.get<Stored>(key)
      if (!stored || stored.account !== account || this.expired(stored, Date.now())) return null
      // Acknowledged by asking past them: dropped.
      if (stored.queue.some(q => q.seq <= after)) {
        stored.queue = stored.queue.filter(q => q.seq > after)
        await this.ctx.storage.put(key, stored)
      }
      return stored
    }
    let stored = await read()
    if (!stored) return json({ error: 'not found' }, 404)
    if (!stored.queue.length && !stored.ended && wait > 0) {
      await this.waitFor(session, wait * 1000)
      stored = await read()
      // Ended by the browser's own `end` while this poll waited.
      if (!stored) return json({ messages: [], cursor: after, ended: true })
    }
    return json({
      messages: stored.queue.map(q => q.message),
      cursor: stored.queue.at(-1)?.seq ?? after,
      ended: stored.ended === true,
    })
  }

  private waitFor(session: string, ms: number): Promise<void> {
    return new Promise(resolve => {
      const set = this.waiters.get(session) ?? new Set()
      this.waiters.set(session, set)
      const done = () => {
        clearTimeout(timer)
        set.delete(done)
        if (!set.size) this.waiters.delete(session)
        resolve()
      }
      const timer = setTimeout(done, ms)
      set.add(done)
    })
  }

  private wake(session: string) {
    for (const done of [...(this.waiters.get(session) ?? [])]) done()
  }

  /// What the runner says. Only ever about a session in this object, which is
  /// to say one opened for this runner; anything else is dropped unread.
  async webSocketMessage(ws: WebSocket, data: string | ArrayBuffer): Promise<void> {
    if (typeof data !== 'string' || data.length > MAX_SDP + 1024) return
    let raw: unknown
    try {
      raw = JSON.parse(data)
    } catch {
      return
    }
    const message = messageOf(raw, 'runner')
    if (!message) return
    const key = `s:${message.session}`
    const stored = await this.ctx.storage.get<Stored>(key)
    const now = Date.now()
    if (!stored || stored.ended || this.expired(stored, now)) return
    // A full queue drops what the runner says next, but never its end.
    if (stored.queue.length >= MAX_QUEUE && message.type !== 'end') return
    stored.seq += 1
    stored.queue.push({ seq: stored.seq, message })
    stored.touched = now
    if (message.type === 'end') stored.ended = true
    await this.ctx.storage.put(key, stored)
    this.wake(message.session)
  }

  async webSocketClose(ws: WebSocket, code: number): Promise<void> {
    try {
      ws.close(code === 1005 || code === 1006 ? 1000 : code, 'closed')
    } catch {}
  }

  /// Sessions past their time go; the alarm comes back while any remain.
  async alarm(): Promise<void> {
    await this.sessions(Date.now())
    const rest = await this.ctx.storage.list<Stored>({ prefix: 's:' })
    if (rest.size) await this.schedule(Date.now())
  }

  private async schedule(now: number) {
    if ((await this.ctx.storage.getAlarm()) === null) await this.ctx.storage.setAlarm(now + SESSION_TTL_MS)
  }

  private expired(stored: Stored, now: number): boolean {
    return now - stored.touched > SESSION_TTL_MS
  }

  /// The live sessions, after deleting the expired ones.
  private async sessions(now: number): Promise<Map<string, Stored>> {
    const all = await this.ctx.storage.list<Stored>({ prefix: 's:' })
    const gone = [...all].filter(([, s]) => this.expired(s, now)).map(([k]) => k)
    if (gone.length) await this.ctx.storage.delete(gone)
    for (const k of gone) all.delete(k)
    return all
  }
}
