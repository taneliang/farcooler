#!/usr/bin/env node
// Captures the Plan as the work screen draws it (ov-775): 1440 px wide and a 390 px phone, each in light and dark.
//
//   node test/browser/plan-capture.mjs OUT_DIR
//
// The page is src/client/index.html with app.css and the real view code over
// the plan test/browser/plan-harness.ts writes (ov-867's C10f scenario); one
// phone shot saves a Next line and has it refused as a member's is, to show
// that sentence. Chromium comes
// from playwright-core (PLAYWRIGHT_BROWSERS_PATH).

import { mkdirSync, readFileSync } from 'node:fs'
import { join, resolve } from 'node:path'

import { build } from 'esbuild'
import { chromium } from 'playwright-core'

const out = resolve(process.argv[2] ?? 'plan-shots')
mkdirSync(out, { recursive: true })
const bundle = await build({ entryPoints: ['test/browser/plan-harness.ts'], bundle: true, write: false, format: 'esm', target: 'es2022', logLevel: 'error' })
const css = readFileSync('src/client/app.css', 'utf8')
const html = readFileSync('src/client/index.html', 'utf8')
  .replace('<link rel="stylesheet" href="{{app.css}}">', `<style>${css}</style>`)
  .replace('<script type="module" src="{{app.js}}"></script>', '')
  .replace('</body>', `<script type="module">${bundle.outputFiles[0].text}</script></body>`)

const browser = await chromium.launch({ executablePath: process.env.CHROME_PATH })
async function shoot(name, { colorScheme, metrics, refuse = false, full = true }) {
  const context = await browser.newContext({ colorScheme, viewport: { width: metrics.width, height: metrics.height }, deviceScaleFactor: metrics.mobile ? 3 : 2, hasTouch: metrics.mobile, isMobile: metrics.mobile })
  const page = await context.newPage()
  await page.setContent(refuse ? html.replace('<head>', '<head><script>window.refuse = true</script>') : html)
  await page.waitForFunction(() => window.done === true)
  console.log(name, 'title:', await page.title(), 'overflow-x:', await page.evaluate(() => document.documentElement.scrollWidth > innerWidth))
  // The plan scrolls inside the work screen: grow the page to its height.
  if (full) {
    const h = await page.evaluate(() => document.getElementById('plan').scrollHeight + 60)
    await page.setViewportSize({ width: metrics.width, height: Math.max(metrics.height, h) })
  }
  await page.screenshot({ path: join(out, `${name}.png`), fullPage: false })
  await context.close()
}
for (const scheme of ['light', 'dark']) {
  await shoot(`plan-1440-${scheme}`, { colorScheme: scheme, metrics: { width: 1440, height: 900, mobile: false } })
  await shoot(`plan-390-${scheme}`, { colorScheme: scheme, metrics: { width: 390, height: 844, mobile: true } })
}
await shoot('plan-390-light-refused', { colorScheme: 'light', metrics: { width: 390, height: 844, mobile: true }, refuse: true, full: false })
await browser.close()
