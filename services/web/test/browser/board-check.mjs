// Run from services/web with WORK_VIEW_BIN set. Optionally pass a screenshot directory.
import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import { mkdirSync, readFileSync } from 'node:fs'
import { join } from 'node:path'
import { build } from 'esbuild'
import { chromium } from 'playwright-core'
import { harnessHtml, styles } from '../../scripts/styles.mjs'
import { filledButtons } from './shell-audit.mjs'

const origin = 'https://board.test'
const destination = origin + '/?runner=canary&workspace=delivery&view=board'
const output = process.argv[2]
if (output) mkdirSync(output, { recursive: true })
const html = harnessHtml()
  .replaceAll('{{app.css}}', '/app.css').replaceAll('{{app.js}}', '/board.js')
const css = styles().toString()
const bundle = await build({ entryPoints: ['test/browser/board-harness.ts'], bundle: true, write: false, format: 'esm' })
const project = (path, args) => execFileSync(process.env.WORK_VIEW_BIN, [path], {
  input: JSON.stringify(args), encoding: 'utf8',
})
const board = {
  reads: { floor_ms: 0, opened: [] },
  tasks: [
    { key: 'ov-1439', status: 'done', title: 'Mac-to-web parity inventory' },
    { key: 'ov-1444', status: 'done', title: 'Web: start and recover work on a paired runner' },
    { key: 'ov-1445', status: 'in_progress', title: 'Web: permanent orchestrator and workstation navigation' },
    { key: 'ov-1447', status: 'in_review', title: 'Local mode on Linux, then Windows via WSL2' },
    { key: 'ov-1440', status: 'todo', title: 'Make the web and Linux checks required for the web-first pivot' },
    { key: 'ov-1446', status: 'backlog', title: 'Web: board, tickets and review without the Mac app',
      intent: 'Web-first pivot, step 3 of the order in the Mac-to-web parity inventory. Take a ticket from creation through review and landing without leaving the browser.',
      acceptance: [
        { text: 'The web client has the workspace board: columns, ticket create and edit, start, shared unread state, and links from a ticket to its agent and worktree.', met: true },
        { text: 'Files and Changes: browse a worktree’s files and diffs, comment inline, approve, and return to the same review position after a reload. Anchors and reviewed-state rules live in Rust.', met: false },
        { text: 'Plan sits beside the orchestrator, and Engine shows stations with ticket drill-down, using the existing workflow actions.', met: false },
        { text: 'A scripted run against a fake runner takes one ticket from create to landed using only web actions.', met: false },
        { text: 'Large boards stay within the response size limit by paging in the core.', met: true },
      ],
      constraints: [
        'Feature logic (state, rules, labels, allowed actions) lives in the Rust core or crates/client and reaches the page through WASM or the wire.',
        'Keep host pinning, per-device grants, read/control permissions and passkey re-authentication as they are.',
      ],
      labels: ['Web first'],
    },
  ].map((task, i) => ({ ...task, id: task.key, updated_at: i + 1, resource_version: 1 })),
}
const agent = '00000000-0000-0000-0000-00000000000a'
const tree = '00000000-0000-0000-0000-00000000000b'
const commands = []
let time = 10

function answer(args, scope) {
  if (args.method === 'layout') return project('/layout', args.request)
  if (args.method === 'work') {
    const { workspace, repo } = args
    return project('/projection', [{
      scope,
      workspaces: [{ id: workspace, name: 'Web first', repository: repo, isMain: true, orchestrator: 'seat' }],
      repositories: [{ id: repo, label: 'Far Cooler development' }],
      worktrees: [{ id: 'main', label: 'Main', repository: repo, workspace }],
      terminals: [{ id: 'seat', title: 'Orchestrator', worktree: 'main', workspace,
        agent: true, orchestrator: true, state: 'running', columns: 80, rows: 24 }],
    }, { workspace }])
  }
  if (args.method === 'form') return project('/ticket-form', {})
  if (args.method === 'board') {
    assert.equal(args.workspace, 'delivery')
    assert.equal(args.repository, 'repo')
    return project('/board', [board, scope])
  }
  const task = board.tasks.find(task => task.id === args.task)
  if (args.method === 'ticket') {
    assert.ok(task)
    const start = task.status === 'backlog' ? 'start-guard' : null
    return project('/ticket', [task, scope, start, task.status === 'in_progress' ? [agent, tree] : [null, null]])
  }
  assert.equal(scope, 'control')
  assert.equal(args.workspace, 'delivery')
  commands.push(args)
  switch (args.action) {
    case 'mark':
      if (task) {
        const found = board.reads.opened.find(mark => mark.task_id === task.id)
        if (found) found.opened_ms = Math.max(found.opened_ms, args.through)
        else board.reads.opened.push({ task_id: task.id, opened_ms: args.through })
      } else board.reads.floor_ms = Math.max(board.reads.floor_ms, args.through)
      return 'null'
    case 'create':
      board.tasks.push({ ...args.draft, id: 'new', key: 'ov-2000', status: 'backlog',
        acceptance: [{ text: args.draft.acceptance, met: false }], constraints: [], labels: [],
        updated_at: ++time, resource_version: 1 })
      return '"new"'
    case 'edit':
      if (args.version !== String(task.resource_version)) throw new Error('The ticket changed while editing.')
      Object.assign(task, { title: args.draft.title, intent: args.draft.intent, updated_at: ++time,
        resource_version: task.resource_version + 1 })
      return '"new"'
    case 'start':
      assert.equal(args.guard, 'start-guard')
      assert.equal(task.status, 'backlog')
      Object.assign(task, { status: 'in_progress', updated_at: ++time })
      return 'null'
    default: throw new Error('Unexpected call')
  }
}

