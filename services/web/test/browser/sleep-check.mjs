#!/usr/bin/env node
// What the web client does when a tab sleeps (ov-840, measuring ov-772's
// reconnect), in Playwright's three engines, against a real sshd in front of
// crates/web's fake daemon, the real wasm, and the real reconnect.ts.
//
//   node test/browser/sleep-check.mjs --fake-daemon BINARY [--dist dist]
//        [--engines chromium,webkit,firefox] [--cases a,b,c] [--no-clock]
//        [--json FILE]
//
// Every engine signs with the same software passkey (a P-256 key made here and
// handed to the page as `navigator.credentials`), because only Chromium has a
// virtual authenticator. A "touch" is one `credentials.get`. The tunnel dial is
// the same WebSocket splice csp-check.mjs uses, so this measures the page, not
// the DERP path (that needs a live relay: the second half of ov-840).
//
// How a sleep is simulated, and what each part is and is not:
//
// - The tab goes hidden, then visible: `document.visibilityState` is
//   overridden and `visibilitychange` dispatched. Headless engines never
//   hide a page by themselves, so this tests the page's handler, not the
//   engine's own decision to throttle.
// - Frozen JavaScript: Playwright's fake clock is paused, so no page timer
//   fires, then set forward by the sleep (1 or 10 minutes, never waited out)
//   and resumed, which makes every overdue timer fire at once, as a thawed
//   tab's do. In Chromium CDP `Page.setWebLifecycleState` frozen is asked for
//   too, but headless Chromium does not stop a visible page's timers with it
//   (measured: a 100 ms timer still ticked), so it adds nothing. Network
//   callbacks still run during the pause, which a real freeze does not allow.
// - A dead tunnel: the splice drops every byte both ways without closing
//   anything, as a relay that forgot the tunnel does.
// - Lid closed: a freeze and a dead tunnel and the browser offline for the
//   whole sleep, then `online`. An approximation: a real lid close also stops
//   the network stack and the radio, and the OS's TCP timers.

import { execFileSync, spawnSync } from 'node:child_process'
import { webcrypto } from 'node:crypto'
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { createConnection, createServer } from 'node:net'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { pathToFileURL } from 'node:url'
import { brotliDecompressSync } from 'node:zlib'

import { build } from 'esbuild'
import { chromium, firefox, webkit } from 'playwright-core'

const args = process.argv.slice(2)
const flag = name => {
  const at = args.indexOf(name)
  return at >= 0 ? args[at + 1] : undefined
}
const dist = resolve(flag('--dist') ?? 'dist')
const fakeDaemon = flag('--fake-daemon') && resolve(flag('--fake-daemon'))
if (!fakeDaemon) throw new Error('--fake-daemon BINARY is required')
const engineNames = (flag('--engines') ?? 'chromium,webkit,firefox').split(',')
const only = flag('--cases')?.split(',')
const jsonOut = flag('--json')
const ENGINES = { chromium, webkit, firefox }

const toml = readFileSync('wrangler.toml', 'utf8')
const canary = toml.slice(toml.indexOf('[env.canary.vars]'))
const setting = name => canary.match(new RegExp(`^${name} = "([^"]*)"`, 'm'))?.[1] ?? ''
const env = { DERP_HOSTS: setting('DERP_HOSTS'), DERP_MAP_ORIGIN: setting('DERP_MAP_ORIGIN') }
const built = JSON.parse(readFileSync(join(dist, 'version.json'), 'utf8')).channel
const HOST = built === 'stable' || built === 'preview' ? 'app.farcooler.com' : 'app.canary.farcooler.com'
const origin = `https://${HOST}`

const scratch = mkdtempSync(join(tmpdir(), 'fc-sleep-'))
const bundled = join(scratch, 'shell-server.mjs')
await build({ entryPoints: ['test/browser/shell-server.ts'], bundle: true, platform: 'node', format: 'esm', outfile: bundled, logLevel: 'error' })
const { shellHandler } = await import(pathToFileURL(bundled).href)
const handle = shellHandler(path => (existsSync(join(dist, path)) ? readFileSync(join(dist, path)) : null), env, { csp: [] })

