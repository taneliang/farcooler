/// The client shell (ov-764, plan-763 slice 1): enroll this browser with the
/// owner's phone, then open one of a runner's terminals.
///
/// 1. Both wasm modules load at boot — never lazily, because a deploy replaces
///    every hashed file and an open tab must not ask for one that's gone.
/// 2. A browser with no runners shows today's v2 offer as a QR code. The phone
///    runs its existing ceremony on it, and the browser reads the phone's reply
///    through the camera. crates/web accepts the reply with the ceremony's own
///    rules and keeps each runner's host key.
/// 3. A browser with runners dials the first one it can reach through the
///    tunnel, SSHes in with that host key pinned, lists its terminals and
///    attaches the one picked.
///
/// The SSH key is a passkey (ov-767). Safari shows a passkey prompt only
/// from a click, so every prompt here follows a button: Make Passkey before
/// the code, Connect before each sign-in, and Use Passkey if the browser
/// refused to prompt because that click was too long ago (the dial and the
/// SSH handshake come between). Each connection costs one touch.
///
/// Every value from the network goes into the page through `textContent`,
/// never markup: the CSP's Trusted Types (`require-trusted-types-for
/// 'script'`) refuse an HTML string sink outright.

import init, { acceptReply, connect, fingerprint, freshnessMs, makeOffer, newReplySecret, offerIsStale, openReply, passkeyKeyLine, registrationData, registrationProof, rpId, type WebRunner } from 'fc-web'

import { connectFailure, joinDetail, joinTitle, linkNotice, MISMATCH, registerFailure, replyRefusal, sealedRefusal, unreachable } from './copy'
import { assertWith, canUsePasskeys, userHandle, type Device, forgetDevice, type Granted, loadDevice, makePasskey, type Passkey, saveDevice, type Signed } from './device'
import { Board, type BoardView, countLabel, countOf, pageTitle } from './needs'
import { renderNeeds } from './needs-view'
import { type PlanTab, startPlanTab } from './plan-tab'
import { ChatView } from './chat'
import { askToUsePasskey } from './passkey-prompt'
import { drawCode, scan } from './qr'
import { PaneSet } from './panes'
import { comparisonButtons, type Opened, registerDevice, waitForReply } from './sealed'
import { type LinkView, Reconnector } from './reconnect'
import { PaneView } from './terminal'
import { loadTailcat, type Tailcat, type TunnelConn } from './tunnel'

const $ = <T extends HTMLElement = HTMLElement>(id: string) => document.getElementById(id) as T

/// A call to the relay, through this Worker (services/web/src/api.ts). The
/// `x-fc` header and the same-origin `Origin` are its CSRF check.
async function relayRaw(route: string, body: unknown = {}): Promise<{ status: number; body: any }> {
  const response = await fetch(`/api/relay/${route}`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', 'x-fc': '1' },
    body: JSON.stringify(body),
    credentials: 'same-origin',
  })
  if (response.status === 401) {
    location.assign('/')
    return { status: 401, body: null }
  }
  return { status: response.status, body: await response.json().catch(() => null) }
}

async function relay(route: string, body: unknown = {}): Promise<any> {
  const answer = await relayRaw(route, body)
  return answer.status >= 200 && answer.status < 300 ? answer.body : null
}

/// Never reloads on its own: a reload mid-keystroke would lose what was typed.
async function checkVersion(): Promise<void> {
  const response = await fetch('/version.json', { cache: 'no-store' }).catch(() => null)
  const deployed = response?.ok ? await response.json().catch(() => null) : null
  if (typeof deployed?.build !== 'string' || deployed.build === __BUILD__) return
  $('update').removeAttribute('hidden')
}

const SCREENS = ['status', 'enroll', 'compare', 'join', 'work']

function show(screen: string): void {
  for (const id of SCREENS) $(id).toggleAttribute('hidden', id !== screen)
}

/// A full-page message, with at most one button, and Cancel when `cancel`
/// is given.
function status(title: string, detail = '', action?: { label: string; run: () => void }, cancel?: () => void): void {
  $('status-title').textContent = title
  $('status-detail').textContent = detail
  const button = $<HTMLButtonElement>('status-action')
  button.hidden = !action
  button.onclick = action ? () => action.run() : null
  if (action) button.textContent = action.label
  const quit = $<HTMLButtonElement>('status-cancel')
  quit.hidden = !cancel
  quit.onclick = cancel ? () => cancel() : null
  show('status')
}

