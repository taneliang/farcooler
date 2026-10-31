#!/usr/bin/env node
// The conversation view in headless Chromium (ov-776), under the CSP the shell
// is served with (Trusted Types included), against a scripted runner.
//
//   node test/browser/chat-check.mjs [--screenshots DIR]
//
// It checks, in a real browser:
// - the rows of a long conversation are drawn as the Mac's kinds: the person's
//   bubbles, plain replies, a folded tool run, cards, a task list;
// - a row the runner changes is on screen within a second (acceptance 1);
// - a typed message goes out as one compose, leaves the box, and is drawn as
//   the person's turn (acceptance 2); a message that fails stays in the box;
// - images (ov-871): an attached, pasted or dropped image waits beside the box
//   as a thumbnail, can be taken out, and goes with the words as one compose;
//   one past the runner's limit is refused with the apps' sentence;
// - Stop is one interrupt and the working line goes (acceptance 3);
// - no CSP violation; no horizontal scroll at phone width.
//
// With --screenshots, writes desktop and phone captures in light and dark. The
// phone is Chromium's mobile emulation (CDP Emulation.setDeviceMetricsOverride
// with mobile=true), since headless --window-size can't go below about 500 px.

import { createServer } from 'node:http'
import { mkdirSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { build } from 'esbuild'
import { chromium } from 'playwright-core'

const args = process.argv.slice(2)
const shots = args.includes('--screenshots') ? args[args.indexOf('--screenshots') + 1] : null
if (shots) mkdirSync(shots, { recursive: true })

const out = mkdtempSync(join(tmpdir(), 'fc-chat-'))
const bundle = await build({ entryPoints: ['test/browser/chat-page.ts'], bundle: true, format: 'esm', target: 'es2022', write: false })
const css = readFileSync('src/client/app.css', 'utf8') + readFileSync('src/client/chat.css', 'utf8')
const html = `<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>Far Cooler</title>
<link rel="stylesheet" href="/app.css"><script type="module" src="/app.js"></script></head><body>
<header class="bar"><span class="title">Far Cooler</span><span class="who">owner@example.com</span><button class="quiet" type="button">Sign Out</button></header>
<main><section class="work" id="work"><nav class="sidebar" aria-label="Terminals"><span class="runner">studio</span><div class="terminal-list">
<button class="terminal-choice" aria-current="true" type="button">Fix the web build</button><button class="terminal-choice" type="button">Plan the release</button><button class="terminal-choice" type="button">shell</button></div></nav>
<div class="pane"><div class="terminal" id="terminal"></div></div></section></main></body></html>`
const files = { '/': [html, 'text/html'], '/app.js': [bundle.outputFiles[0].text, 'text/javascript'], '/app.css': [css, 'text/css'] }
// img-src as the shell's own policy has it (src/headers.ts): thumbnails of
// attached images are blob: URLs.
const csp = "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self' data: blob:; connect-src 'self'; require-trusted-types-for 'script'; trusted-types fc"
const server = createServer((req, res) => {
  const hit = files[req.url]
  if (!hit) return void res.writeHead(404).end()
  res.writeHead(200, { 'content-type': hit[1], 'content-security-policy': csp }).end(hit[0])
}).listen(0, '127.0.0.1')
await new Promise(done => server.once('listening', done))
const origin = `http://127.0.0.1:${server.address().port}/`

const failures = []
const check = (ok, what) => {
  console.log(`${ok ? 'ok  ' : 'FAIL'} ${what}`)
  if (!ok) failures.push(what)
}

const browser = await chromium.launch({ executablePath: process.env.CHROMIUM || undefined })
const violations = []

async function open(options) {
  const context = await browser.newContext(options)
  const page = await context.newPage()
  await page.addInitScript(() => document.addEventListener('securitypolicyviolation', e => window.__violations = [...(window.__violations ?? []), e.effectiveDirective]))
  page.on('pageerror', e => violations.push(`pageerror ${e.message}`))
  await page.goto(origin)
  await page.waitForSelector('.chat-list .bubble')
  return { context, page }
}

const desktop = { viewport: { width: 1280, height: 800 }, colorScheme: 'light' }
const phone = { viewport: { width: 390, height: 844 }, deviceScaleFactor: 3, isMobile: true, hasTouch: true, colorScheme: 'dark' }

// The behavior, on the desktop.
{
  const { context, page } = await open(desktop)
  const count = sel => page.locator(sel).count()
  check((await count('.bubble')) >= 3, 'the person’s messages are bubbles')
  check((await count('.disclosure')) >= 2, 'tool calls are lines that open')
  check((await page.locator('.tool-name', { hasText: 'Ran 1 commands' }).count()) === 0, 'a lone call is not called a run')
  check((await page.locator('.tool-name', { hasText: /^Used 3 tools$/ }).count()) === 1, 'a run of three calls is one line')
  check((await count('.card.subagent')) === 1 && (await count('.card.tasks')) === 1 && (await count('.card.ask')) === 1, 'subagent, ask and task list are cards')
  check((await page.locator('.turn-end', { hasText: 'Took 2:22' }).count()) === 1, 'a finished turn says how long it took')
  check(await page.locator('.chat-working').isVisible(), 'the working line shows while the newest turn is busy')
  check((await page.locator('.prose code', { hasText: 'build.mjs' }).count()) === 1 && (await count('.prose pre')) === 1, 'inline code and a code block are drawn as such')

  // Acceptance 1: a row changed on the runner is on screen within a second.
  const seen = await page.evaluate(async () => {
    const { runner } = window.__chat
    const started = performance.now()
    runner.put({ id: 'p:live', turn: 'turn:2', provisional: false, kind: { Prose: { text: 'The size gate passes at 412 KB.', conclusion: false } } })
    const list = document.querySelector('.chat-list')
    while (!list.textContent.includes('412 KB')) {
      if (performance.now() - started > 3000) return -1
      await new Promise(r => setTimeout(r, 5))
    }
    const first = performance.now() - started
    runner.put({ id: 'p:live', turn: 'turn:2', provisional: false, kind: { Prose: { text: 'The size gate passes at 412 KB, under the limit.', conclusion: false } } })
    const second = performance.now()
    while (!list.textContent.includes('under the limit')) {
      if (performance.now() - second > 3000) return -1
      await new Promise(r => setTimeout(r, 5))
    }
    return Math.max(first, performance.now() - second)
  })
  check(seen >= 0 && seen < 1000, `a new row and a changed row are drawn in step with the runner (${seen.toFixed(0)} ms)`)

  // Acceptance 2: a message is one compose, leaves the box, shows as the person's.
  const bubbles = await count('.bubble')
  await page.fill('.chat-input', '  Run the whole suite when you are done  ')
  check(!(await page.locator('.chat-send').isDisabled()), 'Send is on once there are words')
  await page.press('.chat-input', 'Enter')
  await page.waitForFunction(n => document.querySelectorAll('.bubble').length > n, bubbles)
  const sent = await page.evaluate(() => window.__chat.runner.composed)
  check(JSON.stringify(sent) === JSON.stringify(['Run the whole suite when you are done']), 'Return sends the trimmed words as one message')
  check((await page.inputValue('.chat-input')) === '' && (await page.locator('.chat-send').isDisabled()), 'the box empties and Send goes off once it is taken')
  await page.fill('.chat-input', 'Line one')
  await page.press('.chat-input', 'Shift+Enter')
  check((await page.inputValue('.chat-input')).includes('\n') && (await page.evaluate(() => window.__chat.runner.composed.length)) === 1, 'Shift+Return is a new line, not a send')

  // A message that fails stays in the box with a sentence, not a Rust word.
  await page.evaluate(() => (window.__chat.runner.failing = 'closed'))
  await page.fill('.chat-input', 'This one will not go')
  await page.click('.chat-send')
  await page.waitForSelector('.chat-problem:not([hidden])')
  check((await page.inputValue('.chat-input')) === 'This one will not go', 'a message that failed is still in the box')
  check(!/closed/.test(await page.textContent('.chat-problem')) && /studio/.test(await page.textContent('.chat-problem')), 'and the page says why in its own words')
  await page.evaluate(() => (window.__chat.runner.failing = null))
  await page.fill('.chat-input', '')

  // Images: attach, paste and drop wait beside the box; Send sends them with the words.
  const png = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==', 'base64')
  const chooser = page.waitForEvent('filechooser')
  await page.click('.chat-attach')
  await (await chooser).setFiles([{ name: 'pixel.png', mimeType: 'image/png', buffer: png }])
  await page.waitForSelector('.chat-thumb')
  check((await count('.chat-thumb')) === 1 && !(await page.locator('.chat-send').isDisabled()), 'an attached image waits beside the box, and Send is on with no words')
  check((await page.locator('.chat-thumb img').getAttribute('alt')) === 'pixel.png', 'the thumbnail is named for the screen reader')
  await page.evaluate(
    async b64 => {
      const bytes = Uint8Array.from(atob(b64), c => c.charCodeAt(0))
      const box = document.querySelector('.chat-input')
      const paste = new DataTransfer()
      paste.items.add(new File([bytes], 'pasted.png', { type: 'image/png' }))
      box.dispatchEvent(new ClipboardEvent('paste', { clipboardData: paste, bubbles: true, cancelable: true }))
      const drop = new DataTransfer()
      drop.items.add(new File([bytes], 'dropped.png', { type: 'image/png' }))
      const host = document.querySelector('.chat-host')
      host.dispatchEvent(new DragEvent('dragover', { dataTransfer: drop, bubbles: true, cancelable: true }))
      window.__dropping = host.dataset.dropping
      host.dispatchEvent(new DragEvent('drop', { dataTransfer: drop, bubbles: true, cancelable: true }))
    },
    png.toString('base64'),
  )
  await page.waitForFunction(() => document.querySelectorAll('.chat-thumb').length === 3)
  check((await page.evaluate(() => window.__dropping)) === 'true', 'a file dragged over the conversation shows the drop target')
  check((await page.evaluate(() => document.querySelector('.chat-host').dataset.dropping)) === undefined, 'and the target goes when it is dropped')
  await page.click('.chat-thumb-remove >> nth=1')
  check((await count('.chat-thumb')) === 2, 'an image can be taken out before it is sent')
  await page.fill('.chat-input', 'What is in these?')
  await page.press('.chat-input', 'Enter')
  await page.waitForFunction(() => window.__chat.runner.composedImages.length === 1)
  const sentImages = await page.evaluate(() => window.__chat.runner.composedImages[0])
  check(JSON.stringify(sentImages) === JSON.stringify([`image/png:${png.length}`, `image/png:${png.length}`]), 'both images went with the words as one compose')
  check((await count('.chat-thumb')) === 0 && (await page.inputValue('.chat-input')) === '', 'the thumbnails and the box empty once it is taken')
  check((await page.evaluate(() => window.__chat.runner.composed.at(-1))) === 'What is in these?', 'the words went with them')

  // Pasted text is left to the box.
  await page.evaluate(() => {
    const t = new DataTransfer()
    t.setData('text/plain', 'just words')
    const e = new ClipboardEvent('paste', { clipboardData: t, bubbles: true, cancelable: true })
    window.__textPasteHeld = !document.querySelector('.chat-input').dispatchEvent(e)
  })
  check((await page.evaluate(() => window.__textPasteHeld)) === false, 'pasting text is left to the box')

  // Past the runner's limit: refused with the apps' sentence, nothing waits.
  await page.evaluate(() => (window.__chat.runner.limits = { images: true, each: 50, total: 80 }))
  const recount = await page.evaluate(() => window.__chat.runner.composedImages.length)
  const big = page.waitForEvent('filechooser')
  await page.click('.chat-attach')
  await (await big).setFiles([{ name: 'big.png', mimeType: 'image/png', buffer: Buffer.alloc(200, 1) }])
  await page.waitForSelector('.chat-problem:not([hidden])')
  check((await page.textContent('.chat-problem')) === 'That image is too large to send. Use a smaller one.', 'an image past the runner’s limit is refused with the apps’ sentence')
  check((await count('.chat-thumb')) === 0 && (await page.evaluate(() => window.__chat.runner.composedImages.length)) === recount, 'and nothing waits or goes')
  const nonImage = page.waitForEvent('filechooser')
  await page.click('.chat-attach')
  await (await nonImage).setFiles([{ name: 'notes.txt', mimeType: 'text/plain', buffer: Buffer.from('hello') }])
  await page.waitForFunction(() => document.querySelector('.chat-problem').textContent.includes('Only PNG'))
  check((await count('.chat-thumb')) === 0, 'a file that is not an image is left out, with a sentence')
  await page.evaluate(() => (window.__chat.runner.limits = { images: true, each: 1_000_000, total: 3_000_000 }))

  // A runner that refuses an image for size says so in the apps' words and keeps the box.
  const refused = page.waitForEvent('filechooser')
  await page.click('.chat-attach')
  await (await refused).setFiles([{ name: 'pixel.png', mimeType: 'image/png', buffer: png }])
  await page.waitForSelector('.chat-thumb')
  await page.evaluate(() => Object.assign(window.__chat.runner, { failing: 'refused', failingWhat: 'images_too_large' }))
  await page.fill('.chat-input', 'Will not go')
  await page.click('.chat-send')
  await page.waitForFunction(() => document.querySelector('.chat-problem').textContent.includes('too large to send together'))
  check((await count('.chat-thumb')) === 1 && (await page.inputValue('.chat-input')) === 'Will not go', 'an image the runner refused stays beside the box, with the words')
  await page.evaluate(() => Object.assign(window.__chat.runner, { failing: null, failingWhat: '' }))
  await page.click('.chat-thumb-remove')
  await page.fill('.chat-input', '')

  // Acceptance 3: Stop is one interrupt, and the turn ends.
  await page.click('.chat-stop')
  await page.waitForSelector('.chat-working', { state: 'hidden' })
  check((await page.evaluate(() => window.__chat.runner.interrupts)) === 1, 'Stop asks the runner to interrupt, once')
  check((await page.locator('.turn-end', { hasText: 'Interrupted' }).count()) >= 1, 'and the turn is shown interrupted')

  check((await page.evaluate(() => window.__violations ?? [])).length === 0, 'no CSP violation')
  await context.close()
}

// Phone width: nothing overflows sideways.
{
  const { context, page } = await open(phone)
  const wide = await page.evaluate(() => document.documentElement.scrollWidth > document.documentElement.clientWidth || document.querySelector('.chat-scroll').scrollWidth > document.querySelector('.chat-scroll').clientWidth)
  check(!wide, 'no sideways scroll at phone width')
  check(await page.locator('.chat-input').isVisible(), 'the box is on screen on a phone')
  await context.close()
}

if (shots) {
  for (const [name, options] of [
    ['desktop-light', desktop],
    ['desktop-dark', { ...desktop, colorScheme: 'dark' }],
    ['phone-dark', phone],
    ['phone-light', { ...phone, colorScheme: 'light' }],
  ]) {
    const { context, page } = await open(options)
    await page.waitForTimeout(300)
    await page.screenshot({ path: join(shots, `chat-${name}.png`) })
    // The tool run opened.
    await page.locator('.disclosure > summary', { hasText: 'Used 3 tools' }).click()
    await page.waitForTimeout(100)
    await page.screenshot({ path: join(shots, `chat-${name}-open.png`) })
    await context.close()
  }
}

await browser.close()
server.close()
for (const v of violations) failures.push(v)
if (failures.length) {
  console.error(`chat-check: ${failures.length} failed`)
  process.exit(1)
}
console.log('chat-check: all passed')
