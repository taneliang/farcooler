// The work screen's Needs You half, for needs-capture.mjs: the page's own
// markup, stylesheet and view code, over the fixture's studio list instead of a
// tunnel. The tab switch and the counts mirror app.ts's drawNeeds and showTab.

import init, { needsYouView } from 'fc-web'
import { useNeedsYouPresenter } from '../../src/client/needs-present'

import fixture from '../../../../test/fixtures/needs-you.json'
import { Board, type BoardView, countLabel, countOf, pageTitle } from '../../src/client/needs'
import { renderNeeds } from '../../src/client/needs-view'
import { harnessShell, runnerNamed } from './harness-shell'
import { shell } from '../../src/client/shell/store'

declare const __NEEDS_WASM__: string
await init({ module_or_path: __NEEDS_WASM__ })
useNeedsYouPresenter(needsYouView)

const $ = (id: string) => document.getElementById(id) as HTMLElement
const NOW = 1_790_000_000_000
const list = JSON.stringify((fixture.runners as any[])[0].needs_you)
let drawn = ''

const board = new Board({
  runner: 'studio',
  list: async () => list,
  ask: async () => {
    throw Object.assign(new Error('refused'), { code: 'scope-denied', what: '' })
  },
  decide: async () => {},
  open: () => {},
  show: (view: BoardView) => {
    const count = countOf(view.items)
    shell.command('nav', 'needs', { label: countLabel(count) })
    $('bar-count').hidden = count === 0
    $('bar-count').textContent = `${count} Waiting`
    document.title = pageTitle(count)
    drawn = renderNeeds($('needs'), view, NOW, { canOpen: () => true, press: (item, press) => void board.press(item, press) }, drawn)
  },
})

harnessShell({ runner: runnerNamed('studio'), page: 'needs' })
$('who').textContent = 'owner@example.com'
$('work').hidden = false
$('status').hidden = true
$('needs').hidden = false
$('pane').hidden = true
$('terminal-list').hidden = true
await board.refresh()
// The refusal a Read-scoped device gets, on the first ask.
const ask = (board as any).items[0]
await board.press(ask, { kind: 'ask', terminal: ask.terminal.id, ask: ask.askId, option: 'allow' })
;(window as any).done = true