/// What this browser calls itself on the phone's confirmation screen.
function deviceName(): string {
  const ua = navigator.userAgent
  const browser = /Edg\//.test(ua) ? 'Edge' : /Firefox\//.test(ua) ? 'Firefox' : /Chrome\//.test(ua) ? 'Chrome' : /Safari\//.test(ua) ? 'Safari' : 'Browser'
  const os = /Windows/.test(ua) ? 'Windows' : /Mac OS X/.test(ua) ? 'Mac' : /CrOS/.test(ua) ? 'ChromeOS' : /Linux/.test(ua) ? 'Linux' : 'Computer'
  return `${browser} on ${os}`
}

interface Session {
  device: Device
  /// The passkey's OpenSSH line, once there is a passkey.
  keyLine: string
  email: string
  /// The passkey's user handle for this account (`userHandle`).
  user: Uint8Array
  tailcat: Tailcat
}

// MARK: The passkey

/// Ask for this browser's passkey, from a button.
function passkeyScreen(session: Session, migrated: boolean, notice = ''): void {
  const why = migrated ? 'Far Cooler now signs in to your runners with a passkey instead of a key kept in this site’s storage, so this browser needs adding again. ' : ''
  status(
    'Make a passkey for this browser',
    notice ||
      `${why}Your passkey stays in this device’s password manager or security key, and Far Cooler never sees it. You’ll touch it once each time this browser connects to a runner.`,
    {
      label: 'Make Passkey',
      run: () =>
        void makePasskey(rpId(), session.user, session.email).then(
          async passkey => {
            session.device.passkey = passkey
            await saveDevice(session.device)
            session.keyLine = keyLineOf(passkey)
            void enroll(session)
          },
          e =>
            passkeyScreen(
              session,
              migrated,
              (e as Error).message === 'declined'
                ? 'Far Cooler didn’t get a passkey. Select Make Passkey and follow your browser’s prompt.'
                : 'This browser couldn’t make a passkey Far Cooler can use. Use a current version of Chrome, Safari, Edge or Firefox.',
            ),
        ),
    },
  )
}

function keyLineOf(passkey: Passkey): string {
  return passkeyKeyLine(passkey.spki, 'farcooler-web')
}

/// The timer that retires the code on screen when it expires.
let retiring = 0

/// The secret half of the reply key behind the code on screen. In memory, and
/// in this tab's `sessionStorage` so a reload keeps the code answerable; never
/// in IndexedDB, which outlives the tab. A code whose secret is gone is retired.
const SECRET = 'fc-reply-secret'
let replySecret: string | null = null
function keepSecret(secret: string): void {
  replySecret = secret
  try {
    sessionStorage.setItem(SECRET, secret)
  } catch {}
}
function loadSecret(): string | null {
  if (replySecret) return replySecret
  try {
    replySecret = sessionStorage.getItem(SECRET)
  } catch {}
  return replySecret
}
function dropSecret(): void {
  replySecret = null
  try {
    sessionStorage.removeItem(SECRET)
  } catch {}
}

/// Which wait for a sealed reply is current; a new count cancels the one before.
let polling = 0
const stopPolling = () => void ++polling

// MARK: Registering

/// Put this browser's passkey on the account's device list, so the phone can
/// certify it and it shows there as a device (ov-771). One touch. Tried as soon
/// as it is needed; if the browser refuses to prompt without a click (Safari),
/// a button asks for the click.
async function register(session: Session): Promise<void> {
  const { device } = session
  const passkey = device.passkey as Passkey
  const rp = rpId()
  status('Adding this browser to your account…', 'Touch your passkey if your browser asks.')
  const done = await registerDevice(
    {
      call: relayRaw,
      sign: data => assertWith(passkey, rp, data),
      data: registrationData,
      proof: (account, challenge, keyLine, signed) =>
        registrationProof(account, challenge, keyLine, new Uint8Array(signed.authenticatorData), new Uint8Array(signed.clientDataJSON), new Uint8Array(signed.signature)),
    },
    device.account,
    session.keyLine,
    deviceName(),
  )
  if (!done.ok) {
    status('Add this browser to your account', registerFailure(done.word), { label: 'Register This Browser', run: () => void register(session) })
    return
  }
  device.registered = true
  await saveDevice(device)
  void enroll(session)
}

