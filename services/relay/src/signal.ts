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
/// - `end {reason}`: either way. The relay writes two of its own: `revoked`,
///   to the runner, when the session's device or account loses access, and
///   `runner_revoked`, to the browser, when the runner's token does.
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
/// it, and only while that account is in the runner's team: checked before
/// the runner's object is touched. Inside the object a session belongs to its
/// runner: a runner's message naming any other session is dropped.
///
/// **Access is checked again, not only at the door** (ov-1541). Revoking a
/// runner's token, revoking a device or removing a member does not wait for a
/// socket or a session to time out:
///
/// - the object rechecks the runner's token (`runnerOfDaemon`) before it
///   relays anything to the runner or takes anything from it, and closes a
///   socket whose token is gone, so no offer reaches a revoked runner;
/// - every poll rechecks the session's device the way `open` checked it, and
///   ends the session if it no longer passes;
/// - the routes that revoke or remove call `recheckSignaling` for the runners
///   they touch, so a socket closes and a session ends at once, not at its
///   next message.
///
/// **A session is bounded** whatever its sides do: `SESSION_LIFETIME_MS` from
/// its open however busy, `MAX_MESSAGES` and `MAX_BYTES` each way. A session
/// the runner ended, or whose servers never reached the runner, gives up its
/// slot at once.

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
/// The longest a session lives, however busy: a send refreshes the idle
/// clock above, never this one. Twice the idle limit, which is more than a
/// handshake needs; ICE restarts open sessions of their own.
export const SESSION_LIFETIME_MS = 10 * 60 * 1000
/// How long a session the runner ended stays readable, so the browser can
/// collect the runner's last messages. It holds no slot meanwhile.
export const ENDED_GRACE_MS = 30 * 1000
/// Messages a session holds for the browser before it drops the runner's next.
/// A trickle is a dozen candidates or so.
export const MAX_QUEUE = 64
/// Messages and bytes each side may send in one session. An offer or answer
/// and a few dozen candidates is a handshake; these are ample for that and
/// put a ceiling on what one session can push at the other side.
export const MAX_MESSAGES = 128
export const MAX_BYTES = 128 * 1024
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
  const runner = await runnerOfDaemon(env, daemon.id)
  // A token from before the directory (`/v1/daemons`) names no runner, and no
  // browser could ever be signaling it.
  if (!runner) return json({ error: 'not a runner' }, 403)
  // The token stays here: the object is told which token row the socket
  // stands for, so it can tell when that row is gone, and never the token.
  const headers = new Headers(request.headers)
  headers.delete('authorization')
  const url = new URL('https://signal/runner')
  url.searchParams.set('runner', runner)
  url.searchParams.set('daemon', daemon.id)
  return await objectFor(env, runner).fetch(new Request(url, { method: 'GET', headers }))
}

/// The runner a daemon token stands for, while the token is live: null for a
/// token that is gone, expired, or names no runner. A token two runner rows
/// name is refused rather than given to whichever row comes first: nothing
/// should ever write that, and if something does, neither runner can be told
/// apart by its token.
export async function runnerOfDaemon(env: Env, daemon: string): Promise<string | null> {
  const rows = await env.DB.prepare(
    `SELECT n.id FROM runners n JOIN daemons d ON d.id = n.daemon_id
     WHERE d.id = ? AND (d.expires_at IS NULL OR d.expires_at > ?) LIMIT 2`,
  )
    .bind(daemon, Date.now())
    .all<{ id: string }>()
  if (rows.results.length > 1) console.error('rtc: a daemon token names two runners; refusing it')
  return rows.results.length === 1 ? rows.results[0].id : null
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
  // A refusal here has already given the slot back (`handIce`).
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
  const account = session.userId
  // Before any object exists for it: a runner id that is no runner, or one
  // whose team the caller isn't in, wakes nothing. The object then checks the
  // session's own device (`Signal.poll`).
  const member = await env.DB.prepare(
    `SELECT 1 AS ok FROM runners n JOIN memberships m ON m.team_id = n.team_id WHERE n.id = ? AND m.account_id = ?`,
  )
    .bind(body.runner, account)
    .first<{ ok: number }>()
  if (!member) return json({ error: 'not found' }, 404)
  const after = Number.isSafeInteger(body.after) ? (body.after as number) : 0
  const wait = typeof body.wait === 'number' && body.wait > 0 ? Math.min(body.wait, POLL_WAIT_MAX_S) : 0
  return await objectFor(env, body.runner).fetch(internal('poll', { runner: body.runner, account, session: body.session, after, wait }))
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
  if (!(await isEnrolled(env, body.runner, account, body.device))) return json({ error: 'not found' }, 404)

  const prints = await env.DB.prepare(`SELECT fingerprint FROM device_fingerprints WHERE device_id = ? ORDER BY fingerprint`)
    .bind(body.device)
    .all<{ fingerprint: string }>()
  return {
    account,
    runner: body.runner,
    device: { id: body.device, account, fingerprints: prints.results.map(p => p.fingerprint) },
    body,
  }
}

