// The work screen's Needs You half, for needs-capture.mjs: the page's own
// markup, stylesheet and view code, over the fixture's studio list instead of a
// tunnel. The tab switch and the counts mirror app.ts's drawNeeds and showTab.

import fixture from '../../../../test/fixtures/needs-you.json'
import { Board, type BoardView, countLabel, countOf, pageTitle } from '../../src/client/needs'
import { renderNeeds } from '../../src/client/needs-view'

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
    $('show-needs').textContent = countLabel(count)
    $('bar-count').hidden = count === 0
    $('bar-count').textContent = `${count} Waiting`
    document.title = pageTitle(count)
    drawn = renderNeeds($('needs'), view, NOW, { canOpen: () => true, press: (item, press) => void board.press(item, press) }, drawn)
  },
})

$('runner-name').textContent = 'studio'
$('who').textContent = 'owner@example.com'
$('show-needs').addEventListener('click', () => {})
$('work').hidden = false
$('status').hidden = true
$('needs').hidden = false
$('pane').hidden = true
$('terminal-list').hidden = true
$('show-terminals').setAttribute('aria-pressed', 'false')
$('show-needs').setAttribute('aria-pressed', 'true')
await board.refresh()
// The refusal a Read-scoped device gets, on the first ask.
const ask = (board as any).items[0]
await board.press(ask, { kind: 'ask', terminal: ask.terminal.id, ask: ask.askId, option: 'allow' })
;(window as any).done = true