// MARK: Enrolling

/// Show this browser's code, and read the phone's reply to it.
///
/// A code is answerable for the ceremony's window (two minutes) from when it
/// went up, as on every other device: the reply leg carries no proof of who
/// wrote it, and everything a forged reply must echo is printed in the code. So
/// a stored code past the window is discarded for a new one, a code on screen
/// is replaced when it expires, and a reply that comes later is refused in
/// wasm (`acceptReply`).
async function enroll(session: Session, notice = ''): Promise<void> {
  const { device } = session
  clearTimeout(retiring)
  stopPolling()
  if (!device.registered) {
    // Tried without a click first; a refusal to prompt becomes a button.
    await register(session)
    return
  }
  if (device.offer && (offerIsStale(device.offerShownAt ?? 0) || !loadSecret())) {
    delete device.offer
    delete device.offerShownAt
  }
  if (!device.offer) {
    const secret = newReplySecret()
    keepSecret(secret)
    device.offer = makeOffer(deviceName(), device.account, session.keyLine, device.nodePub, secret)
    device.offerShownAt = Date.now()
    await saveDevice(device)
  }

  drawCode($<HTMLCanvasElement>('offer-code'), device.offer)
  $('offer-fingerprint').textContent = `This browser’s passkey: ${fingerprint(session.keyLine) ?? ''}`
  const note = $('enroll-note')
  note.textContent = notice
  note.hidden = notice === ''
  show('enroll')
  // Replaced on time, unless the camera is open: a reply read then is judged
  // by the window anyway, and a new code mid-scan would only confuse.
  retiring = window.setTimeout(() => {
    if ($('enroll').hidden || !$('camera').hidden) return
    void enroll(session, 'That code expired, so here’s a new one. Scan it with your phone, then scan its reply within two minutes.')
  }, Math.max(0, (device.offerShownAt ?? 0) + freshnessMs() - Date.now()) + 500)

  void waitForSealedReply(session, note)

  $<HTMLButtonElement>('start-over').onclick = async () => {
    dropSecret()
    delete device.offer
    delete device.offerShownAt
    await saveDevice(device)
    void enroll(session)
  }
  $<HTMLButtonElement>('scan').onclick = async () => {
    const camera = $('camera')
    camera.hidden = false
    note.hidden = true
    let scanning: { stop: () => void } | null = null
    $<HTMLButtonElement>('camera-cancel').onclick = () => {
      scanning?.stop()
      camera.hidden = true
    }
    try {
      scanning = await scan($<HTMLVideoElement>('camera-video'), text => {
        // The offer's own code is on this screen too; only a reply counts.
        if (!/"runners"\s*:/.test(text)) return false
        camera.hidden = true
        void takeReply(session, text)
        return true
      })
    } catch {
      camera.hidden = true
      note.textContent = "Far Cooler can’t use a camera here. Allow camera access for this site, then select Scan Reply again."
      note.hidden = false
    }
  }
}

/// Wait for the phone's sealed reply to the code on screen. When one opens, the
/// person compares its code; nothing is kept before they say it matches.
function waitForSealedReply(session: Session, note: HTMLElement): void {
  const { device } = session
  const offer = device.offer
  if (!offer) return
  const ceremony = (JSON.parse(offer) as { ceremony: string }).ceremony
  const mine = ++polling
  void waitForReply(
    {
      read: id => relayRaw('ceremony/reply', { ceremony: id }),
      open: answer => JSON.parse(openReply(answer, device.offer ?? '', loadSecret() ?? '', !device.offer, device.offerShownAt ?? 0)) as Opened,
      now: () => Date.now(),
      sleep: ms => new Promise(resolve => setTimeout(resolve, ms)),
      cancelled: () => mine !== polling,
      refused: word => {
        note.textContent = sealedRefusal(word)
        note.hidden = false
      },
    },
    ceremony,
  ).then(async waited => {
    if (waited.kind === 'opened') compare(session, waited.opened)
    else if (waited.kind === 'ended' && mine === polling) {
      // Too late for this code, and so for any reply to it: a new one.
      dropSecret()
      delete device.offer
      delete device.offerShownAt
      await saveDevice(device)
      void enroll(session, sealedRefusal(waited.word))
    }
  })
}

