#!/usr/bin/env node
// The client shell in headless Chromium, under the CSP production sends,
// failing on any violation (ov-764 acceptance 4).
//
//   node test/browser/csp-check.mjs [--dist dist] [--fake-runner BINARY] [--screenshots DIR] [--cpu-throttle N] [--tunnel-delay MS]
//
// Serves dist/ (build.sh's output, with the real wasm) through
// shell-server.ts, which uses the Worker's own header and asset code, and
// walks the page on one Chromium profile, relaunched where a step needs a
// different fake camera:
//
// - boot (both wasm modules, a WebCrypto Ed25519 key, a node key minted in
//   Go), the enrollment screen and the camera;
// - a stored code past the ceremony's window is replaced, and another
//   account's stored key and runners are discarded;
// - with --fake-runner: a phone's reply played through the fake camera and
//   read after the window is refused; a teammate's reply asks before it joins;
//   joining opens a terminal on crates/web's fake runner (`cargo build
//   --example fake_runner`, started and stopped here) and the runner gets the
//   keystrokes; the same runner with its pin altered is refused. The page's
//   tunnel dial is swapped, by this harness only, for a WebSocket spliced to
//   the fake runner; everything above the dial is the page's own: SSH in
//   wasm, the WebCrypto signature, the pinned host key, terminal.list, xterm;
// - Sign Out deletes the key and the runners.

// What the swapped dial leaves out is Tailcat's own traffic: the DERP
// WebSocket and its `/derp/probe` fetch, which the CSP must allow too. CI
// can't depend on a public relay, so that path was checked by hand through a
// real tunnel (ov-764 lane report), and it is what the owner's live check
// exercises.
//
// Chromium comes from playwright-core; set PLAYWRIGHT_BROWSERS_PATH to where
// `npx playwright-core install chromium-headless-shell` put it.

