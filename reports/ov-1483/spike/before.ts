// Compiled by verify.mjs against the checked-in client and the shared fixtures.
import { mountBoard } from 'legacy/board-screen'
import { CanvasAddon } from '@xterm/addon-canvas'
import { FitAddon } from '@xterm/addon-fit'
import { Terminal } from '@xterm/xterm'
import { RunnerSession, spaces, stages } from './runner'

const element = (id: string) => document.getElementById(id)!
const label = (id: string, text: string) => { element(id).textContent = text }
const session = new RunnerSession()
const stage = element('side-stage')
const term = new Terminal({ fontSize: 13, screenReaderMode: true })
const fit = new FitAddon()
term.loadAddon(new CanvasAddon())
term.loadAddon(fit)

document.documentElement.style.colorScheme = 'dark'
element('status').hidden = true
element('work').hidden = false
element('board').hidden = false
element('pane').hidden = true
element('workspace-side').hidden = false
element('show-start-work').hidden = false
element('workspace-link').hidden = false
element('show-board').removeAttribute('disabled')
element('show-board').setAttribute('aria-pressed', 'true')
element('show-terminals').setAttribute('aria-pressed', 'false')
label('runner-name', 'Cosmo')
label('show-board', 'Board')
label('show-orchestrator', 'Orchestrator')
label('show-commands', 'Commands')
label('show-start-work', 'Start Work')
label('workspace-link', 'Workspace Link')
label('side-title', 'orchestrator')
label('close-split', 'Close')
label('who', 'Canary')
const picker = element('space-picker')
picker.hidden = false
picker.replaceChildren(...spaces.map(({ id }) => new Option('Main', id)))
element('terminal-list').replaceChildren(...['Terminal 2', 'shell', 'Terminal 4', 'orchestrator', 'orchestrator', 'orchestrator',
  'Terminal 7', 'orchestrator', 'orchestrator', 'orchestrator', 'ov-788', 'ov-1021'].map((name, index) => {
  const link = document.createElement('a')
  link.className = 'terminal-choice'
  link.href = '#terminal-' + index
  link.textContent = name
  if (index === 9) link.setAttribute('aria-current', 'true')
  return link
}))

const runner = {
  board: async () => JSON.stringify({
    title: 'Board', empty: 'No tickets', refresh: 'Refresh',
    columns: stages.map(title => ({
      title, rows: session.read().boards.overnight.filter(card => card.stage === title).map(card => ({ ...card, id: card.key })),
    })),
  }),
  board_ticket: async () => '',
  board_intent: async () => '',
}
mountBoard({
  root: element('board'), dialog: element('board-form') as HTMLDialogElement, current: () => runner,
  create: () => '', failure: (error: Error) => error.message, href: () => new URL(location.href),
  openTicket() {}, openAgent() {}, openWorktree() {}, openActivity() {},
}).show(true, 'overnight', 'overnight')

term.open(stage)
session.attach(bytes => term.write(bytes))
new ResizeObserver(() => { if (stage.offsetWidth) fit.fit() }).observe(stage)