const browser = await chromium.launch({ executablePath: process.env.CHROME_PATH })
async function page(scope, width = 1280, query = '') {
  const context = await browser.newContext({ viewport: { width, height: 850 }, isMobile: width < 500 })
  await context.route(origin + '/**', route => {
    switch (new URL(route.request().url()).pathname) {
      case '/app.css': return route.fulfill({ contentType: 'text/css', body: css })
      case '/board.js': return route.fulfill({ contentType: 'text/javascript', body: bundle.outputFiles[0].text })
      case '/board-rpc':
        try { return route.fulfill({ body: answer(JSON.parse(route.request().postData()), scope) }) }
        catch (error) { return route.fulfill({ status: 400, body: error.message }) }
      default: return route.fulfill({ contentType: 'text/html', body: html })
    }
  })
  const p = await context.newPage()
  p.setDefaultTimeout(5000)
  await p.goto(destination + query)
  return p
}

async function layoutPage(width, openSplit = true) {
  const tab = await page('control', width, '&layout')
  await tab.emulateMedia({ colorScheme: 'dark', reducedMotion: 'reduce' })
  await tab.locator('#board-title').waitFor({ state: 'attached' })
  if (openSplit) {
    await tab.locator('#show-orchestrator').click()
    await tab.getByRole('region', { name: 'Orchestrator', exact: true }).waitFor()
  }
  return tab
}

