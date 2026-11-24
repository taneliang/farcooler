#!/usr/bin/env node
// Run from services/web with WORK_VIEW_BIN set. Playwright uses its installed
// Chromium unless CHROME_PATH is set. An optional argument captures screenshots.
import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import { mkdirSync, readFileSync } from 'node:fs'
import { join } from 'node:path'
import { build } from 'esbuild'
import { chromium } from 'playwright-core'

const out = process.argv[2]
if (out) mkdirSync(out, { recursive: true })
const bundle = await build({ entryPoints: ['test/browser/work-harness.ts'], bundle: true, write: false, format: 'esm' })
const html = readFileSync('src/client/index.html', 'utf8')
  .replaceAll('{{app.css}}', '/app.css').replaceAll('{{app.js}}', '/harness.js')
const css = ['node_modules/@xterm/xterm/css/xterm.css', 'src/client/app.css', 'src/client/chat.css']
  .map(p => readFileSync(p, 'utf8')).join('\n')
const origin = 'https://work.test'
const browser = await chromium.launch({ executablePath: process.env.CHROME_PATH })
const failures = []

async function test(name, run, { query = '', width = 1280 } = {}) {
  const context = await browser.newContext({ viewport: { width, height: 850 }, isMobile: width < 500 })
  await context.route(origin + '/**', async route => {
    const path = new URL(route.request().url()).pathname
    if (path === '/projection' || path === '/failure') {
      const body = execFileSync(process.env.WORK_VIEW_BIN, [path], { input: route.request().postData(), encoding: 'utf8' })
      return route.fulfill({ body })
    }
    if (path === '/harness.js') return route.fulfill({ contentType: 'text/javascript', body: bundle.outputFiles[0].text })
    if (path === '/app.css') return route.fulfill({ contentType: 'text/css', body: css })
    await route.fulfill({ contentType: 'text/html', body: html })
  })
  const page = await context.newPage()
  page.setDefaultTimeout(5000)
  try {
    await page.goto(origin + '/' + query)
    await run(page, context)
    console.log('PASS', name)
  } catch (e) {
    failures.push(name)
    console.error('FAIL', name, e.message)
  } finally {
    await context.close()
  }
}

const work = async page => {
  await page.getByRole('button', { name: 'Start Work', exact: true }).click()
  return page.getByRole('dialog', { name: 'Start Work' })
}
const rpc = async (page, action) => page.evaluate(a => window.calls.filter(c => c.action === a).at(-1), action)

