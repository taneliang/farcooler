import assert from 'node:assert/strict'
import { spawn, execFileSync } from 'node:child_process'
import { once } from 'node:events'
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { createInterface } from 'node:readline'
import { build } from 'esbuild'
import { Miniflare } from 'miniflare'
import { chromium } from 'playwright-core'

const root = resolve(process.env.OV1483_REPO ?? '../../..')
const scratch = process.env.FARCOOLER_SCRATCH
assert(scratch, 'FARCOOLER_SCRATCH is required')
const dist = resolve('dist')
const output = join(root, 'reports/ov-1483/captures')
mkdirSync(output, { recursive: true })
const src = join(root, 'services/web/src')
const origin = 'https://app.canary.farcooler.com'
const canary = readFileSync(join(root, 'services/web/wrangler.toml'), 'utf8').split('[env.canary.vars]')[1]
const config = key => {
  const line = canary.match(new RegExp('^' + key + ' = "([^"]*)"', 'm'))
  assert(line, key)
  return line[1]
}
const encode = value => Buffer.from(JSON.stringify(value)).toString('base64url')
const access = encode({ alg: 'RS256' }) + '.' + encode({ sub: 'fixture-owner', exp: Math.floor(Date.now() / 1000) + 600 }) + '.fixture'
const cookie = '__Host-fc_session=' + encode({ a: access, r: 'fixture-refresh', c: 'fixture-csrf' })

const workerSource = await build({
  entryPoints: [join(src, 'index.ts')], bundle: true, format: 'esm', write: false,
})
const mf = new Miniflare({
  name: "ov1483-web",
  modules: true,
  script: workerSource.outputFiles[0].text,
  compatibilityDate: '2026-01-15',
  assets: { directory: dist, binding: 'ASSETS', routerConfig: { invoke_user_worker_ahead_of_assets: true }, assetConfig: { html_handling: 'none' } },
  bindings: {
    APP_HOST: new URL(origin).hostname,
    APP_RELAY_URL: 'https://fixture.invalid',
    APP_WORKOS_CLIENT_ID: 'client_fixture',
    WEB_CLIENT: 'owner',
    WEB_ACCOUNTS: 'fixture-owner',
    DERP_MAP_ORIGIN: config('DERP_MAP_ORIGIN'),
    DERP_HOSTS: config('DERP_HOSTS'),
  },
  outboundService: async request => {
    assert.equal(request.url, 'https://fixture.invalid/v1/directory/me')
    assert.equal(request.headers.get('authorization'), 'Bearer ' + access)
    return Response.json({ user: { id: 'fixture-owner' } })
  },
})

function observe(page) {
  const failures = []
  page.on('pageerror', error => failures.push(error.message))
  page.on('console', message => {
    if (message.type() === 'error') failures.push(message.text())
  })
  page.on('response', response => {
    if (response.status() >= 400 && response.request().resourceType() !== 'other') failures.push(response.status() + ' ' + response.url())
  })
  return page.addInitScript(() => {
    window.violations = []
    window.addEventListener('securitypolicyviolation', event => window.violations.push(event.effectiveDirective + ' ' + event.blockedURI))
  }).then(() => async () => {
    assert.deepEqual(await page.evaluate(() => window.violations), [])
    assert.deepEqual(failures, [])
  })
}

async function viewport(page, width, height = 800) {
  await page.setViewportSize({ width, height })
  assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'horizontal page overflow')
}

async function paintedTerminal(page) {
  await page.waitForFunction(() => {
    const canvas = document.querySelector('.xterm-text-layer')
    if (!canvas?.width || !canvas.height || !canvas.getBoundingClientRect().width) return false
    const pixels = canvas.getContext('2d').getImageData(0, 0, Math.min(300, canvas.width), Math.min(72, canvas.height)).data
    return pixels.some((value, index) => index % 4 === 1 && value > 100)
  }, null, { polling: 100 })
  // Visibility and fitting can recreate the canvas between frames. Capture
  // only after the observer, xterm, and the browser have presented the new size.
  await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))))
}