/// The reply's own code, to match against the phone's before it is kept.
function compare(session: Session, opened: Opened): void {
  stopPolling()
  clearTimeout(retiring)
  $('compare-code').textContent = opened.code
  show('compare')
  // Nothing was kept on a mismatch and the code is still unanswered: the QR
  // reply and a later sealed one still work.
  const buttons = comparisonButtons(opened, taken => keep(session, taken), () => void enroll(session, MISMATCH))
  const yes = $<HTMLButtonElement>('compare-yes')
  const no = $<HTMLButtonElement>('compare-no')
  yes.disabled = false
  no.disabled = false
  // The first tap answers, so both buttons go dark at once: no second save.
  yes.onclick = () => {
    yes.disabled = true
    no.disabled = true
    void buttons.yes()
  }
  no.onclick = () => {
    yes.disabled = true
    no.disabled = true
    void buttons.no()
  }
}

/// Keep what a confirmed reply grants.
async function keep(session: Session, opened: Opened): Promise<void> {
  await finishTaken(session, { granted_by: opened.granted_by, runners: opened.runners })
}

async function takeReply(session: Session, reply: string): Promise<void> {
  const { device } = session
  let taken: { granted_by: string | null; runners: Granted[] }
  try {
    taken = JSON.parse(acceptReply(reply, device.offer ?? '', !device.offer, device.offerShownAt ?? 0))
  } catch (e) {
    const word = (e as Error).message
    if (word === 'stale') {
      // Too late for this code, and so for any reply to it: a new one.
      delete device.offer
      delete device.offerShownAt
      await saveDevice(device)
      void enroll(session, replyRefusal(word))
      return
    }
    const note = $('enroll-note')
    note.textContent = replyRefusal(word)
    note.hidden = false
    return
  }
  await finishTaken(session, taken)
}

/// A reply the person has accepted (a scanned one, or a sealed one whose code
/// they matched): the offer is spent, so a second reply to it is refused.
async function finishTaken(session: Session, taken: { granted_by: string | null; runners: Granted[] }): Promise<void> {
  const { device } = session
  stopPolling()
  clearTimeout(retiring)
  dropSecret()
  delete device.offer
  delete device.offerShownAt
  await saveDevice(device)
  if (taken.granted_by) {
    confirmJoin(session, taken.granted_by, taken.runners)
    return
  }
  device.runners = taken.runners
  await saveDevice(device)
  void work(session)
}

/// Another account's runners: asked about, never joined silently, as a phone
/// asks (apps/ios/FarCooler/Ceremony/JoinView.swift, TeammateJoinPrompt).
function confirmJoin(session: Session, from: string, runners: Granted[]): void {
  $('join-title').textContent = joinTitle(runners.map(r => r.label))
  $('join-detail').textContent = joinDetail(from)
  const list = $('join-runners')
  list.replaceChildren(
    ...(runners.length > 1
      ? runners.map(r => {
          const item = document.createElement('li')
          item.textContent = r.label
          return item
        })
      : []),
  )
  show('join')
  $<HTMLButtonElement>('join-yes').onclick = async () => {
    session.device.runners = runners
    await saveDevice(session.device)
    void work(session)
  }
  $<HTMLButtonElement>('join-no').onclick = () => void enroll(session)
}

// MARK: Working

/// The terminals on the page, so a second visit to `work` replaces them.
let panes: PaneSet<WebRunner> | null = null

// MARK: Needs You

/// How often the list is read while the tab is in view. The runner pushes
/// nothing a browser listens to yet, so this is what shows an answer made
/// on another device.
const NEEDS_POLL_MS = 5000

/// The runner's Needs You on the page, read again every few seconds.
let board: Board | null = null
let boardTimer = 0
let drawn = ''

/// The terminal buttons by id, which is what an Open on an item can go to.
const buttonFor = new Map<string, HTMLButtonElement>()

/// The plan tab, while a runner is connected.
let planTab: PlanTab | null = null

