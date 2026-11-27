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

async function test(name, run, { query = '', width = 1280, sharedWriter = false } = {}) {
  const context = await browser.newContext({ viewport: { width, height: 850 }, isMobile: width < 500 })
  const writers = new Map()
  const displaced = new Map()
  await context.route(origin + '/**', async route => {
    const path = new URL(route.request().url()).pathname
    if (path === '/writer') {
      const { client, terminal, action } = JSON.parse(route.request().postData())
      const key = sharedWriter ? terminal : `${client}:${terminal}`
      const owner = writers.get(key)
      if (action === 'write') return route.fulfill({ status: owner === client ? 200 : 409 })
      if (action === 'take-over' || (action === 'claim' && !owner)) {
        if (owner && owner !== client) displaced.set(key, owner)
        writers.set(key, client)
      } else if (action === 'release' && owner === client) writers.delete(key)
      const writable = writers.get(key) === client
      const occupied = writers.has(key)
      return route.fulfill({ json: {
        writable,
        label: writable ? 'Writer' : 'Viewer',
        reason: writable ? '' : displaced.get(key) === client ? 'Another client took over this terminal.'
          : occupied ? 'Another client is writing to this terminal.' : 'This terminal is available for input.',
        action: writable ? null : occupied ? { label: 'Take Over', intent: 'take-over' } : { label: 'Request Input', intent: 'claim' },
      } })
    }
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
  for (const width of [320, 1280]) await test(`workspace terminal sidebar at ${width}px`, async page => {
    const picker = page.getByLabel('Workspace', { exact: true })
    const list = page.locator('#terminal-list')
    const visibleIds = () => list.locator('a:visible').evaluateAll(links => links.map(link => link.dataset.terminal))
    await list.locator('a[data-terminal=seat]').waitFor()
    const shell = list.locator('a[data-terminal=shell]')
    await shell.click()
    await page.waitForURL('**terminal=shell')
    await page.getByText('Writer', { exact: true }).waitFor()
    assert.equal(await page.evaluate(() => document.documentElement.scrollWidth), width)
    assert.deepEqual(await visibleIds(), ['seat', 'shell', 'lost-seat'])
    assert.equal(await list.locator('a[data-terminal=seat]').innerText(), 'Orchestrator')
    assert.deepEqual(await picker.locator('option').allTextContents(), ['Main · overnight', 'Main · Verdela'])
    const lost = list.getByRole('link', { name: 'orchestrator', exact: true })
    assert.equal(await lost.getAttribute('aria-description'), 'Lost')
    const recover = list.getByRole('button', { name: 'Recover orchestrator', exact: true })
    assert.equal(await recover.isVisible(), true)
    if (out) await page.screenshot({ path: join(out, `sidebar-${width}.png`) })
    const unavailable = list.getByText('Unavailable Terminals', { exact: true })
    await unavailable.click()
    assert.deepEqual(await visibleIds(), ['seat', 'shell', 'lost-seat', 'exited-seat'])
    await unavailable.click()
    await recover.click()
    await page.waitForURL('**terminal=recovered')
    assert.equal((await rpc(page, 'restart')).terminal, 'lost-seat')
    await page.locator('#terminal-state').filter({ hasText: /^Running$/ }).waitFor()
    await page.goBack()
    await page.waitForURL('**terminal=shell')
    await page.locator('a[data-terminal=shell][aria-current=true]').waitFor()
    await picker.selectOption('next')
    await list.locator('a[data-terminal=other-seat]').waitFor()
    assert.deepEqual(await visibleIds(), ['other-seat'])
    assert.equal(await list.locator('a[data-terminal=other-seat]').innerText(), 'Orchestrator')
  }, { width, query: '?sidebar' })

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

  await test('restore a writer after its runner connection is freed', async page => {
    await page.getByText('Writer', { exact: true }).waitFor()
    await page.locator('[data-attached=true]:visible').waitFor()
    await page.evaluate(() => window.reconnectWriter())
    await page.getByText('Writer', { exact: true }).waitFor()
    await page.locator('.xterm-helper-textarea:visible').press('r')
    await page.waitForFunction(() => window.calls.some(c => c.action === 'write' && c.text === 'r'))
  })

  await test('hold keystrokes while the initial writer claim is pending', async (_first, context) => {
    const page = await context.newPage()
    page.setDefaultTimeout(5000)
    let grant
    const waiting = new Promise(resolve => { grant = resolve })
    try {
      await page.route('**/writer', async route => {
        if (JSON.parse(route.request().postData()).action === 'claim') await waiting
        await route.fallback()
      })
      await page.goto(origin + '/')
      await page.locator('[data-attached=true]:visible').waitFor()
      await page.locator('.xterm-helper-textarea:visible').press('e')
      assert.equal(await rpc(page, 'write'), undefined)
      grant()
      await page.getByText('Writer', { exact: true }).waitFor()
      await page.waitForFunction(() => window.calls.some(c => c.action === 'write' && c.text === 'e'))
    } finally {
      grant()
      await page.close()
    }
  })

  await test('read-only grant explains why terminal commands are unavailable', async page => {
    await page.getByText('Read-only', { exact: true }).waitFor()
    await page.locator('#pane-note').filter({ hasText: /This browser has read-only access/ }).waitFor()
    assert.equal(await page.getByRole('button', { name: 'Rename' }).count(), 0)
    assert.equal(await page.locator('#lost-terminals a').count(), 1)
    assert.equal(await page.locator('#lost-terminals button').count(), 0)
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

  // Each page has its own session, even when both use the same grant.
  for (const width of [320, 1280]) {
    await test(`exclusive writer and explicit takeover at ${width}px`, async (first, context) => {
      const second = await context.newPage()
      try {
        await first.getByText('Writer', { exact: true }).waitFor()
        await second.goto(origin + '/')
        await second.getByText('Another client is writing to this terminal.', { exact: true }).waitFor({ timeout: 2500 })
        assert.equal(await first.getByText('Writer', { exact: true }).count(), 1)
        for (const page of [first, second]) await page.locator('[data-attached=true]:visible').waitFor()
        const input = page => page.locator('.xterm-helper-textarea:visible')
        await input(first).press('a')
        await first.waitForFunction(() => window.calls.some(c => c.action === 'write'))
        await input(second).press('b')
        assert.equal(await rpc(second, 'write'), undefined)
        await second.getByRole('button', { name: 'Take Over' }).click()
        await second.getByText('Writer', { exact: true }).waitFor()
        await first.getByText('Another client took over this terminal.', { exact: true }).waitFor()
        assert.equal(await first.getByText('Writer', { exact: true }).count(), 0)
        await input(second).press('c')
        await second.waitForFunction(() => window.calls.some(c => c.action === 'write' && c.text === 'c'))
        await input(first).press('d')
        assert.equal((await rpc(first, 'write')).text, 'a')
        for (const page of [first, second]) {
          assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth))
        }
        if (out) {
          await first.screenshot({ path: join(out, `viewer-${width}.png`) })
          await second.screenshot({ path: join(out, `writer-${width}.png`) })
        }
      } finally {
        await second.close()
      }
    }, { width, sharedWriter: true })
  }
} finally {
  await browser.close()
}
assert.deepEqual(failures, [])
