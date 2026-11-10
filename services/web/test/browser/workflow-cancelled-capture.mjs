#!/usr/bin/env node
// Captures a cancelled run with a long reason (ov-1053): a 320 px phone and desktop, light and dark, through CDP
// mobile emulation. It also fails if the card scrolls sideways or the reason is clipped.
//
//   node test/browser/workflow-cancelled-capture.mjs OUT_DIR

import { mkdirSync, readFileSync } from 'node:fs'
import { join, resolve } from 'node:path'

import { build } from 'esbuild'
import { chromium } from 'playwright-core'

const out = resolve(process.argv[2] ?? 'workflow-cancelled-shots')
mkdirSync(out, { recursive: true })
const bundle = await build({ entryPoints: ['test/browser/workflow-cancelled-harness.ts'], bundle: true, write: false, format: 'esm', target: 'es2022', logLevel: 'error' })
const css = readFileSync('src/client/app.css', 'utf8')
const html = readFileSync('src/client/index.html', 'utf8')
  .replace('<link rel="stylesheet" href="{{app.css}}">', `<style>${css}</style>`)
  .replace('<script type="module" src="{{app.js}}"></script>', '')
  .replace('</body>', `<script type="module">${bundle.outputFiles[0].text}</script></body>`)

const browser = await chromium.launch({ executablePath: process.env.CHROME_PATH })
let failed = false
async function shoot(name, { colorScheme, width, height, mobile }) {
  const context = await browser.newContext({ colorScheme, viewport: { width, height } })
  const page = await context.newPage()
  if (mobile) {
    const cdp = await context.newCDPSession(page)
    await cdp.send('Emulation.setDeviceMetricsOverride', { width, height, deviceScaleFactor: 3, mobile: true })
  }
  await page.setContent(html)
  await page.waitForFunction(() => window.done === true)
  const m = await page.evaluate(() => {
    const p = [...document.querySelectorAll('.workflow-runs p')].find(n => n.textContent.includes('The board this ran on'))
    const r = p.getBoundingClientRect(); const d = document.getElementById('workflow-runs')
    return { text: p.textContent, right: r.right, vw: document.documentElement.clientWidth, scrollW: document.documentElement.scrollWidth, clipped: p.scrollWidth > p.clientWidth, sheetScrollsSideways: d.scrollWidth > d.clientWidth }
  })
  console.log(name, JSON.stringify(m))
  if (m.right > m.vw || m.sheetScrollsSideways || m.scrollW > m.vw || m.clipped) { failed = true; console.error(name, 'overflows') }
  await page.locator('#workflow-runs').screenshot({ path: join(out, `${name}.png`) })
  await context.close()
}
for (const scheme of ['light', 'dark']) {
  await shoot(`workflow-cancelled-phone-320-${scheme}`, { colorScheme: scheme, width: 320, height: 1500, mobile: true })
  await shoot(`workflow-cancelled-desktop-${scheme}`, { colorScheme: scheme, width: 1280, height: 800, mobile: false })
}
await browser.close()
process.exit(failed ? 1 : 0)