async function serve(route) {
  const request = route.request()
  const method = request.method()
  const body = method === 'GET' || method === 'HEAD' ? undefined : (request.postDataBuffer() ?? undefined)
  const response = await handle(new Request(request.url(), { method, headers: request.headers(), body }))
  const headers = {}
  response.headers.forEach((value, key) => (headers[key] = value))
  let bytes = Buffer.from(await response.arrayBuffer())
  if (headers['content-encoding'] === 'br') {
    bytes = brotliDecompressSync(bytes)
    delete headers['content-encoding']
    delete headers['content-length']
  }
  await route.fulfill({ status: response.status, headers, body: bytes })
}

// --- The runner: a real sshd, the fake daemon behind it (csp-check.mjs's).
const me = execFileSync('/usr/bin/id', ['-un']).toString().trim()
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
const pidFile = join(dir, 'sshd.pid')
const typedLog = join(dir, 'typed')
writeFileSync(keys, '')
writeFileSync(
  join(dir, 'sshd_config'),
  [`Port ${port}`, 'ListenAddress 127.0.0.1', `HostKey ${join(dir, 'host')}`, `AuthorizedKeysFile ${keys}`, `PidFile ${pidFile}`, 'StrictModes no', 'UsePAM no', 'PasswordAuthentication no', 'KbdInteractiveAuthentication no', 'MaxSessions 10', 'LogLevel DEBUG1', ''].join('\n'),
)
const started = spawnSync('/usr/sbin/sshd', ['-f', join(dir, 'sshd_config'), '-E', join(dir, 'sshd.log')])
if (started.status !== 0) throw new Error(`sshd did not start: ${started.stderr}`)
let sshdPid = 0
for (let i = 0; i < 200 && !sshdPid; i++) {
  sshdPid = existsSync(pidFile) ? Number(readFileSync(pidFile, 'utf8').trim()) || 0 : 0
  if (!sshdPid) await new Promise(r => setTimeout(r, 50))
}
if (!sshdPid) throw new Error('sshd wrote no pid')
const typed = () =>
  (existsSync(typedLog) ? readFileSync(typedLog, 'utf8') : '')
    .split('\n')
    .filter(Boolean)
    .map(hex => Buffer.from(hex, 'hex').toString())
    .join('')

// --- The passkey: one P-256 key for every engine.
const keyPair = await webcrypto.subtle.generateKey({ name: 'ECDSA', namedCurve: 'P-256' }, true, ['sign'])
const spki = Array.from(Buffer.from(await webcrypto.subtle.exportKey('spki', keyPair.publicKey)))
const pkcs8 = Array.from(Buffer.from(await webcrypto.subtle.exportKey('pkcs8', keyPair.privateKey)))
const credentialId = Array.from(webcrypto.getRandomValues(new Uint8Array(16)))

/// `navigator.credentials` made of that key. Counts each sign-in in `__touches`.
function softPasskey({ spki, pkcs8, id, origin, rpId }) {
  const b64url = bytes => btoa(String.fromCharCode(...new Uint8Array(bytes))).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '')
  globalThis.__touches = 0
  let key
  const privateKey = async () => (key ??= await crypto.subtle.importKey('pkcs8', new Uint8Array(pkcs8), { name: 'ECDSA', namedCurve: 'P-256' }, false, ['sign']))
  const der = raw => {
    const int = part => {
      let i = 0
      while (i < part.length - 1 && part[i] === 0) i++
      let v = Array.from(part.slice(i))
      if (v[0] & 0x80) v = [0, ...v]
      return [2, v.length, ...v]
    }
    const body = [...int(raw.slice(0, 32)), ...int(raw.slice(32))]
    return new Uint8Array([0x30, body.length, ...body])
  }
  Object.defineProperty(navigator, 'credentials', {
    configurable: true,
    value: {
      create: async () => ({
        rawId: new Uint8Array(id).buffer,
        response: { getPublicKey: () => new Uint8Array(spki).buffer, getPublicKeyAlgorithm: () => -7 },
      }),
      get: async options => {
        globalThis.__touches++
        const rpHash = new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(rpId)))
        const authenticatorData = new Uint8Array(37)
        authenticatorData.set(rpHash)
        authenticatorData[32] = 0x01
        authenticatorData[36] = globalThis.__touches & 0xff
        const clientDataJSON = new TextEncoder().encode(JSON.stringify({ type: 'webauthn.get', challenge: b64url(options.publicKey.challenge), origin, crossOrigin: false }))
        const hash = new Uint8Array(await crypto.subtle.digest('SHA-256', clientDataJSON))
        const signed = new Uint8Array([...authenticatorData, ...hash])
        const raw = new Uint8Array(await crypto.subtle.sign({ name: 'ECDSA', hash: 'SHA-256' }, await privateKey(), signed))
        return {
          rawId: new Uint8Array(id).buffer,
          response: { authenticatorData: authenticatorData.buffer, clientDataJSON: clientDataJSON.buffer, signature: der(raw).buffer },
        }
      },
    },
  })
}

