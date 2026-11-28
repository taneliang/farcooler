#!/usr/bin/env node
// The message box through the page's two-pane layout, in Chromium (ov-1495).
// A pane that comes back (a split closed and reopened, another terminal and
// back) or changes width must leave its box usable: one whole line at least,
// its placeholder showing, focus on a click in the middle of the field, and
// what is typed there sent.
//
//   WEB_WASM_DIR=<built wasm directory> node test/browser/split-check.mjs [screenshot dir]
//
// Chromium comes from playwright-core (CHROME_PATH overrides it).

import assert from 'node:assert/strict'
import { mkdirSync, readFileSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { build } from 'esbuild'
import { chromium } from 'playwright-core'
import { harnessHtml, styles } from '../../scripts/styles.mjs'

const out = process.argv[2]
if (out) mkdirSync(out, { recursive: true })
const wasmDir = process.env.WEB_WASM_DIR
if (!wasmDir) throw new Error('Set WEB_WASM_DIR to the built crates/web wasm directory')
const wasm = readFileSync(join(wasmDir, 'farcooler_web.wasm'))
const bundle = await build({ alias: { 'fc-web': resolve(wasmDir, 'farcooler_web.js') }, entryPoints: ['test/browser/split-harness.ts'], bundle: true, write: false, format: 'esm', target: 'es2022', logLevel: 'error' })
const html = harnessHtml().replaceAll('{{app.css}}', '/app.css').replaceAll('{{app.js}}', '/harness.js')
const css = styles().toString()
const origin = 'https://split.test'
const browser = await chromium.launch({ executablePath: process.env.CHROME_PATH })
const failures = []

async function test(name, run, { width = 1280 } = {}) {
  const context = await browser.newContext({ viewport: { width, height: 800 }, colorScheme: 'dark' })
  await context.route(origin + '/**', route => {
    const path = new URL(route.request().url()).pathname
    if (path === '/harness.js') return route.fulfill({ contentType: 'text/javascript', body: bundle.outputFiles[0].text })
    if (path === '/farcooler_web.wasm') return route.fulfill({ contentType: 'application/wasm', body: wasm })
    if (path === '/app.css') return route.fulfill({ contentType: 'text/css', body: css })
    return route.fulfill({ contentType: 'text/html', body: html })
  })
  const page = await context.newPage()
  const errors = []
  page.on('pageerror', e => errors.push(e.message))
  page.setDefaultTimeout(5000)
  try {
    await page.goto(origin + '/')
    await page.locator('#terminal .chat-list .bubble').waitFor()
    await run(page)
    assert.deepEqual(errors, [], 'the page threw')
    console.log('ok  ', name)
  } catch (e) {
    failures.push(name)
    console.error('FAIL', name, '-', e.message)
  } finally {
    await context.close()
  }
}

const box = (page, stage) => page.locator(`#${stage} .workspace-screen:not([hidden]) .chat-input`)
const frame = page => page.evaluate(() => new Promise(done => requestAnimationFrame(() => requestAnimationFrame(done))))
const measure = input => input.evaluate(el => {
  const style = getComputedStyle(el)
  const line = parseFloat(style.lineHeight) + parseFloat(style.paddingTop) + parseFloat(style.paddingBottom)
  return { height: el.getBoundingClientRect().height, line, scroll: el.scrollHeight, client: el.clientHeight, placeholder: el.placeholder }
})

async function shot(page, stage, name) {
  if (!out) return
  await box(page, stage).scrollIntoViewIfNeeded()
  await page.screenshot({ path: join(out, `${name}.png`) })
}

/// The box a person meets: whole, focused by a click in the middle of the
/// field, and sending what they type there.
async function usable(page, stage, terminal, words) {
  const input = box(page, stage)
  await input.waitFor({ state: 'attached' })
  await frame(page)
  const empty = await measure(input)
  assert.equal(empty.placeholder, 'Message')
  assert.ok(empty.height >= empty.line - 0.5, `the box is ${empty.height}px tall; one line is ${empty.line}px`)
  await page.evaluate(() => document.activeElement?.blur())
  // At phone width the page's header can leave the foot below the fold.
  await input.scrollIntoViewIfNeeded()
  const field = await input.locator('..').boundingBox()
  const own = await input.boundingBox()
  const at = { x: own.x + own.width / 2, y: field.y + field.height / 2 }
  await page.mouse.click(at.x, at.y)
  const hit = await page.evaluate(({ x, y }) => { const el = document.elementFromPoint(x, y); return `${el?.tagName}.${el?.className}` }, at)
  assert.ok(await input.evaluate(el => document.activeElement === el), `a click in the middle of the field did not focus the box; it hit ${hit}`)
  await page.keyboard.type(words)
  assert.equal(await input.inputValue(), words)
  await page.keyboard.press('Enter')
  await page.waitForFunction(([t, w]) => window.__split.composed.some(c => c[0] === t && c[1] === w), [terminal, words])
  await page.waitForFunction(el => el.value === '', await input.elementHandle())
}

for (const width of [1280, 320]) {
  await test(`a split closed and reopened keeps its message box usable at ${width}`, async page => {
    // Another agent in the main pane, so closing the split puts the
    // orchestrator's pane away rather than moving it across.
    await page.evaluate(() => window.__split.run('orchestrator'))
    await page.evaluate(() => window.__split.open('fix', 'main'))
    await usable(page, 'side-stage', 'seat', 'Before closing the split.')
    await page.locator('#close-split').click()
    await page.evaluate(() => window.__split.run('orchestrator'))
    await frame(page)
    await shot(page, 'side-stage', `reopened-${width}`)
    await usable(page, 'side-stage', 'seat', 'After reopening the split.')
  }, { width })

  await test(`another terminal and back keeps the message box usable at ${width}`, async page => {
    await page.evaluate(() => window.__split.open('fix', 'main'))
    await usable(page, 'terminal', 'fix', 'To the second agent.')
    await page.evaluate(() => window.__split.open('seat', 'main'))
    await frame(page)
    await shot(page, 'terminal', `switched-back-${width}`)
    await usable(page, 'terminal', 'seat', 'Back on the first agent.')
  }, { width })
}

await test('a draft still fits after the window narrows and widens, split open and reopened', async page => {
  await page.evaluate(() => window.__split.run('orchestrator'))
  const input = box(page, 'side-stage')
  const draft = 'Check the release logs, reconcile the deployment manifests, and say which of the two is out of date before changing either.'
  await input.fill(draft)
  const fits = async at => {
    await frame(page)
    const m = await measure(input)
    assert.ok(m.height >= m.line - 0.5 && m.scroll <= m.client + 1, `${at}: the box shows ${m.client}px of ${m.scroll}px`)
    assert.ok(m.client <= m.scroll + 1, `${at}: the box is ${m.client}px for ${m.scroll}px of words`)
  }
  await fits('1280')
  const wide = (await measure(input)).height
  await page.setViewportSize({ width: 320, height: 800 })
  await fits('narrowed to 320')
  assert.ok((await measure(input)).height > wide, 'the draft did not wrap when narrowed')
  await page.locator('#close-split').click()
  await page.setViewportSize({ width: 1280, height: 800 })
  await page.evaluate(() => window.__split.run('orchestrator'))
  await fits('reopened at 1280')
})

await test('a reply that lands while its pane is away leaves the box usable', async page => {
  await page.evaluate(() => window.__split.open('fix', 'main'))
  await page.evaluate(() => window.__split.run('orchestrator'))
  const input = box(page, 'side-stage')
  await input.fill('Sent just before closing.')
  await page.evaluate(() => window.__split.hold())
  await input.press('Enter')
  await page.locator('#close-split').click()
  await page.evaluate(() => window.__land())
  await frame(page)
  await page.evaluate(() => window.__split.run('orchestrator'))
  await usable(page, 'side-stage', 'seat', 'And one after.')
})

await browser.close()
if (failures.length) {
  console.error(`split-check: ${failures.length} failed`)
  process.exit(1)
}
console.log('split-check: all passed')
