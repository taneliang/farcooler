#!/usr/bin/env node
// The Engine view in a browser (ov-1486), over the page the shared core made from a real runner's one-reply
// read of a 340-item board. First make that page, then run this from services/web:
//
//   FARCOOLER_ENGINE_FIXTURE=$PWD/engine.json cargo test -p farcooler-daemon --lib ov1486_a_board
//   node test/browser/engine-check.mjs engine.json [CAPTURE_DIR]
import assert from 'node:assert/strict'
import { mkdirSync, readFileSync } from 'node:fs'
import { join, resolve } from 'node:path'
import { build } from 'esbuild'
import { chromium } from 'playwright-core'
import { harnessHtml, styles } from '../../scripts/styles.mjs'
import { filledButtons } from './shell-audit.mjs'

const [fixturePath, captures] = process.argv.slice(2)
assert(fixturePath, 'give the fixture the daemon test wrote')
const fixture = JSON.parse(readFileSync(fixturePath, 'utf8'))
const out = captures ? resolve(captures) : null
if (out) mkdirSync(out, { recursive: true })
const bundle = await build({ entryPoints: ['test/browser/engine-harness.ts'], bundle: true, write: false, format: 'esm', target: 'es2022', logLevel: 'error' })
const css = styles().toString()
const html = harnessHtml().replaceAll('{{app.css}}', '/app.css').replaceAll('{{app.js}}', '/harness.js')
const origin = 'https://engine.test'
const browser = await chromium.launch({ executablePath: process.env.CHROME_PATH })
const failures = []
/** From the page's open to every row on screen, on this machine with the read already answered. */
const RENDER_BUDGET_MS = 3000

const rows = [...fixture.page.view.stations, ...fixture.page.view.landings].flatMap(s => s.tickets)
const rowIds = new Set(rows.map(t => t.id))
const offered = kind => Object.entries(fixture.page.actions).find(([, offers]) => offers.some(o => o.kind === kind))

async function test(name, run, { width = 1280, colorScheme = 'light' } = {}) {
  const context = await browser.newContext({ viewport: { width, height: 850 }, isMobile: width < 500, colorScheme })
  await context.route(origin + '/**', route => {
    const path = new URL(route.request().url()).pathname
    if (path === '/harness.js') return route.fulfill({ contentType: 'text/javascript', body: bundle.outputFiles[0].text })
    if (path === '/app.css') return route.fulfill({ contentType: 'text/css', body: css })
    return route.fulfill({ contentType: 'text/html', body: html })
  })
  await context.addInitScript(value => { window.__fixture = value }, fixture)
  const page = await context.newPage()
  page.setDefaultTimeout(5000)
  const errors = []
  page.on('pageerror', e => errors.push(e.message))
  try {
    await page.goto(origin + '/')
    await page.locator('#engine li.engine-ticket').first().waitFor()
    await run(page)
    assert.deepEqual(errors, [])
    console.log(`ok ${name}`)
  } catch (error) {
    failures.push(name)
    console.error(`FAILED ${name}: ${error.stack ?? error}`)
  }
  await context.close()
}
const sent = (page, method) => page.evaluate(m => window.engine.sent.filter(s => s.method === m).map(s => s.args), method)
const row = (page, id) => page.locator(`#engine [data-item="${id}"]`)
const said = page => page.locator('#engine .engine-outcome')

/** Nothing scrolls sideways, and no control is cut or squeezed into a column of letters. */
async function fits(page) {
  const bad = await page.evaluate(() => {
    const sheet = document.getElementById('engine')
    const body = sheet.querySelector('.engine')
    const out = []
    if (document.documentElement.scrollWidth > document.documentElement.clientWidth) out.push('the page scrolls sideways')
    if (body.scrollWidth > body.clientWidth) out.push(`the view scrolls sideways: ${body.scrollWidth} > ${body.clientWidth}`)
    const edge = body.getBoundingClientRect().right
    for (const node of body.querySelectorAll('button, h3, h4, p, .engine-question, li, textarea')) {
      if (!node.checkVisibility()) continue
      const box = node.getBoundingClientRect()
      if (box.right > edge + 0.5) out.push(`past the edge: ${node.textContent.slice(0, 60)}`)
      if (node.tagName === 'BUTTON' && (box.width < 56 || node.scrollWidth > node.clientWidth + 1)) {
        out.push(`squeezed button: ${node.textContent} at ${box.width}`)
      }
    }
    return out.slice(0, 8)
  })
  assert.deepEqual(bad, [])
}