async function capture(page, name, fullPage = false) {
  await page.mouse.move(0, 0)
  await page.screenshot({ path: join(output, name + '.png'), fullPage, animations: 'disabled' })
}

async function exercise(page, base, policy, name, save = false) {
  const check = await observe(page)
  const url = base + '/?theme=dark#/workspace/overnight'
  await viewport(page, 1280)
  const shell = await page.goto(url)
  assert.equal(shell.headers()['content-security-policy'], policy)
  await page.getByRole('heading', { name: 'Board', exact: true }).waitFor()
  await page.locator('.xterm-accessibility-tree').getByText('✓ Runner connected').waitFor({ state: 'attached' })
  await paintedTerminal(page)
  if (save) await capture(page, 'after-board-1280')
  assert.equal(await page.locator('.primary:visible').count(), 1)
  const picker = page.getByRole('combobox', { name: 'Workspace' })
  await picker.focus()
  await page.keyboard.press('Space')
  await page.getByRole('option', { name: 'Nightly builds' }).click()
  await page.getByText('Check nightly artifacts').waitFor()
  await page.goBack()
  await page.getByText('Choose a web design system').waitFor()
  const start = page.getByRole('button', { name: 'Start work' })
  await start.click()
  await page.getByRole('dialog', { name: 'Start work' }).waitFor()
  assert.equal(await page.locator('.dialog .primary:visible').count(), 1)
  await page.keyboard.press('Escape')
  await page.getByRole('dialog').waitFor({ state: 'hidden' })
  assert(await start.evaluate(node => document.activeElement === node), 'dialog should restore focus')
  await start.click()
  await page.getByRole('button', { name: 'Start ticket' }).click()
  await page.getByRole('region', { name: 'In progress' }).getByText('Choose a web design system').waitFor()

  // Compare the actual xterm node across router transitions, then verify its
  // byte stream still emits and accepts input and that recovery reaches the UI.
  await page.evaluate(() => { window.originalTerminal = document.querySelector('.xterm') })
  await page.getByRole('link', { name: 'Orchestrator', exact: true }).click()
  assert(await page.evaluate(() => window.originalTerminal === document.querySelector('.xterm')))
  const terminal = page.locator('.xterm-helper-textarea')
  const focusTerminal = page.getByRole('button', { name: 'Focus terminal' })
  assert.equal(await page.locator('.primary:visible').count(), 1)
  await focusTerminal.click()
  assert(await terminal.evaluate(node => document.activeElement === node), 'terminal should take focus')
  await page.keyboard.type('status')
  await page.keyboard.press('Enter')
  await page.locator('.xterm-accessibility-tree').getByText('Connected · orchestrator running').waitFor({ state: 'attached' })
  await page.keyboard.type('exit')
  await page.keyboard.press('Enter')
  await page.getByRole('status').getByText('Session lost').waitFor()
  assert.equal(await page.locator('.primary:visible').count(), 1)
  await paintedTerminal(page)
  if (save) await capture(page, 'after-recover-1280')
  await page.getByRole('button', { name: 'Recover terminal' }).click()
  await page.getByRole('status').getByText('Connected').waitFor()
  await page.locator('.xterm-accessibility-tree').getByText('Session recovered.').waitFor({ state: 'attached' })

  for (const width of [320, 1280]) {
    await viewport(page, width)
    await page.goto(url)
    await page.reload()
    await page.getByRole('heading', { name: 'Board', exact: true }).waitFor()
    await viewport(page, width)
    if (save && width === 320) await capture(page, 'after-board-320', true)
    await page.getByRole('link', { name: 'Orchestrator', exact: true }).click()
    await page.locator('.xterm-accessibility-tree').getByText('✓ Runner connected').waitFor({ state: 'attached' })
    await paintedTerminal(page)
    if (save && width === 320) await capture(page, 'after-terminal-320')

    await page.goto(base + '/#/design')
    await page.getByRole('heading', { name: 'Workspace design system' }).waitFor()
    await viewport(page, width)
    if (save) await capture(page, 'sample-light-' + width, true)
    await page.getByRole('button', { name: 'Dark mode' }).click()
    assert.equal(await page.locator('html').getAttribute('data-theme'), 'dark')
    if (save) await capture(page, 'sample-dark-' + width, true)
  }
  await check()
  console.log(name + ': shell header identical, no page/asset/CSP errors, controls and terminal flow passed at 320 and 1280')
}