/// Which part of the work screen is in front.
function showTab(tab: 'terminals' | 'needs' | 'plan'): void {
  const needs = tab === 'needs'
  const plan = tab === 'plan'
  $('needs').hidden = !needs
  $('plan').hidden = !plan
  $('pane').hidden = needs || plan
  $('terminal-list').hidden = needs || plan
  $('show-terminals').setAttribute('aria-pressed', String(tab === 'terminals'))
  $('show-needs').setAttribute('aria-pressed', String(needs))
  $('show-plan').setAttribute('aria-pressed', String(plan))
  planTab?.show(plan)
  if (needs) void board?.refresh()
}

/// Put the board's view on the page: the list, the count on the button, the
/// bar and the tab's title.
function drawNeeds(view: BoardView): void {
  const count = countOf(view.items)
  $('show-needs').textContent = countLabel(count)
  const bar = $<HTMLButtonElement>('bar-count')
  bar.hidden = count === 0
  bar.textContent = `${count} Waiting`
  document.title = pageTitle(count)
  drawn = renderNeeds(
    $('needs'),
    view,
    Date.now(),
    {
      canOpen: terminal => buttonFor.has(terminal),
      press: (item, press) => {
        if (press.kind === 'open') showTab('terminals')
        void board?.press(item, press)
      },
    },
    drawn,
  )
}

function stopNeeds(): void {
  planTab?.stop()
  planTab = null
  clearInterval(boardTimer)
  board?.stop()
  board = null
  drawn = ''
  document.title = pageTitle(0)
  $('bar-count').hidden = true
}

async function work(session: Session): Promise<void> {
  const runners = session.device.runners ?? []
  const target = runners.find(r => r.why === '')
  if (!target) {
    const first = runners[0]
    status(
      'No runner here can be opened from a browser yet',
      first ? unreachable(first.why, first.label, first.cause) : 'Your phone granted no runners.',
      { label: 'Add Again', run: () => void addAgain(session) },
    )
    return
  }

  // A button first: the passkey prompt this leads to needs a click (Safari).
  status(`Connect to ${target.label}`, 'You’ll touch your passkey to sign in.', { label: 'Connect', run: () => void connectTo(session, target) })
}

/// The passkey's signature over the SSH login's data: asked for straight
/// away, which Chrome, Edge and Firefox allow whenever, and Safari while the
/// Connect click still counts. If the browser refused to prompt (Safari, once
/// the dial and the handshake outlasted that click) or the person dismissed
/// the prompt, a Use Passkey button asks again, with the call made inside its
/// click, until Cancel or its window closes (`askToUsePasskey`). Either way
/// one touch signs.
function signIn(session: Session, label: string): (challenge: Uint8Array) => Promise<Signed> {
  const passkey = session.device.passkey as Passkey
  const rp = rpId()
  return async challenge => {
    try {
      return await assertWith(passkey, rp, challenge)
    } catch (e) {
      if ((e as Error).message !== 'declined') throw e
    }
    return await askToUsePasskey(
      (use, cancel) =>
        status(
          `Sign in to ${label}`,
          'Select Use Passkey, then touch your passkey.',
          {
            label: 'Use Passkey',
            run: () => {
              status(`Connecting to ${label}…`)
              use()
            },
          },
          () => {
            status(`Connecting to ${label}…`)
            cancel()
          },
        ),
      () => assertWith(passkey, rp, challenge),
    )
  }
}

/// The connection on the page, so a tab coming back or the browser coming
/// online can ask whether it still answers (ov-772).
let link: Reconnector<WebRunner, TunnelConn> | null = null

/// What the page says about the connection: nothing while it is up, else a
/// notice above the terminal, with Reconnect when a click is wanted.
function showLink(view: LinkView, label: string): void {
  // The first connection is still listing terminals; the screen is not up yet.
  if (view.kind !== 'up' || panes) show('work')
  const notice = linkNotice(view, label)
  $('link-note').hidden = !notice
  if (!notice) return
  $('link-title').textContent = notice.title
  $('link-detail').textContent = notice.detail
  const action = $<HTMLButtonElement>('link-action')
  action.hidden = !notice.action
  if (notice.action) action.textContent = notice.action
  action.onclick = () => void link?.reconnect()
}