assert(rowIds.size >= 300, `the fixture holds ${rowIds.size} tickets`)
assert(fixture.stateBytes > 1024 * 1024 && fixture.replyBytes <= 1024 * 1024,
  `the whole state (${fixture.stateBytes}) is past a frame and the reply (${fixture.replyBytes}) fits one`)

for (const width of [320, 1280]) {
  await test(`every ticket of a ${rowIds.size}-ticket board shows with its step and state at ${width}`, async page => {
    await page.waitForFunction(n => document.querySelectorAll('#engine li.engine-ticket').length === n, rowIds.size)
    const took = await page.evaluate(() => performance.now() - window.engine.started)
    assert(took < RENDER_BUDGET_MS, `rows took ${Math.round(took)} ms`)
    console.log(`   ${rowIds.size} rows in ${Math.round(took)} ms; one read of ${fixture.replyBytes} bytes`)
    assert.equal((await sent(page, 'engine.read')).length, 1, 'one read draws the view')
    const missing = await page.evaluate(rows => rows.filter(t => {
      const node = document.querySelector(`#engine li.engine-ticket[data-item="${t.id}"]`)
      return !node || !node.querySelector('h4').textContent.includes(t.title) ||
        node.querySelector('.engine-status').textContent !== `${t.step} · ${t.status}`
    }).map(t => t.id), rows)
    assert.deepEqual(missing, [])
    for (const landing of fixture.page.view.landings) {
      assert.match(await row(page, landing.id).innerText(), new RegExp(landing.label))
    }
    // The whole question: the approval's code is only in it.
    assert.equal(await page.locator('#engine .engine-question').textContent(), fixture.page.drafts[0].question)
    await fits(page)
    assert.deepEqual(await filledButtons(page), [])
  }, { width })

  await test(`a press asks first, and only the confirmation sends it at ${width}`, async page => {
    const [item, offers] = offered('resolve_cancellation')
    const offer = offers.find(o => o.kind === 'resolve_cancellation')
    await row(page, item).getByRole('button', { name: offer.label, exact: true }).click()
    assert.deepEqual(await sent(page, 'engine.act'), [], 'the first press sends nothing')
    const box = row(page, item).locator('.engine-confirm')
    assert.equal(await box.locator('p').textContent(), offer.confirm)
    assert.equal((await filledButtons(page)).length, 0, 'a destructive confirmation is not the filled button')
    await box.getByRole('button', { name: fixture.page.words.keep }).click()
    assert.equal(await row(page, item).locator('.engine-confirm').count(), 0)
    assert.deepEqual(await sent(page, 'engine.act'), [])
    await row(page, item).getByRole('button', { name: offer.label, exact: true }).click()
    await row(page, item).getByLabel(offer.reasonLabel).fill('I checked its processes and the worktree.')
    await fits(page)
    await page.evaluate(() => window.engine.answer({ ok: true, message: 'The cancellation was resolved.', detail: null }))
    await row(page, item).locator('.engine-confirm').getByRole('button', { name: offer.label }).click()
    await said(page).getByText('The cancellation was resolved.').waitFor()
    assert.deepEqual(await sent(page, 'engine.act'), [{ workspace: fixture.workspace, selection: {
      item, kind: 'resolve_cancellation', guard: offer.guard, reason: 'I checked its processes and the worktree.' } }])
    assert((await sent(page, 'engine.read')).length >= 2, 'the view reads again after a press')
  }, { width })
}

for (const kind of ['cancel', 'resolve', 'settle_landed', 'settle_not_landed']) {
  await test(`${kind} sends the row and kind the person chose`, async page => {
    const [item, offers] = offered(kind)
    const offer = offers.find(o => o.kind === kind)
    await row(page, item).getByRole('button', { name: offer.label, exact: true }).first().click()
    await page.locator('#engine .engine-confirm').getByRole('button', { name: offer.label, exact: true }).click()
    await said(page).waitFor()
    assert.deepEqual(await sent(page, 'engine.act'),
      [{ workspace: fixture.workspace, selection: { item, kind, guard: offer.guard } }])
  })
}

