#!/usr/bin/env node
// What an agent sends, in the conversation view (ov-1509), in headless
// Chromium under the shell's image policy (blob: and data: only), against a
// scripted runner that hands over real PNG bytes.
//
//   WEB_WASM_DIR=<built wasm directory> node test/browser/chat-files-check.mjs [--screenshots DIR]
//
// It checks:
// - an image sent with SendUserFile is drawn inline, inside the column, out of
//   any folded tool run, and opens full size;
// - another file is a row with its name and size, and Download saves the
//   runner's bytes under that name;
// - a large image waits behind a button that says its size;
// - a fetch that fails says so, and Try Again fetches again;
// - a reply's Markdown draws an image only where the runner says it was sent;
//   a remote one stays alt text and nothing leaves the origin;
// - a prompt's own images are drawn on the person's message;
// - twenty images: only those near the screen are fetched, three at a time,
//   and the page keeps up (the measures are printed);
// - 320 and 1280 wide: no sideways scroll, no CSP violation; at 320 the chat
//   has the screen's width, an image opens by tap and a file downloads by tap.

import { createServer } from 'node:http'
import { mkdirSync, readFileSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { deflateSync, crc32 } from 'node:zlib'

import { build } from 'esbuild'
import { chromium } from 'playwright-core'
import { harnessHtml, styles } from '../../scripts/styles.mjs'

const args = process.argv.slice(2)
const shots = args.includes('--screenshots') ? args[args.indexOf('--screenshots') + 1] : null
if (shots) mkdirSync(shots, { recursive: true })

/// A real PNG, `width` by `height`: a board of colored cards on a dark ground,
/// so a capture shows whether it is drawn whole and at its own shape.
function png(width, height, hue = 0) {
  const raw = Buffer.alloc((width * 3 + 1) * height)
  const colors = [[10, 132, 255], [50, 215, 75], [255, 159, 10], [191, 90, 242]]
  for (let y = 0; y < height; y++) {
    for (let x = 0; x < width; x++) {
      const column = Math.floor((x / width) * 4)
      const inCard = (x % (width / 4)) > width / 80 && (y % (height / 5)) > height / 40 && y > height / 8
      const [r, g, b] = inCard ? colors[(column + hue) % 4] : [28, 28, 30]
      raw.set([r, g, b], y * (width * 3 + 1) + 1 + x * 3)
    }
  }
  const chunk = (type, data) => {
    const body = Buffer.concat([Buffer.from(type), data])
    const out = Buffer.alloc(data.length + 12)
    out.writeUInt32BE(data.length, 0)
    body.copy(out, 4)
    out.writeUInt32BE(crc32(body), data.length + 8)
    return out
  }
  const head = Buffer.alloc(13)
  head.writeUInt32BE(width, 0)
  head.writeUInt32BE(height, 4)
  head.set([8, 2, 0, 0, 0], 8)
  return Buffer.concat([Buffer.from('89504e470d0a1a0a', 'hex'), chunk('IHDR', head), chunk('IDAT', deflateSync(raw)), chunk('IEND', Buffer.alloc(0))])
}

const wasmDir = process.env.WEB_WASM_DIR
if (!wasmDir) throw new Error('Set WEB_WASM_DIR to the built crates/web wasm directory')
const wasm = readFileSync(join(wasmDir, 'farcooler_web.wasm'))
const bundle = await build({ alias: { 'fc-web': resolve(wasmDir, 'farcooler_web.js') }, entryPoints: ['test/browser/chat-files-harness.ts'], bundle: true, format: 'esm', target: 'es2022', jsx: 'automatic', write: false })
const css = styles().toString()
const html = harnessHtml().replace('{{app.css}}', '/app.css').replace('{{app.js}}', '/app.js')
const files = { '/': [html, 'text/html'], '/app.js': [bundle.outputFiles[0].text, 'text/javascript'], '/app.css': [css, 'text/css'], '/farcooler_web.wasm': [wasm, 'application/wasm'] }
// img-src as the shell's own policy has it (src/headers.ts), and no more.
const csp = "default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; img-src 'self' data: blob:; connect-src 'self'; require-trusted-types-for 'script'; trusted-types fc"
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
const problems = []
const elsewhere = []

async function open(options) {
  const context = await browser.newContext({ acceptDownloads: true, ...options })
  const page = await context.newPage()
  page.setDefaultTimeout(5000)
  await page.addInitScript(() => document.addEventListener('securitypolicyviolation', e => window.__violations = [...(window.__violations ?? []), `${e.effectiveDirective} ${e.blockedURI}`]))
  page.on('pageerror', e => problems.push(`pageerror ${e.message}`))
  page.on('request', r => { if (!r.url().startsWith(origin) && !r.url().startsWith(`blob:${origin}`)) elsewhere.push(r.url()) })
  await page.goto(origin)
  await page.waitForSelector('.chat-list .bubble')
  return { context, page }
}

const CAPTURE = png(1600, 1000)
const REPORT = Buffer.from('%PDF-1.4\nthe report\n')
const PHONE = png(390, 844, 2)

const file = (name, bytes, extra = {}) => ({
  name, mime: name.endsWith('.png') ? 'image/png' : 'application/pdf', size: bytes, size_words: bytes >= 1e6 ? `${(bytes / 1e6).toFixed(1)} MB` : `${Math.round(bytes / 1000)} KB`,
  image: name.endsWith('.png'), auto: name.endsWith('.png') && bytes <= 4e6, fetchable: bytes <= 32e6, ...extra,
})
const sendRow = (id, sent) => ({ id, turn: 'turn:2', provisional: false, kind: { Tool: { name: 'SendUserFile', status: 'Done', summary: '', diff: [], input: 'files: …', sent } } })

/// Hands the scripted runner `entries`, each `[row#index, base64]`.
const hold = (page, entries) => page.evaluate(entries => {
  for (const [key, text] of entries) window.__chat.runner.bytes.set(key, Uint8Array.from(atob(text), c => c.charCodeAt(0)))
}, entries.map(([key, bytes]) => [key, bytes.toString('base64')]))
const put = (page, row) => page.evaluate(row => window.__chat.runner.put(row), row)
const fetched = page => page.evaluate(() => window.__chat.runner.fetched)
const drawn = (page, locator) => locator.evaluate(async img => { await img.decode(); return { natural: img.naturalWidth, width: img.getBoundingClientRect().width, src: img.src } })
const noSidewaysScroll = page => page.evaluate(() => {
  const scroller = document.querySelector('.chat-scroll')
  return document.documentElement.scrollWidth <= innerWidth && scroller.scrollWidth <= scroller.clientWidth
})
/// Every picture in the transcript is inside the column it is drawn in.
const inColumn = page => page.evaluate(() => {
  const column = document.querySelector('.chat-list').getBoundingClientRect()
  return [...document.querySelectorAll('.chat-list img')].every(img => { const r = img.getBoundingClientRect(); return r.left >= column.left - 0.5 && r.right <= column.right + 0.5 })
})

async function sends(page) {
  await hold(page, [['tool:send#0', CAPTURE], ['tool:send#1', REPORT], ['tool:send#2', PHONE]])
  await put(page, sendRow('tool:send', { caption: 'The board at 1280, the report, and the phone width.', files: [
    file('board-1280.png', CAPTURE.length), file('design-review-of-the-board-with-a-very-long-name-that-must-wrap.pdf', 2_400_000), file('phone-390.png', 7_200_000),
  ] }))
}

const run = async (what, body) => {
  try { await body() } catch (e) { check(false, `${what}: ${e.message.split('\n')[0]}`) }
}

const desktop = { viewport: { width: 1280, height: 800 }, colorScheme: 'light' }
const narrow = { viewport: { width: 320, height: 700 }, deviceScaleFactor: 2, isMobile: true, hasTouch: true, colorScheme: 'dark' }

await run('an image the agent sends', async () => {
  const { context, page } = await open(desktop)
  await sends(page)
  const image = page.getByRole('img', { name: 'board-1280.png', exact: true })
  await image.waitFor()
  const shown = await drawn(page, image)
  check(shown.natural === 1600 && shown.width > 300 && shown.width <= 760 && shown.src.startsWith('blob:'), `a sent image is drawn inline, sized to the column (${Math.round(shown.width)} px of 1600, from a blob:)`)
  check(await image.isVisible() && await image.evaluate(img => !img.closest('details')), 'it is not folded into a run of tool calls')
  check(await page.getByText('The board at 1280, the report, and the phone width.', { exact: true }).isVisible(), 'its caption is shown')
  check(await inColumn(page) && await noSidewaysScroll(page), 'nothing leaves the column at 1280')
  if (shots) { await image.scrollIntoViewIfNeeded(); await page.screenshot({ path: join(shots, 'sent-1280.png') }) }

  await page.getByRole('button', { name: 'Open board-1280.png at full size', exact: true }).click()
  const viewer = page.getByRole('dialog', { name: 'board-1280.png', exact: true })
  await viewer.waitFor()
  await viewer.getByRole('button', { name: 'Actual Size', exact: true }).click()
  const full = await drawn(page, viewer.locator('img'))
  check(Math.round(full.width) === 1600, `a click opens it, and Actual Size draws all ${Math.round(full.width)} px`)
  if (shots) await page.screenshot({ path: join(shots, 'viewer-1280.png') })
  await viewer.getByRole('button', { name: 'Close', exact: true }).click()
  await page.waitForFunction(() => !document.querySelector('.chat-viewer'))
  await page.getByRole('button', { name: 'Open board-1280.png at full size', exact: true }).click()
  await page.keyboard.press('Escape')
  await page.waitForFunction(() => !document.querySelector('.chat-viewer'))
  check(true, 'Close and Escape both put it away')

  const report = page.locator('.chat-file', { hasText: 'very-long-name' })
  check(await report.getByText('2.4 MB', { exact: true }).isVisible(), 'another file is a row with its name and size')
  check(!(await fetched(page)).includes('file tool:send#1'), 'and is not fetched until asked for')
  const [download] = await Promise.all([page.waitForEvent('download'), report.getByRole('button', { name: 'Download', exact: true }).click()])
  const saved = readFileSync(await download.path())
  check(download.suggestedFilename() === 'design-review-of-the-board-with-a-very-long-name-that-must-wrap.pdf' && saved.equals(REPORT), 'Download saves the runner’s bytes under that name')

  const load = page.getByRole('button', { name: 'Load Image (7.2 MB)', exact: true })
  await load.scrollIntoViewIfNeeded()
  await page.waitForTimeout(200)
  check(await load.isVisible() && !(await fetched(page)).includes('file tool:send#2'), 'a large image waits, saying its size')
  await load.click()
  await page.getByRole('img', { name: 'phone-390.png', exact: true }).waitFor()
  check((await drawn(page, page.getByRole('img', { name: 'phone-390.png', exact: true }))).natural === 390, 'and is drawn once clicked')

  // A redraw of the row fetches nothing again.
  const before = (await fetched(page)).length
  await sends(page)
  await page.waitForTimeout(300)
  check((await fetched(page)).length === before && await page.getByRole('img', { name: 'board-1280.png', exact: true }).isVisible(), 'a row drawn again fetches nothing again')

  await put(page, sendRow('tool:gone', { files: [file('gone.png', 1000), file('archive.zip', 40_000_000, { fetchable: false, mime: 'application/zip' })] }))
  const failed = page.getByText('Couldn’t load this file. It may have moved or changed since it was sent.', { exact: true })
  await failed.waitFor()
  check(true, 'a file that can’t be read says so')
  await hold(page, [['tool:gone#0', png(40, 30)]])
  await page.getByRole('button', { name: 'Try Again', exact: true }).click()
  await page.getByRole('img', { name: 'gone.png', exact: true }).waitFor()
  check(true, 'Try Again fetches it')
  const zip = page.locator('.chat-file', { hasText: 'archive.zip' })
  check(await zip.getByText('Too large to download here', { exact: false }).isVisible() && !(await zip.getByRole('button').count()), 'a file past the cap offers no download')
  check(await page.evaluate(() => !window.__violations), 'no CSP violation with files')
  await context.close()
})

await run('images in Markdown and on a prompt', async () => {
  const { context, page } = await open(desktop)
  await hold(page, [['tool:send#0', CAPTURE], ['turn:pics#0', PHONE], ['turn:pics#1', CAPTURE]])
  await put(page, { id: 'p:md', turn: 'turn:2', provisional: false, kind: { Prose: {
    text: 'Here it is: ![the board](</work/shots/board 1.png>)\n\nNot sent: ![a key](/home/me/.ssh/key.png) and ![a tracker](https://tracker.example.com/pixel.png)', conclusion: false,
    images: [{ src: '/work/shots/board 1.png', row: 'tool:send', index: 0, ...file('board 1.png', CAPTURE.length) }],
  } } })
  const inline = page.getByRole('img', { name: 'the board', exact: true })
  await inline.waitFor()
  check((await drawn(page, inline)).natural === 1600, 'Markdown that names a sent image draws it')
  const prose = page.locator('.prose', { hasText: 'Not sent' })
  check((await prose.locator('img').count()) === 1 && (await prose.textContent()).includes('a key') && (await prose.textContent()).includes('a tracker'), 'any other image stays alt text')
  check(!(await fetched(page)).some(f => f.includes('.ssh')), 'and is never asked for')
  check(await inColumn(page), 'the inline image is inside the column')

  await put(page, { id: 'turn:pics', turn: null, provisional: false, kind: { Turn: { prompt: 'What is wrong here?', origin: 'Typed', outcome: null, activity: 'Idle', background_running: 0, images: [{ mime: 'image/png' }, { mime: 'image/png' }] } } })
  const mine = page.getByRole('img', { name: 'Attached image 2', exact: true })
  await mine.waitFor()
  check((await drawn(page, mine)).natural === 1600 && (await page.getByRole('img', { name: 'Attached image 1', exact: true }).count()) === 1, 'a prompt’s images are drawn on the person’s message')
  check((await fetched(page)).includes('image turn:pics#1'), 'through agent.image')
  if (shots) await page.screenshot({ path: join(shots, 'markdown-and-prompt-1280.png') })
  check(await page.evaluate(() => !window.__violations), 'no CSP violation with Markdown images')
  await context.close()
})

await run('at 320 wide', async () => {
  const { context, page } = await open(narrow)
  await sends(page)
  const image = page.getByRole('img', { name: 'board-1280.png', exact: true })
  await image.waitFor()
  const shown = await drawn(page, image)
  // The shell stacks at this width, so the column is the screen less its padding.
  check(shown.width >= 320 - 48 && shown.width <= 320 - 24 && await inColumn(page) && await noSidewaysScroll(page), `at 320 the image fills the column (${Math.round(shown.width)} px) with no sideways scroll`)
  if (shots) { await image.scrollIntoViewIfNeeded(); await page.screenshot({ path: join(shots, 'sent-320.png') }) }
  await page.locator('.chat-file').first().scrollIntoViewIfNeeded()
  if (shots) await page.screenshot({ path: join(shots, 'file-row-320.png') })
  await image.tap()
  const viewer = page.getByRole('dialog', { name: 'board-1280.png', exact: true })
  await viewer.waitFor()
  const box = await viewer.boundingBox()
  check(box.x >= 0 && box.x + box.width <= 320 && await viewer.getByRole('button', { name: 'Close', exact: true }).isVisible(), 'the full-size view fits a 320 screen')
  if (shots) await page.screenshot({ path: join(shots, 'viewer-320.png') })
  await viewer.getByRole('button', { name: 'Close', exact: true }).tap()
  await page.waitForFunction(() => !document.querySelector('.chat-viewer'))
  const save = page.locator('.chat-file', { hasText: 'very-long-name' }).getByRole('button', { name: 'Download', exact: true })
  const [download] = await Promise.all([page.waitForEvent('download'), save.tap()])
  check(readFileSync(await download.path()).equals(REPORT) && await noSidewaysScroll(page), 'at 320 a tap on Download saves the file')
  await context.close()
})

await run('twenty images', async () => {
  const { context, page } = await open(desktop)
  const pictures = Array.from({ length: 20 }, (_, i) => png(1600, 1000, i))
  await hold(page, pictures.map((bytes, i) => [`tool:many${i}#0`, bytes]))
  await page.evaluate(() => {
    window.__chat.runner.fetchDelay = 40
    window.__long = []
    new PerformanceObserver(list => window.__long.push(...list.getEntries().map(e => e.duration))).observe({ type: 'longtask' })
  })
  const started = Date.now()
  await page.evaluate(rows => { for (const row of rows) window.__chat.runner.put(row) },
    pictures.map((bytes, i) => sendRow(`tool:many${i}`, { files: [file(`capture-${i}.png`, bytes.length)] })))
  await page.getByRole('img', { name: 'capture-19.png', exact: true }).waitFor()
  const newest = Date.now() - started
  await page.waitForTimeout(800)
  const early = (await fetched(page)).length
  check(early > 0 && early < 20, `at the bottom of twenty images only those near the screen are fetched (${early} of 20)`)

  // A reply streaming below them: each change is on screen on time.
  const frames = await page.evaluate(async () => {
    const list = document.querySelector('.chat-list')
    const times = []
    for (let i = 0; i < 40; i++) {
      const from = performance.now()
      window.__chat.runner.put({ id: 'p:stream', turn: 'turn:2', provisional: false, kind: { Prose: { text: `Streaming word ${i}.`, conclusion: false } } })
      while (!list.textContent.includes(`Streaming word ${i}.`)) await new Promise(r => setTimeout(r, 1))
      await new Promise(r => requestAnimationFrame(() => r()))
      times.push(performance.now() - from)
    }
    return times.sort((a, b) => a - b)
  })

  // Scroll up through all of them.
  const scrolled = Date.now()
  for (let i = 19; i >= 0; i--) {
    await page.locator(`.chat-image[data-name="capture-${i}.png"]`).scrollIntoViewIfNeeded()
    await page.getByRole('img', { name: `capture-${i}.png`, exact: true }).waitFor()
  }
  const walk = Date.now() - scrolled
  const asked = await fetched(page)
  const most = await page.evaluate(() => window.__chat.runner.mostAtOnce)
  const long = await page.evaluate(() => window.__long)
  check(asked.length === 20 && new Set(asked).size === 20, 'scrolling through them fetches each once')
  check(most <= 3, `never more than three at once (${most})`)
  const p95 = frames[Math.floor(frames.length * 0.95)]
  check(p95 < 100, `a reply streams under them on time (p95 ${p95.toFixed(0)} ms from change to frame)`)
  console.log(`measure: 20 images of 1600x1000 (${Math.round(pictures.reduce((n, p) => n + p.length, 0) / 1000)} KB in all), 40 ms a fetch: newest drawn in ${newest} ms; ${early} fetched at the bottom; change-to-frame median ${frames[20].toFixed(0)} ms, p95 ${p95.toFixed(0)} ms; ${walk} ms to scroll through all 20; long tasks ${long.length}, longest ${Math.round(Math.max(0, ...long))} ms`)
  if (shots) await page.screenshot({ path: join(shots, 'twenty-1280.png') })
  check(await page.evaluate(() => !window.__violations), 'no CSP violation with twenty')
  await context.close()
})

check(elsewhere.length === 0, `nothing was fetched from anywhere else ${elsewhere.join(' ')}`)
check(problems.length === 0, `no page errors ${problems.join('; ')}`)
await browser.close()
server.close()
if (failures.length) {
  console.error(`chat-files-check: ${failures.length} failed`)
  process.exit(1)
}
console.log('chat-files-check: all passed')