import { spawn } from 'node:child_process'
import { createHash } from 'node:crypto'
import { readFileSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { createServer } from 'node:http'
import { createConnection } from 'node:net'
import { createInterface } from 'node:readline'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { pathToFileURL } from 'node:url'

import { build } from 'esbuild'
import { chromium } from 'playwright-core'
import qrcode from 'qrcode-generator'
import { WebSocketServer } from 'ws'

const args = process.argv.slice(2)
const flag = name => {
  const at = args.indexOf(name)
  return at >= 0 ? args[at + 1] : undefined
}
const dist = resolve(flag('--dist') ?? 'dist')
const fakeRunner = flag('--fake-runner')
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

const server = createServer(async (req, res) => {
  const chunks = []
  for await (const chunk of req) chunks.push(chunk)
  const body = req.method === 'GET' || req.method === 'HEAD' ? undefined : Buffer.concat(chunks)
  const response = await handle(new Request(`http://${req.headers.host}${req.url}`, { method: req.method, headers: req.headers, body }))
  const headers = {}
  response.headers.forEach((value, key) => (headers[key] = value))
  res.writeHead(response.status, headers)
  res.end(Buffer.from(await response.arrayBuffer()))
})

// The fake runner, and the test-only tunnel: a WebSocket per dial, spliced
// to it. It prints its port and host key first, then a line per write.
let runner = null
if (fakeRunner) {
  const child = spawn(fakeRunner, [], { stdio: ['ignore', 'pipe', 'inherit'] })
  const lines = createInterface({ input: child.stdout })
  const typed = []
  const first = await new Promise((resolve, reject) => {
    lines.once('line', line => resolve(JSON.parse(line)))
    child.once('exit', code => reject(new Error(`the fake runner exited (${code})`)))
  })
  lines.on('line', line => {
    if (line.startsWith('typed ')) typed.push(Buffer.from(line.slice(6), 'hex').toString())
  })
  runner = { child, port: first.port, hostKey: first.host_key, typed }
  const sockets = new WebSocketServer({ server, path: '/test-tunnel' })
  sockets.on('connection', ws => {
    const tcp = createConnection({ host: '127.0.0.1', port: runner.port })
    tcp.on('data', data => later(() => ws.send(data)))
    tcp.on('close', () => later(() => ws.close()))
    tcp.on('error', () => ws.close())
    ws.on('message', data => later(() => tcp.write(data)))
    ws.on('close', () => tcp.destroy())
  })
}

await new Promise(done => server.listen(0, '127.0.0.1', done))
const origin = `http://127.0.0.1:${server.address().port}`

const failures = []
const profile = mkdtempSync(join(tmpdir(), 'fc-csp-profile-'))
const violations = []
const errors = []

/// One launch of Chromium on one profile, so IndexedDB (the device) survives
/// from launch to launch, with `video` as the fake camera when given.
async function launch(video) {
  const args = ['--use-fake-ui-for-media-stream', '--use-fake-device-for-media-stream']
  if (video) args.push(`--use-file-for-fake-video-capture=${video}`)
  const context = await chromium.launchPersistentContext(profile, { args, permissions: ['camera'], viewport: { width: 1280, height: 800 } })
  await context.exposeFunction('reportViolation', v => violations.push(v))
  await context.addInitScript(() => {
    document.addEventListener('securitypolicyviolation', e =>
      window.reportViolation({ directive: e.effectiveDirective, blocked: e.blockedURI, sample: e.sample, source: `${e.sourceFile}:${e.lineNumber}` }),
    )
  })
  if (runner) await context.addInitScript(swapDial)
  const page = context.pages()[0] ?? (await context.newPage())
  if (throttle > 1) await (await context.newCDPSession(page)).send('Emulation.setCPUThrottlingRate', { rate: throttle })
  page.on('pageerror', e => errors.push(`page error: ${e.message}`))
  page.on('console', m => {
    if (m.type() === 'error') errors.push(`console: ${m.text()}`)
  })
  return { context, page }
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
            const ws = new WebSocket(`ws://${location.host}/test-tunnel`)
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

/// The stored device, or an edit to it, through the page's own IndexedDB.
const device = page =>
  page.evaluate(async () => {
    const db = await new Promise(r => (indexedDB.open('farcooler').onsuccess = e => r(e.target.result)))
    const d = await new Promise(r => (db.transaction('device').objectStore('device').get('this').onsuccess = e => r(e.target.result)))
    db.close()
    return d ? { ...d, ssh: { extractable: d.ssh.privateKey.extractable, type: d.ssh.privateKey.algorithm.name } } : null
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
const enrolling = page =>
  page.waitForFunction(() => !document.getElementById('enroll')?.hidden && document.getElementById('offer-fingerprint')?.textContent?.includes('SHA256:'), null, { timeout: 120_000 })

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
    runners: [{ id: 'r1', label: 'studio', alias: 'studio', user: 'owner', host_key: runner.hostKey, reach: { kind: 'tailcat', token: 'test' }, pending: false }],
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

const built = JSON.parse(readFileSync(join(dist, 'version.json'), 'utf8')).channel
let session = await launch()
try {
  // A step that times out stops the walk, and the violations it caused are
  // still reported: they are usually why it timed out.
  try {
    let { page } = session
    // Boot and enroll.
    await page.goto(origin)
    await enrolling(page)
    console.log(`csp-check: enrollment screen up, ${await page.textContent('#offer-fingerprint')}`)
    await shoot(page, 'enroll')
    let kept = await device(page)
    const offer = JSON.parse(kept.offer)
    if (kept.ssh.extractable || kept.ssh.type !== 'Ed25519') failures.push(`the SSH key is ${kept.ssh.type}, extractable ${kept.ssh.extractable}`)
    if (kept.account !== 'acct-test') failures.push(`the device is kept for ${kept.account}, not the signed-in account`)
    if (offer.channel !== built || offer.account !== 'acct-test' || offer.node_key.length !== 43) failures.push(`the offer is wrong: ${JSON.stringify(offer)}`)

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

    // Another account's device is never used: its key and runners go.
    const before = kept.key ?? JSON.parse(kept.offer).key_a
    await edit(page, { account: 'acct-someone-else', runners: [{ id: 'x', label: 'theirs', user: 'u', host_key: 'SHA256:x', token: 't', why: '' }] })
    await page.reload()
    await enrolling(page)
    kept = await device(page)
    if (kept.account !== 'acct-test' || kept.runners || JSON.parse(kept.offer).key_a === before) failures.push("another account's key or runners were used")
    else console.log("csp-check: another account's device was discarded")

    if (runner) {
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

      // A teammate's reply asks before it joins, then the runner opens.
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

      // The joined runner: a terminal opens and echoes what is typed.
      // Typed as soon as the terminal has focus, which is BEFORE the runner
      // has accepted the attach on a slow link: the page must hold those
      // keystrokes, not drop them (CI's slower runner caught it dropping them).
      await page.waitForSelector('#work:not([hidden]) .terminal-choice', { timeout: 120_000 })
      await page.waitForFunction(() => document.activeElement?.closest('#terminal .xterm'), null, { timeout: 30_000 })
      const early = !(await page.$('#terminal[data-attached="true"]'))
      await page.keyboard.type('echo hi')
      await page.waitForSelector('#terminal[data-attached="true"]', { timeout: 60_000 })
      // Then waited for, on the runner's own record of what arrived.
      const deadline = Date.now() + 30_000
      while (runner.typed.join('') !== 'echo hi' && Date.now() < deadline) await new Promise(r => setTimeout(r, 100))
      const typed = runner.typed.join('')
      if (typed !== 'echo hi') failures.push(`the runner got ${JSON.stringify(typed)}, not "echo hi"`)
      else console.log(`csp-check: the reply read by the camera opened a terminal, and the runner got the keystrokes${early ? ' typed before the pane was live' : ''}`)
      await shoot(page, 'terminal')

      // The pin altered: refused with the host-key sentence, and no terminal.
      await edit(page, { runners: [{ id: 'r1', label: 'studio', user: 'owner', host_key: 'SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA', token: 'test', why: '' }] })
      await page.reload()
      await page.waitForFunction(() => document.getElementById('status-detail')?.textContent?.includes('host key has changed'), null, { timeout: 120_000 })
      if (await page.isVisible('#work')) failures.push('a terminal opened with the pin altered')
      console.log('csp-check: an altered pin was refused with the host-key sentence')
      await shoot(page, 'refused')
    }

    // Sign Out takes the key and the runners with it.
    await page.click('form[action="/sign-out"] button')
    await page.waitForURL(`${origin}/signed-out`, { timeout: 30_000 })
    await page.goto(`${origin}/version.json`)
    if (await device(page)) failures.push('Sign Out left the key and runners behind')
    else console.log('csp-check: Sign Out deleted the key and runners')
  } catch (e) {
    failures.push(`the walk stopped: ${String(e.message).split('\n')[0]}`)
  }

  await new Promise(r => setTimeout(r, 500))
  for (const v of violations) failures.push(`CSP violation: ${v.directive} blocked ${v.blocked || '(inline)'} at ${v.source} ${v.sample ?? ''}`)
  for (const r of reports.csp) failures.push(`CSP report: ${r.slice(0, 300)}`)
  for (const e of errors) if (!/Failed to load resource/.test(e)) failures.push(e)
} finally {
  await session.context.close()
  server.close()
  runner?.child.kill()
  rmSync(scratch, { recursive: true, force: true })
  rmSync(profile, { recursive: true, force: true })
}

if (failures.length) {
  for (const f of failures) console.error(`csp-check: ${f}`)
  process.exit(1)
}
console.log('csp-check: no CSP violations')
