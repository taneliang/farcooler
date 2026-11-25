// Run with REVIEW_BROWSER_BRIDGE set to the fixture's scratch directory.
import assert from 'node:assert/strict'
import { createConnection } from 'node:net'
import { mkdirSync, readFileSync } from 'node:fs'
import { join } from 'node:path'
import { build } from 'esbuild'
import { chromium } from 'playwright-core'

const fixture = JSON.parse(readFileSync(join(process.env.REVIEW_BROWSER_BRIDGE, 'ready.json'), 'utf8'))
const origin = 'https://review.test'
const destination = new URL(origin)
destination.search = new URLSearchParams({
  runner: 'canary', workspace: fixture.workspace, repo: fixture.repo, view: 'board', ticket: fixture.ticket,
})
const html = readFileSync('src/client/index.html', 'utf8')
  .replaceAll('{{app.css}}', '/app.css').replaceAll('{{app.js}}', '/board.js')
const css = readFileSync('src/client/app.css', 'utf8')
const bundle = await build({ entryPoints: ['test/browser/board-harness.ts'], bundle: true, write: false, format: 'esm' })
const output = process.argv[2]
if (output) mkdirSync(output, { recursive: true })

function rpc(request) {
  return new Promise((resolve, reject) => {
    const socket = createConnection({ host: '127.0.0.1', port: fixture.port })
    let raw = ''
    socket.setEncoding('utf8')
    socket.once('connect', () => socket.write(JSON.stringify(request) + '\n'))
    socket.on('data', data => { raw += data })
    socket.once('error', reject)
    socket.once('end', () => {
      try { resolve(JSON.parse(raw)) } catch (error) { reject(error) }
    })
  })
}

const browser = await chromium.launch({ executablePath: process.env.CHROME_PATH })
const errors = []
async function page(width = 1280) {
  const context = await browser.newContext({ viewport: { width, height: 900 }, isMobile: width < 500 })
  await context.route(origin + '/**', async route => {
    switch (new URL(route.request().url()).pathname) {
      case '/app.css': return route.fulfill({ contentType: 'text/css', body: css })
      case '/board.js': return route.fulfill({ contentType: 'text/javascript', body: bundle.outputFiles[0].text })
      case '/board-rpc': {
        const reply = await rpc(JSON.parse(route.request().postData()))
        return route.fulfill({ status: reply.error ? 400 : 200, contentType: 'application/json', body: JSON.stringify(reply.error ? reply : reply.value) })
      }
      default: return route.fulfill({ contentType: 'text/html', body: html })
    }
  })
  const tab = await context.newPage()
  tab.on('pageerror', error => errors.push(error.message))
  tab.setDefaultTimeout(10000)
  await tab.goto(destination.href)
  return tab
}

const button = (tab, name) => tab.getByRole('button', { name, exact: true })
const comment = 'Keep the browser boundaries beside the parity inventory so the rollout can be reviewed in one place.'
async function capture(tab, name) {
  assert.equal(await tab.evaluate(() => document.documentElement.scrollWidth), tab.viewportSize().width)
  if (output) await tab.screenshot({ path: join(output, name) })
}

try {
  for (const width of [320, 1280]) {
    const tab = await page(width)
    await button(tab, 'Files').click()
    await button(tab, 'reports').click()
    await button(tab, 'context.md').click()
    const preview = tab.locator('.review-preview')
    await preview.waitFor()
    assert.ok((await preview.textContent()).length > 10000)
    await capture(tab, `files-${width}.png`)
    await button(tab, 'Up one folder').click()
    await button(tab, 'previous.md').waitFor()
    await button(tab, 'Changes').click()
    await button(tab, 'reports/previous.md').click()
    const line = tab.locator('.review-line.added').filter({ hasText: '# ov-1439' }).getByRole('button').first()
    await line.click()
    await tab.getByLabel('Inline comment').waitFor()
    if (width === 320) {
      await tab.getByLabel('Inline comment').fill(comment)
      await button(tab, 'Post comment').click()
    }
    await tab.locator('blockquote').getByText(comment).waitFor()
    await button(tab, 'Mark reviewed').click()
    await button(tab, 'Mark unreviewed').waitFor()
    const anchor = await tab.locator('.review-line:has([aria-pressed="true"])').getAttribute('id')
    const board = tab.locator('#board')
    await board.evaluate(node => { node.querySelector('blockquote').scrollIntoView({ block: 'center' }) })
    const before = await board.evaluate(node => node.scrollTop)
    assert.ok(before > 0)
    await tab.reload()
    await button(tab, 'Mark unreviewed').waitFor()
    const restored = tab.locator(`[id="${anchor}"] [aria-pressed="true"]`)
    await restored.waitFor()
    assert.equal(await board.evaluate(node => node.scrollTop), before)
    await capture(tab, `changes-${width}.png`)
    await tab.close()
  }

  const tab = await page()
  await button(tab, 'Changes').click()
  await button(tab, 'Approve').waitFor()
  assert.equal((await rpc({ method: 'move' })).error, undefined)
  await button(tab, 'Approve').click()
  await tab.getByText('The candidate or diff changed. Refresh before continuing.').waitFor()
  await button(tab, 'Refresh').click()
  await tab.getByText('The candidate changed. Review the current diff before continuing.').waitFor()
  assert.equal(await button(tab, 'Approve').count(), 0)
  assert.equal((await rpc({ method: 'restore' })).error, undefined)
  await button(tab, 'Refresh').click()
  await button(tab, 'Approve').click()
  await tab.getByText('Approval accepted', { exact: true }).waitFor()
  assert.equal(await button(tab, 'Approve').count(), 0)
  assert.deepEqual(errors, [])
  console.log('PASS files, candidate diff, shared inline comment, reviewed state, reload and exact-candidate approval')
} finally {
  await browser.close()
  await rpc({ method: 'stop' })
}
