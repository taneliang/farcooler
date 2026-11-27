#!/usr/bin/env node
// Captures the Plan as the work screen draws it (ov-775): 1440 px wide and a 390 px phone, each in light and dark.
//
//   node test/browser/plan-capture.mjs OUT_DIR
//
// The page is src/client/index.html with app.css and the real view code over
// the plan test/browser/plan-harness.ts writes (ov-867's C10f scenario); one
// phone shot saves a Next line and has it refused as a member's is, to show
// that sentence. The plan's view model is Rust's and wasm can't be built here
// at will: PLAN_VIEW_BIN is a program that reads a request on stdin and
// prints crates/client's `plan_view::json` answer (the capture plays each
// request the page makes through it). Chromium comes
// from playwright-core (PLAYWRIGHT_BROWSERS_PATH).

import { execFileSync } from 'node:child_process'
import { mkdirSync, readFileSync } from 'node:fs'
import { join, resolve } from 'node:path'

import { build } from 'esbuild'
import { chromium } from 'playwright-core'
import { harnessHtml, styles } from '../../scripts/styles.mjs'

const out = resolve(process.argv[2] ?? 'plan-shots')
mkdirSync(out, { recursive: true })
const bundle = await build({ entryPoints: ['test/browser/plan-harness.ts'], bundle: true, write: false, format: 'esm', target: 'es2022', logLevel: 'error' })
const css = styles().toString()
const html = harnessHtml()
  .replace('<link rel="stylesheet" href="{{app.css}}">', () => `<style>${css}</style>`)
  .replace('<script type="module" src="{{app.js}}"></script>', '')
  // A function, so the bundle's own `$&` and `$'` stay as written.
  .replace('</body>', () => `<script type="module">${bundle.outputFiles[0].text}</script></body>`)

const browser = await chromium.launch({ executablePath: process.env.CHROME_PATH })
async function shoot(name, { colorScheme, metrics, refuse = false, full = true, long = false, fixture = long ? 'long' : '' }) {
  const context = await browser.newContext({ colorScheme, viewport: { width: metrics.width, height: metrics.height }, deviceScaleFactor: metrics.mobile ? (long ? 2 : 3) : 2, hasTouch: metrics.mobile, isMobile: metrics.mobile })
  const page = await context.newPage()
  const flags = `${refuse ? 'window.refuse = true;' : ''}${fixture ? `window.fixture = "${fixture}";` : ''}`
  const load = async extra => {
    const head = `${flags}${extra}`
    await page.setContent(head ? html.replace('<head>', `<head><script>${head}</script>`) : html)
    await page.waitForFunction(() => window.done === true)
  }
  // Record the requests the page makes of the view model, answer them, and
  // draw again until it asks nothing new (Needs You's count follows from the
  // first answer, so a second request can come).
  const answers = {}
  for (let pass = 0; pass < 4; pass++) {
    await load(`window.answers = ${JSON.stringify(answers)};`)
    const fresh = (await page.evaluate(() => window.requests)).filter(r => !(r in answers))
    if (fresh.length === 0) break
    for (const request of new Set(fresh)) answers[request] = execFileSync(process.env.PLAN_VIEW_BIN, [], { input: request, maxBuffer: 64 << 20 }).toString()
  }
  console.log(name, 'title:', await page.title(), 'overflow-x:', await page.evaluate(() => document.documentElement.scrollWidth > innerWidth))
  // The plan scrolls inside the work screen: grow the page to its height.
  if (full) {
    const h = await page.evaluate(() => document.getElementById('plan').scrollHeight + 60)
    // Chromium repeats the top of a page taller than about 4000 px: a long
    // plan is shot in pieces of 3500 px, named -1, -2, ...
    const piece = 3500
    await page.setViewportSize({ width: metrics.width, height: Math.min(Math.max(metrics.height, h), piece + 100) })
    if (h > piece + 100) {
      for (let n = 0, y = 0; y < h; n++, y += piece) {
        await page.evaluate(y => ((document.getElementById('plan').scrollTop = y), window.scrollTo(0, y)), y)
        await page.screenshot({ path: join(out, `${name}-${n + 1}.png`), fullPage: false })
      }
      await context.close()
      return
    }
  }
  await page.screenshot({ path: join(out, `${name}.png`), fullPage: false })
  await context.close()
}
// `runs`: the plan with the engine's run on three tickets (ov-1040), and the same plan without them, at 320 px.
if (process.argv[3] === 'runs') {
  for (const scheme of ['light', 'dark']) {
    for (const fixture of ['runs', 'runs-before']) {
      await shoot(`plan-${fixture}-320-${scheme}`, { colorScheme: scheme, metrics: { width: 320, height: 1900, mobile: true }, fixture, full: false })
    }
  }
  await browser.close()
  process.exit(0)
}
for (const scheme of ['light', 'dark']) {
  await shoot(`plan-1440-${scheme}`, { colorScheme: scheme, metrics: { width: 1440, height: 900, mobile: false } })
  await shoot(`plan-390-${scheme}`, { colorScheme: scheme, metrics: { width: 390, height: 844, mobile: true } })
}
// The narrowest width, with the default scenario and with a long real plan
// (test/fixtures/plan-android-long.json).
for (const scheme of ['light', 'dark']) {
  await shoot(`plan-360-${scheme}`, { colorScheme: scheme, metrics: { width: 360, height: 800, mobile: true } })
  await shoot(`plan-360-long-${scheme}`, { colorScheme: scheme, metrics: { width: 360, height: 800, mobile: true }, long: true })
}
await shoot('plan-390-light-refused', { colorScheme: 'light', metrics: { width: 390, height: 844, mobile: true }, refuse: true, full: false })
await browser.close()
