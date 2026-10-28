#!/usr/bin/env node
// The client shell in headless Chromium, under the CSP production sends,
// failing on any violation (ov-764 acceptance 4), with its SSH key a passkey
// held by Chromium's virtual authenticator (ov-767).
//
//   node test/browser/csp-check.mjs [--dist dist] [--fake-daemon BINARY] [--screenshots DIR] [--cpu-throttle N] [--tunnel-delay MS]
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
//   enrollment screen with the passkey's sk-ecdsa line in the code, and the
//   camera;
// - a stored code past the ceremony's window is replaced, another account's
//   passkey and runners are discarded, and a slice-1 device (a WebCrypto
//   Ed25519 key) is asked for a passkey and a new enrollment;
// - with --fake-daemon: a phone's reply played through the fake camera and
//   read after the window is refused; a teammate's reply asks before it joins;
//   Connect opens a terminal through a REAL sshd (only sshd verifies a
//   passkey's webauthn-sk signature) whose Key A line is the offer's sk-ecdsa
//   key and whose forced command is crates/web's fake daemon
//   (`cargo build --example fake_daemon`); the runner gets the keystrokes and
//   sshd logs ECDSA-SK; eight reconnects cost one touch each; the same runner
//   with its pin altered is refused without a touch. The page's tunnel dial
//   is swapped, by this harness only, for a WebSocket spliced to that sshd;
//   everything above the dial is the page's own: SSH in wasm, the passkey's
//   assertion, the pinned host key, terminal.list, xterm;
// - the passkey can't be used from the parent domain's other child
//   (canary.farcooler.com for canary): Chromium refuses it (ov-767
//   acceptance 2);
// - Sign Out deletes the passkey's id and the runners.
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
import qrcode from 'qrcode-generator'

