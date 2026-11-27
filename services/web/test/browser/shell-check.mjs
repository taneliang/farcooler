#!/usr/bin/env node
// The React shell (ov-1510) in Chromium, over index.html's parked views with
// long real names (shell-harness.ts):
//
// - focus moves through the sidebar, the page header and the account in
//   document order, every stop showing a focus ring;
// - both pickers open, move and choose from the keyboard, skip a disabled
//   runner, and give focus back to their trigger; Escape closes one without
//   choosing;
// - a view link and Start Work work from the keyboard, Start Work's and
//   Commands' dialogs take focus and Escape gives it back to their opener;
// - at 320, 768 and 1280 wide, in light and dark: no sideways scroll, at most
//   one filled button, and the token pairs meet WCAG AA (printed as a table).
//
//   node test/browser/shell-check.mjs [screenshot dir]
//
// Chromium comes from playwright-core (CHROME_PATH overrides it).

import assert from 'node:assert/strict'
import { mkdirSync } from 'node:fs'
import { join } from 'node:path'
import { build } from 'esbuild'
import { chromium } from 'playwright-core'
import { harnessHtml, styles } from '../../scripts/styles.mjs'
import { contrastPairs, filledButtons } from './shell-audit.mjs'

const out = process.argv[2]
if (out) mkdirSync(out, { recursive: true })
const origin = 'https://shell.test'
const html = harnessHtml().replaceAll('{{app.css}}', '/app.css').replaceAll('{{app.js}}', '/shell.js')
const css = styles().toString()
const bundle = await build({ entryPoints: ['test/browser/shell-harness.ts'], bundle: true, write: false, format: 'esm', logLevel: 'error' })
const browser = await chromium.launch({ executablePath: process.env.CHROME_PATH })
const failures = []

async function open({ width = 1280, scheme = 'light', query = '' } = {}) {
  const context = await browser.newContext({ viewport: { width, height: 800 }, colorScheme: scheme, isMobile: width < 500 })
  await context.route(origin + '/**', route => {
    const path = new URL(route.request().url()).pathname
    if (path === '/shell.js') return route.fulfill({ contentType: 'text/javascript', body: bundle.outputFiles[0].text })
    if (path === '/app.css') return route.fulfill({ contentType: 'text/css', body: css })
    return route.fulfill({ contentType: 'text/html', body: html })
  })
  const page = await context.newPage()
  const errors = []
  page.on('pageerror', e => errors.push(e.message))
  page.setDefaultTimeout(5000)
  await page.goto(origin + '/' + query)
  await page.locator('#space-picker').waitFor()
  return { page, context, errors }
}