async function connectTo(session: Session, target: Granted): Promise<void> {
  status(`Connecting to ${target.label}…`)
  let runner: WebRunner
  const reconnector = new Reconnector<WebRunner, TunnelConn>({
    dial: () => session.tailcat.dial(target.token, session.device.nodePriv),
    discard: conn => conn.close(),
    connect: (conn, onLost) => connect(conn, target.host_key, target.user, session.keyLine, signIn(session, target.label), onLost),
    resume: async next => {
      await panes?.resume(next, () => ended(next))
      void board?.refresh()
    },
    show: view => showLink(view, target.label),
    now: () => Date.now(),
  })
  /// A pane's stream ended: said so unless the whole connection did, which
  /// the notice above the terminal already says.
  const ended = (from: WebRunner) => {
    // Asked a moment later: the loss reaches the page as the pane's end and as
    // `onLost`, in either order. The reconnector remembers it; the wasm object
    // cannot be asked once it is freed.
    setTimeout(() => {
      if (!reconnector.hasLost(from) && reconnector.current === from) paneNote('This terminal’s stream ended. Select it to open it again.')
    }, 100)
  }
  link = reconnector
  let dialed: TunnelConn | null = null
  try {
    const conn = await session.tailcat.dial(target.token, session.device.nodePriv)
    dialed = conn
    runner = await reconnector.open(conn)
  } catch (e) {
    dialed?.close()
    const word = (e as Error).message
    const action =
      word === 'host_key_changed' || word === 'host_key_unknown' || word === 'key_rejected'
        ? { label: 'Add Again', run: () => void addAgain(session) }
        : { label: 'Connect', run: () => void connectTo(session, target) }
    status(`Can’t connect to ${target.label}`, connectFailure(word, target.label), action)
    return
  }

  let terminals: { id: string; title: string; state: string; agent: boolean }[]
  try {
    terminals = JSON.parse(await runner.terminals())
  } catch (e) {
    status(`Can’t list ${target.label}’s terminals`, connectFailure((e as Error).message, target.label), {
      label: 'Connect',
      run: () => void connectTo(session, target),
    })
    return
  }

  $('runner-name').textContent = target.label
  const list = $('terminal-list')
  list.replaceChildren()
  show('work')

  stopNeeds()
  const opening = (id: string) => void buttonFor.get(id)?.click()
  buttonFor.clear()
  const mine = new Board({
    runner: target.label,
    list: async () => {
      const current = reconnector.current
      if (!current) throw new Error('closed')
      return await current.needsYou()
    },
    ask: async (terminal, ask, option) => {
      const current = reconnector.current
      if (!current) throw new Error('closed')
      await current.answer(terminal, ask, option)
    },
    decide: async (task, body) => {
      const current = reconnector.current
      if (!current) throw new Error('closed')
      await current.answerDecision(task, body)
    },
    show: drawNeeds,
    open: opening,
  })
  board = mine
  planTab = startPlanTab($('plan'), target.label, () => (reconnector.current && !reconnector.hasLost(reconnector.current) ? reconnector.current : null))
  showTab('terminals')
  void mine.refresh()
  boardTimer = window.setInterval(() => {
    if (document.visibilityState === 'visible' && reconnector.current && !reconnector.hasLost(reconnector.current)) void mine.refresh()
  }, NEEDS_POLL_MS)
  if (terminals.length === 0) {
    paneNote(`${target.label} has no terminals yet. Open one in Far Cooler on your Mac or phone, then reload.`)
    return
  }

  panes?.dispose()
  // One xterm per terminal, each in a host of its own inside the stage. Only
  // the one in front is displayed; the rest keep their screens and, up to the
  // channel budget, their streams (panes.ts). An agent's terminal opens as
  // its conversation (chat.ts), which rides the control session and holds no
  // channel.
  const agents = new Set(terminals.filter(t => t.agent).map(t => t.id))
  const set = new PaneSet<WebRunner>(id => {
    const host = document.createElement('div')
    host.className = 'terminal-host'
    $('terminal').append(host)
    if (agents.has(id)) return new ChatView(host, target.label)
    return new PaneView(host, word => paneNote(connectFailure(word, target.label)))
  })
  panes = set
  const open = async (id: string, button: HTMLButtonElement) => {
    for (const b of list.querySelectorAll('button')) b.removeAttribute('aria-current')
    button.setAttribute('aria-current', 'true')
    paneNote('')
    const current = reconnector.current
    if (!current) return
    try {
      await set.open(id, current, () => ended(current))
    } catch (e) {
      paneNote(connectFailure((e as Error).message, target.label))
    }
  }
  const buttons = terminals.map(t => {
    const button = document.createElement('button')
    button.type = 'button'
    button.className = 'terminal-choice'
    button.textContent = t.title || 'Terminal'
    button.title = t.state
    if (t.agent) button.dataset.agent = 'true'
    button.onclick = () => void open(t.id, button)
    buttonFor.set(t.id, button)
    list.append(button)
    return button
  })
  const first = Math.max(0, terminals.findIndex(t => t.state === 'running'))
  void open(terminals[first].id, buttons[first])
}

