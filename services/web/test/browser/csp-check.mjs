#!/usr/bin/env node
// The client shell in headless Chromium, under the CSP production sends,
// failing on any violation (ov-764 acceptance 4), with its SSH key a passkey
// held by Chromium's virtual authenticator (ov-767).
//
//   node test/browser/csp-check.mjs [--dist dist] [--fake-daemon BINARY] [--screenshots DIR] [--cpu-throttle N] [--tunnel-delay MS] [--max-sessions N]
//
// To run just the workspace walks, add --workstation-only with --fake-daemon.
// --workstation-screenshots DIR saves the split at phone and desktop widths.
//
// Serves dist/ (build.sh's output, with the real wasm) through
// shell-server.ts, which uses the Worker's own header and asset code, at
// https://<the build's app host>/ — routed inside the browser, so nothing goes
// over the network. The page has to be on that exact host: the bundle makes
// and uses its passkey for it, and a browser allows that only from there.
//
// It walks the page on one Chromium profile, relaunched where a step needs a
// different fake camera (the passkey is carried across by the DevTools
// WebAuthn domain, as a person's would stay in their authenticator):
//
// - boot (both wasm modules, a node key minted in Go), Make Passkey, the
//   enrollment screen with the passkey's sk-ecdsa line in the code, and no
//   unsigned camera fallback;
// - a stored code past the ceremony's window is replaced, another account's
//   passkey and runners are discarded, and a slice-1 device (a WebCrypto
//   Ed25519 key) is asked for a passkey and a new enrollment;
// - with --fake-daemon: an unsigned teammate reply is refused; a signed
//   reply verifies the device and runner authority and asks before it joins;
//   Connect opens a terminal through a REAL sshd (only sshd verifies a
//   passkey's webauthn-sk signature) whose Key A line is the offer's sk-ecdsa
//   key and whose forced command is crates/web's fake daemon
//   (`cargo build --example fake_daemon`); the runner gets the keystrokes and
//   sshd logs ECDSA-SK; eight reconnects cost one touch each; the same runner
//   with its pin altered is refused without a touch. The page's tunnel dial
//   is swapped, by this harness only, for a WebSocket spliced to that sshd;
//   everything above the dial is the page's own: SSH in wasm, the passkey's
//   assertion, the pinned host key, terminal.list, xterm;
// - one runner with several workspaces (ov-871, ov-1445): a picker scopes the terminals,
//   Needs You and the plan to one workspace at a time and remembers the pick;
//   a card opens its detail from a Needs You item and from a plan card, where a
//   decision is answered and a note is added; an agent's pane switches between
//   its conversation and its terminal; images attached, pasted and dropped
//   reach the agent as one compose, and one past the limit is refused;
// - on an authenticator with PRF (ov-779): the node key is derived from the
//   passkey and a sealed copy reaches the relay; after the site's storage is
//   cleared, Use Passkey brings the runners back at one touch and no
//   ceremony, with the enrolled node key. Every step before it runs on an
//   authenticator without PRF, which is the browser that keeps working as
//   before;
// - the passkey can't be used from the parent domain's other child
//   (canary.farcooler.com for canary): Chromium refuses it (ov-767
//   acceptance 2);
// - Sign Out deletes the passkey's id, the runners and the passkey's sealed copy at the relay.
//
// What the swapped dial leaves out is Tailcat's own traffic: the DERP
// WebSocket and its `/derp/probe` fetch, which the CSP must allow too. CI
// can't depend on a public relay, so that path was checked by hand through a
// real tunnel (ov-764 lane report), and it is what the owner's live check
// exercises.
//
// Chromium comes from playwright-core; set PLAYWRIGHT_BROWSERS_PATH to where
// `npx playwright-core install chromium-headless-shell` put it.