const args = process.argv.slice(2)
const flag = name => {
  const at = args.indexOf(name)
  return at >= 0 ? args[at + 1] : undefined
}
const dist = resolve(flag('--dist') ?? 'dist')
const fakeDaemon = flag('--fake-daemon') && resolve(flag('--fake-daemon'))
// For a person judging the screens: each one at a desktop and a phone width.
const shots = flag('--screenshots')
// Slow the page's CPU by this factor (Chromium's own throttling), to run the
// walk on a slow machine. A slow link is --tunnel-delay, which CI uses.
const throttle = Number(flag('--cpu-throttle') ?? 1)
// Delay each tunnel message by this many ms each way, for a slow network.
const delay = Number(flag('--tunnel-delay') ?? 0)
const later = send => (delay > 0 ? setTimeout(send, delay) : send())
const shoot = async (page, name) => {
  if (!shots) return
  for (const [width, height] of [[1280, 800], [390, 844]]) {
    await page.setViewportSize({ width, height })
    await page.waitForTimeout(300)
    await page.screenshot({ path: join(shots, `${name}-${width}.png`) })
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
const handle = shellHandler(read, env, reports)

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
let runner = null
if (fakeDaemon) {
  if (!existsSync('/usr/sbin/sshd')) throw new Error('no /usr/sbin/sshd here, so the passkey login cannot be checked')
  const dir = join(scratch, 'sshd')
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
      // The channel budget (ov-774) is checked against sshd's own default.
      'MaxSessions 10',
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
  runner = {
    pid,
    port,
    hostKey,
    log,
    /// The Key A line the phone's enrollment has the daemon write, in the
    /// fence's shape: restricted, with the daemon as its forced command.
    enroll: keyA => writeFileSync(keys, `restrict,command="${fakeDaemon} ${typedLog}" ${keyA}\n`),
    typed: () =>
      (existsSync(typedLog) ? readFileSync(typedLog, 'utf8') : '')
        .split('\n')
        .filter(Boolean)
        .map(hex => Buffer.from(hex, 'hex').toString())
        .join(''),
  }
}

const failures = []
const profile = mkdtempSync(join(tmpdir(), 'fc-csp-profile-'))
const violations = []
const errors = []
/// The passkey, carried from launch to launch as an authenticator keeps it.
let credential = null
/// Every assertion a virtual authenticator made: one per touch.
let touches = 0

/// A virtual authenticator on `page`: platform, resident keys, presence
/// simulated (a touch), holding `credential` if there is one.
async function authenticator(context, page) {
  const cdp = await context.newCDPSession(page)
  await cdp.send('WebAuthn.enable', { enableUI: false })
  const { authenticatorId } = await cdp.send('WebAuthn.addVirtualAuthenticator', {
    options: { protocol: 'ctap2', transport: 'internal', hasResidentKey: true, hasUserVerification: true, isUserVerified: true, automaticPresenceSimulation: true },
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
/// from launch to launch, with `video` as the fake camera when given.
async function launch(video) {
  const args = ['--use-fake-ui-for-media-stream', '--use-fake-device-for-media-stream']
  if (video) args.push(`--use-file-for-fake-video-capture=${video}`)
  const context = await chromium.launchPersistentContext(profile, { args, permissions: ['camera'], viewport: { width: 1280, height: 800 } })
  await context.route(`${origin}/**`, serve)
  await context.route(`https://${SIBLING}/**`, serve)
  if (runner) {
    await context.routeWebSocket(`wss://${HOST}/test-tunnel`, ws => {
      const tcp = createConnection({ host: '127.0.0.1', port: runner.port })
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
  if (runner) await context.addInitScript(swapDial)
  if (runner) await context.addInitScript(refusable)
  const page = context.pages()[0] ?? (await context.newPage())
  if (throttle > 1) await (await context.newCDPSession(page)).send('Emulation.setCPUThrottlingRate', { rate: throttle })
  watch(page)
  const passkeys = await authenticator(context, page)
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
        dial: () =>
          new Promise((resolve, reject) => {
            const ws = new WebSocket(`wss://${location.host}/test-tunnel`)
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

/// The stored device, or an edit to it, through the page's own IndexedDB.
const device = page =>
  page.evaluate(async () => {
    const db = await new Promise(r => (indexedDB.open('farcooler').onsuccess = e => r(e.target.result)))
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

/// A phone's reply to `offer`, as the QR code a fake camera shows: a y4m file
/// of the code on white, the shape Chromium's fake capture device plays.
function replyVideo(offer, grantedBy, name) {
  const blob = Buffer.from(offer.key_a.split(' ')[1], 'base64')
  const target = `SHA256:${createHash('sha256').update(blob).digest('base64').replace(/=+$/, '')}`
  const reply = JSON.stringify({
    v: 2,
    ceremony: offer.ceremony,
    account: offer.account,
    channel: offer.channel,
    target,
    runners: [{ id: 'r1', label: 'studio', alias: 'studio', user: me, host_key: runner.hostKey, reach: { kind: 'tailcat', token: 'test' }, pending: false }],
    ...(grantedBy ? { granted_by: grantedBy } : {}),
  })
  const code = qrcode(0, 'L')
  code.addData(reply, 'Byte')
  code.make()
  const [W, H] = [640, 480]
  const modules = code.getModuleCount()
  const scale = Math.floor((H - 40) / (modules + 8))
  const left = Math.floor((W - modules * scale) / 2)
  const top = Math.floor((H - modules * scale) / 2)
  const y = Buffer.alloc(W * H, 235)
  for (let r = 0; r < modules; r++)
    for (let c = 0; c < modules; c++)
      if (code.isDark(r, c))
        for (let dy = 0; dy < scale; dy++) y.fill(16, (top + r * scale + dy) * W + left + c * scale, (top + r * scale + dy) * W + left + (c + 1) * scale)
  const chroma = Buffer.alloc((W / 2) * (H / 2) * 2, 128)
  const frame = Buffer.concat([Buffer.from('FRAME\n'), y, chroma])
  const path = join(scratch, `${name}.y4m`)
  writeFileSync(path, Buffer.concat([Buffer.from(`YUV4MPEG2 W${W} H${H} F10:1 Ip A1:1 C420jpeg\n`), ...Array(10).fill(frame)]))
  return path
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
  let open = 0
  let peak = 0
  const refused = []
  let at = 0
  for (const line of text.split('\n')) {
    if (/channel \d+: new session|channel \d+: new \[server-session\]/.test(line)) open++
    else if (/channel \d+: free: server-session/.test(line)) open--
    if (at >= from) peak = Math.max(peak, open)
    if (/no more sessions|open failed|administratively prohibited/.test(line)) refused.push(line.trim())
    at += line.length + 1
  }
  return { peak, open, refused, end: text.length }
}

const sorted = xs => [...xs].sort((a, b) => a - b)
const percentile = (xs, p) => sorted(xs)[Math.min(xs.length - 1, Math.ceil(xs.length * p) - 1)]

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
      `csp-check: echo while ${LOADS} x seq 1 2000000 ran in another pane, page time (tunnel delay ${2 * delay} ms taken off), cpu throttle ${throttle}x: ` +
        `n=${under.length} p50=${percentile(under, 0.5).toFixed(1)} ms p95=${p95.toFixed(1)} ms max=${Math.max(...under).toFixed(1)} ms, ${chunks} chunks of load parsed`,
    )
    if (p95 > 100) failures.push(`keystroke echo under load was ${p95.toFixed(1)} ms at p95, over 100 ms`)
  }
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
    if (touches !== 0) failures.push(`making the passkey cost ${touches} sign-ins`)
    console.log(`csp-check: the offer's key is ${key.type} for ${key.application}`)

    await page.click('#scan')
    await page.waitForFunction(() => document.querySelector('#camera-video')?.readyState >= 2, null, { timeout: 30_000 })
    await page.waitForTimeout(1000)
    await page.click('#camera-cancel')
    console.log('csp-check: the camera opened and closed')

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

    if (runner) {
      runner.enroll(JSON.parse((await device(page)).offer).key_a)
      // A reply read after the window is refused, through the real camera path.
      let current = JSON.parse((await device(page)).offer)
      await session.context.close()
      session = await launch(replyVideo(current, null, 'late'))
      page = session.page
      await page.clock.install()
      await page.goto(origin)
      await enrolling(page)
      await page.clock.setSystemTime(Date.now() + 130_000)
      await page.click('#scan')
      await page.waitForFunction(() => document.getElementById('enroll-note')?.textContent?.includes('came after'), null, { timeout: 60_000 })
      kept = await device(page)
      if (kept.runners) failures.push('a reply after the window was taken')
      else console.log('csp-check: a reply after the window was refused')
      // The harness skewed this page's clock; put the new code's time back.
      await edit(page, { offerShownAt: Date.now() })

      // A teammate's reply asks before it joins.
      current = JSON.parse(kept.offer)
      await session.context.close()
      session = await launch(replyVideo(current, 'ana@example.com', 'teammate'))
      page = session.page
      await page.goto(origin)
      await enrolling(page)
      await page.click('#scan')
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
      await shoot(page, 'terminal')

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
      await edit(page, { runners: [{ id: 'r1', label: 'studio', user: me, host_key: 'SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA', token: 'test', why: '' }] })
      await page.reload()
      const pinned = touches
      await statusSays(page, 'Connect to studio')
      await page.click('#status-action')
      await page.waitForFunction(() => document.getElementById('status-detail')?.textContent?.includes('host key has changed'), null, { timeout: 120_000 })
      if (await page.isVisible('#work')) failures.push('a terminal opened with the pin altered')
      if (touches !== pinned) failures.push('the passkey was asked to sign for a runner whose pin was altered')
      console.log('csp-check: an altered pin was refused with the host-key sentence, without a touch')
      await shoot(page, 'refused')
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
  } catch (e) {
    failures.push(`the walk stopped: ${String(e.message).split('\n')[0]}`)
  }

  await new Promise(r => setTimeout(r, 500))
  for (const v of violations) failures.push(`CSP violation: ${v.directive} blocked ${v.blocked || '(inline)'} at ${v.source} ${v.sample ?? ''}`)
  for (const r of reports.csp) failures.push(`CSP report: ${r.slice(0, 300)}`)
  for (const e of errors) if (!/Failed to load resource/.test(e)) failures.push(e)
} finally {
  await session.context.close()
  // By pid, never by pattern: this machine may run other sshds.
  if (runner) spawnSync('/bin/kill', [String(runner.pid)])
  rmSync(scratch, { recursive: true, force: true })
  rmSync(profile, { recursive: true, force: true })
}

if (failures.length) {
  for (const f of failures) console.error(`csp-check: ${f}`)
  process.exit(1)
}
console.log('csp-check: no CSP violations')