/// Whether `account`'s `device` may signal `runner`, by the three rules at
/// the top of this file. The one statement of them: `open` and `send` ask it
/// at the door, and the object asks it again of every session it holds.
async function isEnrolled(env: Env, runner: string, account: string, device: string): Promise<boolean> {
  const found = await env.DB.prepare(
    `SELECT 1 AS ok FROM device_keys d JOIN runners n ON n.id = ?3
     WHERE d.id = ?1 AND d.account_id = ?2 AND d.state IN ('trusted', 'certified')
       AND EXISTS (SELECT 1 FROM memberships m WHERE m.team_id = n.team_id AND m.account_id = ?2)
       AND EXISTS (
         SELECT 1 FROM runner_inventory i, json_each(i.entries) e
         JOIN device_fingerprints f ON f.fingerprint = json_extract(e.value, '$.fingerprint')
         WHERE i.runner_id = n.id AND f.device_id = d.id)`,
  )
    .bind(device, account, runner)
    .first<{ ok: number }>()
  return !!found
}

/// Tell each of these runners' objects to check its socket and its sessions
/// against the directory now, rather than at their next message. For the
/// routes that revoke a token or a device or remove a member, after their
/// write. Never throws: the write it follows has happened either way, and the
/// object checks again on the next message regardless.
export async function recheckSignaling(env: Env, runners: Iterable<string>): Promise<void> {
  await Promise.all(
    [...new Set(runners)].map(async runner => {
      try {
        const response = await objectFor(env, runner).fetch(internal('recheck', { runner }))
        await response.body?.cancel()
      } catch {
        console.error('rtc: a runner could not be told to recheck its signaling')
      }
    }),
  )
}

