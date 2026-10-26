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
/// Every value from the network goes into the page through `textContent`,
/// never markup: the CSP's Trusted Types (`require-trusted-types-for
/// 'script'`) refuse an HTML string sink outright.

import init, { acceptReply, connect, fingerprint, makeOffer, publicKeyLine, type WebRunner } from 'fc-web'

import { connectFailure, replyRefusal, unreachable } from './copy'
import { type Device, type Granted, loadDevice, publicKeyBytes, saveDevice, signer } from './device'
import { drawCode, scan } from './qr'
import { PaneView } from './terminal'
import { loadTailcat, type Tailcat } from './tunnel'

const $ = <T extends HTMLElement = HTMLElement>(id: string) => document.getElementById(id) as T

/// A call to the relay, through this Worker (services/web/src/api.ts). The
/// `x-fc` header and the same-origin `Origin` are its CSRF check.
async function relay(route: string, body: unknown = {}): Promise<any> {
  const response = await fetch(`/api/relay/${route}`, {
    method: 'POST',
    headers: { 'content-type': 'application/json', 'x-fc': '1' },
    body: JSON.stringify(body),
    credentials: 'same-origin',
  })
  if (response.status === 401) {
    location.assign('/')
    return null
  }
  return response.ok ? await response.json() : null
}

/// Never reloads on its own: a reload mid-keystroke would lose what was typed.
async function checkVersion(): Promise<void> {
  const response = await fetch('/version.json', { cache: 'no-store' }).catch(() => null)
  const deployed = response?.ok ? await response.json().catch(() => null) : null
  if (typeof deployed?.build !== 'string' || deployed.build === __BUILD__) return
  $('update').removeAttribute('hidden')
}

const SCREENS = ['status', 'enroll', 'work']

function show(screen: string): void {
  for (const id of SCREENS) $(id).toggleAttribute('hidden', id !== screen)
}

/// A full-page message, with at most one button.
function status(title: string, detail = '', action?: { label: string; run: () => void }): void {
  $('status-title').textContent = title
  $('status-detail').textContent = detail
  const button = $<HTMLButtonElement>('status-action')
  button.hidden = !action
  button.onclick = action ? () => action.run() : null
  if (action) button.textContent = action.label
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
  keyLine: string
  tailcat: Tailcat
}

// MARK: Enrolling

async function enroll(session: Session): Promise<void> {
  const { device } = session
  if (!device.offer) {
    const me = await relay('directory/me').catch(() => null)
    const account = typeof me?.user?.id === 'string' ? me.user.id : null
    if (!account) {
      status("Far Cooler couldn't check your account", 'Reload the page to try again.', { label: 'Reload', run: () => location.reload() })
      return
    }
    device.offer = makeOffer(deviceName(), account, session.keyLine, device.nodePub)
    await saveDevice(device)
  }

  drawCode($<HTMLCanvasElement>('offer-code'), device.offer)
  $('offer-fingerprint').textContent = `This browser’s key: ${fingerprint(session.keyLine) ?? ''}`
  const note = $('enroll-note')
  note.hidden = true
  show('enroll')

  $<HTMLButtonElement>('start-over').onclick = async () => {
    delete device.offer
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
      note.textContent = "Far Cooler can't use a camera here. Allow camera access for this site, then select Scan Reply again."
      note.hidden = false
    }
  }
}

async function takeReply(session: Session, reply: string): Promise<void> {
  const { device } = session
  let runners: Granted[]
  try {
    runners = JSON.parse(acceptReply(reply, device.offer ?? '', !device.offer)) as Granted[]
  } catch (e) {
    const note = $('enroll-note')
    note.textContent = replyRefusal((e as Error).message)
    note.hidden = false
    return
  }
  // Taken: the offer is spent, so a second reply to it is refused.
  delete device.offer
  device.runners = runners
  await saveDevice(device)
  void work(session)
}

// MARK: Working

async function work(session: Session): Promise<void> {
  const runners = session.device.runners ?? []
  const target = runners.find(r => r.why === '')
  if (!target) {
    const first = runners[0]
    status(
      'No runner here can be opened from a browser yet',
      first ? unreachable(first.why, first.label) : 'Your phone granted no runners.',
      { label: 'Add Again', run: () => void addAgain(session) },
    )
    return
  }

  status(`Connecting to ${target.label}…`)
  let runner: WebRunner
  try {
    const conn = await session.tailcat.dial(target.token, session.device.nodePriv)
    runner = await connect(conn, target.host_key, target.user, session.keyLine, signer(session.device))
  } catch (e) {
    const word = (e as Error).message
    const action =
      word === 'host_key_changed' || word === 'host_key_unknown' || word === 'key_rejected'
        ? { label: 'Add Again', run: () => void addAgain(session) }
        : { label: 'Try Again', run: () => void work(session) }
    status(`Can’t connect to ${target.label}`, connectFailure(word, target.label), action)
    return
  }

  let terminals: { id: string; title: string; state: string }[]
  try {
    terminals = JSON.parse(await runner.terminals())
  } catch (e) {
    status(`Can’t list ${target.label}’s terminals`, connectFailure((e as Error).message, target.label), {
      label: 'Try Again',
      run: () => void work(session),
    })
    return
  }

  $('runner-name').textContent = target.label
  const list = $('terminal-list')
  list.replaceChildren()
  show('work')
  if (terminals.length === 0) {
    paneNote(`${target.label} has no terminals yet. Open one in Far Cooler on your Mac or phone, then reload.`)
    return
  }

  const view = new PaneView($('terminal'), word => paneNote(connectFailure(word, target.label)))
  const open = async (id: string, button: HTMLButtonElement) => {
    for (const b of list.querySelectorAll('button')) b.removeAttribute('aria-current')
    button.setAttribute('aria-current', 'true')
    paneNote('')
    try {
      await view.attach(runner, id, () => paneNote('This terminal’s stream ended. Select it to open it again.'))
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
    button.onclick = () => void open(t.id, button)
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

async function addAgain(session: Session): Promise<void> {
  delete session.device.runners
  delete session.device.offer
  await saveDevice(session.device)
  void enroll(session)
}

// MARK: Boot

async function showWho(): Promise<void> {
  const me = await relay('directory/me').catch(() => null)
  const email = typeof me?.user?.email === 'string' ? me.user.email : null
  if (email) $('who').textContent = email
}

async function boot(): Promise<void> {
  $('reload').addEventListener('click', () => location.reload())
  window.addEventListener('focus', () => void checkVersion())
  void showWho()

  let tailcat: Tailcat
  try {
    ;[, tailcat] = await Promise.all([init({ module_or_path: __WASM__.web }), loadTailcat(__WASM__.tailcat)])
  } catch {
    status("Far Cooler couldn't start in this browser", 'Reload the page to try again.', { label: 'Reload', run: () => location.reload() })
    return
  }

  let device: Device
  try {
    device = await loadDevice(() => tailcat.mintNodeKey())
  } catch (e) {
    status(
      'This browser can’t hold a Far Cooler key',
      (e as Error).message === 'no_ed25519'
        ? 'Far Cooler needs a browser that can make Ed25519 keys. Use a current version of Chrome, Safari or Firefox.'
        : 'Far Cooler keeps its key in this site’s storage. Allow site data for this site, then reload.',
    )
    return
  }
  const keyLine = publicKeyLine(await publicKeyBytes(device), 'farcooler-web')
  const session: Session = { device, keyLine, tailcat }
  if (device.runners?.length) void work(session)
  else void enroll(session)
}

void boot()