async function captureBefore(browser, policy) {
  const bundle = await build({
    entryPoints: ['before.ts'], bundle: true, format: 'esm', write: false,
    alias: { 'legacy/board-screen': join(src, 'client/board-screen.ts') },
  })
  const styles = ['node_modules/@xterm/xterm/css/xterm.css', ...['app.css', 'chat.css'].map(name => join(src, 'client', name))]
    .map(path => readFileSync(path, 'utf8')).join('\n')
  const html = readFileSync(join(src, 'client/index.html'), 'utf8').replace('{{app.css}}', '/before.css').replace('{{app.js}}', '/before.js')
  const page = await browser.newPage({ colorScheme: 'dark' })
  try {
    const check = await observe(page)
    await page.route('https://before.invalid/**', route => {
      const path = new URL(route.request().url()).pathname
      const file = {
        '/': [html, 'text/html; charset=utf-8'],
        '/before.css': [styles, 'text/css; charset=utf-8'],
        '/before.js': [bundle.outputFiles[0].text, 'text/javascript; charset=utf-8'],
      }[path]
      return route.fulfill(file ? { body: file[0], contentType: file[1], headers: { 'content-security-policy': policy } } : { status: 204 })
    })
    await viewport(page, 1280)
    await page.goto('https://before.invalid/')
    await page.getByRole('heading', { name: 'Board', exact: true }).waitFor()
    await page.locator('.xterm-accessibility-tree').getByText('✓ Runner connected').waitFor({ state: 'attached' })
    await paintedTerminal(page)
    await capture(page, 'before-board-1280')
    await check()
  } finally {
    await page.close()
  }
}

let browser
let rust
try {
  const worker = await mf.getWorker('ov1483-web')
  const shell = await worker.fetch(origin + '/', { headers: { cookie } })
  assert.equal(shell.status, 200, await shell.clone().text())
  assert.equal(await shell.text(), readFileSync(join(dist, 'index.html'), 'utf8'))
  const policy = shell.headers.get('content-security-policy')
  assert(policy?.includes("style-src 'self'") && policy.includes("require-trusted-types-for 'script'"))
  writeFileSync(join(scratch, 'ov1483-policy.txt'), policy + '\n')
  browser = await chromium.launch({ executablePath: process.env.CHROMIUM_PATH || undefined })
  const page = await browser.newPage()
  await page.route(origin + '/**', async route => {
    const request = route.request()
    const reply = await worker.fetch(request.url(), {
      method: request.method(), headers: { ...request.headers(), cookie }, body: request.postDataBuffer(),
    })
    await route.fulfill({
      status: reply.status, headers: Object.fromEntries(reply.headers),
      body: Buffer.from(await reply.arrayBuffer()),
    })
  })
  await exercise(page, origin, policy, 'Cloudflare Worker (local workerd)', true)
  await page.close()
  await captureBefore(browser, policy)

  const binary = join(scratch, 'ov1483-loopback')
  execFileSync('rustc', ['loopback.rs', '-o', binary], { stdio: 'inherit' })
  rust = spawn(binary, [dist, policy], { stdio: ['ignore', 'pipe', 'inherit'] })
  const stopped = once(rust, 'exit')
  const lines = createInterface({ input: rust.stdout })
  const [host] = await Promise.race([once(lines, 'line'), stopped.then(() => { throw new Error('Rust server exited') })])
  await exercise(await browser.newPage(), 'http://' + host, policy, 'Rust loopback fixture')
} finally {
  try {
    await browser?.close()
  } finally {
    if (rust && rust.exitCode === null && rust.signalCode === null) {
      const stopped = once(rust, 'exit')
      rust.kill()
      await stopped
    }
    await mf.dispose()
  }
}
