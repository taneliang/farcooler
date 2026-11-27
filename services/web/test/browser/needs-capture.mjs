#!/usr/bin/env node
// Captures Needs You as the work screen draws it (ov-773): desktop and a 375 px phone, each in light and dark, through CDP mobile emulation (headless Chrome's
// --window-size can't go below about 500 px).
//
//   node test/browser/needs-capture.mjs OUT_DIR
//
// The page is src/client/index.html with app.css and the real view code over
// test/fixtures/needs-you.json's studio list; the first ask's Allow is
// refused as a Read-scoped device's is, to show that sentence. Chromium comes
// from playwright-core (PLAYWRIGHT_BROWSERS_PATH). WEB_WASM_DIR must contain
// the real farcooler_web.js and farcooler_web.wasm from crates/web/build.sh.

import { mkdirSync, readFileSync } from 'node:fs'
import { join, resolve } from 'node:path'

import { build } from 'esbuild'
import { chromium } from 'playwright-core'
import { harnessHtml, styles } from '../../scripts/styles.mjs'

const out = resolve(process.argv[2] ?? 'needs-shots')
mkdirSync(out, { recursive: true })
const wasmDir = process.env.WEB_WASM_DIR
if (!wasmDir) throw new Error('Set WEB_WASM_DIR to the built crates/web wasm directory')
const wasm = readFileSync(join(wasmDir, 'farcooler_web.wasm'))
const bundle = await build({ alias: { 'fc-web': resolve(wasmDir, 'farcooler_web.js') }, define: { __NEEDS_WASM__: JSON.stringify(`data:application/wasm;base64,${wasm.toString('base64')}`) }, entryPoints: ['test/browser/needs-harness.ts'], bundle: true, write: false, format: 'esm', target: 'es2022', logLevel: 'error' })
const css = styles().toString()
const html = harnessHtml()
  .replace('<link rel="stylesheet" href="{{app.css}}">', () => `<style>${css}</style>`)
  .replace('<script type="module" src="{{app.js}}"></script>', '')
  // A function, so the bundle's own `$&` and `$'` stay as written.
  .replace('</body>', () => `<script type="module">${bundle.outputFiles[0].text}</script></body>`)

const browser = await chromium.launch({ executablePath: process.env.CHROME_PATH })
async function shoot(name, { colorScheme, metrics }) {
  const context = await browser.newContext({ colorScheme, viewport: metrics.mobile ? { width: metrics.width, height: metrics.height } : { width: metrics.width, height: metrics.height } })
  const page = await context.newPage()
  if (metrics.mobile) {
    const cdp = await context.newCDPSession(page)
    await cdp.send('Emulation.setDeviceMetricsOverride', { width: metrics.width, height: metrics.height, deviceScaleFactor: 3, mobile: true })
    await cdp.send('Emulation.setTouchEmulationEnabled', { enabled: true })
  }
  const errors = []
  page.on('pageerror', error => errors.push(error.message))
  await page.setContent(html)
  await page.waitForFunction(() => window.done === true)
  if (errors.length) throw new Error(errors.join('\n'))
  if (await page.locator('#bar-count').isHidden()) throw new Error('Needs You fixture did not draw its count')
  if (!(await page.locator('#needs').innerText()).includes('This browser can only look at studio')) throw new Error('The refused answer did not draw its notice')
  console.log(name, 'title:', await page.title())
  await page.screenshot({ path: join(out, `${name}.png`), fullPage: false })
  await context.close()
}
for (const scheme of ['light', 'dark']) {
  await shoot(`needs-desktop-${scheme}`, { colorScheme: scheme, metrics: { width: 1280, height: 800, mobile: false } })
  await shoot(`needs-phone-${scheme}`, { colorScheme: scheme, metrics: { width: 375, height: 812, mobile: true } })
}
await browser.close()