try {
  if (output) {
    for (const width of [320, 1280]) {
      const tab = await layoutPage(width, width === 1280)
      await tab.mouse.move(width - 1, 0)
      await tab.screenshot({ path: join(output, `sidebar-${width}.png`) })
      await tab.context().close()
    }
  }

  for (const width of [320, 641, 1280]) {
    const tab = await layoutPage(width)
    const tabs = tab.getByRole('navigation', { name: 'Views' }).getByRole('link')
    assert.deepEqual(await tabs.allTextContents(), ['Terminals', 'Needs You (123)', 'Board', 'Plan'])
    assert.deepEqual(await tabs.evaluateAll(buttons => buttons.filter(button => {
      const range = document.createRange()
      range.selectNodeContents(button)
      const label = range.getBoundingClientRect()
      const bounds = button.getBoundingClientRect()
      const sidebar = button.closest('.sidebar').getBoundingClientRect()
      return label.left < bounds.left || label.right > bounds.right ||
        label.top < bounds.top || label.bottom > bounds.bottom ||
        label.left < sidebar.left || label.right > sidebar.right || button.scrollWidth > button.clientWidth
    }).map(button => button.textContent)), [], `section labels fit at ${width}px`)
    assert.deepEqual(await filledButtons(tab), ['#show-start-work Start Work'], `filled actions at ${width}px`)
    await tab.context().close()
  }
  console.log('PASS full view labels and one filled action at 320, 641 and 1280px')

  for (const width of [1280, 1440]) {
    const tab = await layoutPage(width)
    const pane = tab.getByRole('region', { name: 'Orchestrator', exact: true })
    const columns = tab.getByRole('region', { name: 'Board', exact: true })
    const stages = columns.getByRole('heading', { level: 2 })
    assert.equal(await stages.count(), JSON.parse(project('/board', [board, 'control'])).columns.length)
    assert.ok(await columns.evaluate(node => node.scrollWidth > node.clientWidth), `board scrolls sideways at ${width}px`)
    assert.equal(await stages.evaluateAll(headings => new Set(headings.map(h => h.getBoundingClientRect().top)).size), 1)
    await columns.focus()
    await tab.keyboard.press('ArrowRight')
    await tab.waitForFunction(() => document.querySelector('.board-columns').scrollLeft > 0)
    for (const stage of await stages.all()) {
      await stage.scrollIntoViewIfNeeded()
      assert.ok(await stage.evaluate(node => {
        const stage = node.getBoundingClientRect()
        const scroller = node.closest('.board-columns').getBoundingClientRect()
        return stage.left >= scroller.left - 1 && stage.right <= scroller.right + 1
      }), `stage is reachable at ${width}px: ${await stage.textContent()}`)
      assert.ok(await pane.isVisible())
    }
    const position = await columns.evaluate(node => node.scrollLeft)
    const ticket = board.tasks[0]
    const originalTitle = ticket.title
    ticket.title += ' (updated)'
    try {
      await tab.getByRole('button', { name: 'Refresh', exact: true }).click()
      await columns.getByText(ticket.title, { exact: true }).waitFor()
      assert.equal(await columns.evaluate(node => node.scrollLeft), position)
    } finally {
      ticket.title = originalTitle
    }
    assert.equal(await tab.evaluate(() => document.documentElement.scrollWidth), width)
    await tab.reload()
    await pane.waitFor()
    await tab.locator('#close-split').click()
    await tab.reload()
    await tab.locator('#board-title').waitFor({ state: 'attached' })
    assert.equal(await pane.isVisible(), false)
    await tab.context().close()
  }
  console.log('PASS all board stages reachable beside the orchestrator at 1280 and 1440px, with split state restored after reload')

  const editor = await page('control')
  const reader = await page('read')
  const action = name => editor.getByRole('button', { name, exact: true })
  await reader.getByText('6 unread', { exact: true }).waitFor()
  assert.equal(await reader.getByRole('button', { name: 'New Ticket' }).count(), 0)
  await action('New Ticket').click()
  const dialog = editor.getByRole('dialog')
  await dialog.getByLabel('Title').fill('Restore browser review position after a daemon reconnect')
  await dialog.getByLabel('Intent').fill('Keep the selected candidate, file and line when the browser reloads.')
  await dialog.getByLabel('Acceptance criteria').fill('Returning to an updated candidate resolves the anchor in Rust.')
  await dialog.getByRole('button', { name: 'Create Ticket' }).click()
  await editor.waitForURL('**ticket=new')
  await editor.getByRole('heading', { name: 'Restore browser review position after a daemon reconnect' }).waitFor()
  await action('Edit Ticket').click()
  await dialog.getByLabel('Title').fill('Retain exact browser review position after reconnect')
  await dialog.getByRole('button', { name: 'Save Ticket' }).click()
  await editor.getByRole('heading', { name: 'Retain exact browser review position after reconnect' }).waitFor()
  await action('Edit Ticket').click()
  await dialog.getByLabel('Title').fill('Concurrent edit draft')
  board.tasks.at(-1).resource_version++
  await dialog.getByRole('button', { name: 'Save Ticket' }).click()
  await dialog.getByRole('alert').waitFor()
  assert.equal(await dialog.getByLabel('Title').inputValue(), 'Concurrent edit draft')
  await dialog.getByRole('button', { name: 'Cancel' }).click()
  await action('Start Ticket').click()
  await editor.getByRole('link', { name: 'Open Agent' }).waitFor()
  for (const [name, param, id] of [['Open Agent', 'terminal', agent], ['Open Worktree', 'worktree', tree]]) {
    const link = editor.getByRole('link', { name })
    assert.equal(new URL(await link.getAttribute('href')).searchParams.get(param), id)
    await link.click()
    await editor.waitForURL(url => url.searchParams.get(param) === id)
    await editor.goBack()
    await link.waitFor()
  }
  const direct = await editor.context().newPage()
  await direct.goto(editor.url())
  await direct.getByRole('link', { name: 'Open Worktree' }).waitFor()
  await direct.close()
  await editor.reload()
  await editor.getByRole('link', { name: 'Open Worktree' }).waitFor()
  await editor.getByRole('link', { name: 'Back to Board' }).click()
  await editor.locator('#board-title').waitFor({ state: 'attached' })
  await editor.goBack()
  await editor.getByRole('link', { name: 'Open Agent' }).waitFor()
  await editor.getByRole('link', { name: 'Back to Board' }).click()
  await action('Mark Board Read').click()
  await reader.getByRole('button', { name: 'Refresh' }).click()
  await reader.getByText('0 unread', { exact: true }).waitFor()
  assert.equal(commands.filter(c => c.action === 'start').length, 1)
  assert.ok(commands.some(c => c.action === 'mark' && c.task === 'new'))
  console.log('PASS create, edit, refuse stale save, start, reload, history, linked destinations and shared unread')

  if (output) {
    board.reads.floor_ms = 3
    for (const width of [320, 1280]) {
      const capture = await page('control', width)
      const card = capture.getByRole('link', { name: /ov-1446/ })
      await card.waitFor()
      const fit = async () => {
        const height = await capture.locator('#board').evaluate(node => node.scrollHeight + node.getBoundingClientRect().top)
        await capture.setViewportSize({ width, height: Math.max(850, Math.ceil(height)) })
      }
      await fit()
      await capture.screenshot({ path: join(output, `board-${width}.png`) })
      await card.click()
      await capture.getByRole('heading', { name: 'Acceptance criteria' }).waitFor()
      await capture.setViewportSize({ width, height: 850 })
      await fit()
      assert.equal(await capture.evaluate(() => document.documentElement.scrollWidth), width)
      await capture.screenshot({ path: join(output, `ticket-${width}.png`) })
      await capture.context().close()
    }
  }
} finally {
  await browser.close()
}