import { execFileSync, spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { existsSync, readFileSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { createConnection, createServer } from 'node:net'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { pathToFileURL } from 'node:url'
import { brotliDecompressSync } from 'node:zlib'

import { build } from 'esbuild'
import { chromium } from 'playwright-core'

const args = process.argv.slice(2)
const flag = name => {
  const at = args.indexOf(name)
  return at >= 0 ? args[at + 1] : undefined
}
const dist = resolve(flag('--dist') ?? 'dist')
const fakeDaemon = flag('--fake-daemon') && resolve(flag('--fake-daemon'))
// For a person judging the screens: each one at a desktop and a phone width.
const shots = flag('--screenshots')
const workstationShots = flag('--workstation-screenshots')
const workstationOnly = args.includes('--workstation-only')
if (workstationOnly && !fakeDaemon) throw new Error('--workstation-only needs --fake-daemon')
// Slow the page's CPU by this factor (Chromium's own throttling), to run the
// walk on a slow machine. A slow link is --tunnel-delay, which CI uses.
const throttle = Number(flag('--cpu-throttle') ?? 1)
// Delay each tunnel message by this many ms each way, for a slow network.
const delay = Number(flag('--tunnel-delay') ?? 0)
// The scratch sshd's MaxSessions. Below 10, the walk is the low-limit check
// alone (ov-846): panes learn the limit from the first refusal.
const maxSessions = Number(flag('--max-sessions') ?? 10)
const later = send => (delay > 0 ? setTimeout(send, delay) : send())
const shoot = async (page, name) => {
  const directory = name.startsWith('workstation-') ? workstationShots ?? shots : shots
  if (!directory) return
  for (const [width, height] of [[1280, 800], [390, 844]]) {
    await page.setViewportSize({ width, height })
    await page.waitForTimeout(300)
    await page.screenshot({ path: join(directory, `${name}-${width}.png`) })
  }
  await page.setViewportSize({ width: 1280, height: 800 })
}

// The canary channel's DERP settings, from wrangler.toml.
const toml = readFileSync('wrangler.toml', 'utf8')
const canary = toml.slice(toml.indexOf('[env.canary.vars]'))
const setting = name => canary.match(new RegExp(`^${name} = "([^"]*)"`, 'm'))?.[1] ?? ''
const env = { DERP_HOSTS: setting('DERP_HOSTS'), DERP_MAP_ORIGIN: setting('DERP_MAP_ORIGIN') }

// The app host this bundle signs for: crates/web's rpId(), from the channel
// stamped into it (Channel::app_host). A local build is canary's.
const built = JSON.parse(readFileSync(join(dist, 'version.json'), 'utf8')).channel
const HOST = built === 'stable' || built === 'preview' ? 'app.farcooler.com' : 'app.canary.farcooler.com'
const origin = `https://${HOST}`
// Another child of the parent domain, which must not be able to use the
// passkey (ov-767 acceptance 2).
const SIBLING = HOST === 'app.farcooler.com' ? 'www.farcooler.com' : 'canary.farcooler.com'

const scratch = mkdtempSync(join(tmpdir(), 'fc-csp-'))
const bundled = join(scratch, 'shell-server.mjs')
await build({ entryPoints: ['test/browser/shell-server.ts'], bundle: true, platform: 'node', format: 'esm', outfile: bundled, logLevel: 'error' })
const { shellHandler } = await import(pathToFileURL(bundled).href)

const reports = { csp: [] }
const read = path => {
  try {
    return readFileSync(join(dist, path))
  } catch {
    return null
  }
}
/// How long the stub relay holds `directory/me`, for the slow-start step.
let meDelay = 0
const handle = shellHandler(read, env, reports, { meDelay: () => meDelay })

/// Answer a routed request with the Worker's own handler.
async function serve(route) {
  const request = route.request()
  const method = request.method()
  const body = method === 'GET' || method === 'HEAD' ? undefined : (request.postDataBuffer() ?? undefined)
  let response = await handle(new Request(request.url(), { method, headers: request.headers(), body }))
  // Chromium takes a fulfilled redirect to the network, where these hosts
  // don't resolve, so the one redirect the shell makes (Sign Out's 303) is
  // followed here.
  if (response.status === 303) response = await handle(new Request(new URL(response.headers.get('location'), request.url()), { headers: request.headers() }))
  const headers = {}
  response.headers.forEach((value, key) => (headers[key] = value))
  let bytes = Buffer.from(await response.arrayBuffer())
  // A fulfilled body reaches the page as given, never decoded, so the
  // Worker's brotli (each .wasm, served with content-encoding: br) is
  // undone here, where a network stack would.
  if (headers['content-encoding'] === 'br') {
    bytes = brotliDecompressSync(bytes)
    delete headers['content-encoding']
    delete headers['content-length']
  }
  await route.fulfill({ status: response.status, headers, body: bytes })
}

// The runner: a real sshd on a loopback port, whose one Key A line is written
// once the page's offer names its passkey, in front of the fake daemon.
const me = execFileSync('/usr/bin/id', ['-un']).toString().trim()
/// A scratch sshd in front of the fake daemon, named `name` (its one ask says
/// so), on its own loopback port. The walk uses one; the every-runner step
/// (ov-861) uses a second.
async function startRunner(name) {
  if (!existsSync('/usr/sbin/sshd')) throw new Error('no /usr/sbin/sshd here, so the passkey login cannot be checked')
  const dir = join(scratch, name === 'studio' ? 'sshd' : `sshd-${name}`)
  execFileSync('/bin/mkdir', [dir])
  execFileSync('/usr/bin/ssh-keygen', ['-q', '-t', 'ed25519', '-N', '', '-C', 'host', '-f', join(dir, 'host')])
  const hostKey = execFileSync('/usr/bin/ssh-keygen', ['-lf', join(dir, 'host.pub')]).toString().split(' ')[1]
  const port = await new Promise(done => {
    const probe = createServer().listen(0, '127.0.0.1', () => {
      const { port } = probe.address()
      probe.close(() => done(port))
    })
  })
  const keys = join(dir, 'authorized_keys')
  writeFileSync(keys, '')
  const config = join(dir, 'sshd_config')
  const pidFile = join(dir, 'sshd.pid')
  writeFileSync(
    config,
    [
      `Port ${port}`,
      'ListenAddress 127.0.0.1',
      `HostKey ${join(dir, 'host')}`,
      `AuthorizedKeysFile ${keys}`,
      `PidFile ${pidFile}`,
      'StrictModes no',
      'UsePAM no',
      'PasswordAuthentication no',
      'KbdInteractiveAuthentication no',
      // The channel budget (ov-774) is checked against sshd's own default;
      // --max-sessions sets a lower one (ov-846).
      `MaxSessions ${maxSessions}`,
      'LogLevel DEBUG1',
      '',
    ].join('\n'),
  )
  const log = join(dir, 'sshd.log')
  const started = spawnSync('/usr/sbin/sshd', ['-f', config, '-E', log])
  if (started.status !== 0) throw new Error(`sshd did not start: ${started.stderr}`)
  let pid = 0
  for (let i = 0; i < 200 && !pid; i++) {
    pid = existsSync(pidFile) ? Number(readFileSync(pidFile, 'utf8').trim()) || 0 : 0
    if (!pid) await new Promise(r => setTimeout(r, 50))
  }
  if (!pid) throw new Error(`sshd wrote no pid; see ${log}`)
  const typedLog = join(dir, 'typed')
  let enrolled = ''
  return {
    name,
    dir,
    pid,
    port,
    hostKey,
    log,
    /// The Key A line the phone's enrollment has the daemon write, in the
    /// fence's shape: restricted, with the daemon as its forced command.
    enroll: keyA => (enrolled = keyA) && writeFileSync(keys, `restrict,command="cd '${dir}' && '${fakeDaemon}' typed ${name}" ${keyA}\n`),
    typed: () =>
      (existsSync(typedLog) ? readFileSync(typedLog, 'utf8') : '')
        .split('\n')
        .filter(Boolean)
        .map(hex => Buffer.from(hex, 'hex').toString())
        .join(''),
    keyA: () => enrolled,
    /// What the fake daemon was answered, one `<ask id> <option>` a line.
    answers: () => (existsSync(join(dir, 'answers')) ? readFileSync(join(dir, 'answers'), 'utf8').trim().split('\n') : []),
  }
}
let runner = fakeDaemon ? await startRunner('studio') : null
/// The second runner, started by the every-runner step.
let second = null

const failures = []
const profile = mkdtempSync(join(tmpdir(), 'fc-csp-profile-'))
const violations = []
const errors = []
/// The passkey, carried from launch to launch as an authenticator keeps it.
let credential = null
/// Every assertion a virtual authenticator made: one per touch.
let touches = 0

/// A virtual authenticator on `page`: platform, resident keys, presence
/// simulated (a touch), holding `credential` if there is one. With `prf` it
/// answers the WebAuthn PRF extension, as a passkey manager does (ov-779);
/// without it, as a browser or authenticator with none.
async function authenticator(context, page, prf = false) {
  const cdp = await context.newCDPSession(page)
  await cdp.send('WebAuthn.enable', { enableUI: false })
  const { authenticatorId } = await cdp.send('WebAuthn.addVirtualAuthenticator', {
    options: { protocol: 'ctap2', transport: 'internal', hasResidentKey: true, hasUserVerification: true, isUserVerified: true, automaticPresenceSimulation: true, ...(prf ? { hasPrf: true } : {}) },
  })
  if (credential) await cdp.send('WebAuthn.addCredential', { authenticatorId, credential })
  cdp.on('WebAuthn.credentialAsserted', () => touches++)
  return {
    /// The credentials it holds. The one whose id is `keep` (the device's
    /// passkey, base64) is carried to the next launch: getCredentials has no
    /// order, and an old one carried instead signs nothing.
    async held(keep) {
      const { credentials } = await cdp.send('WebAuthn.getCredentials', { authenticatorId })
      if (keep) credential = credentials.find(c => c.credentialId === keep) ?? null
      return credentials
    },
  }
}

/// One launch of Chromium on one profile, so IndexedDB (the device) survives
/// from launch to launch, with `video` as the fake camera when given. `prf`
/// and `dir` are the eviction walk's: an authenticator with PRF, and another
/// profile for a second browser.
async function launch(video, secret, { prf = false, dir = profile } = {}) {
  const args = ['--use-fake-ui-for-media-stream', '--use-fake-device-for-media-stream']
  if (video) args.push(`--use-file-for-fake-video-capture=${video}`)
  const context = await chromium.launchPersistentContext(dir, { executablePath: process.env.CHROME_PATH, args, permissions: ['camera'], viewport: { width: 1280, height: 800 } })
  await context.route(`${origin}/**`, serve)
  await context.route(`https://${SIBLING}/**`, serve)
  if (runner) {
    // The path names the runner by the token its enrollment carries.
    await context.routeWebSocket(url => url.host === HOST && url.pathname.startsWith('/test-tunnel/'), ws => {
      const token = new URL(ws.url()).pathname.split('/').pop()
      const target = token === 'test2' && second ? second : runner
      const tcp = createConnection({ host: '127.0.0.1', port: target.port })
      tcp.on('data', data => later(() => ws.send(data)))
      tcp.on('close', () => later(() => ws.close()))
      tcp.on('error', () => ws.close())
      ws.onMessage(data => later(() => tcp.write(typeof data === 'string' ? Buffer.from(data) : data)))
      ws.onClose(() => tcp.destroy())
    })
  }
  await context.exposeFunction('reportViolation', v => violations.push(v))
  await context.addInitScript(() => {
    document.addEventListener('securitypolicyviolation', e =>
      window.reportViolation({ directive: e.effectiveDirective, blocked: e.blockedURI, sample: e.sample, source: `${e.sourceFile}:${e.lineNumber}` }),
    )
  })
  // The code on screen is answerable only while its secret half is in the tab's
  // sessionStorage (the page never keeps it in IndexedDB), and a relaunch is a
  // new tab: carry the secret across as the person's own tab would hold it, so
  // the stored code is still the one the fake camera's reply answers.
  if (secret) await context.addInitScript(s => sessionStorage.getItem('fc-reply-secret') ?? sessionStorage.setItem('fc-reply-secret', s), secret)
  if (runner) await context.addInitScript(swapDial)
  if (runner) await context.addInitScript(refusable)
  const page = context.pages()[0] ?? (await context.newPage())
  if (throttle > 1) await (await context.newCDPSession(page)).send('Emulation.setCPUThrottlingRate', { rate: throttle })
  watch(page)
  const passkeys = await authenticator(context, page, prf)
  return { context, page, passkeys }
}

function watch(page) {
  page.on('pageerror', e => errors.push(`page error: ${e.message}`))
  page.on('console', m => {
    if (m.type() === 'error') errors.push(`console: ${m.text()}`)
  })
}

/// The page's tunnel dial, swapped for a WebSocket to /test-tunnel. Installed
/// before the page's own script runs, by trapping the global the Go module sets.
function swapDial() {
  let real
  Object.defineProperty(globalThis, 'fcTailcat', {
    configurable: true,
    get: () => real,
    set: value => {
      real = {
        mintNodeKey: () => value.mintNodeKey(),
        // The real Go derivation: the eviction walk checks its node key.
        nodeKeyFromSeed: seed => value.nodeKeyFromSeed(seed),
        dial: token =>
          new Promise((resolve, reject) => {
            const ws = new WebSocket(`wss://${location.host}/test-tunnel/${token}`)
            ws.binaryType = 'arraybuffer'
            const queue = []
            let waiting = null
            let closed = false
            ws.onmessage = e => {
              const bytes = new Uint8Array(e.data)
              if (waiting) {
                waiting(bytes)
                waiting = null
              } else queue.push(bytes)
            }
            ws.onclose = () => {
              closed = true
              if (waiting) waiting(null)
            }
            ws.onerror = () => reject(new Error('io'))
            ws.onopen = () =>
              resolve({
                read: () => (queue.length ? Promise.resolve(queue.shift()) : closed ? Promise.resolve(null) : new Promise(r => (waiting = r))),
                write: bytes => ws.send(bytes.slice()),
                close: () => ws.close(),
              })
          }),
      }
    },
  })
}

/// `navigator.credentials.get`, made to refuse once as Safari does without a
/// fresh click (NotAllowedError, no prompt) when the page sets
/// `__fcRefuseOnce`. Installed before the page's own script runs.
function refusable() {
  const real = navigator.credentials.get.bind(navigator.credentials)
  navigator.credentials.get = options => {
    if (!globalThis.__fcRefuseOnce) return real(options)
    globalThis.__fcRefuseOnce = false
    return Promise.reject(new DOMException('refused without a click', 'NotAllowedError'))
  }
}

/// Clear everything this origin keeps in `page`'s browser, as the person's
/// "clear site data" or Safari's seven-day rule does.
async function clearSite(context, page) {
  const cdp = await context.newCDPSession(page)
  await cdp.send('Storage.clearDataForOrigin', { origin, storageTypes: 'all' })
  // Never detached: detaching any DevTools session on the page takes its
  // virtual authenticators with it (measured: the next create is refused
  // NotSupportedError), and those hold the passkey.
}

/// ov-779: the node key survives storage eviction. On an authenticator with
/// PRF, a fresh browser derives its node key from the passkey and the sealed
/// copy reaches the relay; once the site's storage is cleared, Use Passkey is
/// one touch and Connect the usual one, with no new passkey and no code, and
/// the node key is the enrolled one.
///
/// All on one launch and one authenticator. Chromium's virtual authenticator
/// keeps a credential's PRF secret to itself: one carried to another
/// authenticator through WebAuthn.getCredentials and addCredential answers no
/// PRF at all (measured, ov-779), and DevTools has no field for it. So the
/// enrollment's runners are written into the device as a taken reply would
/// write them, rather than relaunching for the camera, and a second browser
/// sharing a synced passkey can't be played here: that the same PRF output
/// derives the same node key is test/vault.test.ts's, and that a synced
/// passkey gives the same output on two devices is the passkey provider's,
/// checked on real devices.
/// The eviction walk's passkey, as the relay keys its copy: Sign Out must
/// delete that copy.
let walkCredential = null

async function evictionWalk() {
  credential = null
  await session.context.close()
  session = await launch(null, null, { prf: true })
  const { page } = session
  await page.goto(`${origin}/version.json`)
  await clearSite(session.context, page)
  reports.vaults?.clear()
  await page.goto(origin)
  await makePasskey(page)
  const fresh = await device(page)
  const offer = JSON.parse(fresh.offer)
  if (!fresh.nodeDerived) failures.push('a passkey with PRF did not derive the node key')
  if (offer.node_key !== fresh.nodePub) failures.push('the offer names a node key other than the derived one')
  // The phone's reply, taken: the runner admits the offer's key, and the
  // device keeps the runner and no offer.
  runner.enroll(offer.key_a)
  await edit(page, { offer: null, offerShownAt: null, runners: [{ id: 'r1', label: 'studio', user: me, host_key: runner.hostKey, token: 'test', why: '' }] })
  const before = (await device(page)).vaulted
  await page.reload()
  await connect(page)
  let enrolled = await device(page)
  const id = enrolled.passkey.id.replaceAll('+', '-').replaceAll('/', '_').replaceAll('=', '')
  // The copy is saved after the login, not during it: wait for the one with
  // the runner in it.
  const saved = Date.now()
  while (enrolled.vaulted === before && Date.now() - saved < 10_000) enrolled = await device(page)
  if (enrolled.vaulted === before) failures.push('the sealed copy was not saved again once the device had its runner')
  const copy = reports.vaults?.get(id)
  if (!copy) failures.push('no sealed copy reached the relay')
  else {
    const clear = Buffer.from(copy.sealed, 'base64url').toString('latin1')
    if (['studio', runner.hostKey, '"runners"', enrolled.nodePriv].some(s => clear.includes(s))) failures.push('the sealed copy holds a runner, host key or node key in the clear')
  }
  console.log(`csp-check: a passkey with PRF derived the node key the offer named, and its sealed copy reached the relay (${reports.vaultSaves} saves)`)

  // A full account refuses the copy: the page says so, and the next touch
  // after there's room saves it again (ov-779 review M1, L2).
  reports.vaults.delete(id)
  reports.vaultFull = true
  await page.reload()
  await connect(page)
  await page.waitForSelector('#copy-note:not([hidden])', { timeout: 10_000 }).catch(() => failures.push('a refused copy wasn’t shown'))
  const said = await page.textContent('#copy-note-text')
  if (!/Sign out of a browser you no longer use/.test(said ?? '')) failures.push(`a full account’s refusal said “${said}”`)
  await shoot(page, 'copy-refused')
  reports.vaultFull = false
  await page.reload()
  await connect(page)
  const healing = Date.now()
  while (!reports.vaults.has(id) && Date.now() - healing < 10_000) await page.waitForTimeout(100)
  if (!reports.vaults.has(id)) failures.push('the copy a full account refused wasn’t saved again at the next touch')
  else console.log('csp-check: a full account’s refusal was shown, and the next touch saved the copy again')
  walkCredential = id

  // The site's storage is cleared: one touch brings the runners back.
  const registrations = reports.registered
  await clearSite(session.context, page)
  if (await device(page)) failures.push('clearing the site left the device behind')
  await page.reload()
  await statusSays(page, 'Reconnect with your passkey')
  await shoot(page, 'restore')
  const at = touches
  await page.click('#status-action')
  await statusSays(page, 'Connect to studio')
  const restoring = touches - at
  const restored = await device(page)
  if (restoring !== 1) failures.push(`restoring cost ${restoring} touches, not 1`)
  if (restored.offer || reports.registered !== registrations || !(await page.isHidden('#enroll'))) failures.push('restoring went through a ceremony')
  if (restored.nodePriv !== enrolled.nodePriv || restored.nodePub !== enrolled.nodePub || !restored.nodeDerived) failures.push('the restored node key is not the enrolled one')
  if (restored.passkey?.id !== enrolled.passkey.id || !restored.registered) failures.push('the restored device has another passkey, or is not registered')
  await connect(page)
  // The sibling-domain step after this asks for this passkey.
  await session.passkeys.held(enrolled.passkey.id)
  console.log(`csp-check: after the site's storage was cleared, Use Passkey cost ${restoring} touch and no ceremony, the node key was the enrolled one, and Connect opened the terminal at ${touches - at - restoring} more`)
}

/// The stored device, or an edit to it, through the page's own IndexedDB.
/// Null when there is no database: the open that finds none is aborted, so
/// reading never makes the empty one the page would then fail to set up.
const device = page =>
  page.evaluate(async () => {
    const db = await new Promise(r => {
      const open = indexedDB.open('farcooler')
      open.onupgradeneeded = e => e.target.transaction.abort()
      open.onsuccess = e => r(e.target.result)
      open.onerror = () => r(null)
    })
    if (!db) return null
    const d = await new Promise(r => (db.transaction('device').objectStore('device').get('this').onsuccess = e => r(e.target.result)))
    db.close()
    return d ? { ...d, passkey: d.passkey ? { id: btoa(String.fromCharCode(...d.passkey.id)), spki: d.passkey.spki.length } : undefined } : null
  })
const edit = (page, change) =>
  page.evaluate(async change => {
    const db = await new Promise(r => (indexedDB.open('farcooler').onsuccess = e => r(e.target.result)))
    const store = () => db.transaction('device', 'readwrite').objectStore('device')
    const d = await new Promise(r => (store().get('this').onsuccess = e => r(e.target.result)))
    for (const [k, v] of Object.entries(change)) {
      if (v === null) delete d[k]
      else d[k] = v
    }
    await new Promise(r => (store().put(d, 'this').onsuccess = r))
    db.close()
  }, change)
const replySecret = page => page.evaluate(() => sessionStorage.getItem('fc-reply-secret'))
const statusSays = (page, text, timeout = 120_000) =>
  page.waitForFunction(
    text => !document.getElementById('status')?.hidden && document.getElementById('status')?.textContent?.includes(text),
    text,
    { timeout },
  )
const enrolling = page =>
  page.waitForFunction(() => !document.getElementById('enroll')?.hidden && document.getElementById('offer-fingerprint')?.textContent?.includes('SHA256:'), null, { timeout: 120_000 })

/// Make Passkey, from the screen that asks for it.
async function makePasskey(page) {
  await statusSays(page, 'Make a passkey')
  await page.click('#status-action')
  await enrolling(page)
}

/// The type and application string of an sk-ecdsa key line: the latter is
/// its passkey's RP ID.
function skKey(line) {
  const blob = Buffer.from(line.split(' ')[1], 'base64')
  let at = 0
  const string = () => {
    const n = blob.readUInt32BE(at)
    const out = blob.subarray(at + 4, at + 4 + n)
    at += 4 + n
    return out
  }
  const fields = [string(), string(), string(), string()]
  return { type: fields[0].toString(), application: fields[3].toString() }
}

/// A device-signed reply with a runner attestation, made by the Rust core.
function signedReply(offer, unsigned = false) {
  return JSON.parse(execFileSync('cargo', ['run', '--quiet', '--manifest-path', '../../crates/web/Cargo.toml',
    '--example', 'ceremony_reply', '--', JSON.stringify(offer), runner.hostKey, me, ...(unsigned ? ['unsigned'] : [])],
    { encoding: 'utf8' }))
}

/// The terminal in front, once its runner has accepted the attach.
const LIVE = '#terminal .terminal-host:not([hidden])[data-attached="true"]'

/// Connect from the button, and wait for the terminal to take focus, which it
/// does as its attach starts; `live` waits for the runner to accept it too.
async function connect(page, live = true) {
  await statusSays(page, 'Connect to studio')
  await page.click('#status-action')
  await page.waitForSelector('#work:not([hidden]) .terminal-choice', { timeout: 120_000 })
  await page.waitForFunction(() => document.activeElement?.closest('#terminal .xterm'), null, { timeout: 30_000 })
  if (live) await page.waitForSelector(LIVE, { timeout: 60_000 })
}

/// Channels sshd has open for sessions from its log: the most at once after
/// log offset `from`, and every refusal logged. A channel is counted from the
/// line that opens it to the line that frees it.
function channels(from = 0) {
  const text = readFileSync(runner.log, 'utf8')
  const held = new Set()
  let peak = 0
  const refused = []
  let at = 0
  for (const line of text.split('\n')) {
    const opened = /channel (\d+): new session|channel (\d+): new \[server-session\]/.exec(line)
    const freed = /channel (\d+): free: server-session/.exec(line) ?? /session open failed, free channel (\d+)/.exec(line)
    if (at >= from) peak = Math.max(peak, held.size)
    if (opened) held.add(opened[1] ?? opened[2])
    else if (freed) held.delete(freed[1])
    // One line for each refusal ("session open failed" follows it).
    if (/no more sessions|administratively prohibited/.test(line)) refused.push(line.trim())
    at += line.length + 1
  }
  return { peak, open: held.size, refused, end: text.length }
}

const sorted = xs => [...xs].sort((a, b) => a - b)
const percentile = (xs, p) => sorted(xs)[Math.min(xs.length - 1, Math.ceil(xs.length * p) - 1)]

/// Ends the walk once the low-limit check has run.
class WalkDone extends Error {}

/// A runner whose sshd allows fewer sessions than the page assumes (ov-846):
/// eight panes opened in a row cost one refusal, the first, and no more. The
/// page says nothing about it.
async function lowLimit(page) {
  const buttons = page.locator('.terminal-choice')
  const mark = channels().end
  for (let i = 0; i < 8; i++) {
    await buttons.nth(i).click()
    await page.waitForFunction(i => document.querySelectorAll('.terminal-choice')[i]?.getAttribute('aria-current') === 'true', i)
    await page.waitForSelector(LIVE, { timeout: 60_000 })
  }
  await page.waitForTimeout(500)
  const opened = channels(mark)
  if (opened.refused.length < 1) failures.push(`MaxSessions ${maxSessions} refused nothing for eight panes, so the check proved nothing`)
  if (opened.refused.length > 1) failures.push(`${opened.refused.length} refusals for eight panes, and only the first is expected: ${opened.refused[1]}`)
  // A refused session is opened and freed within a few log lines, so the
  // peak may count it once more.
  if (opened.peak > maxSessions + opened.refused.length) failures.push(`eight panes held ${opened.peak} channels at once, and sshd allows ${maxSessions}`)
  const note = await page.evaluate(() => document.querySelector('#pane-note')?.textContent ?? '')
  if (note.trim()) failures.push(`the page said "${note}" about a refusal it recovered from`)
  console.log(`csp-check: eight panes on MaxSessions ${maxSessions}: ${opened.refused.length} refusal, at most ${opened.peak} channels, the page silent`)
}

/// How many `seq 1 2000000` are started at once in the loaded pane.
const LOADS = 4

/// Many panes on one connection (ov-774): twelve opened one after another
/// never hold more than ten of sshd's channels, a pane let go of attaches
/// again when it is returned to, Japanese IME text commits once and whole,
/// and a keystroke's echo in one pane stays quick while `seq 1 2000000` fills
/// another.
async function panes(page) {
  const buttons = page.locator('.terminal-choice')
  const total = await buttons.count()
  if (total !== 12) failures.push(`the runner lists ${total} terminals, not 12`)
  await page.evaluate(() => {
    const fc = (globalThis.__fc = { downs: [], echoes: [], load: [] })
    document.addEventListener('keydown', e => fc.downs.push(e.timeStamp), true)
    // Told by the pane after each chunk is parsed: in front, or hidden.
    globalThis.__fcMeasure = inFront => (inFront ? fc.echoes : fc.load).push(performance.now())
  })
  const mark = channels().end

  for (let i = 0; i < 12; i++) {
    await buttons.nth(i).click()
    await page.waitForFunction(i => document.querySelectorAll('.terminal-choice')[i]?.getAttribute('aria-current') === 'true', i)
    await page.waitForSelector(LIVE, { timeout: 60_000 })
  }
  await page.waitForTimeout(500)
  const opened = channels(mark)
  // A floor, so that a log wording the patterns don't match (peak 0) cannot pass.
  if (opened.peak < 7) failures.push(`sshd's log showed ${opened.peak} channels at most for twelve panes, and at least 7 are expected (a control channel and 6 panes), so the log wording may have changed`)
  if (opened.peak > 10) failures.push(`twelve panes held ${opened.peak} channels at once, and sshd allows 10`)
  if (opened.refused.length) failures.push(`sshd refused a channel: ${opened.refused[0]}`)
  const hosts = await page.evaluate(() => document.querySelectorAll('#terminal .terminal-host').length)
  if (hosts !== 12) failures.push(`${hosts} terminals are on the page, not 12`)
  console.log(`csp-check: twelve panes opened one after another held at most ${opened.peak} of sshd's 10 channels, with ${opened.refused.length} refusals`)

  // The first was let go of for room; going back attaches it again.
  await buttons.nth(0).click()
  await page.waitForSelector(LIVE, { timeout: 60_000 })
  await page.waitForTimeout(500)
  const echoes = await page.evaluate(() => __fc.echoes.length)
  await page.keyboard.type('x')
  const back = await page
    .waitForFunction(n => __fc.echoes.length > n, echoes, { timeout: 10_000 })
    .then(
      () => true,
      () => false,
    )
  if (!back) failures.push('a pane that was let go of did not echo after it was returned to')
  else console.log('csp-check: the first pane, let go of for room, attached again and echoed')

  // IME: composition sends nothing, and the commit sends the text once.
  const cdp = await session.context.newCDPSession(page)
  const before = runner.typed()
  for (const text of ['に', 'にほ', '日本']) await cdp.send('Input.imeSetComposition', { text, selectionStart: text.length, selectionEnd: text.length })
  await page.waitForTimeout(300)
  const composing = runner.typed().slice(before.length)
  await cdp.send('Input.insertText', { text: '日本語' })
  const deadline = Date.now() + 10_000
  while (runner.typed().length === before.length + composing.length && Date.now() < deadline) await new Promise(r => setTimeout(r, 100))
  await page.waitForTimeout(300)
  const committed = runner.typed().slice(before.length)
  if (composing !== '') failures.push(`composition sent ${JSON.stringify(composing)} before it was committed`)
  if (committed !== '日本語') failures.push(`IME committed ${JSON.stringify(committed)}, not "日本語"`)
  else console.log('csp-check: Japanese IME composition sent nothing, and its commit sent 日本語 once (Chromium; Safari is checked by hand)')

  // A load in one pane, typing in another.
  await buttons.nth(1).click()
  await page.waitForSelector(LIVE, { timeout: 60_000 })
  await page.waitForTimeout(500)
  // The command takes about three seconds to print, and a keystroke's echo
  // takes a round trip, so one run would leave too few samples. Four at once
  // make a load four times heavier than the one asked for, for about twelve.
  for (let i = 0; i < LOADS; i++) {
    await page.keyboard.type('seq 1 2000000')
    await page.keyboard.press('Enter')
  }
  await buttons.nth(2).click()
  await page.waitForSelector(LIVE, { timeout: 60_000 })
  await page.waitForTimeout(500)
  const started = await page
    .waitForFunction(() => __fc.load.length > 20, null, { timeout: 60_000 })
    .then(
      () => true,
      () => false,
    )
  if (!started) {
    failures.push('the load in the hidden pane never started')
    return
  }
  const samples = []
  let loaded = 0
  for (let i = 0; i < 100; i++) {
    const n = await page.evaluate(() => __fc.echoes.length)
    await page.keyboard.press('a')
    const ok = await page.waitForFunction(n => __fc.echoes.length > n, n, { timeout: 10_000 }).then(
      () => true,
      () => false,
    )
    if (!ok) {
      failures.push('a keystroke under load was never echoed')
      break
    }
    const got = await page.evaluate(n => {
      const down = __fc.downs[__fc.downs.length - 1]
      const load = __fc.load[__fc.load.length - 1] ?? 0
      return { ms: __fc.echoes[n] - down, underLoad: performance.now() - load < 250, chunks: __fc.load.length }
    }, n)
    // The tunnel's own delay (--tunnel-delay, both ways) is the network's.
    samples.push({ ms: got.ms - 2 * delay, underLoad: got.underLoad })
    await page.waitForTimeout(40 + Math.floor(Math.random() * 40))
  }
  const under = samples.filter(s => s.underLoad).map(s => s.ms)
  const load = await page.evaluate(() => ({ chunks: __fc.load.length, first: __fc.load[0], last: __fc.load.at(-1), now: performance.now() }))
  const chunks = load.chunks
  console.log(`csp-check: the load parsed ${chunks} chunks from ${load.first.toFixed(0)} to ${load.last.toFixed(0)} ms, ${(load.now - load.last).toFixed(0)} ms before the last sample`)
  if (under.length < 20) failures.push(`only ${under.length} of ${samples.length} keystrokes were typed while the load was running`)
  else {
    const p95 = percentile(under, 0.95)
    console.log(
      `csp-check: echo while ${LOADS} x seq 1 2000000 ran in another pane, keydown to xterm's parse callback, not paint, with the configured 2 x tunnel delay (${2 * delay} ms) taken off, not a measured one, cpu throttle ${throttle}x: ` +
        `n=${under.length} p50=${percentile(under, 0.5).toFixed(1)} ms p95=${p95.toFixed(1)} ms max=${Math.max(...under).toFixed(1)} ms, ${chunks} chunks of load parsed`,
    )
    if (p95 > 100) failures.push(`keystroke echo under load was ${p95.toFixed(1)} ms at p95, over 100 ms`)
  }
}

/// Needs You across every runner this browser is enrolled on (ov-861): two
/// real sshds, each in front of a fake daemon with one ask waiting.
/// On reload only the foreground runner needs a touch. The other offers
/// Connect with an explanation. One more touch shows both asks;
/// an answer reaches only the runner the row is on; and with the second runner
/// offline, the first one's ask is still listed after reload.
async function everyRunner(page) {
  second = await startRunner('build-box')
  second.enroll(runner.keyA())
  const studio = { id: 'r1', label: 'studio', user: me, host_key: runner.hostKey, token: 'test', why: '' }
  await edit(page, { runners: [studio] })
  // The previous walk may have used a different granted runner ID.
  await page.goto(origin)
  await statusSays(page, 'Connect to studio')
  await page.click('#status-action')
  await page.waitForFunction(() => Boolean(history.state?.workspaceScope), null, { timeout: 120_000 })
  await edit(page, {
    runners: [
      studio,
      { id: 'r2', label: 'build-box', user: me, host_key: second.hostKey, token: 'test2', why: '' },
    ],
  })
  const before = touches
  await page.reload()
  await statusSays(page, 'Connect to studio')
  await page.click('#status-action')
  await page.waitForSelector('#work:not([hidden])', { timeout: 120_000 })
  await page.click('#show-needs')
  const rows = () => page.$$eval('#needs .need', rows => rows.map(r => ({ text: r.textContent, runner: r.querySelector('.need-runner')?.textContent ?? '' })))
  const deferred = page.locator('.need-runner-note').filter({ hasText: 'build-box' })
  const connectOther = deferred.getByRole('button', { name: 'Connect', exact: true })
  const secondCard = page.locator('#needs .need').nth(1)
  await connectOther.waitFor({ timeout: 60_000 })
  const deferredText = await deferred.evaluate(note => note.firstChild?.textContent)
  if (deferredText !== 'build-box is not connected yet. Select Connect, then touch your passkey when your browser asks.') {
    failures.push(`the deferred runner notice says ${JSON.stringify(deferredText)}`)
  }
  if (touches - before !== 1) failures.push(`returning with two runners cost ${touches - before} touches before Connect, expected 1`)
  await connectOther.click()
  await secondCard.waitFor({ timeout: 60_000 })
  const both = await rows()
  const names = both.map(r => r.runner).sort()
  if (JSON.stringify(names) !== JSON.stringify(['build-box', 'studio'])) failures.push(`the rows name ${JSON.stringify(names)}, not both runners`)
  for (const r of both) if (!r.text.includes(`Run the tests on ${r.runner}?`)) failures.push(`the ${r.runner} row says ${JSON.stringify(r.text)}`)
  if (touches - before !== 2) failures.push(`connecting to two runners cost ${touches - before} touches, not 2`)
  if ((await page.textContent('#show-needs')) !== 'Needs You (2)') failures.push(`the Needs You button says ${await page.textContent('#show-needs')}`)
  await shoot(page, 'every-runner')
  console.log('csp-check: Needs You listed both runners’ asks, each row naming its runner, at one touch each')

  // An answer reaches the runner the row is on, and only that one.
  const card = page.locator('#needs .need', { hasText: 'Run the tests on build-box?' })
  await card.getByRole('button', { name: 'Allow' }).click()
  await page.waitForFunction(() => document.querySelectorAll('#needs .need').length === 1, null, { timeout: 30_000 })
  const left = (await rows())[0]
  if (left.runner !== 'studio') failures.push(`after answering build-box's ask the list holds ${JSON.stringify(left)}`)
  if (JSON.stringify(second.answers()) !== JSON.stringify(['ask-build-box allow'])) failures.push(`build-box was answered ${JSON.stringify(second.answers())}`)
  if (runner.answers().length) failures.push(`studio was answered ${JSON.stringify(runner.answers())} for build-box's ask`)
  else console.log('csp-check: an answer reached build-box and not studio')

  // With the second runner offline, the first's ask stays visible on reload.
  spawnSync('/bin/kill', [String(second.pid)])
  await page.reload()
  await statusSays(page, 'Connect to studio')
  await page.click('#status-action')
  await page.waitForSelector('#work:not([hidden])', { timeout: 120_000 })
  await page.click('#show-needs')
  await page.waitForFunction(() => document.querySelector('.need-runner-note')?.textContent?.includes('build-box'), null, { timeout: 60_000 })
  // Studio's own list is read over its own connection, and may land after the note.
  await page.waitForFunction(() => document.querySelectorAll('#needs .need').length === 1, null, { timeout: 60_000 })
  const note = await page.textContent('.need-runner-note')
  const kept = await rows()
  if (kept.length !== 1 || kept[0].runner !== 'studio') failures.push(`with build-box unreachable the list holds ${JSON.stringify(kept)}`)
  if (!(await page.locator('.need-runner-note button', { hasText: 'Connect' }).count())) failures.push(`the note for the unreachable runner has no Connect button: ${note}`)
  await shoot(page, 'runner-unreachable')
  console.log(`csp-check: with build-box offline, studio's ask stayed listed and the page said "${note.trim()}"`)
}

/// One runner, three workspaces (ov-871, ov-1445). The fake daemon's Main workspace has
/// twelve shells, one ask and the plan's Frosted window plane; its Billing has
/// a shell, an agent, an ask, a decision on a card, and a plan with a ticket and
/// its builder on that card. Sandbox has no terminals. The page uses the real sshd.
async function workspaceWalk(page) {
  writeFileSync(join(runner.dir, 'billing'), '')
  await edit(page, { runners: [{ id: 'r1', label: 'studio', user: me, host_key: runner.hostKey, token: 'test', why: '' }] })
  await page.evaluate(() => localStorage.clear())
  await page.goto(origin)
  await statusSays(page, 'Connect to studio')
  await page.click('#status-action')
  await page.waitForSelector('#work:not([hidden]) .terminal-choice', { timeout: 120_000 })
  const titles = () => page.$$eval('.terminal-choice', b => b.map(x => x.textContent))
  const picker = () => page.$$eval('#space-picker option', o => o.map(x => x.textContent))
  const rows = () => page.$$eval('#needs .need-question', r => r.map(x => x.textContent))
  const same = (a, b) => JSON.stringify(a) === JSON.stringify(b)

  // Main is showing: its twelve shells, and nothing of Billing's.
  await page.waitForFunction(() => document.querySelector('#space-picker')?.textContent?.includes('Billing · Studio (2)'), null, { timeout: 60_000 })
  if (!same(await picker(), ['Main · Studio (1)', 'Billing · Studio (2)', 'Sandbox · Studio'])) failures.push(`the picker says ${JSON.stringify(await picker())}`)
  const main = await titles()
  if (main.length !== 12 || main.some(t => /billing/.test(t))) failures.push(`Main lists ${JSON.stringify(main)}`)

  await page.selectOption('#space-picker', { label: 'Sandbox · Studio' })
  await page.waitForSelector('#pane-note:not([hidden])')
  const notice = await page.textContent('#pane-note')
  if (notice !== 'Sandbox has no terminals yet. Use Start Work to open one.' || (await titles()).length) {
    failures.push(`the empty workspace lists ${JSON.stringify(await titles())} and says ${JSON.stringify(notice)}`)
  }
  await page.click('#show-commands')
  await page.locator('#workspace-commands').getByRole('button', { name: 'Start Work', exact: true }).click()
  await page.locator('#start-work[open]').getByRole('button', { name: 'New Worktree' }).waitFor()
  await page.click('#work-done')

  await page.selectOption('#space-picker', { label: 'Main · Studio (1)' })
  await page.click('#show-needs')
  await page.waitForFunction(() => document.querySelectorAll('#needs .need').length === 1, null, { timeout: 30_000 })
  if (!same(await rows(), ['Run the tests on studio?'])) failures.push(`Main's Needs You lists ${JSON.stringify(await rows())}`)
  if ((await page.textContent('#show-needs')) !== 'Needs You (1)') failures.push(`Main's Needs You button says ${await page.textContent('#show-needs')}`)
  if ((await page.textContent('#bar-count')) !== '3 Waiting') failures.push(`the bar says ${await page.textContent('#bar-count')}, not everything waiting`)
  await page.click('#show-plan')
  // Main's one initiative has nothing moving, so the column folds it into its quiet line.
  await page.waitForFunction(() => document.getElementById('plan')?.textContent?.includes('Nothing moving: Visual language'), null, { timeout: 30_000 })
  if ((await page.textContent('#plan')).includes('Invoices')) failures.push("Main's plan shows Billing's theme")
  console.log('csp-check: Main showed its twelve terminals, its one ask and its plan. Sandbox showed the empty-workspace notice.')
  await shoot(page, 'workspace-main')

  // Billing: its own terminals, Needs You and plan.
  await page.selectOption('#space-picker', { label: 'Billing · Studio (2)' })
  await page.waitForFunction(() => document.querySelector('#plan')?.textContent?.includes('PDF export'), null, { timeout: 30_000 })
  const plan = await page.textContent('#plan')
  if (plan.includes('Visual language') || !plan.includes('Invoice export') || (await page.$('.plan-board'))) failures.push(`Billing's plan reads ${JSON.stringify(plan.slice(0, 200))}`)
  await planColumnWalk(page)
  await page.click('#close-split')
  await page.click('#show-needs')
  await page.waitForFunction(() => document.querySelectorAll('#needs .need').length === 2, null, { timeout: 30_000 })
  if (!same(await rows(), ['Run the invoice tests?', 'Which invoice format first?'])) failures.push(`Billing's Needs You lists ${JSON.stringify(await rows())}`)
  if ((await page.textContent('#show-needs')) !== 'Needs You (2)') failures.push(`Billing's Needs You button says ${await page.textContent('#show-needs')}`)
  await shoot(page, 'workspace-needs')
  await page.click('#show-terminals')
  await page.waitForSelector(LIVE, { timeout: 60_000 })
  if (!same(await titles(), ['billing shell', 'billing agent'])) failures.push(`Billing lists ${JSON.stringify(await titles())}`)
  console.log('csp-check: picking Billing swapped the terminals, Needs You (2) and the plan, and opened its first terminal')
  await shoot(page, 'workspace-billing')

  // The workspace pick is retained when returning to the app without a deep link.
  await page.goto(origin)
  await statusSays(page, 'Connect to studio')
  await page.click('#status-action')
  await page.waitForSelector('#work:not([hidden]) .terminal-choice', { timeout: 120_000 })
  if ((await page.inputValue('#space-picker')) === '' || !same(await titles(), ['billing shell', 'billing agent'])) failures.push('the pick was not remembered')
  else console.log('csp-check: the pick survived a reload')
}

/// The plan column (ov-875), on Billing's plan: its initiative with progress
/// and Next, its builder (with its ticket count) and the ticket once, in its pipeline stage, the key in monospace,
/// a Needs You row quoted into the orchestrator's box without sending, a ticket
/// opening its card, and an agent opening its terminal.
async function planColumnWalk(page) {
  const text = async sel => (await page.locator(sel).allTextContents()).map(t => t.replace(/\s+/g, ' ').trim())
  const init = await text('#plan .plan-initiative')
  if (init.length !== 1 || !init[0].includes('Invoices') || !init[0].includes('1 of 2') || !init[0].includes('Next: PDF export')) failures.push(`the initiative reads ${JSON.stringify(init)}`)
  if ((await page.getAttribute('#plan .plan-init', 'title')) !== null && (await page.getAttribute('#plan .plan-init', 'title')) === '') failures.push('the initiative has an empty title')
  const agent = (await text('#plan .plan-agent'))[0] ?? ''
  if (!agent.includes('Builder') || !agent.includes('Writing the PDF') || !agent.includes('5 min') || !agent.includes('1 pipeline ticket assigned') || agent.includes('bil-7')) failures.push(`the builder reads ${JSON.stringify(agent)}`)
  const tickets = await page.locator('#plan .plan-ticket').count()
  if (tickets !== 1) failures.push(`bil-7 is listed ${tickets} times`)
  // ov-895: the roster names the agent and counts its tickets; the ticket
  // itself is listed once, in its own pipeline stage.
  const staged = await text('#plan .plan-stage[data-stage="building"] .plan-ticket')
  if (staged.length !== 1 || !staged[0].includes('bil-7')) failures.push(`the Building stage lists ${JSON.stringify(staged)}`)
  const font = await page.locator('#plan .plan-key').first().evaluate(el => getComputedStyle(el).fontFamily)
  if (!/mono|Menlo/i.test(font)) failures.push(`the ticket's key is set in ${font}`)
  if (await page.locator('#plan hr, #plan [class*="stop" i], #plan [class*="assign" i]').count()) failures.push('the plan has a rule, Stop or Assign')
  const flat = await page.evaluate(() => [...document.querySelectorAll('#plan *')].some(el => getComputedStyle(el).borderBottomWidth !== '0px' && getComputedStyle(el).borderBottomStyle !== 'none' && el.tagName !== 'INPUT' && !el.closest('button')))
  if (flat) failures.push('something in the plan draws a rule')
  console.log('csp-check: the plan column showed the initiative with its progress and Next, the builder with its one ticket, the key in monospace, and no rules')

  // A Needs You row quotes itself into the orchestrator's box and sends nothing.
  const rowsText = await text('#plan .plan-need')
  if (rowsText.length !== 2) failures.push(`the plan's Needs You lists ${JSON.stringify(rowsText)}`)
  const composes = () => (existsSync(join(runner.dir, 'composed')) ? readFileSync(join(runner.dir, 'composed'), 'utf8') : '')
  const before = composes()
  await page.locator('#plan .plan-need').first().click()
  await page.waitForFunction(() => !document.getElementById('plan').hidden && document.querySelector('#side-stage .chat-host:not([hidden]) .chat-input')?.value.includes('About'), null, { timeout: 30_000 })
  const boxText = await page.inputValue('.chat-host:not([hidden]) .chat-input')
  if (boxText !== 'About \u201CRun the invoice tests?\u201D: ') failures.push(`the quote reads ${JSON.stringify(boxText)}`)
  if (composes() !== before) failures.push('quoting sent a message')
  console.log('csp-check: a Needs You row left its quote in the orchestrator’s box, unsent')

  // A ticket opens its card; the sheet closes back to the plan.
  await page.click('#show-plan')
  await page.locator('#plan .plan-ticket').first().click()
  await page.waitForSelector('#detail[open] .detail-note', { timeout: 30_000 })
  if (!(await page.textContent('#detail')).includes('bil-7')) failures.push("the ticket's card sheet is not bil-7's")
  await page.click('#detail-close')
  await page.waitForFunction(() => !document.getElementById('detail').open)

  // An agent opens its terminal.
  await page.click('#show-plan')
  await page.locator('#plan .plan-agent-head').first().click()
  await page.waitForFunction(() => document.querySelector('.terminal-choice[aria-current="true"]')?.textContent === 'billing shell', null, { timeout: 30_000 })
  console.log('csp-check: a ticket opened its card and the builder opened its terminal')
  await page.click('#show-plan')
  await shoot(page, 'workspace-plan')
}

/// ov-1445: real WASM projection, two pane streams and one authenticated session.
async function workstationWalk(page) {
  const assert = (ok, message) => { if (!ok) throw new Error(message) }
  const palette = page.locator('#workspace-commands')
  const main = page.locator('#workspace-main')
  const side = page.locator('#workspace-side')
  const draft = side.getByRole('textbox', { name: 'Message', exact: true })
  const focus = async slot => page.waitForFunction(slot => document.getElementById(`workspace-${slot}`)?.dataset.focused === 'true', slot)
  const commands = () => page.click('#show-commands')
  const action = label => palette.getByRole('button', { name: new RegExp(`^${label}`) })

  await page.click('#show-terminals')
  await commands()
  await action('Split Pane').click()
  await focus('side')
  await page.locator('.terminal-choice', { hasText: 'billing agent' }).click()
  await side.getByRole('button', { name: 'Raw Terminal' }).click()
  await side.locator('.xterm').waitFor()
  await page.waitForSelector('#side-stage [data-attached="true"]:not([hidden])')
  assert(await main.locator('.xterm').isVisible(), 'the main terminal disappeared after splitting')

  await main.locator('.xterm').click()
  await focus('main')
  await page.keyboard.press('Alt+Shift+F')
  assert(await main.getAttribute('data-focused') === 'true', 'the shortcut stole focus from xterm')
  await main.focus()
  await page.keyboard.press('Alt+Shift+F')
  await focus('side')

  await commands()
  await action('Keyboard Shortcuts').click()
  const shortcut = palette.getByRole('textbox', { name: 'Focus Other Pane', exact: true })
  await shortcut.fill('J')
  await shortcut.press('Tab')
  await palette.getByRole('button', { name: 'Close', exact: true }).click()
  await side.focus()
  await page.keyboard.press('Alt+Shift+J')
  await focus('main')

  await page.click('#show-plan')
  await page.locator('#plan .plan-ticket').waitFor()
  await page.click('#show-orchestrator')
  await side.getByRole('button', { name: 'Chat', exact: true }).click()
  await draft.fill('Keep the invoice export draft here.')
  await side.locator('.bubble').first().waitFor()
  await shoot(page, 'workstation-plan')
  await page.click('#show-board')
  const board = page.locator('#board')
  await board.getByRole('link', { name: /Invoice export/ }).click()
  await board.getByRole('heading', { name: 'Invoice export', exact: true }).waitFor()
  assert(await draft.inputValue() === 'Keep the invoice export draft here.', 'the board ticket hid the orchestrator draft')
  await page.goBack()
  await board.getByRole('heading', { name: 'Board', exact: true }).waitFor()
  await page.goBack()
  await page.locator('#plan .plan-ticket').waitFor()
  await page.setViewportSize({ width: 390, height: 560 })
  const panes = await page.locator('.workspace-leaf').evaluateAll(leaves => leaves.map(el => el.getBoundingClientRect().toJSON()))
  assert(panes[0].bottom <= panes[1].top + 1 && panes[1].right <= 390, 'phone panes did not stack within the viewport')

  const plan = page.locator('#plan')
  const position = await plan.evaluate(el => {
    el.scrollTop = 85
    return el.scrollTop
  })
  assert(position > 0, 'the narrow Plan fixture has no scroll position to restore')
  await page.waitForFunction(position => {
    const prefix = 'fc-workspace:'
    return Object.keys(localStorage).some(key => key.startsWith(prefix) && key.includes(':scroll:') &&
      key.includes('"plan"') && localStorage.getItem(key) === String(position))
  }, position)
  // The click scrolls the ticket into view itself. Use its keyboard action
  // with preventScroll to preserve the position being tested.
  await page.locator('#plan .plan-ticket').first().evaluate(button => button.focus({ preventScroll: true }))
  await page.keyboard.press('Enter')
  await page.locator('#detail[open] .detail-note').waitFor()
  assert(await draft.inputValue() === 'Keep the invoice export draft here.', 'ticket hid the orchestrator draft')
  await page.click('#show-workflow')
  await page.locator('#workflow-runs[open]').getByText('No workflow runs.').waitFor()
  assert(await draft.isVisible(), 'workflow hid the orchestrator')
  await page.goBack()
  await page.locator('#detail[open]').waitFor()
  await page.goBack()
  await page.locator('#plan .plan-ticket').waitFor()
  await page.waitForFunction(position => document.getElementById('plan').scrollTop === position, position)
  await page.goForward()
  await page.locator('#detail[open]').waitFor()
  await page.goForward()
  await page.locator('#workflow-runs[open]').getByText('No workflow runs.').waitFor()

  await main.focus()
  const before = touches
  await page.reload()
  await statusSays(page, 'Connect to studio')
  await page.click('#status-action')
  await page.locator('#workflow-runs[open]').getByText('No workflow runs.').waitFor()
  await draft.waitFor()
  assert(await draft.inputValue() === 'Keep the invoice export draft here.', 'reload lost the orchestrator draft')
  assert(await main.getAttribute('data-focused') === 'true', 'reload lost the focused pane')
  assert(touches - before === 1, 'restoring two panes needed more than one passkey touch')
  await main.focus()
  await page.keyboard.press('Alt+Shift+J')
  await focus('side')
  await page.click('#close-split')
  await page.click('#show-terminals')
  const picker = page.locator('#space-picker')
  const billing = await picker.inputValue()
  await picker.selectOption({ index: 0 })
  await page.goBack()
  assert(await picker.inputValue() === billing, 'Back did not restore the workspace')
  await page.goForward()
  assert(await picker.inputValue() !== billing, 'Forward did not restore the switched workspace')
  await page.goBack()
  await page.setViewportSize({ width: 1280, height: 800 })
  console.log('csp-check: split, pane focus, palette and remapping, history and draft restoration passed')
}

/// A card's detail (ov-871), after `workspaceWalk`, on Billing.
async function detailWalk(page) {
  // From a Needs You item: its words, its answer, a note.
  await page.click('#show-needs')
  await page.waitForFunction(() => document.querySelectorAll('#needs .need').length === 2, null, { timeout: 30_000 })
  const decision = page.locator('#needs .need', { hasText: 'Which invoice format first?' })
  await decision.getByRole('button', { name: 'Details' }).click()
  await page.waitForSelector('#detail[open] .detail-note', { timeout: 30_000 })
  const sheet = await page.textContent('#detail')
  for (const word of ['bil-7', 'Invoice export', 'In Progress', 'Customers download an invoice as a file.', 'A PDF opens in Preview', 'PDF or CSV first?', 'Waiting on You'])
    if (!sheet.includes(word)) failures.push(`the card's sheet lacks ${JSON.stringify(word)}`)
  if (!(await page.locator('#detail .detail-answers').getByRole('button', { name: 'PDF' }).count())) failures.push("the card's sheet has no PDF button")
  await shoot(page, 'workspace-detail')
  await page.fill('#detail-note', 'Looks right')
  await page.click('#detail-add')
  await waitForFile(join(runner.dir, 'notes'), '6 Looks right')
  await page.keyboard.press('Escape')
  await page.waitForFunction(() => !document.getElementById('detail').open)
  const focused = await page.evaluate(() => document.activeElement?.textContent)
  if (focused !== 'Details') failures.push(`closing the sheet left focus on ${JSON.stringify(focused)}, not the button that opened it`)
  // Answered from the sheet: the card leaves Needs You, and focus goes to the tab.
  await decision.getByRole('button', { name: 'Details' }).click()
  await page.waitForSelector('#detail[open] .detail-answers', { timeout: 30_000 })
  await page.locator('#detail .detail-answers').getByRole('button', { name: 'PDF' }).click()
  await waitForFile(join(runner.dir, 'notes'), '4 PDF')
  await page.waitForFunction(() => !document.querySelector('#detail .detail-answers'), null, { timeout: 30_000 })
  await page.keyboard.press('Escape')
  await page.waitForFunction(() => !document.getElementById('detail').open)
  // History navigation restores focus after the page changes.
  const landed = await page.waitForFunction(() => document.activeElement?.id === 'show-needs', null, { timeout: 5000 }).then(
    () => true,
    () => false,
  )
  if (!landed) failures.push(`with its card gone, closing the sheet left focus on ${JSON.stringify(await page.evaluate(() => document.activeElement?.id))}, not the Needs You tab`)
  await page.waitForFunction(() => document.querySelectorAll('#needs .need').length === 1, null, { timeout: 30_000 })
  console.log('csp-check: Details opened the card, a note and the PDF answer reached the runner, and closing it put focus back')

  // And from a ticket on the plan.
  await page.click('#show-plan')
  await page.waitForSelector('#plan .plan-ticket', { timeout: 30_000 })
  await page.locator('#plan .plan-ticket', { hasText: 'bil-7' }).click()
  await page.waitForSelector('#detail[open] .detail-note', { timeout: 30_000 })
  if (!(await page.textContent('#detail-title')).includes('Invoice export')) failures.push('a plan ticket did not open its card')
  else console.log('csp-check: a plan ticket opened its card’s detail')
  await page.keyboard.press('Escape')
  await page.waitForFunction(() => !document.getElementById('detail').open)
  await page.click('#show-terminals')
}

/// An agent's conversation and its images (ov-871), after `workspaceWalk`.
async function imageWalk(page) {
  await page.locator('.terminal-choice', { hasText: 'billing agent' }).click()
  await page.waitForSelector('.chat-host:not([hidden]) .bubble', { timeout: 30_000 })
  const png = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==', 'base64')
  const chooser = page.waitForEvent('filechooser')
  await page.click('.chat-host:not([hidden]) .chat-attach')
  await (await chooser).setFiles([{ name: 'pixel.png', mimeType: 'image/png', buffer: png }])
  await page.evaluate(async b64 => {
    const bytes = Uint8Array.from(atob(b64), c => c.charCodeAt(0))
    const pasted = new DataTransfer()
    pasted.items.add(new File([bytes], 'pasted.png', { type: 'image/png' }))
    document.querySelector('.chat-host:not([hidden]) .chat-input').dispatchEvent(new ClipboardEvent('paste', { clipboardData: pasted, bubbles: true, cancelable: true }))
    const dropped = new DataTransfer()
    dropped.items.add(new File([bytes], 'dropped.png', { type: 'image/png' }))
    document.querySelector('.chat-host:not([hidden])').dispatchEvent(new DragEvent('drop', { dataTransfer: dropped, bubbles: true, cancelable: true }))
  }, png.toString('base64'))
  await page.waitForFunction(() => document.querySelectorAll('.chat-host:not([hidden]) .chat-thumb').length === 3)
  await shoot(page, 'workspace-images')
  await page.fill('.chat-host:not([hidden]) .chat-input', 'What is in these?')
  await page.press('.chat-host:not([hidden]) .chat-input', 'Enter')
  await waitForFile(join(runner.dir, 'composed'), 'What is in these?')
  await waitForFile(join(runner.dir, 'images'), `image/png ${png.length}`)
  // The runner has the message once its file says so; the page clears the thumbnails when the answer arrives.
  await page.waitForFunction(() => !document.querySelector('.chat-host:not([hidden]) .chat-thumb'), null, { timeout: 30_000 })
  const images = readFileSync(join(runner.dir, 'images'), 'utf8').trim().split('\n')
  if (images.length !== 3 || images.some(i => i !== `image/png ${png.length}`)) failures.push(`the runner got images ${JSON.stringify(images)}`)
  else console.log('csp-check: an attached, a pasted and a dropped image reached the agent with the words as one compose')
  // Past the runner's limit (no compose_upload here: 900 KiB together): refused, nothing sent.
  const big = page.waitForEvent('filechooser')
  await page.click('.chat-host:not([hidden]) .chat-attach')
  await (await big).setFiles([{ name: 'big.png', mimeType: 'image/png', buffer: Buffer.alloc(1024 * 1024, 1) }])
  await page.waitForFunction(() => document.querySelector('.chat-host:not([hidden]) .chat-problem')?.textContent?.includes('too large'), null, { timeout: 10_000 })
  if (await page.locator('.chat-host:not([hidden]) .chat-thumb').count()) failures.push('an image past the limit waited to be sent')
  else console.log('csp-check: an image past the runner’s limit was refused with a sentence')
}

/// An agent's view switch (ov-871), after `imageWalk`, on the same agent.
async function switchWalk(page) {
  if ((await page.getAttribute('#view-conversation', 'aria-pressed')) !== 'true' || (await page.isHidden('#pane-bar'))) failures.push('an agent did not open as its conversation with the view switch')
  await page.click('#view-terminal')
  await page.waitForSelector(LIVE, { timeout: 60_000 })
  if (await page.locator('.chat-host:not([hidden])').count()) failures.push('the conversation stayed in front of the terminal')
  if ((await page.getAttribute('#view-terminal', 'aria-pressed')) !== 'true') failures.push('the switch does not say Terminal')
  const before = runner.typed()
  await page.keyboard.type('whoami')
  const deadline = Date.now() + 30_000
  while (!runner.typed().endsWith('whoami') && Date.now() < deadline) await new Promise(r => setTimeout(r, 100))
  if (!runner.typed().endsWith('whoami')) failures.push(`the agent's terminal got ${JSON.stringify(runner.typed().slice(before.length))}`)
  await shoot(page, 'workspace-agent-terminal')
  await page.click('#view-conversation')
  await page.waitForSelector('.chat-host:not([hidden]) .bubble', { timeout: 30_000 })
  if (await page.locator('#terminal .terminal-host:not(.chat-host):not([hidden])[data-attached="true"]').count()) failures.push('the terminal kept its channel after switching back to the conversation')
  console.log('csp-check: an agent switched from its conversation to its terminal, took keystrokes there, and came back')
  await shoot(page, 'workspace-agent')
}

/// Wait for a file the fake daemon writes to hold `text` on a line.
async function waitForFile(path, text) {
  const deadline = Date.now() + 30_000
  while (Date.now() < deadline) {
    if (existsSync(path) && readFileSync(path, 'utf8').split('\n').includes(text)) return
    await new Promise(r => setTimeout(r, 100))
  }
  failures.push(`${path.split('/').pop()} never held ${JSON.stringify(text)}`)
}

let session = await launch()
try {
  // A step that times out stops the walk, and the violations it caused are
  // still reported: they are usually why it timed out.
  try {
    let { page } = session
    // Boot, a button for the passkey (Safari prompts only from a click), and
    // the code.
    await page.goto(origin)
    await statusSays(page, 'Make a passkey')
    await shoot(page, 'passkey')
    if ((await device(page))?.passkey) failures.push('a passkey was kept before anybody asked for one')
    await makePasskey(page)
    console.log(`csp-check: a passkey was made and the enrollment screen is up, ${await page.textContent('#offer-fingerprint')}`)
    await shoot(page, 'enroll')
    let kept = await device(page)
    const offer = JSON.parse(kept.offer)
    const made = await session.passkeys.held(kept.passkey?.id)
    const key = skKey(offer.key_a)
    if (key.type !== 'sk-ecdsa-sha2-nistp256@openssh.com' || key.application !== HOST) failures.push(`the offer's key is ${key.type} for ${key.application}`)
    if (made.length !== 1 || made[0].rpId !== HOST || !made[0].isResidentCredential) failures.push(`the authenticator holds ${JSON.stringify(made.map(c => [c.rpId, c.isResidentCredential]))}`)
    if (!kept.passkey || kept.ssh) failures.push('the device keeps something other than a passkey')
    if (kept.account !== 'acct-test') failures.push(`the device is kept for ${kept.account}, not the signed-in account`)
    if (offer.channel !== built || offer.account !== 'acct-test' || offer.node_key.length !== 43) failures.push(`the offer is wrong: ${JSON.stringify(offer)}`)
    // Exactly one: the registration with the directory (ov-771), which proves the
    // browser holds the key. Making the passkey itself, and showing the code, cost none.
    if (touches !== 1) failures.push(`making the passkey cost ${touches} sign-ins, not the one registration`)
    console.log(`csp-check: the offer's key is ${key.type} for ${key.application}`)

    if (workstationOnly) {
      runner.enroll(offer.key_a)
      await everyRunner(page)
      await workspaceWalk(page)
      await workstationWalk(page)
      await detailWalk(page)
      await imageWalk(page)
      await switchWalk(page)
      throw new WalkDone()
    }

    if (await page.$('#scan')) failures.push('an unsigned camera reply can bypass device verification')

    // A stored code past the window is discarded for a new one.
    await edit(page, { offerShownAt: Date.now() - 10 * 60_000 })
    await page.reload()
    await enrolling(page)
    kept = await device(page)
    if (JSON.parse(kept.offer).ceremony === offer.ceremony || Date.now() - kept.offerShownAt > 60_000) failures.push('a stored code past its window was kept')
    else console.log('csp-check: a stored code past its window was replaced')

    // Another account's device is never used: its passkey and runners go.
    await edit(page, { account: 'acct-someone-else', runners: [{ id: 'x', label: 'theirs', user: 'u', host_key: 'SHA256:x', token: 't', why: '' }] })
    await page.reload()
    await statusSays(page, 'Make a passkey')
    kept = await device(page)
    if (kept.account !== 'acct-test' || kept.runners || kept.passkey) failures.push("another account's passkey or runners were used")
    else console.log("csp-check: another account's device was discarded")

    // A slice-1 device, whose key was WebCrypto Ed25519, is asked for a
    // passkey and a new enrollment, and says why.
    await edit(page, { ssh: { slice: 1 }, runners: [{ id: 'old', label: 'old', user: 'u', host_key: 'SHA256:x', token: 't', why: '' }] })
    await page.reload()
    await statusSays(page, 'now signs in to your runners with a passkey')
    kept = await device(page)
    if (kept.ssh || kept.runners) failures.push("a slice-1 device's key or runners were kept")
    else console.log('csp-check: a slice-1 device was asked for a passkey')
    await shoot(page, 'migrated')
    await makePasskey(page)
    await session.passkeys.held((await device(page)).passkey.id)
    if (!credential) failures.push("the authenticator doesn't hold the device's passkey")
    // The same account's passkey has the same user handle every time, so an
    // authenticator replaces it rather than keeping one per enrollment.
    else if (credential.userHandle !== made[0].userHandle) failures.push('a second passkey for the same account has a new user handle')
    else console.log('csp-check: the same account made its passkey under the same user handle')

    // A browser paired by an earlier build (deb71c680, Oct 27) starts on this
    // one (ov-869): its record has a passkey and runners but no `registered`,
    // and runners without `cause`. On a slow start the screen says which step
    // it is on and counts, and at ten seconds offers a way forward, rather
    // than "Starting Far Cooler…" alone; the start then finishes behind it.
    const registered = (await device(page)).registered
    await edit(page, { registered: null, offer: null, offerShownAt: null, runners: [{ id: 'cosmo', label: 'Cosmo', user: 'u', host_key: 'SHA256:x', token: 't', why: '' }] })
    meDelay = 13_000
    // Both wasm modules download while the account is checked, not after it
    // (ov-869). The stub relay holds the account for 13 s and serves the wasm
    // at once, so a start that loads them after the account asks for them
    // only at 13 s or later.
    const wasmAsked = []
    page.on('request', request => /\.wasm$/.test(new URL(request.url()).pathname) && wasmAsked.push(Date.now()))
    const reloaded = Date.now()
    await page.reload()
    await statusSays(page, 'Checking your account…', 3_000).catch(() => failures.push('a slow start never said it was checking the account'))
    const slow = await page
      .waitForFunction(
        () => document.getElementById('status-action')?.textContent === 'Reload' && !document.getElementById('status-action')?.hidden && document.getElementById('status-cancel')?.textContent === 'Sign Out and Start Again' && !document.getElementById('status-cancel')?.hidden && /1\d s so far/.test(document.getElementById('status-detail')?.textContent ?? ''),
        null,
        { timeout: 12_000 },
      )
      .then(
        () => true,
        () => false,
      )
    if (!slow) failures.push(`a slow start showed no way forward by ten seconds: ${JSON.stringify(await page.textContent('#status'))}`)
    else if (wasmAsked.length < 2 || Math.max(...wasmAsked) - reloaded > 6_000) failures.push(`the wasm modules were not both asked for while the account was checked: ${wasmAsked.map(at => at - reloaded)} ms after the reload`)
    else console.log(`csp-check: a slow start offered Reload and Sign Out and Start Again after ${Math.round((Date.now() - reloaded) / 1000)} s`)
    await shoot(page, 'slow-start')
    meDelay = 0
    page.removeAllListeners('request')
    await statusSays(page, 'Connect to Cosmo', 30_000).catch(() => failures.push("an earlier build's paired browser never reached Connect"))
    // Only meaningful while #status itself is shown, as it is on this screen.
    if (await page.isHidden('#status')) failures.push('the Connect screen is not on #status, so its buttons were not checked')
    else if (!(await page.isHidden('#status-cancel'))) failures.push('the start screen’s Sign Out and Start Again stayed up after the start finished')
    else console.log("csp-check: an earlier build's paired browser started on this one, at Connect to Cosmo")
    await edit(page, { registered: registered ?? null, runners: null })
    await page.reload()
    await enrolling(page)

    if (runner) {
      runner.enroll(JSON.parse((await device(page)).offer).key_a)
      // An unsigned teammate reply is refused before the signed reply arrives.
      const current = JSON.parse((await device(page)).offer)
      reports.ceremonyReply = signedReply(current, true)
      await page.waitForTimeout(3000)
      if (!(await page.isHidden('#compare')) || (await device(page)).runners) failures.push('an unsigned teammate reply was offered')
      const signed = signedReply(current)
      reports.ceremonyReply = signed
      await page.waitForSelector('#compare:not([hidden])', { timeout: 60_000 })
      const code = (await page.textContent('#compare-code')).replace(/\s/g, '')
      if (code !== signed.code) failures.push(`the device and browser codes differ: ${code}, ${signed.code}`)
      await page.click('#compare-yes')
      await page.waitForSelector('#join:not([hidden])', { timeout: 60_000 })
      const title = await page.textContent('#join-title')
      if (title !== 'Join Your Teammate’s Runner “studio”?') failures.push(`the join prompt says ${JSON.stringify(title)}`)
      if ((await device(page)).runners) failures.push("a teammate's runners were joined before the person said yes")
      console.log(`csp-check: a teammate's reply asked first: ${title}`)
      await shoot(page, 'join')
      await page.click('#join-yes')

      // Connect is a button, and a terminal opens through sshd at one touch.
      await statusSays(page, 'Connect to studio')
      await shoot(page, 'connect')
      // Typed as soon as the terminal has focus, which is BEFORE the runner
      // has accepted the attach on a slow link: the page must hold those
      // keystrokes, not drop them (CI's slower runner caught it dropping them).
      const before = touches
      await connect(page, false)
      if (touches - before !== 1) failures.push(`connecting cost ${touches - before} touches, not 1`)
      const early = !(await page.$(LIVE))
      await page.keyboard.type('echo hi')
      await page.waitForSelector(LIVE, { timeout: 60_000 })
      // Then waited for, on the runner's own record of what arrived.
      const deadline = Date.now() + 30_000
      while (runner.typed() !== 'echo hi' && Date.now() < deadline) await new Promise(r => setTimeout(r, 100))
      const typed = runner.typed()
      if (typed !== 'echo hi') failures.push(`the runner got ${JSON.stringify(typed)}, not "echo hi"`)
      const log = readFileSync(runner.log, 'utf8')
      if (!log.split('\n').some(l => l.includes('Accepted publickey') && l.includes('ECDSA-SK'))) failures.push('sshd logged no ECDSA-SK login')
      console.log(`csp-check: Connect opened a terminal through sshd with the passkey (ECDSA-SK), and the runner got the keystrokes${early ? ' typed before the pane was live' : ''}`)

      // The plan tab reads the board's plan over the same connection (ov-775).
      await page.click('#show-plan')
      await page.waitForFunction(() => document.getElementById('plan')?.textContent?.includes('Visual language'), null, { timeout: 30_000 })
      console.log('csp-check: the Plan tab showed the runner’s theme')
      await page.click('#show-terminals')
      await shoot(page, 'terminal')

      if (maxSessions < 10) {
        await lowLimit(page)
        throw new WalkDone()
      }
      await panes(page)

      // Each reconnect is one touch, counted on eight.
      const counted = []
      for (let i = 0; i < 8; i++) {
        const at = touches
        await page.reload()
        await connect(page)
        counted.push(touches - at)
      }
      if (counted.some(n => n !== 1)) failures.push(`eight reconnects cost ${JSON.stringify(counted)} touches`)
      else console.log(`csp-check: eight reconnects cost one touch each: ${JSON.stringify(counted)}`)

      // A browser that refuses to prompt (Safari, once the handshake outlasted
      // the Connect click) gets Use Passkey, whose click signs at one touch.
      await page.reload()
      await statusSays(page, 'Connect to studio')
      await page.evaluate(() => (globalThis.__fcRefuseOnce = true))
      let at = touches
      await page.click('#status-action')
      await statusSays(page, 'Sign in to studio')
      await shoot(page, 'use-passkey')
      await page.click('#status-action')
      await page.waitForSelector(LIVE, { timeout: 60_000 })
      if (touches - at !== 1) failures.push(`Use Passkey cost ${touches - at} touches, not 1`)
      else console.log('csp-check: a refused prompt offered Use Passkey, which signed at one touch')
      // Its Cancel ends the attempt with the declined sentence and no touch.
      await page.reload()
      await statusSays(page, 'Connect to studio')
      await page.evaluate(() => (globalThis.__fcRefuseOnce = true))
      at = touches
      await page.click('#status-action')
      await statusSays(page, 'Sign in to studio')
      await page.click('#status-cancel')
      await statusSays(page, 'didn’t get your passkey')
      if (touches !== at) failures.push('Cancel on Use Passkey spent a touch')
      else console.log('csp-check: Cancel on Use Passkey ended the attempt without a touch')

      // The pin altered: refused with the host-key sentence, no terminal, and
      // no touch: the passkey is never asked for a runner it doesn't trust.
      const enrolled = await device(page)
      await edit(page, { runners: enrolled.runners.map(runner => ({
        ...runner,
        host_key: 'SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA',
      })) })
      await page.reload()
      const pinned = touches
      await statusSays(page, 'Connect to studio')
      await page.click('#status-action')
      await page.waitForFunction(() => document.getElementById('status-detail')?.textContent?.includes('host key has changed'), null, { timeout: 120_000 })
      if (await page.isVisible('#work')) failures.push('a terminal opened with the pin altered')
      if (touches !== pinned) failures.push('the passkey was asked to sign for a runner whose pin was altered')
      console.log('csp-check: an altered pin was refused with the host-key sentence, without a touch')
      await shoot(page, 'refused')

      await everyRunner(page)
      await workspaceWalk(page)
      await workstationWalk(page)
      await detailWalk(page)
      await imageWalk(page)
      await switchWalk(page)
      await evictionWalk()
      page = session.page
      await page.goto(origin)
    }

    // ov-767 acceptance 2: the passkey, made for the app host, can't be used
    // from the parent domain's other child. Chromium refuses both asks: for
    // the app host's RP ID (not that origin's to claim) and for its own (no
    // such credential).
    const sibling = await session.context.newPage()
    watch(sibling)
    const siblingKeys = await authenticator(session.context, sibling)
    await sibling.goto(`https://${SIBLING}/version.json`)
    const at = touches
    const refused = await sibling.evaluate(
      async ({ host, sibling, id }) => {
        const ask = rpId =>
          navigator.credentials
            .get({ publicKey: { challenge: new Uint8Array(32), rpId, allowCredentials: [{ type: 'public-key', id: new Uint8Array(id) }], userVerification: 'discouraged', timeout: 5000 } })
            .then(
              () => 'signed',
              e => e.name,
            )
        return { appHost: await ask(host), own: await ask(sibling) }
      },
      { host: HOST, sibling: SIBLING, id: Array.from(Buffer.from(credential.credentialId, 'base64')) },
    )
    const held = await siblingKeys.held(credential.credentialId)
    if (held.length !== 1 || held[0].rpId !== HOST) failures.push('the sibling page had no passkey to refuse')
    if (refused.appHost !== 'SecurityError' || refused.own !== 'NotAllowedError' || touches !== at)
      failures.push(`from ${SIBLING} the passkey was not refused: ${JSON.stringify(refused)}, ${touches - at} touches`)
    else console.log(`csp-check: from ${SIBLING} Chromium refused the passkey: ${JSON.stringify(refused)}`)
    // The shell itself, served from that host, can't make one for the app host.
    await sibling.goto(`https://${SIBLING}/`)
    // With --fake-daemon the account has a sealed copy now (the eviction
    // walk), so the shell opens on Reconnect; Make a New Passkey is the way to
    // the same ask.
    if (runner) {
      await statusSays(sibling, 'Reconnect with your passkey')
      await sibling.click('#status-cancel')
    }
    await statusSays(sibling, 'Make a passkey')
    await sibling.click('#status-action')
    await statusSays(sibling, 'couldn’t make a passkey')
    console.log(`csp-check: the shell served from ${SIBLING} couldn't make a passkey for ${HOST}`)
    await sibling.close()

    // Sign Out takes the passkey's id and the runners with it.
    await page.click('form[action="/sign-out"] button')
    await page.waitForFunction(() => document.body?.innerText === 'signed out', null, { timeout: 30_000 })
    await page.goto(`${origin}/version.json`)
    if (await device(page)) failures.push('Sign Out left the passkey and runners behind')
    else console.log('csp-check: Sign Out deleted the passkey and runners')
    if (walkCredential) {
      if (reports.vaults?.has(walkCredential)) failures.push('Sign Out left the passkey’s sealed copy at the relay')
      else console.log('csp-check: Sign Out deleted the passkey’s sealed copy at the relay')
    }
  } catch (e) {
    if (!(e instanceof WalkDone)) {
      // Where it stopped: the screen on show says more than a timeout does.
      const shown = await Promise.all(
        session.context.pages().map(p => p.evaluate(() => {
          const surfaces = document.querySelectorAll('main > section:not([hidden]), #status:not([hidden]), dialog[open]')
          return [location.hash, ...[...surfaces].map(e => `${e.id}: ${e.textContent?.trim().slice(0, 200)}`)].join(' | ')
        }).catch(() => 'closed')),
      ).catch(() => [])
      const where = e.stack?.split('\n').find(line => line.includes('csp-check.mjs:'))?.trim() ?? ''
      failures.push(`the walk stopped: ${String(e.message).split('\n')[0]} ${where}; on screen: ${shown.join(' || ')}`)
    }
  }

  await new Promise(r => setTimeout(r, 500))
  for (const v of violations) failures.push(`CSP violation: ${v.directive} blocked ${v.blocked || '(inline)'} at ${v.source} ${v.sample ?? ''}`)
  if (!reports.registered) failures.push('the page never registered its passkey with the directory before showing its code')
  for (const r of reports.csp) failures.push(`CSP report: ${r.slice(0, 300)}`)
  for (const e of errors) if (!/Failed to load resource/.test(e)) failures.push(e)
} finally {
  await session.context.close()
  // By pid, never by pattern: this machine may run other sshds.
  for (const r of [runner, second]) if (r) spawnSync('/bin/kill', [String(r.pid)])
  rmSync(scratch, { recursive: true, force: true })
  rmSync(profile, { recursive: true, force: true })
}

if (failures.length) {
  for (const f of failures) console.error(`csp-check: ${f}`)
  process.exit(1)
}
console.log('csp-check: no CSP violations')