try {
  await test('pick workspace and repository, then create and start a shell', async page => {
    await page.getByLabel('Workspace', { exact: true }).selectOption('next')
    await page.waitForURL('**workspace=next')
    const dialog = await work(page)
    await dialog.getByLabel('Repository').selectOption('platform')
    await page.waitForURL('**workspace=delivery')
    await dialog.getByRole('button', { name: 'New Worktree' }).click()
    await dialog.getByLabel('Worktree Name').fill('Verify deployment compatibility')
    await dialog.getByLabel('Branch', { exact: true }).fill('release/verify-deployment-compatibility')
    await dialog.getByRole('button', { name: 'Create Worktree', exact: true }).click()
    await dialog.getByLabel('Worktree', { exact: true }).locator('option[value=created]').waitFor({ state: 'attached' })
    assert.deepEqual(await rpc(page, 'create'), {
      action: 'create', repository: 'platform', workspace: 'delivery', name: 'Verify deployment compatibility', branch: 'release/verify-deployment-compatibility',
    })
    await dialog.getByRole('button', { name: 'Start Terminal' }).click()
    await page.waitForURL('**terminal=shell')
    assert.equal((await rpc(page, 'start')).worktree, 'created')
  })

  await test('resume an existing worktree and start an agent', async page => {
    const dialog = await work(page)
    await dialog.getByLabel('Worktree', { exact: true }).selectOption('release')
    await dialog.getByLabel('Terminal Type').selectOption('codex')
    await dialog.getByRole('button', { name: 'Start Terminal' }).click()
    await page.waitForURL('**terminal=codex')
    assert.deepEqual(await rpc(page, 'start'), { action: 'start', worktree: 'release', preset: 'codex' })
  })

  await test('rename and close a terminal', async page => {
    page.once('dialog', dialog => dialog.accept('Reviewed deployment logs'))
    await page.getByRole('button', { name: 'Rename' }).click()
    await page.getByRole('link', { name: 'Reviewed deployment logs' }).waitFor()
    assert.equal((await rpc(page, 'rename')).terminal, 'shell')
    page.once('dialog', dialog => dialog.dismiss())
    await page.getByRole('button', { name: 'Close Terminal', exact: true }).click()
    assert.equal(await rpc(page, 'close'), undefined)
    page.once('dialog', dialog => dialog.accept())
    await page.getByRole('button', { name: 'Close Terminal', exact: true }).click()
    await page.getByRole('link', { name: 'Reviewed deployment logs' }).waitFor({ state: 'detached' })
    assert.equal((await rpc(page, 'close')).terminal, 'shell')
  })

  await test('recover a lost agent using the returned terminal identity', async page => {
    await page.getByRole('link', { name: /Bring back the observability/ }).click()
    await page.getByRole('button', { name: 'Recover Terminal' }).click()
    await page.waitForURL('**terminal=recovered')
    assert.equal((await rpc(page, 'restart')).terminal, 'lost-agent')
    await page.locator('[data-attached=true]:visible').waitFor()
    assert.equal((await rpc(page, 'attach')).terminal, 'recovered')
  })

  await test('a superseded terminal selection reports failure', async page => {
    await page.locator('[data-attached=true]:visible').waitFor()
    const selected = await page.evaluate(() => Promise.all([
      window.chooseTerminal('lost-agent'),
      window.chooseTerminal('shell'),
    ]))
    assert.deepEqual(selected, [false, true])
  })

  await test('reconnect a stream without restarting the running process', async page => {
    await page.getByText('Writer', { exact: true }).waitFor()
    await page.locator('[data-attached=true]:visible').waitFor()
    await page.evaluate(() => window.dropStream())
    await page.getByRole('button', { name: 'Reconnect Terminal' }).click()
    await page.getByRole('button', { name: 'Reconnect Terminal' }).waitFor({ state: 'hidden' })
    assert.equal(await rpc(page, 'restart'), undefined)
    await page.waitForFunction(() => window.calls.filter(c => c.action === 'attach').length === 2)
  })

  await test('read-only grant explains why terminal commands are unavailable', async page => {
    await page.getByText('Read-only', { exact: true }).waitFor()
    await page.locator('#pane-note').filter({ hasText: /This browser has read-only access/ }).waitFor()
    assert.equal(await page.getByRole('button', { name: 'Rename' }).count(), 0)
    assert.equal(await rpc(page, 'attach'), undefined)
  }, { query: '?read' })

  await test('an actual scope refusal is explained in plain words', async page => {
    page.once('dialog', dialog => dialog.accept('Refused rename'))
    await page.getByRole('button', { name: 'Rename' }).click()
    await page.locator('#pane-note').filter({ hasText: /This browser has read-only access/ }).waitFor()
  }, { query: '?refuse' })

  await test('terminal and workspace links survive reload, new tab and Back', async (page, context) => {
    const terminal = page.getByRole('link', { name: /Bring back the observability/ })
    await terminal.waitFor()
    const href = await terminal.getAttribute('href')
    const tab = await context.newPage()
    tab.setDefaultTimeout(5000)
    await tab.goto(new URL(href, origin).href)
    await tab.getByRole('button', { name: 'Recover Terminal' }).waitFor()
    await tab.reload()
    await tab.getByRole('button', { name: 'Recover Terminal' }).waitFor()
    await page.getByLabel('Workspace', { exact: true }).selectOption('next')
    await page.waitForURL('**workspace=next')
    const workspace = await page.getByRole('link', { name: 'Workspace Link' }).getAttribute('href')
    await tab.goto(new URL(workspace, origin).href)
    await tab.waitForFunction(() => document.getElementById('space-picker').value === 'next')
    await page.goBack()
    await page.waitForFunction(() => document.getElementById('space-picker').value === 'delivery')
  })

  await test('a worktree link opens the exact checkout without selecting a different terminal', async page => {
    const dialog = page.getByRole('dialog', { name: 'Start Work' })
    await dialog.waitFor()
    assert.equal(await dialog.getByLabel('Worktree', { exact: true }).inputValue(), 'release')
    await page.reload()
    await dialog.waitFor()
    assert.equal(await dialog.getByLabel('Worktree', { exact: true }).inputValue(), 'release')
    assert.equal(new URL(page.url()).searchParams.get('terminal'), null)
  }, { query: '?runner=paired-runner&workspace=delivery&worktree=release' })

  for (const width of [320, 1280]) await test(`long labels at ${width}px`, async page => {
    await page.getByText('Writer', { exact: true }).waitFor()
    await page.locator('[data-attached=true]:visible').waitFor()
    if (out) await page.screenshot({ path: join(out, `terminal-${width}.png`) })
    const dialog = await work(page)
    assert.equal(await dialog.evaluate(el => el.scrollWidth <= el.clientWidth), true)
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth), width)
    assert.equal(await page.evaluate(() => window.visualViewport.offsetLeft), 0)
    if (out) await page.screenshot({ path: join(out, `start-work-${width}.png`) })
  }, { width })
} finally {
  await browser.close()
}
assert.deepEqual(failures, [])
