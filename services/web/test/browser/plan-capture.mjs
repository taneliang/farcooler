#!/usr/bin/env node
// Captures the Plan as the work screen draws it (ov-775): desktop in light,
// and a phone in dark through CDP mobile emulation (headless Chrome's
// --window-size can't go below about 500 px).
//
//   node test/browser/plan-capture.mjs OUT_DIR
//
// The page is src/client/index.html with app.css and the real view code over
// test/fixtures/plan.json with a few long rows; on the phone a Next line is
// saved and refused as a member's is, to show that sentence. Chromium comes
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
async function shoot(name, { colorScheme, metrics, refuse = false }) {
  const context = await browser.newContext({ colorScheme, viewport: metrics.mobile ? { width: metrics.width, height: metrics.height } : { width: metrics.width, height: metrics.height } })
  const page = await context.newPage()
  if (metrics.mobile) {
    const cdp = await context.newCDPSession(page)
    await cdp.send('Emulation.setDeviceMetricsOverride', { width: metrics.width, height: metrics.height, deviceScaleFactor: 3, mobile: true })
    await cdp.send('Emulation.setTouchEmulationEnabled', { enabled: true })
  }
  await page.setContent(refuse ? html.replace('<head>', '<head><script>window.refuse = true</script>') : html)
  await page.waitForFunction(() => window.done === true)
  console.log(name, 'title:', await page.title())
  await page.screenshot({ path: join(out, `${name}.png`), fullPage: false })
  await context.close()
}
await shoot('plan-desktop-light', { colorScheme: 'light', metrics: { width: 1280, height: 800, mobile: false } })
await shoot('plan-phone-dark', { colorScheme: 'dark', metrics: { width: 390, height: 844, mobile: true }, refuse: true })
await browser.close()
