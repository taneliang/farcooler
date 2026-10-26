#!/usr/bin/env node
// The client shell in headless Chromium, under the CSP production sends,
// failing on any violation (ov-764 acceptance 4).
//
//   node test/browser/csp-check.mjs [--dist dist] [--fake-runner BINARY] [--screenshots DIR]
//
// Serves dist/ (build.sh's output, with the real wasm) through
// shell-server.ts, which uses the Worker's own header and asset code, and
// walks the page: boot (both wasm modules, a WebCrypto Ed25519 key, a node key
// minted in Go), the enrollment screen (the offer's QR code), and the camera
// (Chromium's fake one).
//
// With --fake-runner, it also opens a terminal. The binary is crates/web's
// example fake runner (`cargo build --example fake_runner`), which this starts
// and stops; the page's tunnel dial is swapped, by this harness only, for a
// WebSocket to this server that is spliced to it. Everything above the dial is the page's own: SSH in
// wasm, the WebCrypto signature, the pinned host key, terminal.list, the pane
// in xterm. Then it does it again with the pin altered and expects the
// host-key sentence and no terminal.
//
// Chromium comes from playwright-core; set PLAYWRIGHT_BROWSERS_PATH to where
// `npx playwright-core install chromium-headless-shell` put it.

import { spawn } from 'node:child_process'
import { readFileSync, mkdtempSync, rmSync } from 'node:fs'
import { createServer } from 'node:http'
import { createConnection } from 'node:net'
import { createInterface } from 'node:readline'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { pathToFileURL } from 'node:url'

import { build } from 'esbuild'
import { chromium } from 'playwright-core'
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
    tcp.on('data', data => ws.send(data))
    tcp.on('close', () => ws.close())
    tcp.on('error', () => ws.close())
    ws.on('message', data => tcp.write(data))
    ws.on('close', () => tcp.destroy())
  })
}

await new Promise(done => server.listen(0, '127.0.0.1', done))
const origin = `http://127.0.0.1:${server.address().port}`

const failures = []
const browser = await chromium.launch({ args: ['--use-fake-ui-for-media-stream', '--use-fake-device-for-media-stream'] })
try {
  const context = await browser.newContext({ permissions: ['camera'], viewport: { width: 1280, height: 800 } })
  const violations = []
  await context.exposeFunction('reportViolation', v => violations.push(v))
  await context.addInitScript(() => {
    document.addEventListener('securitypolicyviolation', e =>
      window.reportViolation({ directive: e.effectiveDirective, blocked: e.blockedURI, sample: e.sample, source: `${e.sourceFile}:${e.lineNumber}` }),
    )
  })
  const page = await context.newPage()
  const errors = []
  page.on('pageerror', e => errors.push(`page error: ${e.message}`))
  page.on('console', m => {
    if (m.type() === 'error') errors.push(`console: ${m.text()}`)
  })

  // A step that times out stops the walk, and the violations it caused are
  // still reported: they are usually why it timed out.
  try {
    // Boot and enroll.
    await page.goto(origin)
    await page.waitForFunction(() => document.getElementById('offer-fingerprint')?.textContent?.includes('SHA256:'), null, { timeout: 120_000 })
    console.log(`csp-check: enrollment screen up, ${await page.textContent('#offer-fingerprint')}`)
    await shoot(page, 'enroll')
    const offer = await page.evaluate(async () => {
      const db = await new Promise(r => (indexedDB.open('farcooler').onsuccess = e => r(e.target.result)))
      const device = await new Promise(r => (db.transaction('device').objectStore('device').get('this').onsuccess = e => r(e.target.result)))
      return { offer: JSON.parse(device.offer), extractable: device.ssh.privateKey.extractable, type: device.ssh.privateKey.algorithm.name }
    })
    if (offer.extractable || offer.type !== 'Ed25519') failures.push(`the SSH key is ${offer.type}, extractable ${offer.extractable}`)
    const built = JSON.parse(readFileSync(join(dist, 'version.json'), 'utf8')).channel
    if (offer.offer.channel !== built || offer.offer.account !== 'acct-test' || offer.offer.node_key.length !== 43) {
      failures.push(`the offer is wrong: ${JSON.stringify(offer.offer)}`)
    }

    await page.click('#scan')
    await page.waitForFunction(() => document.querySelector('#camera-video')?.readyState >= 2, null, { timeout: 30_000 })
    await page.waitForTimeout(1000)
    await page.click('#camera-cancel')
    console.log('csp-check: the camera opened and closed')

    if (runner) {
      // The dial, swapped for a WebSocket to /test-tunnel. Installed before the
      // page's own script runs, by trapping the global the Go module sets.
      await context.addInitScript(() => {
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
      })

      const grant = async hostKeyToPin =>
        page.evaluate(async pin => {
          const db = await new Promise(r => (indexedDB.open('farcooler').onsuccess = e => r(e.target.result)))
          const store = () => db.transaction('device', 'readwrite').objectStore('device')
          const device = await new Promise(r => (store().get('this').onsuccess = e => r(e.target.result)))
          delete device.offer
          device.runners = [{ id: 'r1', label: 'studio', user: 'owner', host_key: pin, token: 'test', why: '' }]
          await new Promise(r => (store().put(device, 'this').onsuccess = r))
        }, hostKeyToPin)

      // The pinned runner: a terminal opens and echoes what is typed.
      await grant(runner.hostKey)
      await page.reload()
      await page.waitForSelector('#work:not([hidden]) .terminal-choice', { timeout: 120_000 })
      await page.waitForSelector('#terminal .xterm', { timeout: 30_000 })
      await page.keyboard.type('echo hi')
      const deadline = Date.now() + 10_000
      while (runner.typed.join('') !== 'echo hi' && Date.now() < deadline) await new Promise(r => setTimeout(r, 100))
      const typed = runner.typed.join('')
      if (typed !== 'echo hi') failures.push(`the runner got ${JSON.stringify(typed)}, not "echo hi"`)
      console.log('csp-check: a terminal opened in xterm and the runner got the keystrokes')
      await shoot(page, 'terminal')

      // The pin altered: refused with the host-key sentence, and no terminal.
      await grant('SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA')
      await page.reload()
      await page.waitForFunction(() => document.getElementById('status-detail')?.textContent?.includes('host key has changed'), null, { timeout: 120_000 })
      if (await page.isVisible('#work')) failures.push('a terminal opened with the pin altered')
      console.log('csp-check: an altered pin was refused with the host-key sentence')
      await shoot(page, 'refused')
    }
  } catch (e) {
    failures.push(`the walk stopped: ${String(e.message).split('\n')[0]}`)
  }

  await page.waitForTimeout(500)
  for (const v of violations) failures.push(`CSP violation: ${v.directive} blocked ${v.blocked || '(inline)'} at ${v.source} ${v.sample ?? ''}`)
  for (const r of reports.csp) failures.push(`CSP report: ${r.slice(0, 300)}`)
  for (const e of errors) if (!/Failed to load resource/.test(e)) failures.push(e)
} finally {
  await browser.close()
  server.close()
  runner?.child.kill()
  rmSync(scratch, { recursive: true, force: true })
}

if (failures.length) {
  for (const f of failures) console.error(`csp-check: ${f}`)
  process.exit(1)
}
console.log('csp-check: no CSP violations')