function paneNote(text: string): void {
  const note = $('pane-note')
  note.textContent = text
  note.hidden = text === ''
}

/// The tab is visible again or the browser is online: ask the runner now
/// rather than wait out the probe interval.
function recheckLink(): void {
  void link?.recheck()
}

async function addAgain(session: Session): Promise<void> {
  link = null
  stopNeeds()
  panes?.dispose()
  panes = null
  dropSecret()
  delete session.device.runners
  delete session.device.offer
  delete session.device.offerShownAt
  await saveDevice(session.device)
  void enroll(session)
}

// MARK: Boot

/// Sign Out takes this browser's key and runners with it, before the form
/// posts. The Worker's answer also clears the origin's storage
/// (`Clear-Site-Data`), which covers a sign-out this page didn't run.
function signOutForgets(): void {
  const form = document.querySelector<HTMLFormElement>('form[action="/sign-out"]')
  if (!form) return
  form.addEventListener('submit', event => {
    event.preventDefault()
    dropSecret()
    void forgetDevice()
      .catch(() => {})
      .finally(() => form.submit())
  })
}

async function boot(): Promise<void> {
  $('reload').addEventListener('click', () => location.reload())
  $('show-terminals').addEventListener('click', () => showTab('terminals'))
  $('show-needs').addEventListener('click', () => showTab('needs'))
  $('show-plan').addEventListener('click', () => showTab('plan'))
  $('bar-count').addEventListener('click', () => showTab('needs'))
  window.addEventListener('focus', () => void checkVersion())
  document.addEventListener('visibilitychange', () => {
    if (document.visibilityState === 'visible') recheckLink()
  })
  window.addEventListener('online', recheckLink)
  signOutForgets()

  // Which account is signed in decides which device this browser may use, so
  // nothing starts without it.
  const me = await relay('directory/me').catch(() => null)
  const account = typeof me?.user?.id === 'string' && me.user.id ? (me.user.id as string) : null
  if (typeof me?.user?.email === 'string') $('who').textContent = me.user.email
  if (!account) {
    status("Far Cooler couldn’t check your account", 'Reload the page to try again.', { label: 'Reload', run: () => location.reload() })
    return
  }

  let tailcat: Tailcat
  try {
    ;[, tailcat] = await Promise.all([init({ module_or_path: __WASM__.web }), loadTailcat(__WASM__.tailcat)])
  } catch {
    status("Far Cooler couldn’t start in this browser", 'Reload the page to try again.', { label: 'Reload', run: () => location.reload() })
    return
  }

  if (!canUsePasskeys()) {
    status('This browser can’t use passkeys', 'Far Cooler signs in to your runners with a passkey. Use a current version of Chrome, Safari, Edge or Firefox.')
    return
  }
  let kept: { device: Device; migrated: boolean }
  try {
    kept = await loadDevice(account, () => tailcat.mintNodeKey())
  } catch {
    status('This browser can’t keep Far Cooler’s settings', 'Far Cooler keeps this browser’s runners in this site’s storage. Allow site data for this site, then reload.')
    return
  }
  const { device, migrated } = kept
  const email = typeof me?.user?.email === 'string' ? (me.user.email as string) : ''
  const session: Session = { device, keyLine: device.passkey ? keyLineOf(device.passkey) : '', email, user: await userHandle(account), tailcat }
  if (!device.passkey) passkeyScreen(session, migrated)
  else if (device.runners?.length) void work(session)
  else void enroll(session)
}

void boot()