/// The tunnel dial, swapped for the splice (csp-check.mjs's `swapDial`).
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
              if (globalThis.__debugWs) console.log(`ws message ${e.data.byteLength} at ${Date.now()}`)
              const bytes = new Uint8Array(e.data)
              if (waiting) {
                waiting(bytes)
                waiting = null
              } else queue.push(bytes)
            }
            ws.onclose = e => {
              if (globalThis.__debugWs) console.log(`ws closed code=${e.code} clean=${e.wasClean} at ${Date.now()}`)
              closed = true
              if (waiting) waiting(null)
            }
            ws.onerror = () => reject(new Error('io'))
            ws.onopen = () =>
              resolve({
                read: () => (queue.length ? Promise.resolve(queue.shift()) : closed ? Promise.resolve(null) : new Promise(r => (waiting = r))),
                write: bytes => ws.send(bytes.slice()),
                close: () => {
                  if (globalThis.__debugWs) console.log(`page closes the socket at ${Date.now()}`)
                  ws.close()
                },
              })
          }),
      }
    },
  })
}

const sleepMs = n => new Promise(r => setTimeout(r, n))
const device = page =>
  page.evaluate(async () => {
    const db = await new Promise(r => (indexedDB.open('farcooler').onsuccess = e => r(e.target.result)))
    const d = await new Promise(r => (db.transaction('device').objectStore('device').get('this').onsuccess = e => r(e.target.result)))
    db.close()
    return d
  })
const edit = (page, change) =>
  page.evaluate(async change => {
    const db = await new Promise(r => (indexedDB.open('farcooler').onsuccess = e => r(e.target.result)))
    const store = () => db.transaction('device', 'readwrite').objectStore('device')
    const d = await new Promise(r => (store().get('this').onsuccess = e => r(e.target.result)))
    Object.assign(d, change)
    await new Promise(r => (store().put(d, 'this').onsuccess = r))
    db.close()
  }, change)
const statusSays = (page, text, timeout = 90_000) =>
  page.waitForFunction(t => !document.getElementById('status')?.hidden && document.getElementById('status')?.textContent?.includes(t), text, { timeout })
const LIVE = '#terminal .terminal-host:not([hidden])[data-attached="true"]'

/// One browser and context: the tunnel splice with a switch for the dead tunnel.
async function open(engine) {
  const browser = await ENGINES[engine].launch()
  const context = await browser.newContext({ viewport: { width: 1280, height: 800 } })
  await context.route(`${origin}/**`, serve)
  const tunnels = new Set()
  await context.routeWebSocket(`wss://${HOST}/test-tunnel`, ws => {
    const tcp = createConnection({ host: '127.0.0.1', port })
    const t = { dead: false }
    tunnels.add(t)
    tcp.on('data', data => !t.dead && ws.send(data))
    tcp.on('close', () => !t.dead && ws.close())
    tcp.on('error', () => !t.dead && ws.close())
    ws.onMessage(data => !t.dead && tcp.write(typeof data === 'string' ? Buffer.from(data) : data))
    ws.onClose(() => tcp.destroy())
  })
  const errors = []
  const page = await context.newPage()
  page.on('pageerror', e => errors.push(`page error: ${e.message}`))
  if (process.env.SLEEP_DEBUG) page.on('console', m => console.log(`  [${engine} console ${m.type()}] ${m.text()}`))
  await context.addInitScript(softPasskey, { spki, pkcs8, id: credentialId, origin, rpId: HOST })
  if (process.env.SLEEP_DEBUG) await context.addInitScript(() => (globalThis.__debugWs = true))
  await context.addInitScript(swapDial)
  if (!args.includes('--no-clock')) await page.clock.install()
  return {
    browser,
    context,
    page,
    errors,
    /// Every tunnel open now goes silent; ones dialed after are fine.
    blackhole() {
      for (const t of tunnels) t.dead = true
    },
    close: () => browser.close(),
  }
}