for (const approve of [true, false]) {
  await test(`${approve ? 'approving' : 'declining'} a draft names the question the person read`, async page => {
    const draft = fixture.page.drafts[0]
    const words = fixture.page.words
    const name = approve ? words.approve : words.decline
    const card = page.locator('#engine .engine-draft')
    await card.getByRole('button', { name, exact: true }).click()
    assert.deepEqual(await sent(page, 'engine.decide'), [])
    assert.equal((await filledButtons(page)).length, 1, 'the confirmation is the one filled button')
    await page.evaluate(() => window.engine.answer({ ok: true, message: 'Done.', detail: null }))
    await card.locator('.engine-confirm').getByRole('button', { name, exact: true }).click()
    await said(page).getByText('Done.').waitFor()
    assert.deepEqual(await sent(page, 'engine.decide'),
      [{ workspace: fixture.workspace, draft: draft.id, note: draft.note, approve }])
  })
}

await test('a refusal and a lost answer are said, with the runner’s words apart', async page => {
  const [item, offers] = offered('settle_landed')
  const offer = offers.find(o => o.kind === 'settle_landed')
  const press = async () => {
    await row(page, item).getByRole('button', { name: offer.label, exact: true }).click()
    await row(page, item).locator('.engine-confirm').getByRole('button', { name: offer.label, exact: true }).click()
  }
  await page.evaluate(() => window.engine.answer({ ok: false, message: 'The runner didn’t do it.', detail: 'the remote disagrees' }))
  await press()
  await said(page).getByText('The runner didn’t do it.').waitFor()
  assert.equal(await said(page).locator('pre').textContent(), 'the remote disagrees')
  await page.locator('#engine .engine-confirm').getByRole('button', { name: fixture.page.words.keep }).click()
  await page.evaluate(() => window.engine.answer('lost'))
  await press()
  await said(page).getByText(fixture.page.words.unconfirmed).waitFor()
})

await test('a dropped connection leaves nothing to press until the view is read again', async page => {
  const [item, offers] = offered('cancel')
  const label = offers.find(o => o.kind === 'cancel').label
  const button = row(page, item).getByRole('button', { name: label, exact: true })
  await page.evaluate(() => window.engine.connection(false))
  assert(await button.isDisabled())
  assert(await page.locator('#engine .engine-draft button').first().isDisabled())
  await page.evaluate(() => window.engine.connection(true))
  await page.waitForFunction(() => !document.querySelector('#engine .engine-draft button').disabled)
  assert(!await button.isDisabled())
  await page.locator('#engine').getByRole('button', { name: fixture.page.words.runs }).click()
  assert.equal(await page.evaluate(() => window.engine.runsOpened()), 1)
})

await test('a link that drops and returns while a press is in flight still ends with a fresh read', async page => {
  const [item, offers] = offered('cancel')
  const label = offers.find(o => o.kind === 'cancel').label
  await row(page, item).getByRole('button', { name: label, exact: true }).click()
  await page.evaluate(() => window.engine.hold())
  await row(page, item).locator('.engine-confirm').getByRole('button', { name: label, exact: true }).click()
  await page.evaluate(() => { window.engine.connection(false); window.engine.connection(true) })
  const before = (await sent(page, 'engine.read')).length
  await page.evaluate(() => window.engine.release())
  await said(page).waitFor()
  await page.waitForFunction(() => !document.querySelector('#engine .engine-draft button').disabled)
  assert.equal((await sent(page, 'engine.read')).length, before + 1)
  assert.equal((await sent(page, 'engine.act')).length, 1, 'the press is never sent twice')
})

if (out) {
  for (const width of [320, 1280]) {
    for (const colorScheme of ['light', 'dark']) {
      await test(`captures at ${width} ${colorScheme}`, async page => {
        const shot = name => page.screenshot({ path: join(out, `${name}-${width}-${colorScheme}.png`) })
        await shot('top')
        const [item, offers] = offered('resolve_cancellation')
        await row(page, item).getByRole('button', { name: offers.find(o => o.kind === 'resolve_cancellation').label }).click()
        await row(page, item).locator('textarea').fill('I checked its processes and the worktree.')
        await row(page, item).evaluate(node => node.scrollIntoView({ block: 'center' }))
        await shot('confirm')
        await page.locator('#engine [data-item^="group-"]').evaluate(node => node.scrollIntoView({ block: 'start' }))
        await shot('landing')
        await page.locator('#engine .engine-draft .engine-buttons').evaluate(node => node.scrollIntoView({ block: 'center' }))
        await shot('draft-buttons')
        await fits(page)
      }, { width, colorScheme })
    }
  }
}

await browser.close()
if (failures.length) { console.error(`${failures.length} failed`); process.exit(1) }