async function test(name, run, options) {
  const { page, context, errors } = await open(options)
  try {
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

const asked = page => page.evaluate(() => window.asked)
/// What has focus: its id, else its role and words.
const focused = page => page.evaluate(() => {
  const el = document.activeElement
  return el?.id || `${el?.getAttribute('role') ?? el?.tagName.toLowerCase()}:${el?.textContent?.trim().slice(0, 30)}`
})
const ring = page => page.evaluate(() => {
  const style = getComputedStyle(document.activeElement)
  return style.outlineStyle !== 'none' && parseFloat(style.outlineWidth) >= 2
})

/// Press ArrowDown until the option named `name` is highlighted: Base UI
/// highlights the chosen option a moment after the list opens, so a count of
/// presses would race it.
async function arrowTo(page, name) {
  const option = page.getByRole('option', { name, exact: true })
  for (let i = 0; i < 6; i++) {
    if (await option.getAttribute('data-highlighted') !== null) return
    await page.keyboard.press('ArrowDown')
    await page.waitForTimeout(50)
  }
  throw new Error(`ArrowDown never reached ${name}`)
}

const ORDER = [
  'runner-picker', 'add-runner', 'space-picker',
  'show-terminals', 'show-needs', 'show-board', 'show-plan',
  'show-orchestrator', 'show-workflow', 'show-commands',
  'workspace-link', 'a:orchestrator', 'a:Check the release logs and rec', 'a:Bring back the observability a',
  'show-start-work',
]

for (const width of [1280, 320]) {
  await test(`focus follows the document through sidebar, header and account at ${width}`, async page => {
    if (width < 681) {
      // Folded: the runner and the tools wait behind one button, and Tab
      // goes from it straight to the workspace.
      await page.keyboard.press('Tab')
      assert.equal(await focused(page), 'rail-more')
      assert.ok(await ring(page), 'rail-more has no focus ring')
      assert.equal(await page.getAttribute('#rail-more', 'aria-expanded'), 'false')
      await page.keyboard.press('Tab')
      assert.equal(await focused(page), 'space-picker')
      await page.keyboard.press('Shift+Tab')
      await page.keyboard.press('Enter')
      assert.equal(await page.getAttribute('#rail-more', 'aria-expanded'), 'true')
      assert.equal(await focused(page), 'rail-more')
    }
    const seen = []
    for (let i = 0; i < 24; i++) {
      await page.keyboard.press('Tab')
      const id = await focused(page)
      seen.push(id)
      if (id.startsWith('button:') || id.startsWith('a:') || /^(show|runner|space|add|workspace|rail)-/.test(id)) {
        assert.ok(await ring(page), `${id} has no focus ring`)
      }
    }
    const once = seen.slice(0, seen.findIndex(id => id.startsWith('body:')))
    // At phone width the terminal strip shows only on Terminals, and this
    // page is the board.
    const order = width < 681 ? ORDER.filter(id => id !== 'workspace-link' && !id.startsWith('a:')) : ORDER
    const stops = once.filter(id => order.includes(id))
    assert.deepEqual(stops, order, `focus went ${JSON.stringify(seen)}`)
    // The page's own content comes after the header, and the account last.
    const after = once.slice(once.indexOf('show-start-work'))
    assert.ok(after.indexOf('bar-count') > after.indexOf('button:ov-1510 Web: React shell on'), `account before the page: ${JSON.stringify(after)}`)
    assert.ok(after.includes('show-saved-copies') && after.includes('button:Sign Out'), `no account stops: ${JSON.stringify(after)}`)
  }, { width })
}

await test('the workspace picker opens, moves and chooses from the keyboard', async page => {
  await page.focus('#space-picker')
  await page.keyboard.press('ArrowDown')
  const list = page.getByRole('listbox')
  await list.waitFor()
  assert.deepEqual((await list.getByRole('option').allTextContents()).map(o => o.replace(/✓$/, '')), [
    'Deployment Safety and Release Engineering · Platform Infrastructure and Deployment Automation (2)',
    'Cross-platform application delivery and release coordination · Far Cooler Client Applications',
    'Sandbox · Studio',
  ])
  await arrowTo(page, 'Cross-platform application delivery and release coordination · Far Cooler Client Applications')
  await page.keyboard.press('Enter')
  await list.waitFor({ state: 'detached' })
  assert.deepEqual(await asked(page), [['space', 'client']])
  assert.equal(await page.getAttribute('#space-picker', 'data-value'), 'client')
  assert.equal(await focused(page), 'space-picker')
  // Escape closes the list and chooses nothing.
  await page.keyboard.press('Enter')
  await list.waitFor()
  await page.keyboard.press('ArrowDown')
  await page.keyboard.press('Escape')
  await list.waitFor({ state: 'detached' })
  assert.deepEqual(await asked(page), [['space', 'client']])
  assert.equal(await focused(page), 'space-picker')
})

await test('the runner picker skips a runner this browser cannot use', async page => {
  await page.focus('#runner-picker')
  await page.keyboard.press('Enter')
  await page.getByRole('listbox').waitFor()
  assert.equal(await page.getByRole('option', { name: 'Retired laptop' }).getAttribute('aria-disabled'), 'true')
  await page.getByRole('option', { name: 'Retired laptop' }).click({ force: true })
  assert.deepEqual(await asked(page), [], 'a disabled runner was chosen')
  await page.keyboard.press('Escape')
  await page.getByRole('listbox').waitFor({ state: 'detached' })
  await page.focus('#runner-picker')
  await page.keyboard.press('Enter')
  await page.getByRole('listbox').waitFor()
  await arrowTo(page, 'Linux build box in the basement')
  await page.keyboard.press('Enter')
  assert.deepEqual(await asked(page), [['runner', 'build-box']])
  await page.getByRole('listbox').waitFor({ state: 'detached' })
  assert.equal(await focused(page), 'runner-picker')
})

await test('a view link and Start Work answer the keyboard, and dialogs give focus back', async page => {
  await page.focus('#show-plan')
  await page.keyboard.press('Enter')
  assert.equal(await page.getAttribute('#show-plan', 'aria-current'), 'page')
  assert.equal(await page.getAttribute('#show-board', 'aria-current'), null)
  assert.equal(await page.textContent('#page-title'), 'Plan')
  // Plan offers no Start Work; Board does.
  assert.equal(await page.locator('#show-start-work').count(), 0)
  await page.focus('#show-board')
  await page.keyboard.press('Enter')
  await page.focus('#show-start-work')
  await page.keyboard.press('Enter')
  await page.locator('#start-work[open]').waitFor()
  assert.ok(await page.evaluate(() => document.getElementById('start-work').contains(document.activeElement)), 'Start Work did not take focus')
  await page.keyboard.press('Escape')
  await page.locator('#start-work[open]').waitFor({ state: 'detached' })
  assert.equal(await focused(page), 'show-start-work')
  await page.focus('#show-commands')
  await page.keyboard.press('Enter')
  await page.locator('#workspace-commands[open]').waitFor()
  assert.equal(await focused(page), 'button:Split Pane')
  await page.keyboard.press('Escape')
  assert.equal(await focused(page), 'show-commands')
  assert.deepEqual(await asked(page), [['tab', 'plan'], ['tab', 'board'], ['start work'], ['tool', 'commands']])
})

const table = {}
for (const scheme of ['light', 'dark']) {
  for (const width of [320, 768, 1280]) {
    await test(`${width} ${scheme}: no sideways scroll, one filled button, AA contrast`, async page => {
      const wide = await page.evaluate(() => document.scrollingElement.scrollWidth)
      assert.ok(wide <= width, `the page is ${wide} px wide`)
      const filled = await filledButtons(page)
      assert.deepEqual(filled, ['#show-start-work Start Work'])
      if (width < 500) {
        const short = await page.$$eval('.view-link, .control', els => els.filter(el => el.getClientRects().length && el.getBoundingClientRect().height < 44).map(el => el.id || el.textContent))
        assert.deepEqual(short, [], 'phone targets under 44 px')
        // The compact header leaves the page near the top, as ov-1483's
        // phone board does: brand, workspace and the views, nothing more.
        assert.equal(await page.getAttribute('.page-head', 'data-page'), 'board')
        const top = await page.evaluate(() => document.querySelector('.page-head').getBoundingClientRect().top)
        assert.ok(top <= 260, `the board's header starts ${top} px down`)
      }
      if (width === 1280) {
        for (const [what, value, minimum] of await contrastPairs(page)) {
          table[what] = { ...table[what], [scheme]: value }
          assert.ok(value >= minimum, `${what} is ${value}:1 in ${scheme}, under ${minimum}:1`)
        }
      }
      if (out) await page.screenshot({ path: join(out, `shell-board-${scheme}-${width}.png`) })
      // Every other page, under the same names: Start Work only where work
      // starts (Terminals), nothing filled on Needs You and Plan.
      for (const [tab, expected] of [['terminals', ['#show-start-work Start Work']], ['needs', []], ['plan', []]]) {
        await page.click(`#show-${tab}`)
        await page.waitForFunction(tab => document.querySelector(`#show-${tab}`).getAttribute('aria-current') === 'page', tab)
        assert.deepEqual(await filledButtons(page), expected, `filled buttons on ${tab}`)
        assert.ok(await page.evaluate(() => document.scrollingElement.scrollWidth) <= width, `${tab} scrolls sideways`)
        if (out) await page.screenshot({ path: join(out, `shell-${tab}-${scheme}-${width}.png`) })
      }
    }, { width, scheme })
  }
}
console.log('\n| Pair | Light | Dark |\n|---|---:|---:|')
for (const [what, { light, dark }] of Object.entries(table)) console.log(`| ${what} | ${light}:1 | ${dark}:1 |`)

await browser.close()
if (failures.length) {
  console.error(`\nshell-check: ${failures.length} failed`)
  process.exit(1)
}
console.log('\nshell-check: all passed')