/// A fresh page on a connected runner with a live pane; the first call of a
/// context also makes the passkey and enrolls it.
async function connected(s, first) {
  const { page } = s
  await page.goto(origin)
  if (first) {
    await statusSays(page, 'Make a passkey')
    await page.click('#status-action')
    await page.waitForFunction(() => !document.getElementById('enroll')?.hidden && document.getElementById('offer-fingerprint')?.textContent?.includes('SHA256:'), null, { timeout: 90_000 })
    const offer = JSON.parse((await device(page)).offer)
    writeFileSync(keys, `restrict,command="${fakeDaemon} ${typedLog}" ${offer.key_a}\n`)
    await edit(page, { runners: [{ id: 'r1', label: 'studio', user: me, host_key: hostKey, token: 'test', why: '' }] })
    await page.reload()
  }
  await statusSays(page, 'Connect to studio')
  await page.click('#status-action')
  await page.waitForSelector('#work:not([hidden]) .terminal-choice', { timeout: 90_000 })
  await page.waitForSelector(LIVE, { timeout: 60_000 })
}

/// What the page is showing about the link, and whether the pane is live.
const state = page =>
  page.evaluate(() => {
    const note = document.getElementById('link-note')
    return {
      link: note.hidden ? 'up' : `${document.getElementById('link-title').textContent} | ${document.getElementById('link-detail').textContent}`,
      action: document.getElementById('link-action').hidden ? '' : document.getElementById('link-action').textContent,
      live: !!document.querySelector('#terminal .terminal-host:not([hidden])[data-attached="true"]'),
      paneNote: document.getElementById('pane-note').hidden ? '' : document.getElementById('pane-note').textContent,
      touches: globalThis.__touches,
    }
  })

/// Whether a keystroke reaches the runner now: the one proof the pane works.
async function typesThrough(page, tag) {
  await page.click('#terminal .xterm', { timeout: 5000 }).catch(() => {})
  await page.keyboard.type(tag)
  const deadline = Date.now() + 8000
  while (Date.now() < deadline) {
    if (typed().includes(tag)) return true
    await sleepMs(150)
  }
  return false
}

const T1 = 60_000
const T10 = 600_000

/// A case: how long, and which parts of a sleep.
const CASES = [
  { name: 'idle-20s', ms: 0, dead: false, idle: 20000 },
  { name: 'bg-quick-alive', ms: 0, dead: false },
  { name: 'bg-1min-alive', ms: T1, dead: false },
  { name: 'bg-1min-dead', ms: T1, dead: true },
  { name: 'bg-10min-alive', ms: T10, dead: false },
  { name: 'bg-10min-dead', ms: T10, dead: true },
  { name: 'lid-10min', ms: T10, dead: true, offline: true },
]

const setVisible = (page, visible) =>
  page.evaluate(visible => {
    Object.defineProperty(document, 'visibilityState', { configurable: true, get: () => (visible ? 'visible' : 'hidden') })
    Object.defineProperty(document, 'hidden', { configurable: true, get: () => !visible })
    document.dispatchEvent(new Event('visibilitychange'))
  }, visible)