/// The runners of every team `account` is in: where a device of theirs could
/// have a session open.
export async function runnersOfAccount(env: Env, account: string): Promise<string[]> {
  const rows = await env.DB.prepare(`SELECT n.id FROM runners n JOIN memberships m ON m.team_id = n.team_id WHERE m.account_id = ?`)
    .bind(account)
    .all<{ id: string }>()
  return rows.results.map(r => r.id)
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

function byteLength(text: string): number {
  return new TextEncoder().encode(text).length
}

// MARK: - The object's half: one runner's meeting point

/// What one side has sent in a session.
interface Tally {
  messages: number
  bytes: number
}

interface Stored {
  account: string
  device: Device
  /// When it opened: the hard lifetime runs from here.
  opened: number
  /// Its last message: the idle clock runs from here.
  touched: number
  /// The last sequence number handed out, so a poll's `after` is a cursor.
  seq: number
  /// What the runner sent and the browser hasn't acknowledged by polling past.
  queue: { seq: number; message: Message }[]
  /// The runner ended it: the browser may still read what is queued, until
  /// `ENDED_GRACE_MS` after `endedAt`. It holds no slot.
  ended?: boolean
  endedAt?: number
  /// Browser to runner, and runner to browser.
  up: Tally
  down: Tally
}

/// Who a runner socket stands for: kept on the socket, so it survives
/// hibernation.
interface RunnerTag {
  runner: string
  daemon: string
}

const RUNNER_TAG = 'runner'

/// One runner's signaling: its WebSocket, and the sessions browsers opened
/// with it. Reached only through the worker above, which has already decided
/// who is asking; the object only ever trusts its own storage for which
/// session belongs to whom, and the directory for whether they still may.
export class Signal extends DurableObject<Env> {
  /// Polls waiting for a session's next message. In memory only: a waiting
  /// poll keeps the object awake, and one that arrives later reads storage.
  private waiters = new Map<string, Set<() => void>>()
  /// The runner's messages, one at a time in the order they came. Each one
  /// waits on the directory (`stillTheRunner`), and the runtime delivers the
  /// next message meanwhile; without this a candidate could overtake the
  /// answer it follows.
  private inbox: Promise<void> = Promise.resolve()

  constructor(ctx: DurableObjectState, env: Env) {
    super(ctx, env)
    // Keepalives answered without waking the object.
    ctx.setWebSocketAutoResponse(new WebSocketRequestResponsePair('ping', 'pong'))
  }

  async fetch(request: Request): Promise<Response> {
    const url = new URL(request.url)
    const route = url.pathname.slice(1)
    if (route === 'runner') return this.acceptRunner(url.searchParams.get('runner'), url.searchParams.get('daemon'))
    const body = (await request.json()) as Record<string, any>
    switch (route) {
      case 'open':
        return await this.open(body.session, body.account, body.device)
      case 'ice':
        return await this.handIce(body.session, body.ice)
      case 'send':
        return await this.fromBrowser(body.account, body.device, body.message)
      case 'poll':
        return await this.poll(body.runner, body.account, body.session, body.after, body.wait)
      case 'recheck':
        return await this.recheck(body.runner)
      default:
        return json({ error: 'not found' }, 404)
    }
  }

  /// The runner's socket. A second connection replaces the first, so there is
  /// never a doubt which socket hears a session.
  private acceptRunner(runner: string | null, daemon: string | null): Response {
    if (!runner || !daemon) return json({ error: 'not a runner' }, 403)
    for (const old of this.ctx.getWebSockets(RUNNER_TAG)) {
      try {
        old.close(4000, 'replaced')
      } catch {}
    }
    const pair = new WebSocketPair()
    this.ctx.acceptWebSocket(pair[1], [RUNNER_TAG])
    pair[1].serializeAttachment({ runner, daemon } satisfies RunnerTag)
    return new Response(null, { status: 101, webSocket: pair[0] })
  }

  private openSocket(): WebSocket | undefined {
    return this.ctx.getWebSockets(RUNNER_TAG).find(ws => ws.readyState === WebSocket.OPEN)
  }

  /// The runner's socket, if it has one and the token it opened with is
  /// still that runner's. Asked before anything is relayed to the runner.
  private async runner(): Promise<WebSocket | undefined> {
    const ws = this.openSocket()
    return ws && (await this.stillTheRunner(ws)) ? ws : undefined
  }

  /// Whether `ws`'s token is still its runner's. If not, the socket is closed
  /// and every session it was serving ends, for the browser to read.
  private async stillTheRunner(ws: WebSocket): Promise<boolean> {
    const tag = ws.deserializeAttachment() as RunnerTag | null
    if (tag && (await runnerOfDaemon(this.env, tag.daemon)) === tag.runner) return true
    try {
      ws.close(4001, 'revoked')
    } catch {}
    const now = Date.now()
    for (const [key, stored] of await this.sessions(now)) {
      if (stored.ended) continue
      const session = key.slice(2)
      stored.seq += 1
      stored.queue.push({ seq: stored.seq, message: { session, type: 'end', reason: 'runner_revoked' } })
      stored.ended = true
      stored.endedAt = now
      await this.ctx.storage.put(key, stored)
      this.wake(session)
    }
    await this.schedule()
    return false
  }

  private async open(session: string, account: string, device: Device): Promise<Response> {
    if (!(await this.runner())) return json({ error: 'runner_offline' }, 503)
    const now = Date.now()
    const live = [...(await this.sessions(now)).values()].filter(s => !s.ended)
    if (live.length >= MAX_SESSIONS) return json({ error: 'busy' }, 429)
    const opens = ((await this.ctx.storage.get<number[]>('opens')) ?? []).filter(t => t > now - 60_000)
    if (opens.length >= MAX_OPENS_PER_MINUTE) return json({ error: 'slow down' }, 429)
    const fresh: Stored = { account, device, opened: now, touched: now, seq: 0, queue: [], up: tally(), down: tally() }
    await this.ctx.storage.put({ opens: [...opens, now], [`s:${session}`]: fresh })
    await this.schedule()
    return json({ ok: true })
  }

  /// The session's servers, to the runner. A session whose servers never
  /// reached the runner can't be used, so it goes, and its slot with it.
  private async handIce(session: string, ice: IceServers): Promise<Response> {
    const ws = await this.runner()
    const key = `s:${session}`
    const stored = await this.ctx.storage.get<Stored>(key)
    if (!stored) return json({ error: 'not found' }, 404)
    if (!ws || !sendOn(ws, { session, type: 'ice_servers', ...ice })) {
      await this.ctx.storage.delete(key)
      this.wake(session)
      return json({ error: 'runner_offline' }, 503)
    }
    return json({ ok: true })
  }

  private async fromBrowser(account: string, device: string, message: Message): Promise<Response> {
    // The runner first: a revoked one ends every session, this one included.
    const ws = await this.runner()
    const key = `s:${message.session}`
    const stored = await this.ctx.storage.get<Stored>(key)
    const now = Date.now()
    // Someone else's session reads exactly as no session.
    if (!stored || stored.account !== account || stored.device.id !== device || this.expired(stored, now)) {
      return json({ error: 'not found' }, 404)
    }
    if (stored.ended) return json({ error: 'ended' }, 410)
    if (!ws) return json({ error: 'runner_offline' }, 503)
    const out: Message = message.type === 'offer' ? { ...message, device: stored.device } : message
    const text = JSON.stringify(out)
    const up = stored.up ?? tally()
    // Its end always passes, so a browser at its limit can still let go.
    if (message.type !== 'end' && (up.messages >= MAX_MESSAGES || up.bytes + byteLength(text) > MAX_BYTES)) {
      return json({ error: 'too much' }, 429)
    }
    if (!sendText(ws, text)) return json({ error: 'runner_offline' }, 503)
    if (message.type === 'end') {
      await this.ctx.storage.delete(key)
      this.wake(message.session)
    } else {
      await this.ctx.storage.put(key, { ...stored, touched: now, up: { messages: up.messages + 1, bytes: up.bytes + byteLength(text) } })
    }
    return json({ ok: true })
  }

  /// What the runner sent since `after`, for the account that opened the
  /// session, while its device may still signal this runner. One that may
  /// not any more ends the session, and reads as no session.
  private async poll(runner: string, account: string, session: string, after: number, wait: number): Promise<Response> {
    const key = `s:${session}`
    const read = async () => {
      const stored = await this.ctx.storage.get<Stored>(key)
      if (!stored || stored.account !== account || this.expired(stored, Date.now())) return null
      return stored
    }
    const first = await read()
    if (!first) return json({ error: 'not found' }, 404)
    const device = first.device.id
    if (!(await isEnrolled(this.env, runner, account, device))) {
      await this.endRevoked(session)
      return json({ error: 'not found' }, 404)
    }
    let stored = await this.acknowledge(key, await read(), after)
    if (stored && !stored.queue.length && !stored.ended && wait > 0) {
      await this.waitFor(session, wait * 1000)
      // Asked again: a revoke may be what woke this poll.
      if (!(await isEnrolled(this.env, runner, account, device))) {
        await this.endRevoked(session)
        return json({ error: 'not found' }, 404)
      }
      stored = await this.acknowledge(key, await read(), after)
      // Ended by the browser's own `end` while this poll waited.
      if (!stored) return json({ messages: [], cursor: after, ended: true })
    }
    if (!stored) return json({ error: 'not found' }, 404)
    return json({
      messages: stored.queue.map(q => q.message),
      cursor: stored.queue.at(-1)?.seq ?? after,
      ended: stored.ended === true,
    })
  }

  /// Drops what a poll's `after` acknowledged.
  private async acknowledge(key: string, stored: Stored | null, after: number): Promise<Stored | null> {
    if (stored && stored.queue.some(q => q.seq <= after)) {
      stored.queue = stored.queue.filter(q => q.seq > after)
      await this.ctx.storage.put(key, stored)
    }
    return stored
  }

  /// Ends a session whose device may no longer signal this runner: gone at
  /// once, the runner told (if it still may hear), and any waiting poll woken
  /// to find it refused.
  private async endRevoked(session: string, ws?: WebSocket | null) {
    await this.ctx.storage.delete(`s:${session}`)
    const to = ws === undefined ? await this.runner() : ws
    if (to) sendOn(to, { session, type: 'end', reason: 'revoked' })
    this.wake(session)
  }

  /// Everything checked against the directory now: the runner's token, and
  /// each session's device (`recheckSignaling`).
  private async recheck(runner: string): Promise<Response> {
    const socket = this.openSocket()
    const ws = socket && (await this.stillTheRunner(socket)) ? socket : null
    for (const [key, stored] of await this.sessions(Date.now())) {
      if (stored.ended) continue
      if (!(await isEnrolled(this.env, runner, stored.account, stored.device.id))) await this.endRevoked(key.slice(2), ws)
    }
    return json({ ok: true })
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

  async webSocketMessage(ws: WebSocket, data: string | ArrayBuffer): Promise<void> {
    const turn = this.inbox.then(() => this.fromRunner(ws, data))
    this.inbox = turn.catch(() => {})
    await turn
  }

  /// What the runner says. Only ever about a session in this object, which is
  /// to say one opened for this runner; anything else is dropped unread, as
  /// is everything from a socket whose token has been revoked.
  private async fromRunner(ws: WebSocket, data: string | ArrayBuffer): Promise<void> {
    if (typeof data !== 'string' || data.length > MAX_SDP + 1024) return
    let raw: unknown
    try {
      raw = JSON.parse(data)
    } catch {
      return
    }
    const message = messageOf(raw, 'runner')
    if (!message) return
    if (!(await this.stillTheRunner(ws))) return
    const key = `s:${message.session}`
    const stored = await this.ctx.storage.get<Stored>(key)
    const now = Date.now()
    if (!stored || stored.ended || this.expired(stored, now)) return
    const text = JSON.stringify(message)
    const down = stored.down ?? tally()
    // A full queue, or a runner past its share, drops what it says next, but
    // never its end.
    const over = stored.queue.length >= MAX_QUEUE || down.messages >= MAX_MESSAGES || down.bytes + byteLength(text) > MAX_BYTES
    if (over && message.type !== 'end') return
    stored.seq += 1
    stored.queue.push({ seq: stored.seq, message })
    stored.touched = now
    stored.down = { messages: down.messages + 1, bytes: down.bytes + byteLength(text) }
    if (message.type === 'end') {
      stored.ended = true
      stored.endedAt = now
    }
    await this.ctx.storage.put(key, stored)
    this.wake(message.session)
    if (message.type === 'end') await this.schedule()
  }

  async webSocketClose(ws: WebSocket, code: number): Promise<void> {
    try {
      ws.close(code === 1005 || code === 1006 ? 1000 : code, 'closed')
    } catch {}
  }

  /// Sessions past their time go; the alarm comes back for the next to go.
  async alarm(): Promise<void> {
    await this.sessions(Date.now())
    await this.schedule(true)
  }

  /// The alarm, set for the soonest session to go, unless one is set sooner.
  private async schedule(replace = false) {
    const all = await this.ctx.storage.list<Stored>({ prefix: 's:' })
    if (!all.size) return
    const next = Math.min(...[...all.values()].map(s => this.deadline(s)))
    const current = replace ? null : await this.ctx.storage.getAlarm()
    if (current === null || current > next) await this.ctx.storage.setAlarm(next)
  }

  /// When a session goes: its idle limit or its lifetime, whichever is first;
  /// or, once the runner has ended it, its grace for the browser to read that.
  private deadline(stored: Stored): number {
    if (stored.ended) return (stored.endedAt ?? stored.touched) + ENDED_GRACE_MS
    return Math.min(stored.touched + SESSION_TTL_MS, (stored.opened ?? stored.touched) + SESSION_LIFETIME_MS)
  }

  private expired(stored: Stored, now: number): boolean {
    return now >= this.deadline(stored)
  }

  /// The sessions not yet past their time, after deleting the ones that are.
  private async sessions(now: number): Promise<Map<string, Stored>> {
    const all = await this.ctx.storage.list<Stored>({ prefix: 's:' })
    const gone = [...all].filter(([, s]) => this.expired(s, now)).map(([k]) => k)
    if (gone.length) await this.ctx.storage.delete(gone)
    for (const k of gone) all.delete(k)
    return all
  }
}

function tally(): Tally {
  return { messages: 0, bytes: 0 }
}

function sendOn(ws: WebSocket, message: Message): boolean {
  return sendText(ws, JSON.stringify(message))
}

function sendText(ws: WebSocket, text: string): boolean {
  try {
    ws.send(text)
    return true
  } catch {
    return false
  }
}