async function run(engine, s, c, n) {
  const { page, context } = s
  await connected(s, false)
  const out = { engine, case: c.name }
  const cdp = engine === 'chromium' ? await context.newCDPSession(page) : null
  await sleepMs(c.idle ?? Number(process.env.SLEEP_SETTLE ?? 0))
  if (c.idle) return { ...out, result: (await state(page)).link === 'up' ? 'reconnected without help' : 'lost the pane', detail: `idle ${c.idle} ms: ${(await state(page)).link}` }
  await setVisible(page, false)
  if (c.dead) s.blackhole()
  if (c.offline) {
    await context.setOffline(true)
    await page.evaluate(() => window.dispatchEvent(new Event('offline')))
  }
  const start = Date.now()
  await page.evaluate(() => {
    globalThis.__ticks = 0
    globalThis.__tickTimer ??= setInterval(() => globalThis.__ticks++, 100)
  })
  // Chromium's own freeze is asked for and measured: in headless Chromium it
  // does not stop a visible page's timers (see the report), so every engine
  // also gets the fake-clock freeze, which does.
  if (cdp) await cdp.send('Page.setWebLifecycleState', { state: 'frozen' })
  await page.clock.pauseAt((await page.evaluate(() => Date.now())) + 1000)
  await sleepMs(1500)
  if (cdp) {
    await cdp.send('Page.setWebLifecycleState', { state: 'active' })
    out.ticksWhileFrozen = await page.evaluate(() => globalThis.__ticks)
  }
  if (c.ms) await page.clock.setSystemTime((await page.evaluate(() => Date.now())) + c.ms)
  await page.clock.resume()
  out.how = `fake clock paused 1.5 s real, jumped ${c.ms / 60000} min, resumed${cdp ? `; Chromium's own freeze asked for too, and a 100 ms page timer still ticked ${out.ticksWhileFrozen} times in the 1.5 s` : ''}`
  if (c.offline) {
    await context.setOffline(false)
    await page.evaluate(() => window.dispatchEvent(new Event('online')))
  }
  const woke = Date.now()
  await setVisible(page, true)
  out.slept = `${((woke - start) / 1000).toFixed(1)} s real, ${c.ms / 60000} min on the page's clock`
  // What it settles to without a touch, within a few seconds past the probes.
  let after
  for (let i = 0; i < 60; i++) {
    await sleepMs(250)
    after = await state(page)
    if (after.link !== 'up' || i >= 40) break
  }
  out.notice = after.link
  out.noticedMs = after.link === 'up' ? null : Date.now() - woke
  if (after.link === 'up') {
    const ok = after.live && (await typesThrough(page, `ok${n}a`))
    out.result = ok ? 'reconnected without help' : 'lost the pane'
    out.detail = ok ? 'no notice shown; input reached the runner' : `link up but pane live=${after.live}, note=${JSON.stringify(after.paneNote)}`
  } else if (after.action) {
    const touchesBefore = after.touches
    const clicked = Date.now()
    await page.click('#link-action')
    let end
    for (let i = 0; i < 200; i++) {
      await sleepMs(300)
      end = await state(page)
      if (end.link === 'up' || /Can’t|couldn’t|Couldn’t/.test(end.link)) break
    }
    const ok = end.link === 'up' && end.live && (await typesThrough(page, `ok${n}b`))
    out.result = ok ? 'needed a touch' : 'lost the pane'
    out.detail = `${end.touches - touchesBefore} touch(es), pane back ${Date.now() - clicked} ms after the click; end: ${end.link}; live=${end.live}${end.paneNote ? `; pane note: ${end.paneNote}` : ''}`
  } else {
    out.result = 'lost the pane'
    out.detail = `notice without a button: ${after.link}`
  }
  return out
}

const results = []
const failures = []
try {
  for (const engine of engineNames) {
    let s
    try {
      s = await open(engine)
      await connected(s, true)
      let n = 0
      for (const c of CASES.filter(c => !only || only.includes(c.name))) {
        try {
          const r = await run(engine, s, c, ++n)
          results.push(r)
          console.log(`sleep-check: ${engine} ${c.name}: ${r.result}${r.noticedMs != null ? ` (notice ${r.noticedMs} ms after waking: ${r.notice})` : ''}; ${r.detail}`)
        } catch (e) {
          const r = { engine, case: c.name, result: 'harness error', detail: String(e.message).split('\n')[0] }
          results.push(r)
          console.log(`sleep-check: ${engine} ${c.name}: harness error: ${r.detail}`)
        }
      }
      for (const e of s.errors) console.log(`sleep-check: ${engine} ${e}`)
    } catch (e) {
      failures.push(`${engine} could not run: ${String(e.message).split('\n')[0]}`)
    } finally {
      await s?.close()
    }
  }
} finally {
  spawnSync('/bin/kill', [String(sshdPid)])
  rmSync(scratch, { recursive: true, force: true })
}
if (jsonOut) writeFileSync(jsonOut, JSON.stringify(results, null, 2))
// A woken tab asks at once (reconnect.ts `recheck`), so a dead tunnel shows
// within the probe's 2.5 s deadline and some slack, not after the watch's nine
// quiet seconds.
const NOTICE_WITHIN_MS = 5000
for (const r of results) if (r.noticedMs > NOTICE_WITHIN_MS) Object.assign(r, { result: 'slow to notice', detail: `${r.detail}; the notice took ${r.noticedMs} ms, over ${NOTICE_WITHIN_MS}` })
const bad = results.filter(r => r.result === 'lost the pane' || r.result === 'slow to notice' || r.result === 'harness error')
for (const f of failures) console.error(`sleep-check: ${f}`)
for (const r of bad) console.error(`sleep-check: ${r.engine} ${r.case}: ${r.result}: ${r.detail}`)
process.exit(failures.length || bad.length ? 1 : 0)
